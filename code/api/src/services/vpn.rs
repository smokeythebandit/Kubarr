//! VPN Provider Service
//!
//! Manages VPN providers and app VPN configurations, including K8s secret generation
//! for Gluetun sidecar containers.

use std::collections::BTreeMap;

use chrono::Utc;
use k8s_openapi::api::core::v1::{
    Capabilities, Container, EnvVar, Pod, PodSpec, Secret, SecurityContext,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, PostParams};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::models::prelude::*;
use crate::models::{app_vpn_config, vpn_provider};
use crate::services::K8sClient;
use crate::state::DbConn;

// ============================================================================
// Credential Structures
// ============================================================================

/// WireGuard credentials for Gluetun
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireGuardCredentials {
    pub private_key: String,
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preshared_key: Option<String>,
}

/// OpenVPN credentials for Gluetun
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenVpnCredentials {
    pub username: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_countries: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_cities: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_hostnames: Option<String>,
}

/// Unified credentials enum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VpnCredentials {
    WireGuard(WireGuardCredentials),
    OpenVpn(OpenVpnCredentials),
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request to create a VPN provider
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CreateVpnProviderRequest {
    pub name: String,
    pub vpn_type: vpn_provider::VpnType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_provider: Option<String>,
    pub credentials: serde_json::Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub kill_switch: bool,
    #[serde(default = "default_firewall_subnets")]
    pub firewall_outbound_subnets: String,
}

/// Request to update a VPN provider
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct UpdateVpnProviderRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kill_switch: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firewall_outbound_subnets: Option<String>,
}

/// Response for a VPN provider (without credentials)
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct VpnProviderResponse {
    pub id: i64,
    pub name: String,
    pub vpn_type: vpn_provider::VpnType,
    pub service_provider: Option<String>,
    pub enabled: bool,
    pub kill_switch: bool,
    pub firewall_outbound_subnets: String,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    /// Number of apps using this provider
    pub app_count: usize,
}

/// Request to assign VPN to an app
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct AssignVpnRequest {
    pub vpn_provider_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kill_switch_override: Option<bool>,
    /// Enable VPN port forwarding (NAT-PMP) for incoming connections
    #[serde(default)]
    pub port_forwarding: Option<bool>,
}

/// Response for app VPN configuration
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct AppVpnConfigResponse {
    pub app_name: String,
    pub vpn_provider_id: i64,
    pub vpn_provider_name: String,
    pub kill_switch_override: Option<bool>,
    pub effective_kill_switch: bool,
    pub port_forwarding: bool,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// Supported VPN service provider info
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SupportedProvider {
    pub id: &'static str,
    pub name: &'static str,
    pub vpn_types: Vec<&'static str>,
    pub description: &'static str,
    pub supports_port_forwarding: bool,
}

fn default_true() -> bool {
    true
}

fn default_firewall_subnets() -> String {
    "10.0.0.0/8,172.16.0.0/12,192.168.0.0/16".to_string()
}

// ============================================================================
// VPN Provider Functions
// ============================================================================

/// List all VPN providers
pub async fn list_vpn_providers(db: &DbConn) -> Result<Vec<VpnProviderResponse>> {
    let providers = VpnProvider::find().all(db).await?;

    // Get app counts for each provider
    let app_configs = AppVpnConfig::find().all(db).await?;

    let mut responses = Vec::new();
    for provider in providers {
        let app_count = app_configs
            .iter()
            .filter(|c| c.vpn_provider_id == provider.id)
            .count();

        responses.push(VpnProviderResponse {
            id: provider.id,
            name: provider.name,
            vpn_type: provider.vpn_type,
            service_provider: provider.service_provider,
            enabled: provider.enabled,
            kill_switch: provider.kill_switch,
            firewall_outbound_subnets: provider.firewall_outbound_subnets,
            created_at: provider.created_at,
            updated_at: provider.updated_at,
            app_count,
        });
    }

    Ok(responses)
}

/// Get a VPN provider by ID
pub async fn get_vpn_provider(db: &DbConn, id: i64) -> Result<VpnProviderResponse> {
    let provider = VpnProvider::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("VPN provider {} not found", id)))?;

    let app_count = AppVpnConfig::find()
        .filter(app_vpn_config::Column::VpnProviderId.eq(id))
        .count(db)
        .await? as usize;

    Ok(VpnProviderResponse {
        id: provider.id,
        name: provider.name,
        vpn_type: provider.vpn_type,
        service_provider: provider.service_provider,
        enabled: provider.enabled,
        kill_switch: provider.kill_switch,
        firewall_outbound_subnets: provider.firewall_outbound_subnets,
        created_at: provider.created_at,
        updated_at: provider.updated_at,
        app_count,
    })
}

/// Create a new VPN provider
pub async fn create_vpn_provider(
    db: &DbConn,
    req: CreateVpnProviderRequest,
) -> Result<VpnProviderResponse> {
    // Validate credentials based on VPN type
    validate_credentials(&req.vpn_type, &req.credentials)?;

    let now = Utc::now();
    let credentials_json = serde_json::to_string(&req.credentials)
        .map_err(|e| AppError::BadRequest(format!("Invalid credentials JSON: {}", e)))?;

    let new_provider = vpn_provider::ActiveModel {
        name: Set(req.name),
        vpn_type: Set(req.vpn_type),
        service_provider: Set(req.service_provider),
        credentials_json: Set(credentials_json),
        enabled: Set(req.enabled),
        kill_switch: Set(req.kill_switch),
        firewall_outbound_subnets: Set(req.firewall_outbound_subnets),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };

    let provider = new_provider.insert(db).await?;

    Ok(VpnProviderResponse {
        id: provider.id,
        name: provider.name,
        vpn_type: provider.vpn_type,
        service_provider: provider.service_provider,
        enabled: provider.enabled,
        kill_switch: provider.kill_switch,
        firewall_outbound_subnets: provider.firewall_outbound_subnets,
        created_at: provider.created_at,
        updated_at: provider.updated_at,
        app_count: 0,
    })
}

/// Update a VPN provider
pub async fn update_vpn_provider(
    db: &DbConn,
    id: i64,
    req: UpdateVpnProviderRequest,
) -> Result<VpnProviderResponse> {
    let provider = VpnProvider::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("VPN provider {} not found", id)))?;

    let mut active_model: vpn_provider::ActiveModel = provider.clone().into();

    if let Some(name) = req.name {
        active_model.name = Set(name);
    }
    if let Some(service_provider) = req.service_provider {
        active_model.service_provider = Set(Some(service_provider));
    }
    if let Some(credentials) = req.credentials {
        validate_credentials(&provider.vpn_type, &credentials)?;
        let credentials_json = serde_json::to_string(&credentials)
            .map_err(|e| AppError::BadRequest(format!("Invalid credentials JSON: {}", e)))?;
        active_model.credentials_json = Set(credentials_json);
    }
    if let Some(enabled) = req.enabled {
        active_model.enabled = Set(enabled);
    }
    if let Some(kill_switch) = req.kill_switch {
        active_model.kill_switch = Set(kill_switch);
    }
    if let Some(firewall_outbound_subnets) = req.firewall_outbound_subnets {
        active_model.firewall_outbound_subnets = Set(firewall_outbound_subnets);
    }

    active_model.updated_at = Set(Utc::now());

    let updated = active_model.update(db).await?;

    let app_count = AppVpnConfig::find()
        .filter(app_vpn_config::Column::VpnProviderId.eq(id))
        .count(db)
        .await? as usize;

    Ok(VpnProviderResponse {
        id: updated.id,
        name: updated.name,
        vpn_type: updated.vpn_type,
        service_provider: updated.service_provider,
        enabled: updated.enabled,
        kill_switch: updated.kill_switch,
        firewall_outbound_subnets: updated.firewall_outbound_subnets,
        created_at: updated.created_at,
        updated_at: updated.updated_at,
        app_count,
    })
}

/// Delete a VPN provider
pub async fn delete_vpn_provider(db: &DbConn, k8s: &K8sClient, id: i64) -> Result<()> {
    // Get all apps using this provider
    let app_configs = AppVpnConfig::find()
        .filter(app_vpn_config::Column::VpnProviderId.eq(id))
        .all(db)
        .await?;

    // Delete K8s secrets for all associated apps
    for config in &app_configs {
        if let Err(e) = delete_vpn_secret_for_app(k8s, &config.app_name).await {
            tracing::warn!(
                "Failed to delete VPN secret for app {}: {}",
                config.app_name,
                e
            );
        }
    }

    // Delete the provider (cascade will delete app_vpn_configs)
    VpnProvider::delete_by_id(id).exec(db).await?;

    Ok(())
}

// ============================================================================
// App VPN Config Functions
// ============================================================================

