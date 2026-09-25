//! VPN provider and app VPN configuration endpoints

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use sea_orm::{EntityTrait, TransactionTrait};
use serde::Serialize;
use std::{net::IpAddr, time::Duration};

use crate::error::{AppError, Result};
use crate::middleware::permissions::{Authorized, VpnManage, VpnView};
use crate::models::app_vpn_config;
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::services::app_manager::AppManager;
use crate::services::audit::log_on_transaction;
use crate::services::vpn::{
    self, AppVpnConfigResponse, AssignVpnRequest, CreateVpnProviderRequest, SupportedProvider,
    UpdateVpnProviderRequest, VpnProviderResponse, VpnTestResult,
};
use crate::state::AppState;

/// Create VPN routes
pub fn vpn_routes(state: AppState) -> Router {
    Router::new()
        // VPN providers
        .route("/providers", get(list_providers).post(create_provider))
        .route(
            "/providers/{id}",
            get(get_provider)
                .put(update_provider)
                .delete(delete_provider),
        )
        .route("/providers/{id}/test", post(test_provider))
        // App VPN configs
        .route("/apps", get(list_app_configs))
        .route(
            "/apps/{app_name}",
            get(get_app_config).put(assign_vpn).delete(remove_vpn),
        )
        .route("/apps/{app_name}/forwarded-port", get(get_forwarded_port))
        .route("/apps/{app_name}/public-ip", get(get_public_ip))
        // Supported providers
        .route("/supported-providers", get(list_supported_providers))
        .with_state(state)
}

