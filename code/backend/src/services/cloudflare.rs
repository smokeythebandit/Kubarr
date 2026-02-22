//! Cloudflare Tunnel Service
//!
//! Supports two provisioning modes:
//!
//! **Guided wizard (new):** The user provides a Cloudflare API token; Kubarr
//! creates the tunnel, configures ingress, and creates a DNS CNAME record
//! automatically via the Cloudflare API.
//!
//! **Legacy mode:** The user pastes a pre-created tunnel token directly.

use std::collections::BTreeMap;
use std::process::Command;

use base64::Engine as _;
use chrono::Utc;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, PostParams};
use once_cell::sync::Lazy;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::models::cloudflare_tunnel;
use crate::models::prelude::*;
use crate::services::K8sClient;
use crate::state::DbConn;

// ============================================================================
// Constants
// ============================================================================

const CLOUDFLARED_NAMESPACE: &str = "cloudflared";
const CLOUDFLARED_SECRET_NAME: &str = "cloudflared-tunnel-token";
const CLOUDFLARED_RELEASE_NAME: &str = "cloudflared";
const CLOUDFLARED_CHART_PATH: &str = "/app/charts/cloudflared";
const CF_API_BASE: &str = "https://api.cloudflare.com/client/v4";

// Shared reqwest client for Cloudflare API requests
#[allow(clippy::expect_used)]
static CF_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to build Cloudflare HTTP client")
});

// ============================================================================
// Public Request / Response Types
// ============================================================================

/// Request to validate a Cloudflare API token (Step 1 of wizard)
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ValidateTokenRequest {
    pub api_token: String,
}

/// Zone information returned after token validation
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ZoneInfo {
    pub id: String,
    pub name: String,
}

/// Response from POST /validate-token
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ValidateTokenResponse {
    pub account_id: String,
    pub zones: Vec<ZoneInfo>,
}

/// Request to provision a full tunnel (Step 2 of wizard) — PUT /config
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ProvisionRequest {
    pub name: String,
    pub api_token: String,
    pub account_id: String,
    pub zone_id: String,
    pub zone_name: String,
    pub subdomain: String,
}

