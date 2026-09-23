//! VPN provider and app VPN configuration endpoints

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use sea_orm::{EntityTrait, TransactionTrait};
use serde::Serialize;

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
    let k8s = state.k8s_client.read().await;
    let k8s_client = k8s.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("Kubernetes client not available".to_string())
    })?;

    // Find pods in the app's namespace with the gluetun container
    let pods: kube::api::Api<k8s_openapi::api::core::v1::Pod> =
        kube::api::Api::namespaced(k8s_client.client().clone(), &app_name);
    let pod_list = pods
        .list(&kube::api::ListParams::default())
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Failed to list pods: {}", e)))?;

    // Find a running pod with a gluetun container
    let pod_ip = pod_list
        .items
        .iter()
        .find_map(|pod| {
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
            status.pod_ip.clone()
        })
        .ok_or_else(|| {
            crate::error::AppError::NotFound(format!(
                "No running pod with VPN found for app '{}'",
                app_name
            ))
        })?;

    // Query Gluetun control API for forwarded port
    let url = format!("http://{}:8001/v1/openvpn/portforwarded", pod_ip);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| {
            crate::error::AppError::Internal(format!("Failed to create HTTP client: {}", e))
        })?;

    match client.get(&url).send().await {
        Ok(resp) => {
            let body: serde_json::Value = resp
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({ "port": 0 }));
            let port = body.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
            Ok(Json(serde_json::json!({ "port": port })))
        }
        Err(_) => Ok(Json(serde_json::json!({ "port": 0 }))),
    }
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
}
