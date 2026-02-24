//! VPN provider and app VPN configuration endpoints

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

use crate::error::Result;
use crate::middleware::permissions::{Authorized, VpnManage, VpnView};
use crate::services::deployment::{DeploymentManager, DeploymentRequest};
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
    _auth: Authorized<VpnManage>,
    Json(req): Json<CreateVpnProviderRequest>,
) -> Result<Json<VpnProviderResponse>> {
    let db = state.get_db().await?;
    let provider = vpn::create_vpn_provider(&db, req).await?;
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
    _auth: Authorized<VpnManage>,
    Json(req): Json<UpdateVpnProviderRequest>,
) -> Result<Json<VpnProviderResponse>> {
    let db = state.get_db().await?;
    let provider = vpn::update_vpn_provider(&db, id, req).await?;
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
    _auth: Authorized<VpnManage>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let k8s = state.k8s_client.read().await;
    let client = k8s.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("Kubernetes client not available".to_string())
    })?;
    vpn::delete_vpn_provider(&db, client, id).await?;
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
    _auth: Authorized<VpnManage>,
    Json(req): Json<AssignVpnRequest>,
) -> Result<Json<AppVpnConfigResponse>> {
    let db = state.get_db().await?;
    // Save VPN config to database
    let config = vpn::assign_vpn_to_app(&db, &app_name, req).await?;

    // Trigger redeploy to apply VPN changes
    let k8s = state.k8s_client.read().await;
    if let Some(k8s_client) = k8s.as_ref() {
        let catalog = state.catalog.read().await;
        let deployment_manager = DeploymentManager::with_db(k8s_client, &catalog, &db);
        let deploy_request = DeploymentRequest {
            app_name: app_name.clone(),
            custom_config: std::collections::HashMap::new(),
        };
        match deployment_manager
            .deploy_app(&deploy_request, None, None)
            .await
        {
            Ok(status) => {
                tracing::info!("Redeployed app {} with VPN: {}", app_name, status.message);
            }
            Err(e) => {
                tracing::warn!("Failed to redeploy app {} with VPN: {}", app_name, e);
            }
        }
    }

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
    _auth: Authorized<VpnManage>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let k8s = state.k8s_client.read().await;
    let client = k8s.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("Kubernetes client not available".to_string())
    })?;

    // Remove VPN config from database
    vpn::remove_vpn_from_app(&db, client, &app_name).await?;

    // Trigger redeploy to remove VPN sidecar
    let catalog = state.catalog.read().await;
    let deployment_manager = DeploymentManager::with_db(client, &catalog, &db);
    let deploy_request = DeploymentRequest {
        app_name: app_name.clone(),
        custom_config: std::collections::HashMap::new(),
    };
    match deployment_manager
        .deploy_app(&deploy_request, None, None)
        .await
    {
        Ok(status) => {
            tracing::info!(
                "Redeployed app {} without VPN: {}",
                app_name,
                status.message
            );
        }
        Err(e) => {
            tracing::warn!("Failed to redeploy app {} without VPN: {}", app_name, e);
        }
    }

    Ok(Json(serde_json::json!({ "success": true })))
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
    use crate::services::vpn::{AppVpnConfigResponse, SupportedProvider, VpnProviderResponse};
    use chrono::Utc;

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