/// Public response for the tunnel configuration (secrets masked)
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CloudflareTunnelResponse {
    pub id: i64,
    pub name: String,
    /// Always "****" in API responses
    pub tunnel_token: String,
    pub status: String,
    pub error: Option<String>,
    pub tunnel_id: Option<String>,
    pub zone_id: Option<String>,
    pub zone_name: Option<String>,
    pub subdomain: Option<String>,
    pub hostname: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// Pod-level status returned by GET /status
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CloudflareTunnelStatus {
    pub status: String,
    pub ready_pods: i32,
    pub total_pods: i32,
    pub message: Option<String>,
}

// ============================================================================
// Cloudflare API response envelope
// ============================================================================

#[derive(Deserialize)]
struct CfResponse<T> {
    success: bool,
    errors: Vec<CfApiError>,
    result: Option<T>,
}

#[derive(Deserialize)]
struct CfApiError {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

impl<T> CfResponse<T> {
    fn into_result(self, context: &str) -> Result<T> {
        if !self.success {
            let msg = self
                .errors
                .first()
                .map(|e| e.message.as_str())
                .unwrap_or("unknown error");
            return Err(AppError::BadRequest(format!(
                "Cloudflare API error ({}): {}",
                context, msg
            )));
        }
        self.result.ok_or_else(|| {
            AppError::Internal(format!(
                "Cloudflare API returned no result for: {}",
                context
            ))
        })
    }
}

// ============================================================================
// Cloudflare API helpers
// ============================================================================

/// Verify that the given API token is valid
async fn cf_verify_token(token: &str) -> Result<()> {
    let resp = CF_HTTP_CLIENT
        .get(format!("{}/user/tokens/verify", CF_API_BASE))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    cf.into_result("verify token")?;
    Ok(())
}

/// Return the first account ID accessible with the token
async fn cf_get_account(token: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Account {
        id: String,
    }

    let resp = CF_HTTP_CLIENT
        .get(format!("{}/accounts", CF_API_BASE))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<Vec<Account>> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    let accounts = cf.into_result("list accounts")?;
    accounts.into_iter().next().map(|a| a.id).ok_or_else(|| {
        AppError::BadRequest("No Cloudflare accounts found for this token".to_string())
    })
}

/// List active zones in the account
async fn cf_list_zones(token: &str, account_id: &str) -> Result<Vec<ZoneInfo>> {
    #[derive(Deserialize)]
    struct Zone {
        id: String,
        name: String,
    }

    let resp = CF_HTTP_CLIENT
        .get(format!("{}/zones", CF_API_BASE))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[
            ("account.id", account_id),
            ("status", "active"),
            ("per_page", "50"),
        ])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<Vec<Zone>> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    let zones = cf.into_result("list zones")?;
    Ok(zones
        .into_iter()
        .map(|z| ZoneInfo {
            id: z.id,
            name: z.name,
        })
        .collect())
}

/// Create a new Cloudflare Tunnel and return `(tunnel_id, tunnel_name)`
async fn cf_create_tunnel(token: &str, account_id: &str, name: &str) -> Result<(String, String)> {
    #[derive(Deserialize)]
    struct TunnelResult {
        id: String,
        name: String,
    }

    // Generate a random 32-byte secret encoded as Base64
    let raw: [u8; 32] = rand::random();
    let tunnel_secret = base64::engine::general_purpose::STANDARD.encode(raw);

    let resp = CF_HTTP_CLIENT
        .post(format!(
            "{}/accounts/{}/cfd_tunnel",
            CF_API_BASE, account_id
        ))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "name": name,
            "tunnel_secret": tunnel_secret,
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<TunnelResult> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    let result = cf.into_result("create tunnel")?;
    Ok((result.id, result.name))
}

/// Retrieve the token for an existing tunnel
async fn cf_get_tunnel_token(token: &str, account_id: &str, tunnel_id: &str) -> Result<String> {
    let resp = CF_HTTP_CLIENT
        .get(format!(
            "{}/accounts/{}/cfd_tunnel/{}/token",
            CF_API_BASE, account_id, tunnel_id
        ))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<String> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    cf.into_result("get tunnel token")
}

/// Configure cloudflared ingress rules for the tunnel
async fn cf_configure_ingress(
    token: &str,
    account_id: &str,
    tunnel_id: &str,
    hostname: &str,
) -> Result<()> {
    let resp = CF_HTTP_CLIENT
        .put(format!(
            "{}/accounts/{}/cfd_tunnel/{}/configurations",
            CF_API_BASE, account_id, tunnel_id
        ))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "config": {
                "ingress": [
                    {
                        "hostname": hostname,
                        "service": "http://kubarr-backend.kubarr.svc.cluster.local:8000"
                    },
                    { "service": "http_status:404" }
                ]
            }
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    cf.into_result("configure ingress")?;
    Ok(())
}

/// Create a proxied CNAME DNS record pointing to the tunnel and return `dns_record_id`
async fn cf_create_dns_record(
    token: &str,
    zone_id: &str,
    subdomain: &str,
    tunnel_id: &str,
) -> Result<String> {
    #[derive(Deserialize)]
    struct DnsRecord {
        id: String,
    }

    let resp = CF_HTTP_CLIENT
        .post(format!("{}/zones/{}/dns_records", CF_API_BASE, zone_id))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "type": "CNAME",
            "name": subdomain,
            "content": format!("{}.cfargotunnel.com", tunnel_id),
            "proxied": true,
            "ttl": 1
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<DnsRecord> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    cf.into_result("create DNS record").map(|r| r.id)
}

/// Delete a Cloudflare Tunnel (force=true allows deleting active tunnels)
async fn cf_delete_tunnel(token: &str, account_id: &str, tunnel_id: &str) -> Result<()> {
    let resp = CF_HTTP_CLIENT
        .delete(format!(
            "{}/accounts/{}/cfd_tunnel/{}",
            CF_API_BASE, account_id, tunnel_id
        ))
        .header("Authorization", format!("Bearer {}", token))
        .query(&[("force", "true")])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    cf.into_result("delete tunnel")?;
    Ok(())
}

/// Delete a DNS record
async fn cf_delete_dns_record(token: &str, zone_id: &str, record_id: &str) -> Result<()> {
    let resp = CF_HTTP_CLIENT
        .delete(format!(
            "{}/zones/{}/dns_records/{}",
            CF_API_BASE, zone_id, record_id
        ))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CF API request failed: {}", e)))?;

    let cf: CfResponse<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("CF API parse failed: {}", e)))?;

    cf.into_result("delete DNS record")?;
    Ok(())
}

