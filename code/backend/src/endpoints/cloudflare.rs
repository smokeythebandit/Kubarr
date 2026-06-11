//! Cloudflare Tunnel configuration endpoints

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};

use crate::error::Result;
use crate::middleware::permissions::{Authorized, CloudflareManage, CloudflareView};
use crate::services::cloudflare::{
    self, CloudflareTunnelResponse, CloudflareTunnelStatus, ProvisionRequest, ValidateTokenRequest,
    ValidateTokenResponse,
};
use crate::state::AppState;

/// Create the Cloudflare routes
pub fn cloudflare_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/config",
            get(get_config).put(save_config).delete(delete_config),
        )
        .route("/status", get(get_status))
        .route("/validate-token", post(validate_token))
        .with_state(state)
}

// ============================================================================
// Handlers
// ============================================================================

/// Get Cloudflare tunnel configuration (secrets masked)
#[utoipa::path(
    get,
    path = "/api/cloudflare/config",
    tag = "Cloudflare",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn get_config(
    State(state): State<AppState>,
    _auth: Authorized<CloudflareView>,
) -> Result<Json<Option<CloudflareTunnelResponse>>> {
    let db = state.get_db().await?;
    let config = cloudflare::get_config(&db).await?;
    Ok(Json(config))
}

/// Provision a Cloudflare Tunnel via the guided wizard
#[utoipa::path(
    put,
    path = "/api/cloudflare/config",
    tag = "Cloudflare",
    request_body = serde_json::Value,
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn save_config(
    State(state): State<AppState>,
    _auth: Authorized<CloudflareManage>,
    Json(req): Json<ProvisionRequest>,
) -> Result<Json<CloudflareTunnelResponse>> {
    let db = state.get_db().await?;
    let k8s = state.k8s_client.read().await;
    let client = k8s.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("Kubernetes client not available".to_string())
    })?;
    let result = cloudflare::save_config(&db, client, req).await?;
    Ok(Json(result))
}

/// Delete Cloudflare tunnel configuration and uninstall cloudflared
#[utoipa::path(
    delete,
    path = "/api/cloudflare/config",
    tag = "Cloudflare",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn delete_config(
    State(state): State<AppState>,
    _auth: Authorized<CloudflareManage>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let k8s = state.k8s_client.read().await;
    let client = k8s.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("Kubernetes client not available".to_string())
    })?;
    cloudflare::delete_config(&db, client).await?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// Get real-time pod status for the cloudflared deployment
#[utoipa::path(
    get,
    path = "/api/cloudflare/status",
    tag = "Cloudflare",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn get_status(
    State(state): State<AppState>,
    _auth: Authorized<CloudflareView>,
) -> Result<Json<CloudflareTunnelStatus>> {
    let k8s = state.k8s_client.read().await;
    let client = k8s.as_ref().ok_or_else(|| {
        crate::error::AppError::Internal("Kubernetes client not available".to_string())
    })?;
    let status = cloudflare::get_status(client).await?;
    Ok(Json(status))
}

/// Validate a Cloudflare API token and list accessible zones
#[utoipa::path(
    post,
    path = "/api/cloudflare/validate-token",
    tag = "Cloudflare",
    request_body = serde_json::Value,
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn validate_token(
    _auth: Authorized<CloudflareManage>,
    Json(req): Json<ValidateTokenRequest>,
) -> Result<Json<ValidateTokenResponse>> {
    let result = cloudflare::validate_and_list_zones(&req.api_token).await?;
    Ok(Json(result))
}