// ============================================================================
// Response Types
// ============================================================================

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProvidersResponse {
    pub providers: Vec<VpnProviderResponse>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppConfigsResponse {
    pub configs: Vec<AppVpnConfigResponse>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SupportedProvidersResponse {
    pub providers: Vec<SupportedProvider>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RemoveVpnResponse {
    pub message: String,
    pub operation_id: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PublicIpResponse {
    pub public_ip: Option<String>,
}

// ============================================================================
// VPN Provider Endpoints
// ============================================================================

/// List all VPN providers
#[utoipa::path(
    get,
    path = "/api/vpn/providers",
    tag = "VPN",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn list_providers(
    State(state): State<AppState>,
    _auth: Authorized<VpnView>,
) -> Result<Json<ProvidersResponse>> {
    let db = state.get_db().await?;
    let providers = vpn::list_vpn_providers(&db).await?;
    Ok(Json(ProvidersResponse { providers }))
}

/// Get a VPN provider by ID
#[utoipa::path(
    get,
    path = "/api/vpn/providers/{id}",
    tag = "VPN",
    params(
        ("id" = i64, Path, description = "VPN provider ID")
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn get_provider(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<VpnView>,
) -> Result<Json<VpnProviderResponse>> {
    let db = state.get_db().await?;
    let provider = vpn::get_vpn_provider(&db, id).await?;
    Ok(Json(provider))
}

/// Create a new VPN provider
#[utoipa::path(
    post,
    path = "/api/vpn/providers",
    tag = "VPN",
    request_body = serde_json::Value,
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn create_provider(
    State(state): State<AppState>,
    auth: Authorized<VpnManage>,
    Json(req): Json<CreateVpnProviderRequest>,
) -> Result<Json<VpnProviderResponse>> {
    let db = state.get_db().await?;
    let transaction = db.begin().await?;
    let provider = vpn::create_vpn_provider(&transaction, req).await?;
    log_on_transaction(
        &transaction,
        AuditAction::VpnProviderCreated,
        ResourceType::Vpn,
        Some(provider.id.to_string()),
        Some(auth.user_id()),
        Some(auth.user().username.clone()),
        Some(serde_json::json!({"provider_id": provider.id, "enabled": provider.enabled})),
        None,
        None,
        true,
        None,
    )
    .await?;
    transaction.commit().await?;
    Ok(Json(provider))
}

/// Update a VPN provider
#[utoipa::path(
    put,
    path = "/api/vpn/providers/{id}",
    tag = "VPN",
    params(
        ("id" = i64, Path, description = "VPN provider ID")
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    auth: Authorized<VpnManage>,
    Json(req): Json<UpdateVpnProviderRequest>,
) -> Result<Json<VpnProviderResponse>> {
    // Only static field names, never values (credentials, servers or subnets).
    let mut fields = Vec::new();
    if req.name.is_some() {
        fields.push("name");
    }
    if req.service_provider.is_some() {
        fields.push("service_provider");
    }
    if req.credentials.is_some() {
        fields.push("credentials");
    }
    if req.enabled.is_some() {
        fields.push("enabled");
    }
    if req.kill_switch.is_some() {
        fields.push("kill_switch");
    }
    if req.firewall_outbound_subnets.is_some() {
        fields.push("firewall_outbound_subnets");
    }
    let db = state.get_db().await?;
    let transaction = db.begin().await?;
    let provider = vpn::update_vpn_provider(&transaction, id, req).await?;
    log_on_transaction(
        &transaction,
        AuditAction::VpnProviderUpdated,
        ResourceType::Vpn,
        Some(id.to_string()),
        Some(auth.user_id()),
        Some(auth.user().username.clone()),
        Some(serde_json::json!({"provider_id": id, "enabled": provider.enabled, "fields": fields})),
        None,
        None,
        true,
        None,
    )
    .await?;
    transaction.commit().await?;
    Ok(Json(provider))
}

/// Delete a VPN provider
#[utoipa::path(
    delete,
    path = "/api/vpn/providers/{id}",
    tag = "VPN",
    params(
        ("id" = i64, Path, description = "VPN provider ID")
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    auth: Authorized<VpnManage>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let transaction = db.begin().await?;
    vpn::delete_vpn_provider_in_transaction(&transaction, id).await?;
    log_on_transaction(
        &transaction,
        AuditAction::VpnProviderDeleted,
        ResourceType::Vpn,
        Some(id.to_string()),
        Some(auth.user_id()),
        Some(auth.user().username.clone()),
        Some(serde_json::json!({"provider_id": id})),
        None,
        None,
        true,
        None,
    )
    .await?;
    transaction.commit().await?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// Test VPN provider connection
#[utoipa::path(
    post,
    path = "/api/vpn/providers/{id}/test",
    tag = "VPN",
    params(
        ("id" = i64, Path, description = "VPN provider ID")
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn test_provider(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _auth: Authorized<VpnManage>,
) -> Result<Json<VpnTestResult>> {
    let db = state.get_db().await?;
    let k8s = state.k8s_client.read().await;
    let client = k8s.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("Kubernetes client not available".to_string())
    })?;

    let result = vpn::test_vpn_connection(client, &db, id).await?;
    Ok(Json(result))
}

// ============================================================================
// App VPN Config Endpoints
// ============================================================================

/// List all app VPN configurations
#[utoipa::path(
    get,
    path = "/api/vpn/apps",
    tag = "VPN",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn list_app_configs(
    State(state): State<AppState>,
    _auth: Authorized<VpnView>,
) -> Result<Json<AppConfigsResponse>> {
    let db = state.get_db().await?;
    let configs = vpn::list_app_vpn_configs(&db).await?;
    Ok(Json(AppConfigsResponse { configs }))
}

/// Get app VPN configuration
#[utoipa::path(
    get,
    path = "/api/vpn/apps/{app_name}",
    tag = "VPN",
    params(
        ("app_name" = String, Path, description = "Application name")
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn get_app_config(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<VpnView>,
) -> Result<Json<Option<AppVpnConfigResponse>>> {
    let db = state.get_db().await?;
    let config = vpn::get_app_vpn_config(&db, &app_name).await?;
    Ok(Json(config))
}

/// Assign VPN to an app and redeploy
#[utoipa::path(
    put,
    path = "/api/vpn/apps/{app_name}",
    tag = "VPN",
    params(
        ("app_name" = String, Path, description = "Application name")
    ),
    request_body = serde_json::Value,
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn assign_vpn(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authorized<VpnManage>,
    Json(req): Json<AssignVpnRequest>,
) -> Result<Json<AppVpnConfigResponse>> {
    let db = state.get_db().await?;
    {
        let catalog = state.catalog.read().await;
        let app = catalog.get_app(&app_name).ok_or_else(|| {
            AppError::NotFound(format!("App '{}' not found in catalog", app_name))
        })?;
        if app.is_system {
            return Err(AppError::BadRequest(format!(
                "VPN is not supported for system app '{}'",
                app_name
            )));
        }
    }

    let transaction = db.begin().await?;
    let mut config = vpn::assign_vpn_to_app(&transaction, &app_name, req).await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    let operation = manager
        .enqueue_update_in_transaction(&transaction, &app_name, Some(auth.user_id()))
        .await?;
    log_on_transaction(
        &transaction, AuditAction::VpnAssigned, ResourceType::Vpn, Some(app_name.clone()),
        Some(auth.user_id()), Some(auth.user().username.clone()),
        Some(serde_json::json!({"app_name": app_name, "provider_id": config.vpn_provider_id, "operation_id": operation.id, "phase": "queued"})),
        None, None, true, None,
    ).await?;
    transaction.commit().await?;
    config.operation_id = Some(operation.id);

    Ok(Json(config))
}

/// Remove VPN from an app and redeploy
#[utoipa::path(
    delete,
    path = "/api/vpn/apps/{app_name}",
    tag = "VPN",
    params(
        ("app_name" = String, Path, description = "Application name")
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn remove_vpn(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authorized<VpnManage>,
) -> Result<Json<RemoveVpnResponse>> {
    let db = state.get_db().await?;
    {
        let catalog = state.catalog.read().await;
        let app = catalog.get_app(&app_name).ok_or_else(|| {
            AppError::NotFound(format!("App '{}' not found in catalog", app_name))
        })?;
        if app.is_system {
            return Err(AppError::BadRequest(format!(
                "VPN is not supported for system app '{}'",
                app_name
            )));
        }
    }

    let transaction = db.begin().await?;
    let previous = app_vpn_config::Entity::find_by_id(app_name.as_str())
        .one(&transaction)
        .await?;
    vpn::remove_vpn_from_app(&transaction, &app_name).await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    let operation = manager
        .enqueue_update_in_transaction(&transaction, &app_name, Some(auth.user_id()))
        .await?;
    log_on_transaction(
        &transaction, AuditAction::VpnRemoved, ResourceType::Vpn, Some(app_name.clone()),
        Some(auth.user_id()), Some(auth.user().username.clone()),
        Some(serde_json::json!({"app_name": app_name, "provider_id": previous.map(|c| c.vpn_provider_id), "operation_id": operation.id, "phase": "queued"})),
        None, None, true, None,
    ).await?;
    transaction.commit().await?;

    Ok(Json(RemoveVpnResponse {
        message: format!("VPN removal queued for app '{}'", app_name),
        operation_id: operation.id,
    }))
}

/// Get the VPN forwarded port for an app (queries Gluetun control API)
#[utoipa::path(
    get,
    path = "/api/vpn/apps/{app_name}/forwarded-port",
    tag = "VPN",
    params(
        ("app_name" = String, Path, description = "Application name")
    ),
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn get_forwarded_port(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<VpnView>,
) -> Result<Json<serde_json::Value>> {
    let pod_ip = running_gluetun_pod_ip(&state, &app_name).await?;
    let url = gluetun_portforward_url(pod_ip);
    let client = gluetun_client()?;

    let port = query_forwarded_port(&client, &url).await?;
    Ok(Json(serde_json::json!({ "port": port })))
}

/// Return only the VPN's public IP, or null if Gluetun has not detected one.
#[utoipa::path(
    get,
    path = "/api/vpn/apps/{app_name}/public-ip",
    tag = "VPN",
    params(("app_name" = String, Path, description = "Application name")),
    responses(
        (status = 200, body = PublicIpResponse),
        (status = 404, description = "No running Gluetun pod for this app"),
        (status = 502, description = "Gluetun public IP query failed")
    )
)]
pub(crate) async fn get_public_ip(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<VpnView>,
) -> Result<Json<PublicIpResponse>> {
    let pod_ip = running_gluetun_pod_ip(&state, &app_name).await?;
    let url = gluetun_public_ip_url(pod_ip);
    let client = gluetun_client()?;
    Ok(Json(PublicIpResponse {
        public_ip: query_public_ip(&client, &url).await?,
    }))
}

async fn running_gluetun_pod_ip(state: &AppState, app_name: &str) -> Result<IpAddr> {
    let k8s = state.k8s_client.read().await;
    let k8s_client = k8s.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("Kubernetes client not available".to_string())
    })?;

    // Find pods in the app's namespace with the gluetun container
    let pods: kube::api::Api<k8s_openapi::api::core::v1::Pod> =
        kube::api::Api::namespaced(k8s_client.client().clone(), app_name);
    let pod_list = pods
        .list(&kube::api::ListParams::default())
        .await
        .map_err(|_| AppError::Internal("Failed to list VPN pods".to_string()))?;

    // Find a running pod with a gluetun container
    let pod_ip = running_gluetun_ip(&pod_list.items).ok_or_else(|| {
        crate::error::AppError::NotFound(format!(
            "No running pod with VPN found for app '{}'",
            app_name
        ))
    })?;

    pod_ip
        .parse()
        .map_err(|_| AppError::Internal("VPN pod has an invalid IP address".to_string()))
}

fn running_gluetun_ip(pods: &[k8s_openapi::api::core::v1::Pod]) -> Option<&str> {
    pods.iter().find_map(|pod| {
        let status = pod.status.as_ref()?;
        let phase = status.phase.as_deref()?;
        if phase != "Running" {
            return None;
        }
        // Check if pod has a gluetun container
        let spec = pod.spec.as_ref()?;
        let has_gluetun = spec.containers.iter().any(|c| c.name == "gluetun");
        if !has_gluetun {
            return None;
        }
        status.pod_ip.as_deref()
    })
}

fn gluetun_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AppError::Internal("Failed to create VPN HTTP client".to_string()))
}

fn gluetun_portforward_url(pod_ip: IpAddr) -> String {
    if pod_ip.is_ipv6() {
        format!("http://[{pod_ip}]:8001/v1/portforward")
    } else {
        format!("http://{pod_ip}:8001/v1/portforward")
    }
}

fn gluetun_public_ip_url(pod_ip: IpAddr) -> String {
    if pod_ip.is_ipv6() {
        format!("http://[{pod_ip}]:8001/v1/publicip/ip")
    } else {
        format!("http://{pod_ip}:8001/v1/publicip/ip")
    }
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || ip.is_unspecified()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            // Global unicast only, excluding documentation and IPv4 translations.
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
                && ip.to_ipv4_mapped().is_none()
        }
    }
}