// ============================================================================
// Public Service Functions
// ============================================================================

/// Validate an API token and return the list of accessible zones (wizard Step 1)
pub async fn validate_and_list_zones(token: &str) -> Result<ValidateTokenResponse> {
    cf_verify_token(token).await?;
    let account_id = cf_get_account(token).await?;
    let zones = cf_list_zones(token, &account_id).await?;
    Ok(ValidateTokenResponse { account_id, zones })
}

/// Get the current tunnel configuration from the DB (secrets masked)
pub async fn get_config(db: &DbConn) -> Result<Option<CloudflareTunnelResponse>> {
    let tunnel = CloudflareTunnel::find().one(db).await?;
    Ok(tunnel.map(to_response))
}

/// Provision a Cloudflare Tunnel via the guided wizard (wizard Step 2)
///
/// 1. Upserts a DB row (status=deploying)
/// 2. Creates tunnel + retrieves token via CF API
/// 3. Configures ingress rules + creates DNS CNAME
/// 4. Updates DB with all CF identifiers
/// 5. Creates K8s Secret
/// 6. Spawns background task to run `helm upgrade --install`
/// 7. Returns immediately with status=deploying
pub async fn save_config(
    db: &DbConn,
    k8s: &K8sClient,
    req: ProvisionRequest,
) -> Result<CloudflareTunnelResponse> {
    let now = Utc::now();

    // ── 1. Upsert DB row ─────────────────────────────────────────────────────
    let existing = CloudflareTunnel::find().one(db).await?;
    let tunnel = if let Some(existing) = existing {
        let mut active: cloudflare_tunnel::ActiveModel = existing.into();
        active.name = Set(req.name.clone());
        active.status = Set("deploying".to_string());
        active.error = Set(None);
        active.api_token = Set(Some(req.api_token.clone()));
        active.account_id = Set(Some(req.account_id.clone()));
        active.zone_id = Set(Some(req.zone_id.clone()));
        active.zone_name = Set(Some(req.zone_name.clone()));
        active.subdomain = Set(Some(req.subdomain.clone()));
        // Clear stale CF identifiers until the new ones are created
        active.tunnel_id = Set(None);
        active.dns_record_id = Set(None);
        active.hostname = Set(None);
        active.updated_at = Set(now);
        active.update(db).await?
    } else {
        let new_tunnel = cloudflare_tunnel::ActiveModel {
            name: Set(req.name.clone()),
            tunnel_token: Set(String::new()), // filled after CF API call
            status: Set("deploying".to_string()),
            error: Set(None),
            api_token: Set(Some(req.api_token.clone())),
            account_id: Set(Some(req.account_id.clone())),
            zone_id: Set(Some(req.zone_id.clone())),
            zone_name: Set(Some(req.zone_name.clone())),
            subdomain: Set(Some(req.subdomain.clone())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        new_tunnel.insert(db).await?
    };

    // ── 2–6. Cloudflare API provisioning ────────────────────────────────────
    let provisioning = async {
        let (tunnel_id, _) = cf_create_tunnel(&req.api_token, &req.account_id, &req.name).await?;
        let tunnel_token = cf_get_tunnel_token(&req.api_token, &req.account_id, &tunnel_id).await?;
        let hostname = format!("{}.{}", req.subdomain, req.zone_name);
        cf_configure_ingress(&req.api_token, &req.account_id, &tunnel_id, &hostname).await?;
        let dns_record_id =
            cf_create_dns_record(&req.api_token, &req.zone_id, &req.subdomain, &tunnel_id).await?;
        Ok::<_, AppError>((tunnel_id, tunnel_token, hostname, dns_record_id))
    }
    .await;

    let (cf_tunnel_id, tunnel_token, hostname, dns_record_id) = match provisioning {
        Ok(vals) => vals,
        Err(e) => {
            // Mark row as failed and bubble up the error
            let mut active: cloudflare_tunnel::ActiveModel = tunnel.into();
            active.status = Set("failed".to_string());
            active.error = Set(Some(e.to_string()));
            active.updated_at = Set(Utc::now());
            let _ = active.update(db).await;
            return Err(e);
        }
    };

    // ── 7. Persist CF identifiers ────────────────────────────────────────────
    let mut active: cloudflare_tunnel::ActiveModel = tunnel.into();
    active.tunnel_token = Set(tunnel_token.clone());
    active.tunnel_id = Set(Some(cf_tunnel_id));
    active.hostname = Set(Some(hostname));
    active.dns_record_id = Set(Some(dns_record_id));
    active.updated_at = Set(Utc::now());
    let tunnel = active.update(db).await?;

    // ── 8. K8s Secret ────────────────────────────────────────────────────────
    create_tunnel_secret(k8s, &tunnel_token).await?;

    // ── 9. Background helm deploy ────────────────────────────────────────────
    let db_bg = db.clone();
    let tunnel_db_id = tunnel.id;
    tokio::spawn(async move {
        let set_arg = format!("tunnelToken.existingSecret={}", CLOUDFLARED_SECRET_NAME);
        let helm_result = tokio::task::spawn_blocking(move || {
            run_helm_command(&[
                "upgrade",
                "--install",
                CLOUDFLARED_RELEASE_NAME,
                CLOUDFLARED_CHART_PATH,
                "-n",
                CLOUDFLARED_NAMESPACE,
                "--create-namespace",
                "--set",
                &set_arg,
                "--wait",
                "--timeout",
                "3m",
            ])
        })
        .await;

        let (status, error_msg) = match helm_result {
            Ok(Ok(_)) => ("running".to_string(), None),
            Ok(Err(e)) => ("failed".to_string(), Some(e.to_string())),
            Err(e) => (
                "failed".to_string(),
                Some(format!("helm task panicked: {}", e)),
            ),
        };

        match CloudflareTunnel::find_by_id(tunnel_db_id).one(&db_bg).await {
            Ok(Some(t)) => {
                let mut active: cloudflare_tunnel::ActiveModel = t.into();
                active.status = Set(status);
                active.error = Set(error_msg);
                active.updated_at = Set(Utc::now());
                if let Err(e) = active.update(&db_bg).await {
                    tracing::error!("Failed to update tunnel status after helm deploy: {}", e);
                }
            }
            Ok(None) => {
                tracing::warn!(
                    "Tunnel {} no longer exists after helm deploy — skipping status update",
                    tunnel_db_id
                );
            }
            Err(e) => {
                tracing::error!(
                    "DB error querying tunnel {} after helm deploy: {}",
                    tunnel_db_id,
                    e
                );
            }
        }
    });

    // ── 10. Return config (status = deploying) ───────────────────────────────
    Ok(to_response(tunnel))
}

/// Uninstall cloudflared, clean up Cloudflare API resources, and delete the DB row
pub async fn delete_config(db: &DbConn, k8s: &K8sClient) -> Result<()> {
    let tunnel = CloudflareTunnel::find()
        .one(db)
        .await?
        .ok_or_else(|| AppError::NotFound("No Cloudflare tunnel configured".to_string()))?;

    // Mark as removing
    {
        let mut active: cloudflare_tunnel::ActiveModel = tunnel.clone().into();
        active.status = Set("removing".to_string());
        active.updated_at = Set(Utc::now());
        active.update(db).await?;
    }

    // ── Cloudflare API cleanup ────────────────────────────────────────────────
    if let (Some(api_token), Some(account_id), Some(zone_id)) = (
        tunnel.api_token.as_deref(),
        tunnel.account_id.as_deref(),
        tunnel.zone_id.as_deref(),
    ) {
        if let Some(record_id) = tunnel.dns_record_id.as_deref() {
            if let Err(e) = cf_delete_dns_record(api_token, zone_id, record_id).await {
                tracing::warn!("Failed to delete Cloudflare DNS record: {}", e);
            }
        }
        if let Some(cf_tunnel_id) = tunnel.tunnel_id.as_deref() {
            if let Err(e) = cf_delete_tunnel(api_token, account_id, cf_tunnel_id).await {
                tracing::warn!("Failed to delete Cloudflare tunnel: {}", e);
            }
        }
    }

    // ── Helm uninstall ────────────────────────────────────────────────────────
    if let Err(e) = run_helm_command(&[
        "uninstall",
        CLOUDFLARED_RELEASE_NAME,
        "-n",
        CLOUDFLARED_NAMESPACE,
    ]) {
        tracing::warn!("helm uninstall cloudflared failed (may be OK): {}", e);
    }

    // ── K8s Secret cleanup ────────────────────────────────────────────────────
    if let Err(e) = delete_tunnel_secret(k8s).await {
        tracing::warn!("Failed to delete cloudflared secret: {}", e);
    }

    // ── Delete DB row ─────────────────────────────────────────────────────────
    CloudflareTunnel::delete_by_id(tunnel.id).exec(db).await?;

    Ok(())
}

/// Return real-time pod status from K8s
pub async fn get_status(k8s: &K8sClient) -> Result<CloudflareTunnelStatus> {
    let pods = match k8s.list_pods(CLOUDFLARED_NAMESPACE, None).await {
        Ok(p) => p,
        Err(_) => {
            return Ok(CloudflareTunnelStatus {
                status: "not_deployed".to_string(),
                ready_pods: 0,
                total_pods: 0,
                message: None,
            });
        }
    };

    let total = pods.len() as i32;
    let ready = pods.iter().filter(|p| is_pod_ready(p)).count() as i32;

    Ok(status_from_counts(ready, total))
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Create (or replace) the cloudflared tunnel-token Secret in K8s
async fn create_tunnel_secret(k8s: &K8sClient, token: &str) -> Result<()> {
    let mut data: BTreeMap<String, k8s_openapi::ByteString> = BTreeMap::new();
    data.insert(
        "token".to_string(),
        k8s_openapi::ByteString(token.as_bytes().to_vec()),
    );

    ensure_namespace(k8s, CLOUDFLARED_NAMESPACE).await?;

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(CLOUDFLARED_SECRET_NAME.to_string()),
            namespace: Some(CLOUDFLARED_NAMESPACE.to_string()),
            labels: Some(BTreeMap::from([(
                "app.kubernetes.io/managed-by".to_string(),
                "kubarr".to_string(),
            )])),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    };

    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), CLOUDFLARED_NAMESPACE);
    let _ = secrets
        .delete(CLOUDFLARED_SECRET_NAME, &DeleteParams::default())
        .await;
    secrets.create(&PostParams::default(), &secret).await?;

    Ok(())
}