/// List all app VPN configurations
pub async fn list_app_vpn_configs(db: &DbConn) -> Result<Vec<AppVpnConfigResponse>> {
    let configs = AppVpnConfig::find().all(db).await?;
    let providers = VpnProvider::find().all(db).await?;

    let provider_map: std::collections::HashMap<i64, &vpn_provider::Model> =
        providers.iter().map(|p| (p.id, p)).collect();

    let mut responses = Vec::new();
    for config in configs {
        if let Some(provider) = provider_map.get(&config.vpn_provider_id) {
            let effective_kill_switch = config.kill_switch_override.unwrap_or(provider.kill_switch);
            responses.push(AppVpnConfigResponse {
                app_name: config.app_name,
                vpn_provider_id: config.vpn_provider_id,
                vpn_provider_name: provider.name.clone(),
                kill_switch_override: config.kill_switch_override,
                effective_kill_switch,
                port_forwarding: config.port_forwarding,
                created_at: config.created_at,
                updated_at: config.updated_at,
            });
        }
    }

    Ok(responses)
}

/// Get app VPN configuration
pub async fn get_app_vpn_config(
    db: &DbConn,
    app_name: &str,
) -> Result<Option<AppVpnConfigResponse>> {
    let config = AppVpnConfig::find_by_id(app_name).one(db).await?;

    if let Some(config) = config {
        let provider = VpnProvider::find_by_id(config.vpn_provider_id)
            .one(db)
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "VPN provider {} not found for app {}",
                    config.vpn_provider_id, app_name
                ))
            })?;

        let effective_kill_switch = config.kill_switch_override.unwrap_or(provider.kill_switch);
        Ok(Some(AppVpnConfigResponse {
            app_name: config.app_name,
            vpn_provider_id: config.vpn_provider_id,
            vpn_provider_name: provider.name,
            kill_switch_override: config.kill_switch_override,
            effective_kill_switch,
            port_forwarding: config.port_forwarding,
            created_at: config.created_at,
            updated_at: config.updated_at,
        }))
    } else {
        Ok(None)
    }
}

/// Assign VPN to an app
pub async fn assign_vpn_to_app(
    db: &DbConn,
    app_name: &str,
    req: AssignVpnRequest,
) -> Result<AppVpnConfigResponse> {
    // Verify provider exists and is enabled
    let provider = VpnProvider::find_by_id(req.vpn_provider_id)
        .one(db)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("VPN provider {} not found", req.vpn_provider_id))
        })?;

    if !provider.enabled {
        return Err(AppError::BadRequest(
            "Cannot assign a disabled VPN provider".to_string(),
        ));
    }

    let now = Utc::now();

    // Check if config already exists
    let existing = AppVpnConfig::find_by_id(app_name).one(db).await?;

    let port_forwarding = req.port_forwarding.unwrap_or(false);

    let config = if let Some(_existing) = existing {
        // Update existing
        let mut active_model = app_vpn_config::ActiveModel {
            app_name: Set(app_name.to_string()),
            vpn_provider_id: Set(req.vpn_provider_id),
            kill_switch_override: Set(req.kill_switch_override),
            port_forwarding: Set(port_forwarding),
            updated_at: Set(now),
            ..Default::default()
        };
        active_model.created_at = sea_orm::ActiveValue::NotSet;
        active_model.update(db).await?
    } else {
        // Create new
        let new_config = app_vpn_config::ActiveModel {
            app_name: Set(app_name.to_string()),
            vpn_provider_id: Set(req.vpn_provider_id),
            kill_switch_override: Set(req.kill_switch_override),
            port_forwarding: Set(port_forwarding),
            created_at: Set(now),
            updated_at: Set(now),
        };
        new_config.insert(db).await?
    };

    let effective_kill_switch = config.kill_switch_override.unwrap_or(provider.kill_switch);
    Ok(AppVpnConfigResponse {
        app_name: config.app_name,
        vpn_provider_id: config.vpn_provider_id,
        vpn_provider_name: provider.name,
        kill_switch_override: config.kill_switch_override,
        effective_kill_switch,
        port_forwarding: config.port_forwarding,
        created_at: config.created_at,
        updated_at: config.updated_at,
    })
}

/// Remove VPN from an app
pub async fn remove_vpn_from_app(db: &DbConn, k8s: &K8sClient, app_name: &str) -> Result<()> {
    // Delete K8s secret
    if let Err(e) = delete_vpn_secret_for_app(k8s, app_name).await {
        tracing::warn!("Failed to delete VPN secret for app {}: {}", app_name, e);
    }

    // Delete config
    AppVpnConfig::delete_by_id(app_name).exec(db).await?;

    Ok(())
}

// ============================================================================
// K8s Secret Management
// ============================================================================

/// Create or update a K8s secret for an app's VPN configuration
pub async fn create_vpn_secret_for_app(
    k8s: &K8sClient,
    db: &DbConn,
    app_name: &str,
) -> Result<String> {
    // Get app VPN config
    let config = AppVpnConfig::find_by_id(app_name)
        .one(db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No VPN config for app {}", app_name)))?;

    // Get provider
    let provider = VpnProvider::find_by_id(config.vpn_provider_id)
        .one(db)
        .await?
        .ok_or_else(|| {
            AppError::Internal(format!("VPN provider {} not found", config.vpn_provider_id))
        })?;

    // Parse credentials
    let credentials: serde_json::Value = serde_json::from_str(&provider.credentials_json)
        .map_err(|e| AppError::Internal(format!("Invalid credentials JSON: {}", e)))?;

    // Build secret data based on VPN type
    let mut secret_data: BTreeMap<String, String> = BTreeMap::new();

    // Set VPN type
    secret_data.insert("VPN_TYPE".to_string(), provider.vpn_type.to_string());

    add_service_provider_secret_data(provider.service_provider.as_deref(), &mut secret_data);

    match provider.vpn_type {
        vpn_provider::VpnType::WireGuard => {
            build_wireguard_secret_data(&credentials, &mut secret_data)?;
        }
        vpn_provider::VpnType::OpenVpn => {
            build_openvpn_secret_data(&credentials, &mut secret_data)?;
        }
    }

    // Convert to base64-encoded data
    let encoded_data: BTreeMap<String, k8s_openapi::ByteString> = secret_data
        .into_iter()
        .map(|(k, v)| (k, k8s_openapi::ByteString(v.into_bytes())))
        .collect();

    let secret_name = format!("vpn-{}", app_name);
    let namespace = app_name;

    // Create secret object
    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(secret_name.clone()),
            namespace: Some(namespace.to_string()),
            labels: Some(BTreeMap::from([
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    "kubarr".to_string(),
                ),
                (
                    "kubarr.io/vpn-provider".to_string(),
                    provider.id.to_string(),
                ),
            ])),
            ..Default::default()
        },
        data: Some(encoded_data),
        ..Default::default()
    };

    // Create or update in K8s
    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), namespace);

    // Try to delete existing secret first (simpler than patch)
    let _ = secrets.delete(&secret_name, &DeleteParams::default()).await;

    // Create new secret
    secrets.create(&PostParams::default(), &secret).await?;

    Ok(secret_name)
}

/// Delete the VPN secret for an app
pub async fn delete_vpn_secret_for_app(k8s: &K8sClient, app_name: &str) -> Result<()> {
    let secret_name = format!("vpn-{}", app_name);
    let namespace = app_name;

    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), namespace);

    match secrets.delete(&secret_name, &DeleteParams::default()).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(()), // Not found is OK
        Err(e) => Err(AppError::Internal(format!(
            "Failed to delete VPN secret: {}",
            e
        ))),
    }
}

/// Get VPN deployment config for an app (used by deployment service)
pub async fn get_vpn_deployment_config(
    db: &DbConn,
    app_name: &str,
) -> Result<Option<VpnDeploymentConfig>> {
    let config = AppVpnConfig::find_by_id(app_name).one(db).await?;

    if let Some(config) = config {
        let provider = VpnProvider::find_by_id(config.vpn_provider_id)
            .one(db)
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!("VPN provider {} not found", config.vpn_provider_id))
            })?;

        if !provider.enabled {
            return Ok(None);
        }

        let kill_switch = config.kill_switch_override.unwrap_or(provider.kill_switch);

        Ok(Some(VpnDeploymentConfig {
            enabled: true,
            secret_name: format!("vpn-{}", app_name),
            kill_switch,
            firewall_outbound_subnets: provider.firewall_outbound_subnets,
            port_forwarding: config.port_forwarding,
        }))
    } else {
        Ok(None)
    }
}