fn parse_public_ip(body: &[u8]) -> Option<Option<String>> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let ip = value.as_object()?.get("public_ip")?;
    if ip.is_null() || ip.as_str() == Some("") {
        return Some(None);
    }
    let text = ip.as_str()?;
    let parsed: IpAddr = text.parse().ok()?;
    is_public_ip(parsed).then(|| Some(parsed.to_string()))
}

async fn query_public_ip(client: &reqwest::Client, url: &str) -> Result<Option<String>> {
    let response = client.get(url).send().await.map_err(|error| {
        if error.is_timeout() {
            AppError::BadGateway("VPN public IP query timed out".to_string())
        } else {
            AppError::BadGateway("VPN public IP API is unreachable".to_string())
        }
    })?;
    if !response.status().is_success() {
        return Err(AppError::BadGateway(
            "VPN public IP API returned an error".to_string(),
        ));
    }
    let body = response.bytes().await.map_err(|error| {
        if error.is_timeout() {
            AppError::BadGateway("VPN public IP query timed out".to_string())
        } else {
            AppError::BadGateway("VPN public IP API response could not be read".to_string())
        }
    })?;
    parse_public_ip(&body).ok_or_else(|| {
        AppError::BadGateway("VPN public IP API returned malformed data".to_string())
    })
}