/// Delete the cloudflared tunnel-token Secret from K8s
async fn delete_tunnel_secret(k8s: &K8sClient) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), CLOUDFLARED_NAMESPACE);

    match secrets
        .delete(CLOUDFLARED_SECRET_NAME, &DeleteParams::default())
        .await
    {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(()),
        Err(e) => Err(AppError::Internal(format!(
            "Failed to delete cloudflared secret: {}",
            e
        ))),
    }
}

/// Ensure a K8s namespace exists (create if absent)
async fn ensure_namespace(k8s: &K8sClient, namespace: &str) -> Result<()> {
    use k8s_openapi::api::core::v1::Namespace as K8sNamespace;

    let namespaces: Api<K8sNamespace> = Api::all(k8s.client().clone());

    if namespaces.get(namespace).await.is_ok() {
        return Ok(());
    }

    let ns = K8sNamespace {
        metadata: ObjectMeta {
            name: Some(namespace.to_string()),
            labels: Some(BTreeMap::from([(
                "app.kubernetes.io/managed-by".to_string(),
                "kubarr".to_string(),
            )])),
            ..Default::default()
        },
        ..Default::default()
    };

    namespaces.create(&PostParams::default(), &ns).await?;
    Ok(())
}

/// Run a synchronous Helm command
fn run_helm_command(args: &[&str]) -> Result<String> {
    tracing::info!("Running helm {}", args.join(" "));

    let output = Command::new("helm")
        .args(args)
        .output()
        .map_err(|e| AppError::Internal(format!("Failed to execute helm: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Internal(format!(
            "Helm command failed: {}",
            stderr
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Infer `CloudflareTunnelStatus` from pod counts
fn status_from_counts(ready: i32, total: i32) -> CloudflareTunnelStatus {
    let status = if total == 0 {
        "not_deployed"
    } else if ready == total {
        "running"
    } else {
        "deploying"
    };

    CloudflareTunnelStatus {
        status: status.to_string(),
        ready_pods: ready,
        total_pods: total,
        message: None,
    }
}

/// Check whether a pod has a Ready=True condition
fn is_pod_ready(pod: &k8s_openapi::api::core::v1::Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .map(|c| c.status == "True")
        .unwrap_or(false)
}

/// Convert a DB model to a masked API response
fn to_response(t: cloudflare_tunnel::Model) -> CloudflareTunnelResponse {
    CloudflareTunnelResponse {
        id: t.id,
        name: t.name,
        tunnel_token: "****".to_string(),
        status: t.status,
        error: t.error,
        tunnel_id: t.tunnel_id,
        zone_id: t.zone_id,
        zone_name: t.zone_name,
        subdomain: t.subdomain,
        hostname: t.hostname,
        created_at: t.created_at,
        updated_at: t.updated_at,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // ------------------------------------------------------------------
    // CfResponse::into_result — pure method, no HTTP needed
    // ------------------------------------------------------------------

    #[test]
    fn cf_response_into_result_success_with_value() {
        let resp: CfResponse<String> = CfResponse {
            success: true,
            errors: vec![],
            result: Some("hello".to_string()),
        };
        let result = resp.into_result("test context");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn cf_response_into_result_failure_with_error_message() {
        let resp: CfResponse<String> = CfResponse {
            success: false,
            errors: vec![CfApiError {
                code: 1003,
                message: "Invalid token".to_string(),
            }],
            result: None,
        };
        let result = resp.into_result("verify token");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("Invalid token"),
            "expected error message in: {}",
            msg
        );
    }

    #[test]
    fn cf_response_into_result_failure_no_errors_uses_unknown() {
        let resp: CfResponse<String> = CfResponse {
            success: false,
            errors: vec![],
            result: None,
        };
        let result = resp.into_result("list accounts");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("unknown error"),
            "expected 'unknown error' in: {}",
            msg
        );
    }

    #[test]
    fn cf_response_into_result_success_but_no_result_returns_err() {
        let resp: CfResponse<String> = CfResponse {
            success: true,
            errors: vec![],
            result: None,
        };
        let result = resp.into_result("create tunnel");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("no result"),
            "expected 'no result' in: {}",
            msg
        );
    }

    fn make_model(status: &str, error: Option<&str>) -> cloudflare_tunnel::Model {
        cloudflare_tunnel::Model {
            id: 1,
            name: "test".to_string(),
            tunnel_token: "super-secret".to_string(),
            status: status.to_string(),
            error: error.map(|s| s.to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            api_token: Some("api-tok".to_string()),
            account_id: Some("acc-123".to_string()),
            tunnel_id: Some("tid-456".to_string()),
            zone_id: Some("zid-789".to_string()),
            zone_name: Some("example.com".to_string()),
            subdomain: Some("kubarr".to_string()),
            dns_record_id: Some("rec-abc".to_string()),
            hostname: Some("kubarr.example.com".to_string()),
        }
    }

    #[test]
    fn to_response_masks_secrets() {
        let model = make_model("running", None);
        let resp = to_response(model);
        assert_eq!(resp.tunnel_token, "****");
        // api_token is not in the response struct at all
    }

    #[test]
    fn to_response_exposes_new_fields() {
        let model = make_model("running", None);
        let resp = to_response(model);
        assert_eq!(resp.tunnel_id.as_deref(), Some("tid-456"));
        assert_eq!(resp.zone_name.as_deref(), Some("example.com"));
        assert_eq!(resp.subdomain.as_deref(), Some("kubarr"));
        assert_eq!(resp.hostname.as_deref(), Some("kubarr.example.com"));
    }

    #[test]
    fn to_response_preserves_fields() {
        let model = make_model("failed", Some("helm error"));
        let resp = to_response(model);
        assert_eq!(resp.id, 1);
        assert_eq!(resp.name, "test");
        assert_eq!(resp.status, "failed");
        assert_eq!(resp.error.as_deref(), Some("helm error"));
    }

    // ------------------------------------------------------------------
    // status_from_counts
    // ------------------------------------------------------------------

    #[test]
    fn status_no_pods_is_not_deployed() {
        let s = status_from_counts(0, 0);
        assert_eq!(s.status, "not_deployed");
    }

    #[test]
    fn status_all_ready_is_running() {
        let s = status_from_counts(2, 2);
        assert_eq!(s.status, "running");
    }

    #[test]
    fn status_partial_ready_is_deploying() {
        let s = status_from_counts(1, 2);
        assert_eq!(s.status, "deploying");
    }

    #[test]
    fn status_zero_ready_with_pods_is_deploying() {
        let s = status_from_counts(0, 2);
        assert_eq!(s.status, "deploying");
    }

    // ------------------------------------------------------------------
    // is_pod_ready
    // ------------------------------------------------------------------

    #[test]
    fn pod_with_no_status_is_not_ready() {
        let pod = k8s_openapi::api::core::v1::Pod::default();
        assert!(!is_pod_ready(&pod));
    }

    #[test]
    fn pod_with_ready_true_is_ready() {
        use k8s_openapi::api::core::v1::{Pod, PodCondition, PodStatus};
        let pod = Pod {
            status: Some(PodStatus {
                conditions: Some(vec![PodCondition {
                    type_: "Ready".to_string(),
                    status: "True".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(is_pod_ready(&pod));
    }

    #[test]
    fn pod_with_ready_false_is_not_ready() {
        use k8s_openapi::api::core::v1::{Pod, PodCondition, PodStatus};
        let pod = Pod {
            status: Some(PodStatus {
                conditions: Some(vec![PodCondition {
                    type_: "Ready".to_string(),
                    status: "False".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!is_pod_ready(&pod));
    }
}

#[cfg(test)]
mod tests_serde {
    use super::*;
    use chrono::Utc;

    #[test]
    fn validate_token_request_deser() {
        let r: ValidateTokenRequest =
            serde_json::from_str(r#"{"api_token":"cf_abc123"}"#).expect("deser");
        assert_eq!(r.api_token, "cf_abc123");
    }

    #[test]
    fn zone_info_ser() {
        let z = ZoneInfo {
            id: "zone1".to_string(),
            name: "example.com".to_string(),
        };
        let json = serde_json::to_string(&z).expect("ser");
        assert!(json.contains("\"name\":\"example.com\""));
    }

    #[test]
    fn validate_token_response_ser() {
        let r = ValidateTokenResponse {
            account_id: "acc123".to_string(),
            zones: vec![ZoneInfo {
                id: "z1".to_string(),
                name: "example.com".to_string(),
            }],
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"account_id\":\"acc123\""));
        assert!(json.contains("example.com"));
    }

    #[test]
    fn provision_request_deser() {
        let json = r#"{"name":"my-tunnel","api_token":"tok","account_id":"acc","zone_id":"zid","zone_name":"example.com","subdomain":"kubarr"}"#;
        let r: ProvisionRequest = serde_json::from_str(json).expect("deser");
        assert_eq!(r.name, "my-tunnel");
        assert_eq!(r.subdomain, "kubarr");
    }

    #[test]
    fn cloudflare_tunnel_response_ser_minimal() {
        let r = CloudflareTunnelResponse {
            id: 1,
            name: "my-tunnel".to_string(),
            tunnel_token: "****".to_string(),
            status: "running".to_string(),
            error: None,
            tunnel_id: None,
            zone_id: None,
            zone_name: None,
            subdomain: None,
            hostname: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"status\":\"running\""));
        assert!(json.contains("\"tunnel_token\":\"****\""));
    }

    #[test]
    fn cloudflare_tunnel_response_ser_full() {
        let r = CloudflareTunnelResponse {
            id: 2,
            name: "full-tunnel".to_string(),
            tunnel_token: "****".to_string(),
            status: "running".to_string(),
            error: None,
            tunnel_id: Some("tid-123".to_string()),
            zone_id: Some("zid-456".to_string()),
            zone_name: Some("example.com".to_string()),
            subdomain: Some("kubarr".to_string()),
            hostname: Some("kubarr.example.com".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"hostname\":\"kubarr.example.com\""));
        assert!(json.contains("\"zone_name\":\"example.com\""));
    }

    #[test]
    fn cloudflare_tunnel_status_ser() {
        let s = CloudflareTunnelStatus {
            status: "deploying".to_string(),
            ready_pods: 0,
            total_pods: 1,
            message: Some("Waiting for pod to start".to_string()),
        };
        let json = serde_json::to_string(&s).expect("ser");
        assert!(json.contains("\"status\":\"deploying\""));
        assert!(json.contains("\"total_pods\":1"));
    }

    #[test]
    fn cloudflare_tunnel_status_no_message_ser() {
        let s = CloudflareTunnelStatus {
            status: "not_deployed".to_string(),
            ready_pods: 0,
            total_pods: 0,
            message: None,
        };
        let json = serde_json::to_string(&s).expect("ser");
        assert!(json.contains("\"status\":\"not_deployed\""));
    }

    // ------------------------------------------------------------------
    // CfResponse::into_result
    // ------------------------------------------------------------------

    #[test]
    fn cf_response_into_result_success_returns_value() {
        let cf: CfResponse<String> =
            serde_json::from_str(r#"{"success":true,"errors":[],"result":"hello"}"#).unwrap();
        assert_eq!(cf.into_result("test_ctx").unwrap(), "hello");
    }

    #[test]
    fn cf_response_into_result_error_with_message() {
        let cf: CfResponse<String> = serde_json::from_str(
            r#"{"success":false,"errors":[{"code":1003,"message":"bad token"}],"result":null}"#,
        )
        .unwrap();
        let err = cf.into_result("verify token").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bad token"), "unexpected: {}", msg);
        assert!(msg.contains("verify token"), "unexpected: {}", msg);
    }

    #[test]
    fn cf_response_into_result_error_empty_errors_uses_unknown() {
        let cf: CfResponse<String> =
            serde_json::from_str(r#"{"success":false,"errors":[],"result":null}"#).unwrap();
        let err = cf.into_result("ctx").unwrap_err();
        assert!(
            err.to_string().contains("unknown error"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn cf_response_into_result_success_but_null_result() {
        let cf: CfResponse<String> =
            serde_json::from_str(r#"{"success":true,"errors":[],"result":null}"#).unwrap();
        let err = cf.into_result("list zones").unwrap_err();
        assert!(
            err.to_string().contains("no result") || err.to_string().contains("list zones"),
            "unexpected: {}",
            err
        );
    }
}

// ============================================================================
// DB-based tests
// ============================================================================

#[cfg(test)]
mod tests_db {
    use super::*;

    async fn make_db() -> DbConn {
        crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory SQLite")
    }

    async fn insert_tunnel(db: &DbConn, name: &str, status: &str) -> cloudflare_tunnel::Model {
        use sea_orm::Set;
        cloudflare_tunnel::ActiveModel {
            name: Set(name.to_string()),
            tunnel_token: Set("secret-token".to_string()),
            status: Set(status.to_string()),
            error: Set(None),
            api_token: Set(None),
            account_id: Set(None),
            tunnel_id: Set(None),
            zone_id: Set(None),
            zone_name: Set(None),
            subdomain: Set(None),
            dns_record_id: Set(None),
            hostname: Set(None),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert tunnel")
    }

    #[tokio::test]
    async fn get_config_empty_db_returns_none() {
        let db = make_db().await;
        let result = get_config(&db).await.expect("get_config");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_config_with_record_returns_masked_response() {
        let db = make_db().await;
        insert_tunnel(&db, "my-tunnel", "running").await;

        let result = get_config(&db).await.expect("get_config");
        assert!(result.is_some());
        let resp = result.unwrap();
        assert_eq!(resp.name, "my-tunnel");
        assert_eq!(resp.status, "running");
        // Token should be masked
        assert_eq!(resp.tunnel_token, "****");
    }
}