/// VPN deployment configuration for Helm
#[derive(Debug, Clone, Serialize)]
pub struct VpnDeploymentConfig {
    pub enabled: bool,
    pub secret_name: String,
    pub kill_switch: bool,
    pub firewall_outbound_subnets: String,
    pub port_forwarding: bool,
}

/// VPN connection test result
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct VpnTestResult {
    pub success: bool,
    pub message: String,
    pub public_ip: Option<String>,
}

// ============================================================================
// VPN Connection Testing
// ============================================================================

/// Test VPN connection by creating a temporary Gluetun pod
pub async fn test_vpn_connection(
    k8s: &K8sClient,
    db: &DbConn,
    provider_id: i64,
) -> Result<VpnTestResult> {
    // Get and validate provider
    let provider = VpnProvider::find_by_id(provider_id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("VPN provider {} not found", provider_id)))?;

    if !provider.enabled {
        return Err(AppError::BadRequest(
            "Cannot test a disabled VPN provider".to_string(),
        ));
    }

    // Generate test pod name with random suffix
    let random_suffix: String = (0..8)
        .map(|_| format!("{:x}", rand::random::<u8>() % 16))
        .collect();
    let test_pod_name = format!("vpn-test-{}-{}", provider_id, random_suffix);
    let namespace = "kubarr";

    tracing::info!(
        "Testing VPN provider {} with test pod {}",
        provider.name,
        test_pod_name
    );

    // Create test secret
    let secret_name =
        create_test_vpn_secret(k8s, db, &test_pod_name, namespace, provider_id).await?;

    // Create test pod
    match create_gluetun_test_pod(k8s, &test_pod_name, namespace, &secret_name).await {
        Ok(_) => {
            tracing::info!("Created test pod {}", test_pod_name);
        }
        Err(e) => {
            // Clean up secret on failure
            let _ = delete_test_vpn_secret(k8s, &secret_name, namespace).await;
            return Err(e);
        }
    }

    // Wait for pod to be ready (60 second timeout)
    match wait_for_pod_ready(k8s, &test_pod_name, namespace).await {
        Ok(_) => {
            tracing::info!("Test pod {} is ready", test_pod_name);
        }
        Err(e) => {
            // Clean up on failure
            let _ = delete_test_pod(k8s, &test_pod_name, namespace).await;
            let _ = delete_test_vpn_secret(k8s, &secret_name, namespace).await;
            return Err(e);
        }
    }

    // Query Gluetun API for public IP (30 second timeout)
    let public_ip = match query_gluetun_public_ip(k8s, &test_pod_name, namespace).await {
        Ok(ip) => {
            tracing::info!("VPN connection successful, public IP: {}", ip);
            Some(ip)
        }
        Err(e) => {
            // Clean up on failure
            let _ = delete_test_pod(k8s, &test_pod_name, namespace).await;
            let _ = delete_test_vpn_secret(k8s, &secret_name, namespace).await;
            return Err(e);
        }
    };

    // Clean up test resources
    if let Err(e) = delete_test_pod(k8s, &test_pod_name, namespace).await {
        tracing::warn!("Failed to clean up test pod {}: {}", test_pod_name, e);
    }
    if let Err(e) = delete_test_vpn_secret(k8s, &secret_name, namespace).await {
        tracing::warn!("Failed to clean up test secret {}: {}", secret_name, e);
    }

    Ok(VpnTestResult {
        success: true,
        message: format!(
            "VPN connection successful. Connected through {}.",
            provider.name
        ),
        public_ip,
    })
}

/// Create a K8s secret for VPN test pod
async fn create_test_vpn_secret(
    k8s: &K8sClient,
    db: &DbConn,
    test_pod_name: &str,
    namespace: &str,
    provider_id: i64,
) -> Result<String> {
    // Get provider
    let provider = VpnProvider::find_by_id(provider_id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::Internal(format!("VPN provider {} not found", provider_id)))?;

    // Parse credentials
    let credentials: serde_json::Value = serde_json::from_str(&provider.credentials_json)
        .map_err(|e| AppError::Internal(format!("Invalid credentials JSON: {}", e)))?;

    // Build secret data based on VPN type
    let mut secret_data: BTreeMap<String, String> = BTreeMap::new();

    // Set VPN type
    secret_data.insert("VPN_TYPE".to_string(), provider.vpn_type.to_string());

    add_service_provider_secret_data(provider.service_provider.as_deref(), &mut secret_data);

    match provider.vpn_type {
        vpn_provider::VpnType::WireGuard => {
            build_wireguard_secret_data(&credentials, &mut secret_data)?;
        }
        vpn_provider::VpnType::OpenVpn => {
            build_openvpn_secret_data(&credentials, &mut secret_data)?;
        }
    }

    // Convert to base64-encoded data
    let encoded_data: BTreeMap<String, k8s_openapi::ByteString> = secret_data
        .into_iter()
        .map(|(k, v)| (k, k8s_openapi::ByteString(v.into_bytes())))
        .collect();

    let secret_name = format!("vpn-test-{}", test_pod_name);

    // Create secret object
    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(secret_name.clone()),
            namespace: Some(namespace.to_string()),
            labels: Some(BTreeMap::from([
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    "kubarr".to_string(),
                ),
                ("kubarr.io/vpn-test".to_string(), "true".to_string()),
            ])),
            ..Default::default()
        },
        data: Some(encoded_data),
        ..Default::default()
    };

    // Create in K8s
    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), namespace);

    // Try to delete existing secret first (in case of previous failed test)
    let _ = secrets.delete(&secret_name, &DeleteParams::default()).await;

    // Create new secret
    secrets.create(&PostParams::default(), &secret).await?;

    Ok(secret_name)
}

/// Delete a test VPN secret
async fn delete_test_vpn_secret(k8s: &K8sClient, secret_name: &str, namespace: &str) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), namespace);

    match secrets.delete(secret_name, &DeleteParams::default()).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(()), // Not found is OK
        Err(e) => {
            tracing::warn!("Failed to delete test VPN secret {}: {}", secret_name, e);
            Ok(()) // Don't fail on cleanup errors
        }
    }
}