/// Gluetun v3.41.3 returns {"ports": [uint16, ...], "port": uint16}.
/// The singular field is retained as a fallback for older Gluetun releases.
fn parse_forwarded_port(body: &[u8]) -> Option<u16> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let object = value.as_object()?;
    if let Some(ports) = object.get("ports") {
        let ports = ports.as_array()?;
        let mut parsed = ports.iter().map(|port| {
            port.as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port > 0)
        });
        let first = match parsed.next() {
            Some(Some(port)) => port,
            Some(None) => return None,
            None => 0,
        };
        if parsed.any(|port| port.is_none()) {
            return None;
        }
        // A conflicting legacy field indicates a broken upstream response.
        if let Some(legacy) = object.get("port") {
            if legacy.as_u64() != Some(u64::from(first)) {
                return None;
            }
        }
        Some(first)
    } else {
        object
            .get("port")?
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
    }
}

async fn query_forwarded_port(client: &reqwest::Client, url: &str) -> Result<u16> {
    let response = client.get(url).send().await.map_err(|error| {
        if error.is_timeout() {
            AppError::BadGateway("VPN port query timed out".to_string())
        } else {
            AppError::BadGateway("VPN port API is unreachable".to_string())
        }
    })?;
    if !response.status().is_success() {
        return Err(AppError::BadGateway(
            "VPN port API returned an error".to_string(),
        ));
    }
    let body = response.bytes().await.map_err(|error| {
        if error.is_timeout() {
            AppError::BadGateway("VPN port query timed out".to_string())
        } else {
            AppError::BadGateway("VPN port API response could not be read".to_string())
        }
    })?;
    parse_forwarded_port(&body)
        .ok_or_else(|| AppError::BadGateway("VPN port API returned malformed data".to_string()))
}

// ============================================================================
// Supported Providers Endpoint
// ============================================================================

/// List supported VPN service providers
#[utoipa::path(
    get,
    path = "/api/vpn/supported-providers",
    tag = "VPN",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn list_supported_providers(
    _auth: Authorized<VpnView>,
) -> Result<Json<SupportedProvidersResponse>> {
    let providers = vpn::get_supported_providers();
    Ok(Json(SupportedProvidersResponse { providers }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::get, Router};

    #[test]
    fn providers_response_ser_empty() {
        let r = ProvidersResponse { providers: vec![] };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"providers\":[]"));
    }

    #[test]
    fn app_configs_response_ser_empty() {
        let r = AppConfigsResponse { configs: vec![] };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"configs\":[]"));
    }

    #[test]
    fn supported_providers_response_ser() {
        let providers = crate::services::vpn::get_supported_providers();
        let r = SupportedProvidersResponse { providers };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"providers\""));
        assert!(json.contains("custom"));
    }

    #[test]
    fn gluetun_forwarded_port_parser() {
        for (body, expected) in [
            (r#"{"ports":[51413],"port":51413}"#, Some(51413)),
            (r#"{"ports":[],"port":0}"#, Some(0)),
            (r#"{"port":65535}"#, Some(65535)),
        ] {
            assert_eq!(parse_forwarded_port(body.as_bytes()), expected, "{body}");
        }
        for body in [
            r#"{}"#,
            r#"{"ports":null,"port":0}"#,
            r#"{"ports":[0],"port":0}"#,
            r#"{"ports":[65536],"port":65536}"#,
            r#"{"ports":[51413],"port":42}"#,
            r#"{"ports":[51413,"bad"],"port":51413}"#,
            r#"{"ports":[],"port":12}"#,
            r#"{"port":-1}"#,
            r#"{"port":"51413"}"#,
            r#"{"port":65536}"#,
            "not json",
        ] {
            assert_eq!(parse_forwarded_port(body.as_bytes()), None, "{body}");
        }
    }

    #[test]
    fn gluetun_portforward_url_uses_control_server_port_for_ipv4_and_ipv6() {
        assert_eq!(
            gluetun_portforward_url("10.0.0.2".parse().unwrap()),
            "http://10.0.0.2:8001/v1/portforward"
        );
        assert_eq!(
            gluetun_portforward_url("2001:db8::1".parse().unwrap()),
            "http://[2001:db8::1]:8001/v1/portforward"
        );
    }

    async fn mock_port_api(status: StatusCode, body: &'static str, delay: Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/portforward", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/v1/portforward",
            get(move || async move {
                tokio::time::sleep(delay).await;
                (status, body)
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    #[tokio::test]
    async fn forwarded_port_client_handles_success_and_failures() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(100))
            .no_proxy()
            .build()
            .unwrap();
        for (status, body, expected) in [
            (StatusCode::OK, r#"{"ports":[42345],"port":42345}"#, 42345),
            (StatusCode::OK, r#"{"ports":[],"port":0}"#, 0),
        ] {
            let url = mock_port_api(status, body, Duration::ZERO).await;
            assert_eq!(query_forwarded_port(&client, &url).await.unwrap(), expected);
        }
        for (status, body, message) in [
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "upstream host 10.0.0.2",
                "returned an error",
            ),
            (StatusCode::OK, "not json 10.0.0.2", "malformed data"),
            (StatusCode::OK, r#"{"ports":[70000]}"#, "malformed data"),
        ] {
            let url = mock_port_api(status, body, Duration::ZERO).await;
            let error = query_forwarded_port(&client, &url).await.unwrap_err();
            assert!(matches!(error, AppError::BadGateway(_)));
            assert!(error.to_string().contains(message));
            assert!(!error.to_string().contains("10.0.0.2"));
        }
        let url = mock_port_api(StatusCode::OK, "{}", Duration::from_secs(1)).await;
        let error = query_forwarded_port(&client, &url).await.unwrap_err();
        assert!(matches!(error, AppError::BadGateway(_)));
        assert!(error.to_string().contains("timed out"));
    }

    #[test]
    fn public_ip_parser_accepts_only_global_addresses_or_explicit_absence() {
        for (body, expected) in [
            (
                r#"{"public_ip":"8.8.8.8","city":"hidden","credentials":"hidden"}"#,
                Some(Some("8.8.8.8".to_string())),
            ),
            (
                r#"{"public_ip":"2606:4700:4700::1111"}"#,
                Some(Some("2606:4700:4700::1111".to_string())),
            ),
            (r#"{"public_ip":null}"#, Some(None)),
            (r#"{"public_ip":""}"#, Some(None)),
        ] {
            assert_eq!(parse_public_ip(body.as_bytes()), expected, "{body}");
        }
        for body in [
            "{}",
            "null",
            "not json",
            r#"{"public_ip":42}"#,
            r#"{"public_ip":" 8.8.8.8 "}"#,
            r#"{"public_ip":"host.example"}"#,
            r#"{"public_ip":"127.0.0.1"}"#,
            r#"{"public_ip":"10.0.0.2"}"#,
            r#"{"public_ip":"100.64.1.2"}"#,
            r#"{"public_ip":"169.254.2.3"}"#,
            r#"{"public_ip":"192.0.2.1"}"#,
            r#"{"public_ip":"198.18.0.1"}"#,
            r#"{"public_ip":"224.0.0.1"}"#,
            r#"{"public_ip":"0.1.2.3"}"#,
            r#"{"public_ip":"::1"}"#,
            r#"{"public_ip":"fe80::1"}"#,
            r#"{"public_ip":"fc00::1"}"#,
            r#"{"public_ip":"2001:db8::1"}"#,
            r#"{"public_ip":"::ffff:8.8.8.8"}"#,
        ] {
            assert_eq!(parse_public_ip(body.as_bytes()), None, "{body}");
        }
    }

    #[test]
    fn public_ip_response_exposes_only_the_ip() {
        assert_eq!(
            serde_json::to_value(PublicIpResponse {
                public_ip: Some("8.8.8.8".to_string())
            })
            .unwrap(),
            serde_json::json!({"public_ip":"8.8.8.8"})
        );
        assert_eq!(
            serde_json::to_value(PublicIpResponse { public_ip: None }).unwrap(),
            serde_json::json!({"public_ip":null})
        );
    }

    #[test]
    fn public_ip_url_uses_only_pod_ip_and_fixed_control_path() {
        assert_eq!(
            gluetun_public_ip_url("10.0.0.2".parse().unwrap()),
            "http://10.0.0.2:8001/v1/publicip/ip"
        );
        assert_eq!(
            gluetun_public_ip_url("2001:db8::1".parse().unwrap()),
            "http://[2001:db8::1]:8001/v1/publicip/ip"
        );
    }

    #[test]
    fn public_ip_requires_a_running_pod_with_gluetun() {
        use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, PodStatus};
        let pod = |phase: &str, container: &str| Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: container.to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some(phase.to_string()),
                pod_ip: Some("10.0.0.2".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(running_gluetun_ip(&[]), None);
        assert_eq!(running_gluetun_ip(&[pod("Pending", "gluetun")]), None);
        assert_eq!(running_gluetun_ip(&[pod("Running", "other")]), None);
        assert_eq!(
            running_gluetun_ip(&[pod("Running", "gluetun")]),
            Some("10.0.0.2")
        );
    }

    async fn mock_public_ip_api(status: StatusCode, body: &'static str, delay: Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/publicip/ip", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/v1/publicip/ip",
            get(move || async move {
                tokio::time::sleep(delay).await;
                (status, body)
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    #[tokio::test]
    async fn public_ip_client_handles_success_and_safe_errors() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(100))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        for (body, expected) in [
            (
                r#"{"public_ip":"1.1.1.1","country":"secret"}"#,
                Some("1.1.1.1".to_string()),
            ),
            (
                r#"{"public_ip":"2606:4700::1111"}"#,
                Some("2606:4700::1111".to_string()),
            ),
            (r#"{"public_ip":null}"#, None),
        ] {
            let url = mock_public_ip_api(StatusCode::OK, body, Duration::ZERO).await;
            assert_eq!(query_public_ip(&client, &url).await.unwrap(), expected);
        }
        for (status, body, message) in [
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "secret credentials",
                "returned an error",
            ),
            (StatusCode::OK, "secret credentials", "malformed data"),
            (
                StatusCode::OK,
                r#"{"public_ip":"10.0.0.1","credentials":"secret"}"#,
                "malformed data",
            ),
        ] {
            let url = mock_public_ip_api(status, body, Duration::ZERO).await;
            let error = query_public_ip(&client, &url).await.unwrap_err();
            assert!(matches!(error, AppError::BadGateway(_)));
            assert!(error.to_string().contains(message));
            assert!(!error.to_string().contains("secret"));
        }
        let url = mock_public_ip_api(StatusCode::OK, "{}", Duration::from_secs(1)).await;
        let error = query_public_ip(&client, &url).await.unwrap_err();
        assert!(matches!(error, AppError::BadGateway(_)));
        assert!(error.to_string().contains("timed out"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/publicip/ip", listener.local_addr().unwrap());
        drop(listener);
        let error = query_public_ip(&client, &url).await.unwrap_err();
        assert!(matches!(error, AppError::BadGateway(_)));
        assert!(error.to_string().contains("unreachable"));
    }
}