/// Create a Gluetun test pod
async fn create_gluetun_test_pod(
    k8s: &K8sClient,
    pod_name: &str,
    namespace: &str,
    secret_name: &str,
) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(k8s.client().clone(), namespace);

    // Build environment variables from secret
    let env_vars = vec![EnvVar {
        name: "HEALTH_SERVER_ADDRESS".to_string(),
        value: Some(":9999".to_string()),
        ..Default::default()
    }];

    // Build pod spec
    let pod = Pod {
        metadata: ObjectMeta {
            name: Some(pod_name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(BTreeMap::from([
                (
                    "app.kubernetes.io/name".to_string(),
                    "gluetun-test".to_string(),
                ),
                (
                    "app.kubernetes.io/managed-by".to_string(),
                    "kubarr".to_string(),
                ),
                ("kubarr.io/vpn-test".to_string(), "true".to_string()),
            ])),
            ..Default::default()
        },
        spec: Some(PodSpec {
            containers: vec![Container {
                name: "gluetun".to_string(),
                image: Some(
                    std::env::var("KUBARR_GLUETUN_IMAGE")
                        .unwrap_or_else(|_| "qmcgaw/gluetun:v3.40".to_string()),
                ),
                env: Some(env_vars),
                env_from: Some(vec![k8s_openapi::api::core::v1::EnvFromSource {
                    secret_ref: Some(k8s_openapi::api::core::v1::SecretEnvSource {
                        name: secret_name.to_string(),
                        optional: Some(false),
                    }),
                    ..Default::default()
                }]),
                security_context: Some(SecurityContext {
                    capabilities: Some(Capabilities {
                        add: Some(vec!["NET_ADMIN".to_string()]),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            restart_policy: Some("Never".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Create pod
    pods.create(&PostParams::default(), &pod).await?;

    Ok(())
}

/// Wait for pod to be ready with timeout
async fn wait_for_pod_ready(k8s: &K8sClient, pod_name: &str, namespace: &str) -> Result<()> {
    use std::time::Duration;
    use tokio::time::{sleep, timeout};

    let pods: Api<Pod> = Api::namespaced(k8s.client().clone(), namespace);
    let timeout_duration = Duration::from_secs(60);

    let wait_result = timeout(timeout_duration, async {
        loop {
            match pods.get(pod_name).await {
                Ok(pod) => {
                    if let Some(status) = pod.status {
                        // Check if pod phase is Running
                        if let Some(phase) = status.phase {
                            if phase == "Running" {
                                // Check container statuses
                                if let Some(container_statuses) = status.container_statuses {
                                    let all_ready = container_statuses.iter().all(|cs| cs.ready);
                                    if all_ready {
                                        return Ok(());
                                    }
                                }
                            } else if phase == "Failed" || phase == "Unknown" {
                                let reason = status
                                    .container_statuses
                                    .and_then(|cs| cs.first().cloned())
                                    .and_then(|cs| cs.state)
                                    .and_then(|state| {
                                        state
                                            .terminated
                                            .map(|t| t.reason.unwrap_or_else(|| "Unknown".to_string()))
                                    })
                                    .unwrap_or_else(|| "Pod failed to start".to_string());

                                return Err(AppError::Internal(format!(
                                    "VPN test pod failed: {}. Please check your VPN credentials and try again.",
                                    reason
                                )));
                            }
                        }
                    }
                }
                Err(e) => {
                    return Err(AppError::Internal(format!(
                        "Failed to get pod status: {}",
                        e
                    )));
                }
            }

            sleep(Duration::from_secs(2)).await;
        }
    })
    .await;

    match wait_result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(AppError::Internal(
            "VPN connection test timed out after 60 seconds. The VPN provider may be unreachable or credentials may be invalid.".to_string(),
        )),
    }
}

/// Query Gluetun API for public IP
async fn query_gluetun_public_ip(
    k8s: &K8sClient,
    pod_name: &str,
    namespace: &str,
) -> Result<String> {
    use std::time::Duration;
    use tokio::time::timeout;

    // Get pod IP
    let pods: Api<Pod> = Api::namespaced(k8s.client().clone(), namespace);
    let pod = pods.get(pod_name).await?;

    let pod_ip = pod
        .status
        .and_then(|s| s.pod_ip)
        .ok_or_else(|| AppError::Internal("Pod has no IP address".to_string()))?;

    // Query Gluetun API with timeout
    let timeout_duration = Duration::from_secs(30);
    let url = format!("http://{}:8000/v1/publicip/ip", pod_ip);

    let query_result = timeout(timeout_duration, async {
        let client = reqwest::Client::new();
        let response = client.get(&url).send().await.map_err(|e| {
            AppError::Internal(format!(
                "Failed to connect to VPN: {}. The VPN connection may not be established yet.",
                e
            ))
        })?;

        if !response.status().is_success() {
            return Err(AppError::Internal(format!(
                "VPN API returned error status: {}. Please check your VPN configuration.",
                response.status()
            )));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to parse VPN response: {}", e)))?;

        // Extract public IP from response
        let public_ip = json
            .get("public_ip")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::Internal("VPN response missing public_ip field".to_string())
            })?;

        Ok::<String, AppError>(public_ip.to_string())
    })
    .await;

    match query_result {
        Ok(Ok(ip)) => Ok(ip),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(AppError::Internal(
            "VPN API query timed out after 30 seconds. The VPN connection may be slow or unstable."
                .to_string(),
        )),
    }
}

/// Delete a test pod
async fn delete_test_pod(k8s: &K8sClient, pod_name: &str, namespace: &str) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(k8s.client().clone(), namespace);

    match pods.delete(pod_name, &DeleteParams::default()).await {
        Ok(_) => {
            tracing::info!("Deleted test pod {}", pod_name);
            Ok(())
        }
        Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(()), // Not found is OK
        Err(e) => {
            tracing::warn!("Failed to delete test pod {}: {}", pod_name, e);
            Ok(()) // Don't fail on cleanup errors
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn validate_credentials(
    vpn_type: &vpn_provider::VpnType,
    credentials: &serde_json::Value,
) -> Result<()> {
    match vpn_type {
        vpn_provider::VpnType::WireGuard => {
            // Must have private_key
            if credentials
                .get("private_key")
                .and_then(|v| v.as_str())
                .is_none()
            {
                return Err(AppError::BadRequest(
                    "WireGuard credentials must include private_key".to_string(),
                ));
            }
        }
        vpn_provider::VpnType::OpenVpn => {
            // Must have username and password
            if credentials
                .get("username")
                .and_then(|v| v.as_str())
                .is_none()
            {
                return Err(AppError::BadRequest(
                    "OpenVPN credentials must include username".to_string(),
                ));
            }
            if credentials
                .get("password")
                .and_then(|v| v.as_str())
                .is_none()
            {
                return Err(AppError::BadRequest(
                    "OpenVPN credentials must include password".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn add_service_provider_secret_data(
    service_provider: Option<&str>,
    data: &mut BTreeMap<String, String>,
) {
    // Gluetun requires an explicit provider value, including `custom` for
    // manually supplied WireGuard endpoints.
    if let Some(service_provider) = service_provider {
        data.insert(
            "VPN_SERVICE_PROVIDER".to_string(),
            service_provider.to_string(),
        );
    }
}

fn build_wireguard_secret_data(
    credentials: &serde_json::Value,
    data: &mut BTreeMap<String, String>,
) -> Result<()> {
    // Private key (required)
    if let Some(private_key) = credentials.get("private_key").and_then(|v| v.as_str()) {
        data.insert("WIREGUARD_PRIVATE_KEY".to_string(), private_key.to_string());
    }

    // Addresses
    if let Some(addresses) = credentials.get("addresses").and_then(|v| v.as_array()) {
        let addr_str: Vec<&str> = addresses.iter().filter_map(|v| v.as_str()).collect();
        if !addr_str.is_empty() {
            data.insert("WIREGUARD_ADDRESSES".to_string(), addr_str.join(","));
        }
    }

    // Public key (for custom WireGuard)
    if let Some(public_key) = credentials.get("public_key").and_then(|v| v.as_str()) {
        data.insert("WIREGUARD_PUBLIC_KEY".to_string(), public_key.to_string());
    }

    // Endpoint
    if let Some(endpoint_ip) = credentials.get("endpoint_ip").and_then(|v| v.as_str()) {
        data.insert("VPN_ENDPOINT_IP".to_string(), endpoint_ip.to_string());
    }
    if let Some(endpoint_port) = credentials.get("endpoint_port").and_then(|v| v.as_u64()) {
        data.insert("VPN_ENDPOINT_PORT".to_string(), endpoint_port.to_string());
    }

    // Preshared key
    if let Some(preshared_key) = credentials.get("preshared_key").and_then(|v| v.as_str()) {
        data.insert(
            "WIREGUARD_PRESHARED_KEY".to_string(),
            preshared_key.to_string(),
        );
    }

    Ok(())
}

fn build_openvpn_secret_data(
    credentials: &serde_json::Value,
    data: &mut BTreeMap<String, String>,
) -> Result<()> {
    // Username and password (required)
    if let Some(username) = credentials.get("username").and_then(|v| v.as_str()) {
        data.insert("OPENVPN_USER".to_string(), username.to_string());
    }
    if let Some(password) = credentials.get("password").and_then(|v| v.as_str()) {
        data.insert("OPENVPN_PASSWORD".to_string(), password.to_string());
    }

    // Server selection
    if let Some(countries) = credentials.get("server_countries").and_then(|v| v.as_str()) {
        data.insert("SERVER_COUNTRIES".to_string(), countries.to_string());
    }
    if let Some(cities) = credentials.get("server_cities").and_then(|v| v.as_str()) {
        data.insert("SERVER_CITIES".to_string(), cities.to_string());
    }
    if let Some(hostnames) = credentials.get("server_hostnames").and_then(|v| v.as_str()) {
        data.insert("SERVER_HOSTNAMES".to_string(), hostnames.to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::vpn_provider::VpnType;
    use serde_json::json;

    // -------------------------------------------------------------------------
    // validate_credentials tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_validate_credentials_wireguard_with_private_key_ok() {
        let creds = json!({ "private_key": "wg_priv_key_base64==" });
        assert!(validate_credentials(&VpnType::WireGuard, &creds).is_ok());
    }

    #[test]
    fn test_validate_credentials_wireguard_without_private_key_err() {
        let creds = json!({ "public_key": "wg_pub_key_base64==" });
        let result = validate_credentials(&VpnType::WireGuard, &creds);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => {
                assert!(
                    msg.contains("private_key"),
                    "Expected message about private_key, got: {}",
                    msg
                );
            }
            other => panic!("Expected BadRequest, got {:?}", other),
        }
    }

    #[test]
    fn test_validate_credentials_wireguard_private_key_null_is_err() {
        // null values are treated as None by as_str()
        let creds = json!({ "private_key": null });
        let result = validate_credentials(&VpnType::WireGuard, &creds);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_credentials_openvpn_with_username_and_password_ok() {
        let creds = json!({ "username": "user1", "password": "pass1" });
        assert!(validate_credentials(&VpnType::OpenVpn, &creds).is_ok());
    }

    #[test]
    fn test_validate_credentials_openvpn_without_username_err() {
        let creds = json!({ "password": "pass1" });
        let result = validate_credentials(&VpnType::OpenVpn, &creds);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => {
                assert!(
                    msg.contains("username"),
                    "Expected message about username, got: {}",
                    msg
                );
            }
            other => panic!("Expected BadRequest, got {:?}", other),
        }
    }

    #[test]
    fn test_validate_credentials_openvpn_without_password_err() {
        let creds = json!({ "username": "user1" });
        let result = validate_credentials(&VpnType::OpenVpn, &creds);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => {
                assert!(
                    msg.contains("password"),
                    "Expected message about password, got: {}",
                    msg
                );
            }
            other => panic!("Expected BadRequest, got {:?}", other),
        }
    }

    #[test]
    fn test_validate_credentials_openvpn_empty_object_err() {
        let creds = json!({});
        let result = validate_credentials(&VpnType::OpenVpn, &creds);
        assert!(result.is_err());
    }

    #[test]
    fn test_custom_service_provider_is_added_to_secret_data() {
        let mut data = BTreeMap::new();

        add_service_provider_secret_data(Some("custom"), &mut data);

        assert_eq!(
            data.get("VPN_SERVICE_PROVIDER").map(String::as_str),
            Some("custom")
        );
    }

    #[test]
    fn test_missing_service_provider_is_not_added_to_secret_data() {
        let mut data = BTreeMap::new();

        add_service_provider_secret_data(None, &mut data);

        assert!(!data.contains_key("VPN_SERVICE_PROVIDER"));
    }

    // -------------------------------------------------------------------------
    // build_wireguard_secret_data tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_wireguard_secret_data_minimal_only_private_key() {
        let creds = json!({ "private_key": "my-wg-private-key" });
        let mut data: BTreeMap<String, String> = BTreeMap::new();
        build_wireguard_secret_data(&creds, &mut data).unwrap();

        assert_eq!(
            data.get("WIREGUARD_PRIVATE_KEY").map(String::as_str),
            Some("my-wg-private-key")
        );
        // Optional fields must NOT be present
        assert!(!data.contains_key("WIREGUARD_ADDRESSES"));
        assert!(!data.contains_key("WIREGUARD_PUBLIC_KEY"));
        assert!(!data.contains_key("VPN_ENDPOINT_IP"));
        assert!(!data.contains_key("VPN_ENDPOINT_PORT"));
        assert!(!data.contains_key("WIREGUARD_PRESHARED_KEY"));
    }

    #[test]
    fn test_build_wireguard_secret_data_full_credentials() {
        let creds = json!({
            "private_key": "priv_key",
            "addresses": ["10.0.0.1/32", "fd00::1/128"],
            "public_key": "pub_key",
            "endpoint_ip": "1.2.3.4",
            "endpoint_port": 51820u64,
            "preshared_key": "psk"
        });
        let mut data: BTreeMap<String, String> = BTreeMap::new();
        build_wireguard_secret_data(&creds, &mut data).unwrap();

        assert_eq!(
            data.get("WIREGUARD_PRIVATE_KEY").map(String::as_str),
            Some("priv_key")
        );
        assert_eq!(
            data.get("WIREGUARD_ADDRESSES").map(String::as_str),
            Some("10.0.0.1/32,fd00::1/128")
        );
        assert_eq!(
            data.get("WIREGUARD_PUBLIC_KEY").map(String::as_str),
            Some("pub_key")
        );
        assert_eq!(
            data.get("VPN_ENDPOINT_IP").map(String::as_str),
            Some("1.2.3.4")
        );
        assert_eq!(
            data.get("VPN_ENDPOINT_PORT").map(String::as_str),
            Some("51820")
        );
        assert_eq!(
            data.get("WIREGUARD_PRESHARED_KEY").map(String::as_str),
            Some("psk")
        );
    }

    #[test]
    fn test_build_wireguard_secret_data_addresses_empty_array_not_inserted() {
        // An empty addresses array should not produce the WIREGUARD_ADDRESSES key
        let creds = json!({ "private_key": "pk", "addresses": [] });
        let mut data: BTreeMap<String, String> = BTreeMap::new();
        build_wireguard_secret_data(&creds, &mut data).unwrap();

        assert!(!data.contains_key("WIREGUARD_ADDRESSES"));
    }

    #[test]
    fn test_build_wireguard_secret_data_single_address() {
        let creds = json!({ "private_key": "pk", "addresses": ["10.0.0.2/32"] });
        let mut data: BTreeMap<String, String> = BTreeMap::new();
        build_wireguard_secret_data(&creds, &mut data).unwrap();

        assert_eq!(
            data.get("WIREGUARD_ADDRESSES").map(String::as_str),
            Some("10.0.0.2/32")
        );
    }

    // -------------------------------------------------------------------------
    // build_openvpn_secret_data tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_openvpn_secret_data_minimal_username_and_password() {
        let creds = json!({ "username": "vpnuser", "password": "s3cr3t" });
        let mut data: BTreeMap<String, String> = BTreeMap::new();
        build_openvpn_secret_data(&creds, &mut data).unwrap();

        assert_eq!(
            data.get("OPENVPN_USER").map(String::as_str),
            Some("vpnuser")
        );
        assert_eq!(
            data.get("OPENVPN_PASSWORD").map(String::as_str),
            Some("s3cr3t")
        );
        // Optional server selection keys must NOT be present
        assert!(!data.contains_key("SERVER_COUNTRIES"));
        assert!(!data.contains_key("SERVER_CITIES"));
        assert!(!data.contains_key("SERVER_HOSTNAMES"));
    }

    #[test]
    fn test_build_openvpn_secret_data_full_credentials() {
        let creds = json!({
            "username": "vpnuser",
            "password": "s3cr3t",
            "server_countries": "Netherlands,Germany",
            "server_cities": "Amsterdam",
            "server_hostnames": "nl1.vpn.example.com"
        });
        let mut data: BTreeMap<String, String> = BTreeMap::new();
        build_openvpn_secret_data(&creds, &mut data).unwrap();

        assert_eq!(
            data.get("OPENVPN_USER").map(String::as_str),
            Some("vpnuser")
        );
        assert_eq!(
            data.get("OPENVPN_PASSWORD").map(String::as_str),
            Some("s3cr3t")
        );
        assert_eq!(
            data.get("SERVER_COUNTRIES").map(String::as_str),
            Some("Netherlands,Germany")
        );
        assert_eq!(
            data.get("SERVER_CITIES").map(String::as_str),
            Some("Amsterdam")
        );
        assert_eq!(
            data.get("SERVER_HOSTNAMES").map(String::as_str),
            Some("nl1.vpn.example.com")
        );
    }

    #[test]
    fn test_build_openvpn_secret_data_missing_username_produces_no_key() {
        // build_openvpn_secret_data itself does NOT validate — it silently skips
        // missing fields (validation is separate via validate_credentials).
        let creds = json!({ "password": "s3cr3t" });
        let mut data: BTreeMap<String, String> = BTreeMap::new();
        build_openvpn_secret_data(&creds, &mut data).unwrap();

        assert!(!data.contains_key("OPENVPN_USER"));
        assert_eq!(
            data.get("OPENVPN_PASSWORD").map(String::as_str),
            Some("s3cr3t")
        );
    }

    #[test]
    fn test_build_openvpn_secret_data_partial_server_selection() {
        // Only server_countries provided, others absent
        let creds = json!({
            "username": "user",
            "password": "pass",
            "server_countries": "US"
        });
        let mut data: BTreeMap<String, String> = BTreeMap::new();
        build_openvpn_secret_data(&creds, &mut data).unwrap();

        assert_eq!(data.get("SERVER_COUNTRIES").map(String::as_str), Some("US"));
        assert!(!data.contains_key("SERVER_CITIES"));
        assert!(!data.contains_key("SERVER_HOSTNAMES"));
    }
}

/// Get list of supported VPN service providers
pub fn get_supported_providers() -> Vec<SupportedProvider> {
    vec![
        SupportedProvider {
            id: "custom",
            name: "Custom",
            vpn_types: vec!["wireguard", "openvpn"],
            description: "Custom WireGuard or OpenVPN configuration",
            supports_port_forwarding: true,
        },
        SupportedProvider {
            id: "airvpn",
            name: "AirVPN",
            vpn_types: vec!["wireguard", "openvpn"],
            description: "Privacy-focused VPN with WireGuard and OpenVPN",
            supports_port_forwarding: true,
        },
        SupportedProvider {
            id: "expressvpn",
            name: "ExpressVPN",
            vpn_types: vec!["openvpn"],
            description: "Fast VPN with servers in 94 countries",
            supports_port_forwarding: false,
        },
        SupportedProvider {
            id: "ipvanish",
            name: "IPVanish",
            vpn_types: vec!["openvpn"],
            description: "VPN with configurable encryption",
            supports_port_forwarding: false,
        },
        SupportedProvider {
            id: "mullvad",
            name: "Mullvad",
            vpn_types: vec!["wireguard", "openvpn"],
            description: "Privacy-first VPN, no email required",
            supports_port_forwarding: false,
        },
        SupportedProvider {
            id: "nordvpn",
            name: "NordVPN",
            vpn_types: vec!["wireguard", "openvpn"],
            description: "Popular VPN with NordLynx (WireGuard)",
            supports_port_forwarding: false,
        },
        SupportedProvider {
            id: "private_internet_access",
            name: "Private Internet Access (PIA)",
            vpn_types: vec!["wireguard", "openvpn"],
            description: "Proven no-logs VPN",
            supports_port_forwarding: true,
        },
        SupportedProvider {
            id: "protonvpn",
            name: "ProtonVPN",
            vpn_types: vec!["wireguard", "openvpn"],
            description: "Swiss-based privacy VPN",
            supports_port_forwarding: true,
        },
        SupportedProvider {
            id: "surfshark",
            name: "Surfshark",
            vpn_types: vec!["wireguard", "openvpn"],
            description: "Unlimited device connections",
            supports_port_forwarding: false,
        },
        SupportedProvider {
            id: "windscribe",
            name: "Windscribe",
            vpn_types: vec!["wireguard", "openvpn"],
            description: "VPN with built-in ad blocker",
            supports_port_forwarding: false,
        },
    ]
}

#[cfg(test)]
mod tests_supported_providers {
    use super::*;

    #[test]
    fn get_supported_providers_returns_nonempty_list() {
        let providers = get_supported_providers();
        assert!(!providers.is_empty());
    }

    #[test]
    fn get_supported_providers_contains_custom() {
        let providers = get_supported_providers();
        assert!(providers.iter().any(|p| p.id == "custom"));
    }

    #[test]
    fn get_supported_providers_all_have_vpn_types() {
        let providers = get_supported_providers();
        for p in &providers {
            assert!(
                !p.vpn_types.is_empty(),
                "provider {} has no vpn_types",
                p.id
            );
        }
    }

    #[test]
    fn get_supported_providers_wireguard_and_openvpn_valid_types() {
        let providers = get_supported_providers();
        for p in &providers {
            for vt in &p.vpn_types {
                assert!(
                    *vt == "wireguard" || *vt == "openvpn",
                    "unexpected vpn_type '{}' for provider '{}'",
                    vt,
                    p.id
                );
            }
        }
    }

    #[test]
    fn default_true_returns_true() {
        assert!(default_true());
    }

    #[test]
    fn default_firewall_subnets_returns_private_ranges() {
        let s = default_firewall_subnets();
        assert!(s.contains("10.0.0.0/8"));
        assert!(s.contains("172.16.0.0/12"));
        assert!(s.contains("192.168.0.0/16"));
    }
}

#[cfg(test)]
mod tests_deployment_structs {
    use super::*;

    #[test]
    fn vpn_deployment_config_ser() {
        let c = VpnDeploymentConfig {
            enabled: true,
            secret_name: "my-secret".to_string(),
            kill_switch: true,
            firewall_outbound_subnets: "10.0.0.0/8".to_string(),
            port_forwarding: false,
        };
        let json = serde_json::to_string(&c).expect("ser");
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"secret_name\":\"my-secret\""));
    }

    #[test]
    fn vpn_deployment_config_clone() {
        let c = VpnDeploymentConfig {
            enabled: false,
            secret_name: "s".to_string(),
            kill_switch: false,
            firewall_outbound_subnets: "0.0.0.0/0".to_string(),
            port_forwarding: true,
        };
        let c2 = c.clone();
        assert_eq!(c.enabled, c2.enabled);
        assert_eq!(c.secret_name, c2.secret_name);
    }
}

#[cfg(test)]
mod tests_request_response_serde {
    use super::*;
    use chrono::Utc;

    // --- WireGuardCredentials ---

    #[test]
    fn wireguard_credentials_deser_minimal() {
        let r: WireGuardCredentials =
            serde_json::from_str(r#"{"private_key":"wgprivkey=="}"#).expect("deser");
        assert_eq!(r.private_key, "wgprivkey==");
        assert!(r.addresses.is_empty());
        assert!(r.public_key.is_none());
    }

    #[test]
    fn wireguard_credentials_deser_full() {
        let json = r#"{
            "private_key":"priv==",
            "addresses":["10.0.0.1/24"],
            "public_key":"pub==",
            "endpoint_ip":"1.2.3.4",
            "endpoint_port":51820,
            "preshared_key":"psk=="
        }"#;
        let r: WireGuardCredentials = serde_json::from_str(json).expect("deser");
        assert_eq!(r.private_key, "priv==");
        assert_eq!(r.addresses, vec!["10.0.0.1/24"]);
        assert_eq!(r.public_key.as_deref(), Some("pub=="));
        assert_eq!(r.endpoint_port, Some(51820));
    }

    #[test]
    fn wireguard_credentials_ser_skips_none_fields() {
        let r = WireGuardCredentials {
            private_key: "priv==".to_string(),
            addresses: vec![],
            public_key: None,
            endpoint_ip: None,
            endpoint_port: None,
            preshared_key: None,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"private_key\":\"priv==\""));
        assert!(!json.contains("public_key"));
        assert!(!json.contains("preshared_key"));
    }

    // --- OpenVpnCredentials ---

    #[test]
    fn openvpn_credentials_deser_minimal() {
        let r: OpenVpnCredentials =
            serde_json::from_str(r#"{"username":"user","password":"pass"}"#).expect("deser");
        assert_eq!(r.username, "user");
        assert_eq!(r.password, "pass");
        assert!(r.server_countries.is_none());
    }

    #[test]
    fn openvpn_credentials_deser_full() {
        let json = r#"{"username":"u","password":"p","server_countries":"US","server_cities":"NYC","server_hostnames":"vpn.example.com"}"#;
        let r: OpenVpnCredentials = serde_json::from_str(json).expect("deser");
        assert_eq!(r.server_countries.as_deref(), Some("US"));
        assert_eq!(r.server_cities.as_deref(), Some("NYC"));
    }

    // --- VpnCredentials (untagged enum) ---

    #[test]
    fn vpn_credentials_wireguard_variant() {
        let json = r#"{"private_key":"wgkey=="}"#;
        let r: VpnCredentials = serde_json::from_str(json).expect("deser");
        match r {
            VpnCredentials::WireGuard(wg) => assert_eq!(wg.private_key, "wgkey=="),
            VpnCredentials::OpenVpn(_) => panic!("Expected WireGuard variant"),
        }
    }

    #[test]
    fn vpn_credentials_openvpn_variant() {
        let json = r#"{"username":"user","password":"pass"}"#;
        let r: VpnCredentials = serde_json::from_str(json).expect("deser");
        match r {
            VpnCredentials::OpenVpn(ov) => assert_eq!(ov.username, "user"),
            VpnCredentials::WireGuard(_) => panic!("Expected OpenVpn variant"),
        }
    }

    // --- CreateVpnProviderRequest ---

    #[test]
    fn create_vpn_provider_request_defaults() {
        let json =
            r#"{"name":"myvpn","vpn_type":"wireguard","credentials":{"private_key":"pk=="}}"#;
        let r: CreateVpnProviderRequest = serde_json::from_str(json).expect("deser");
        assert_eq!(r.name, "myvpn");
        assert!(r.enabled);
        assert!(r.kill_switch);
        assert!(!r.firewall_outbound_subnets.is_empty());
    }

    #[test]
    fn create_vpn_provider_request_custom() {
        let json = r#"{"name":"myvpn","vpn_type":"openvpn","credentials":{},"enabled":false,"kill_switch":false,"firewall_outbound_subnets":"10.0.0.0/8"}"#;
        let r: CreateVpnProviderRequest = serde_json::from_str(json).expect("deser");
        assert!(!r.enabled);
        assert!(!r.kill_switch);
        assert_eq!(r.firewall_outbound_subnets, "10.0.0.0/8");
    }

    // --- UpdateVpnProviderRequest ---

    #[test]
    fn update_vpn_provider_request_empty() {
        let r: UpdateVpnProviderRequest = serde_json::from_str("{}").expect("deser");
        assert!(r.name.is_none());
        assert!(r.credentials.is_none());
        assert!(r.enabled.is_none());
    }

    #[test]
    fn update_vpn_provider_request_partial() {
        let r: UpdateVpnProviderRequest =
            serde_json::from_str(r#"{"name":"newname","enabled":true}"#).expect("deser");
        assert_eq!(r.name.as_deref(), Some("newname"));
        assert_eq!(r.enabled, Some(true));
        assert!(r.kill_switch.is_none());
    }

    // --- AssignVpnRequest ---

    #[test]
    fn assign_vpn_request_minimal() {
        let r: AssignVpnRequest = serde_json::from_str(r#"{"vpn_provider_id":5}"#).expect("deser");
        assert_eq!(r.vpn_provider_id, 5);
        assert!(r.kill_switch_override.is_none());
        assert!(r.port_forwarding.is_none());
    }

    #[test]
    fn assign_vpn_request_full() {
        let r: AssignVpnRequest = serde_json::from_str(
            r#"{"vpn_provider_id":3,"kill_switch_override":true,"port_forwarding":true}"#,
        )
        .expect("deser");
        assert_eq!(r.kill_switch_override, Some(true));
        assert_eq!(r.port_forwarding, Some(true));
    }

    // --- VpnProviderResponse ---

    #[test]
    fn vpn_provider_response_ser() {
        use crate::models::vpn_provider::VpnType;
        let r = VpnProviderResponse {
            id: 1,
            name: "MyVPN".to_string(),
            vpn_type: VpnType::WireGuard,
            service_provider: Some("mullvad".to_string()),
            enabled: true,
            kill_switch: false,
            firewall_outbound_subnets: "10.0.0.0/8".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            app_count: 2,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"name\":\"MyVPN\""));
        assert!(json.contains("\"app_count\":2"));
    }

    // --- AppVpnConfigResponse ---

    #[test]
    fn app_vpn_config_response_ser() {
        let r = AppVpnConfigResponse {
            app_name: "sonarr".to_string(),
            vpn_provider_id: 1,
            vpn_provider_name: "MyVPN".to_string(),
            kill_switch_override: None,
            effective_kill_switch: true,
            port_forwarding: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"app_name\":\"sonarr\""));
        assert!(json.contains("\"effective_kill_switch\":true"));
    }

    // --- VpnTestResult ---

    #[test]
    fn vpn_test_result_success_ser() {
        let r = VpnTestResult {
            success: true,
            message: "Connected".to_string(),
            public_ip: Some("1.2.3.4".to_string()),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"public_ip\":\"1.2.3.4\""));
    }

    #[test]
    fn vpn_test_result_failure_ser() {
        let r = VpnTestResult {
            success: false,
            message: "Timeout".to_string(),
            public_ip: None,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"success\":false"));
    }
}

// ============================================================================
// DB-based tests (in-memory SQLite)
// ============================================================================

#[cfg(test)]
mod tests_db {
    use super::*;
    use crate::models::vpn_provider::VpnType;

    async fn make_db() -> DbConn {
        crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory SQLite")
    }

    async fn insert_wg_provider(db: &DbConn, name: &str) -> VpnProviderResponse {
        let req = CreateVpnProviderRequest {
            name: name.to_string(),
            vpn_type: VpnType::WireGuard,
            service_provider: Some("custom".to_string()),
            credentials: serde_json::json!({"private_key": "wgprivkey=="}),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: default_firewall_subnets(),
        };
        create_vpn_provider(db, req).await.expect("create provider")
    }

    // -------------------------------------------------------------------------
    // list_vpn_providers
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn list_vpn_providers_empty_db() {
        let db = make_db().await;
        let result = list_vpn_providers(&db).await.expect("list");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn list_vpn_providers_with_data() {
        let db = make_db().await;
        insert_wg_provider(&db, "myvpn").await;
        let result = list_vpn_providers(&db).await.expect("list");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "myvpn");
        assert_eq!(result[0].vpn_type, VpnType::WireGuard);
        assert_eq!(result[0].app_count, 0);
    }

    #[tokio::test]
    async fn list_vpn_providers_multiple() {
        let db = make_db().await;
        insert_wg_provider(&db, "vpn1").await;
        insert_wg_provider(&db, "vpn2").await;
        let result = list_vpn_providers(&db).await.expect("list");
        assert_eq!(result.len(), 2);
    }

    // -------------------------------------------------------------------------
    // get_vpn_provider
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn get_vpn_provider_not_found() {
        let db = make_db().await;
        let result = get_vpn_provider(&db, 999).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found") || err.to_string().contains("999"));
    }

    #[tokio::test]
    async fn get_vpn_provider_found() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;
        let result = get_vpn_provider(&db, provider.id).await.expect("get");
        assert_eq!(result.id, provider.id);
        assert_eq!(result.name, "testvpn");
        assert_eq!(result.app_count, 0);
    }

    // -------------------------------------------------------------------------
    // create_vpn_provider
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn create_vpn_provider_success_wireguard() {
        let db = make_db().await;
        let req = CreateVpnProviderRequest {
            name: "wg-provider".to_string(),
            vpn_type: VpnType::WireGuard,
            service_provider: None,
            credentials: serde_json::json!({"private_key": "privkey=="}),
            enabled: true,
            kill_switch: false,
            firewall_outbound_subnets: "10.0.0.0/8".to_string(),
        };
        let result = create_vpn_provider(&db, req).await.expect("create");
        assert!(result.id > 0);
        assert_eq!(result.name, "wg-provider");
        assert_eq!(result.app_count, 0);
        assert!(!result.kill_switch);
    }

    #[tokio::test]
    async fn create_vpn_provider_success_openvpn() {
        let db = make_db().await;
        let req = CreateVpnProviderRequest {
            name: "ovpn-provider".to_string(),
            vpn_type: VpnType::OpenVpn,
            service_provider: Some("nordvpn".to_string()),
            credentials: serde_json::json!({"username": "user", "password": "pass"}),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: default_firewall_subnets(),
        };
        let result = create_vpn_provider(&db, req).await.expect("create");
        assert!(result.id > 0);
        assert_eq!(result.vpn_type, VpnType::OpenVpn);
        assert_eq!(result.service_provider.as_deref(), Some("nordvpn"));
    }

    #[tokio::test]
    async fn create_vpn_provider_invalid_credentials_returns_err() {
        let db = make_db().await;
        let req = CreateVpnProviderRequest {
            name: "bad-creds".to_string(),
            vpn_type: VpnType::WireGuard,
            service_provider: None,
            // WireGuard requires private_key
            credentials: serde_json::json!({"username": "user"}),
            enabled: true,
            kill_switch: false,
            firewall_outbound_subnets: "".to_string(),
        };
        let result = create_vpn_provider(&db, req).await;
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // update_vpn_provider
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn update_vpn_provider_name() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "original").await;
        let req = UpdateVpnProviderRequest {
            name: Some("updated".to_string()),
            service_provider: None,
            credentials: None,
            enabled: None,
            kill_switch: None,
            firewall_outbound_subnets: None,
        };
        let result = update_vpn_provider(&db, provider.id, req)
            .await
            .expect("update");
        assert_eq!(result.name, "updated");
    }

    #[tokio::test]
    async fn update_vpn_provider_not_found() {
        let db = make_db().await;
        let req = UpdateVpnProviderRequest {
            name: Some("whatever".to_string()),
            service_provider: None,
            credentials: None,
            enabled: None,
            kill_switch: None,
            firewall_outbound_subnets: None,
        };
        let result = update_vpn_provider(&db, 999, req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_vpn_provider_multiple_fields() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "myvpn").await;
        let req = UpdateVpnProviderRequest {
            name: None,
            service_provider: Some("protonvpn".to_string()),
            credentials: None,
            enabled: Some(false),
            kill_switch: Some(false),
            firewall_outbound_subnets: Some("192.168.0.0/16".to_string()),
        };
        let result = update_vpn_provider(&db, provider.id, req)
            .await
            .expect("update");
        assert!(!result.enabled);
        assert!(!result.kill_switch);
        assert_eq!(result.firewall_outbound_subnets, "192.168.0.0/16");
    }

    // -------------------------------------------------------------------------
    // list_app_vpn_configs
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn list_app_vpn_configs_empty_db() {
        let db = make_db().await;
        let result = list_app_vpn_configs(&db).await.expect("list");
        assert!(result.is_empty());
    }

    // -------------------------------------------------------------------------
    // get_app_vpn_config
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn get_app_vpn_config_not_found_returns_none() {
        let db = make_db().await;
        let result = get_app_vpn_config(&db, "nonexistent").await.expect("get");
        assert!(result.is_none());
    }

    // -------------------------------------------------------------------------
    // assign_vpn_to_app
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn assign_vpn_to_app_success() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        let req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: None,
        };
        let result = assign_vpn_to_app(&db, "sonarr", req).await.expect("assign");
        assert_eq!(result.app_name, "sonarr");
        assert_eq!(result.vpn_provider_id, provider.id);
        assert!(result.effective_kill_switch); // provider kill_switch=true
    }

    #[tokio::test]
    async fn assign_vpn_to_app_provider_not_found() {
        let db = make_db().await;
        let req = AssignVpnRequest {
            vpn_provider_id: 999,
            kill_switch_override: None,
            port_forwarding: None,
        };
        let result = assign_vpn_to_app(&db, "sonarr", req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn assign_vpn_to_app_disabled_provider_returns_err() {
        let db = make_db().await;
        let req_create = CreateVpnProviderRequest {
            name: "disabled-vpn".to_string(),
            vpn_type: VpnType::WireGuard,
            service_provider: None,
            credentials: serde_json::json!({"private_key": "pk=="}),
            enabled: false, // disabled
            kill_switch: false,
            firewall_outbound_subnets: default_firewall_subnets(),
        };
        let provider = create_vpn_provider(&db, req_create).await.expect("create");

        let req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: None,
        };
        let result = assign_vpn_to_app(&db, "sonarr", req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn assign_vpn_to_app_update_existing() {
        let db = make_db().await;
        let provider1 = insert_wg_provider(&db, "vpn1").await;
        let provider2 = insert_wg_provider(&db, "vpn2").await;

        // Assign provider1
        let req1 = AssignVpnRequest {
            vpn_provider_id: provider1.id,
            kill_switch_override: None,
            port_forwarding: None,
        };
        assign_vpn_to_app(&db, "radarr", req1)
            .await
            .expect("assign1");

        // Update to provider2
        let req2 = AssignVpnRequest {
            vpn_provider_id: provider2.id,
            kill_switch_override: Some(false),
            port_forwarding: Some(true),
        };
        let result = assign_vpn_to_app(&db, "radarr", req2)
            .await
            .expect("assign2");
        assert_eq!(result.vpn_provider_id, provider2.id);
        assert_eq!(result.kill_switch_override, Some(false));
        assert!(result.port_forwarding);

        // Verify in DB
        let config = get_app_vpn_config(&db, "radarr").await.expect("get");
        assert!(config.is_some());
        assert_eq!(config.unwrap().vpn_provider_id, provider2.id);
    }

    #[tokio::test]
    async fn get_app_vpn_config_after_assign() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        let req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: Some(false),
            port_forwarding: Some(false),
        };
        assign_vpn_to_app(&db, "lidarr", req).await.expect("assign");

        let result = get_app_vpn_config(&db, "lidarr").await.expect("get");
        assert!(result.is_some());
        let config = result.unwrap();
        assert_eq!(config.app_name, "lidarr");
        assert_eq!(config.kill_switch_override, Some(false));
        assert!(!config.effective_kill_switch);
    }

    #[tokio::test]
    async fn list_app_vpn_configs_with_data() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        // Assign to two apps
        for app_name in &["sonarr", "radarr"] {
            let req = AssignVpnRequest {
                vpn_provider_id: provider.id,
                kill_switch_override: None,
                port_forwarding: None,
            };
            assign_vpn_to_app(&db, app_name, req).await.expect("assign");
        }

        let result = list_app_vpn_configs(&db).await.expect("list");
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn list_vpn_providers_shows_app_count() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        // Assign to two apps
        for app_name in &["sonarr", "radarr"] {
            let req = AssignVpnRequest {
                vpn_provider_id: provider.id,
                kill_switch_override: None,
                port_forwarding: None,
            };
            assign_vpn_to_app(&db, app_name, req).await.expect("assign");
        }

        let result = list_vpn_providers(&db).await.expect("list");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].app_count, 2);
    }

    // -------------------------------------------------------------------------
    // update_vpn_provider — credentials path
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn update_vpn_provider_credentials_path() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        let req = UpdateVpnProviderRequest {
            name: None,
            service_provider: None,
            credentials: Some(serde_json::json!({"private_key": "new_private_key=="})),
            enabled: None,
            kill_switch: None,
            firewall_outbound_subnets: None,
        };

        let result = update_vpn_provider(&db, provider.id, req)
            .await
            .expect("update");
        assert_eq!(result.id, provider.id);
        assert_eq!(result.name, "testvpn");
    }

    #[tokio::test]
    async fn update_vpn_provider_invalid_credentials_in_update_returns_err() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        // WireGuard requires private_key — updating with missing key should fail
        let req = UpdateVpnProviderRequest {
            name: None,
            service_provider: None,
            credentials: Some(serde_json::json!({"public_key": "some_pub_key"})),
            enabled: None,
            kill_switch: None,
            firewall_outbound_subnets: None,
        };

        let result = update_vpn_provider(&db, provider.id, req).await;
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // get_vpn_deployment_config
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn get_vpn_deployment_config_no_config_returns_none() {
        let db = make_db().await;
        let result = get_vpn_deployment_config(&db, "sonarr").await.expect("get");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_vpn_deployment_config_with_enabled_provider_returns_config() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        let assign_req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: None,
        };
        assign_vpn_to_app(&db, "sonarr", assign_req)
            .await
            .expect("assign");

        let result = get_vpn_deployment_config(&db, "sonarr").await.expect("get");
        assert!(result.is_some());
        let config = result.unwrap();
        assert!(config.enabled);
        assert_eq!(config.secret_name, "vpn-sonarr");
        assert!(config.kill_switch); // provider has kill_switch=true
        assert!(!config.port_forwarding);
    }

    #[tokio::test]
    async fn get_vpn_deployment_config_kill_switch_override() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        // Assign with kill_switch_override=false (overrides provider's true)
        let assign_req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: Some(false),
            port_forwarding: None,
        };
        assign_vpn_to_app(&db, "sonarr", assign_req)
            .await
            .expect("assign");

        let result = get_vpn_deployment_config(&db, "sonarr").await.expect("get");
        assert!(result.is_some());
        let config = result.unwrap();
        assert!(!config.kill_switch); // overridden to false
    }

    #[tokio::test]
    async fn get_vpn_deployment_config_disabled_provider_returns_none() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        let assign_req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: None,
        };
        assign_vpn_to_app(&db, "sonarr", assign_req)
            .await
            .expect("assign");

        // Disable the provider
        let update = UpdateVpnProviderRequest {
            name: None,
            service_provider: None,
            credentials: None,
            enabled: Some(false),
            kill_switch: None,
            firewall_outbound_subnets: None,
        };
        update_vpn_provider(&db, provider.id, update)
            .await
            .expect("disable");

        let result = get_vpn_deployment_config(&db, "sonarr").await.expect("get");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_vpn_deployment_config_port_forwarding_enabled() {
        let db = make_db().await;
        let provider = insert_wg_provider(&db, "testvpn").await;

        let assign_req = AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: Some(true),
        };
        assign_vpn_to_app(&db, "sonarr", assign_req)
            .await
            .expect("assign");

        let result = get_vpn_deployment_config(&db, "sonarr").await.expect("get");
        let config = result.expect("should have config");
        assert!(config.port_forwarding);
    }

    #[tokio::test]
    async fn get_app_vpn_config_orphaned_config_returns_error() {
        use crate::models::{app_vpn_config, vpn_provider};
        use chrono::Utc;
        use sea_orm::{
            ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, Set, Statement,
        };
        use sea_orm_migration::MigratorTrait;

        // Use a single-connection pool to ensure FK PRAGMA works consistently
        let mut opts = ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1)
            .min_connections(1)
            .sqlx_logging(false);
        let db = Database::connect(opts).await.expect("connect");
        crate::migrations::Migrator::up(&db, None)
            .await
            .expect("migrate");

        let now = Utc::now();

        // Create a provider first to satisfy FK constraint
        let provider = vpn_provider::ActiveModel {
            name: Set("temp_provider".to_string()),
            vpn_type: Set(crate::models::vpn_provider::VpnType::WireGuard),
            service_provider: Set(None),
            credentials_json: Set(r#"{"private_key":"wgk=="}"#.to_string()),
            enabled: Set(true),
            kill_switch: Set(false),
            firewall_outbound_subnets: Set("10.0.0.0/8".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert provider");

        // Insert app config referencing this provider
        app_vpn_config::ActiveModel {
            app_name: Set("orphan_app".to_string()),
            vpn_provider_id: Set(provider.id),
            kill_switch_override: Set(None),
            port_forwarding: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert app config");

        // Disable FK checks temporarily to delete provider and create orphan
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA foreign_keys = OFF".to_string(),
        ))
        .await
        .expect("disable FK");

        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            format!("DELETE FROM vpn_providers WHERE id = {}", provider.id),
        ))
        .await
        .expect("delete provider");

        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA foreign_keys = ON".to_string(),
        ))
        .await
        .expect("re-enable FK");

        let result = get_app_vpn_config(&db, "orphan_app").await;
        assert!(result.is_err(), "expected error for orphaned config");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("not found")
                || err.to_string().contains(&provider.id.to_string()),
            "unexpected error: {}",
            err
        );
    }
}
