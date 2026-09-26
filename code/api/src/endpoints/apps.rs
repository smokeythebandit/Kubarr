use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use std::collections::HashMap;

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::middleware::permissions::{
    AppsDelete, AppsInstall, AppsRestart, AppsView, Authenticated, Authorized,
};
use crate::models::audit_log::AuditAction;
use crate::services::gpu::{self, GpuNode, GpuSelection};
use crate::services::{
    AppConfig, AppManager, AppOperationResponse, AppStateResponse, DeploymentManager,
    DeploymentRequest, OP_DELETE, OP_INSTALL, OP_RESTART, OP_UPDATE,
};
use crate::state::AppState;

/// Create apps routes
pub fn apps_routes(state: AppState) -> Router {
    Router::new()
        .route("/catalog", get(list_catalog))
        .route("/catalog/{app_name}", get(get_app_from_catalog))
        .route("/catalog/{app_name}/icon", get(get_app_icon))
        .route("/installed", get(list_installed_apps))
        .route("/gpu/nodes", get(gpu_nodes))
        .route("/install", post(install_app))
        .route("/operations", get(list_operations))
        .route("/operations/{operation_id}", get(get_operation))
        .route("/operations/{operation_id}/pause", post(pause_operation))
        .route("/operations/{operation_id}/resume", post(resume_operation))
        .route("/operations/{operation_id}/cancel", post(cancel_operation))
        .route("/operations/{operation_id}/retry", post(retry_operation))
        .route("/states", get(list_app_states))
        .route("/sync", post(sync_charts))
        .route("/sync/status", get(sync_status))
        .route("/categories", get(list_categories))
        .route("/category/{category}", get(get_apps_by_category))
        .route("/{app_name}/state", get(get_app_state))
        .route("/{app_name}/update", post(update_app))
        .route("/{app_name}", delete(delete_app))
        .route("/{app_name}/restart", post(restart_app))
        .route("/{app_name}/health", get(check_app_health))
        .route("/{app_name}/exists", get(check_app_exists))
        .route("/{app_name}/status", get(get_app_status))
        .route("/{app_name}/access", post(log_app_access))
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct NamespaceQuery {
    pub namespace: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct UpdateAppRequest {
    #[serde(default, deserialize_with = "gpu::gpu_field")]
    gpu: Option<Option<GpuSelection>>,
}

/// Discover supported Intel/NVIDIA/AMD node resources and Kubernetes node readiness.
#[utoipa::path(
    get,
    path = "/api/apps/gpu/nodes",
    tag = "Apps",
    responses((status = 200, body = Vec<GpuNode>))
)]
async fn gpu_nodes(
    State(state): State<AppState>,
    _auth: Authorized<AppsView>,
) -> Result<Json<Vec<GpuNode>>> {
    let guard = state.k8s_client.read().await;
    let client = guard
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".into()))?;
    Ok(Json(gpu::list_gpu_nodes(client).await?))
}

// ============================================================================
// Endpoint Handlers
// ============================================================================

/// List all apps in the catalog (excludes hidden apps)
#[utoipa::path(
    get,
    path = "/api/apps/catalog",
    tag = "Apps",
    responses((status = 200, body = serde_json::Value))
)]
async fn list_catalog(
    State(state): State<AppState>,
    _auth: Authorized<AppsView>,
) -> Result<Json<Vec<AppConfig>>> {
    let catalog = state.catalog.read().await;
    let apps: Vec<AppConfig> = catalog
        .get_all_apps()
        .into_iter()
        .filter(|app| !app.is_hidden)
        .cloned()
        .collect();
    Ok(Json(apps))
}

/// Get a specific app from the catalog
#[utoipa::path(
    get,
    path = "/api/apps/catalog/{app_name}",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    responses((status = 200, body = serde_json::Value))
)]
async fn get_app_from_catalog(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<AppsView>,
) -> Result<Json<AppConfig>> {
    let catalog = state.catalog.read().await;
    let app = catalog
        .get_app(&app_name)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("App '{}' not found", app_name)))?;
    Ok(Json(app))
}

/// Get the icon for an app (SVG)
#[utoipa::path(
    get,
    path = "/api/apps/catalog/{app_name}/icon",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    responses((status = 200, description = "SVG icon content", content_type = "image/svg+xml"))
)]
async fn get_app_icon(Path(app_name): Path<String>) -> Result<Response> {
    // Validate app name to prevent path traversal
    if app_name.contains("..") || app_name.contains('/') || app_name.contains('\\') {
        return Err(AppError::BadRequest("Invalid app name".to_string()));
    }

    let icon_path = CONFIG.charts.dir.join(&app_name).join("icon.svg");

    if !icon_path.exists() {
        return Err(AppError::NotFound(format!(
            "Icon not found for app '{}'",
            app_name
        )));
    }

    let content = std::fs::read(&icon_path)
        .map_err(|e| AppError::Internal(format!("Failed to read icon: {}", e)))?;

    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=604800, immutable"),
        ],
        content,
    )
        .into_response())
}

/// List installed apps
#[utoipa::path(
    get,
    path = "/api/apps/installed",
    tag = "Apps",
    responses((status = 200, body = Vec<String>))
)]
async fn list_installed_apps(
    State(state): State<AppState>,
    _auth: Authorized<AppsView>,
) -> Result<Json<Vec<String>>> {
    let k8s = state.k8s_client.read().await;
    let catalog = state.catalog.read().await;

    let apps = if let Some(ref client) = *k8s {
        let manager = DeploymentManager::new(client, &catalog);
        manager.get_deployed_apps().await
    } else {
        Vec::new()
    };

    Ok(Json(apps))
}

/// Install an app
#[utoipa::path(
    post,
    path = "/api/apps/install",
    tag = "Apps",
    request_body = serde_json::Value,
    responses((status = 200, body = serde_json::Value))
)]
async fn install_app(
    State(state): State<AppState>,
    auth: Authorized<AppsInstall>,
    Json(request): Json<DeploymentRequest>,
) -> Result<Json<AppOperationResponse>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    let operation = manager
        .enqueue_gpu_operation(
            &request.app_name,
            OP_INSTALL,
            request.custom_config,
            request.gpu,
            Some(auth.user_id()),
        )
        .await?;
    state.endpoint_cache.invalidate(&request.app_name).await;

    Ok(Json(operation))
}

/// Delete an app
#[utoipa::path(
    delete,
    path = "/api/apps/{app_name}",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    responses((status = 200, body = serde_json::Value))
)]
async fn delete_app(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authorized<AppsDelete>,
) -> Result<Json<AppOperationResponse>> {
    let db = state.get_db().await?;
    let catalog = state.catalog.read().await;

    // Check if this is a system app
    if let Some(app) = catalog.get_app(&app_name) {
        if app.is_system {
            return Err(AppError::Forbidden(format!(
                "Cannot delete system app '{}'",
                app_name
            )));
        }
    }

    drop(catalog);

    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    let operation = manager
        .enqueue_operation(
            &app_name,
            OP_DELETE,
            std::collections::HashMap::new(),
            Some(auth.user_id()),
        )
        .await?;

    // Invalidate endpoint cache for deleted app
    state.endpoint_cache.invalidate(&app_name).await;

    Ok(Json(operation))
}

/// Restart an app
#[utoipa::path(
    post,
    path = "/api/apps/{app_name}/restart",
    tag = "Apps",
    params(
        ("app_name" = String, Path, description = "App name"),
        ("namespace" = Option<String>, Query, description = "Namespace override")
    ),
    responses((status = 200, body = serde_json::Value))
)]
async fn restart_app(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    Query(query): Query<NamespaceQuery>,
    auth: Authorized<AppsRestart>,
) -> Result<Json<AppOperationResponse>> {
    let db = state.get_db().await?;
    let _namespace = query.namespace.unwrap_or_else(|| app_name.clone());
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    let operation = manager
        .enqueue_operation(
            &app_name,
            OP_RESTART,
            std::collections::HashMap::new(),
            Some(auth.user_id()),
        )
        .await?;

    // Invalidate endpoint cache since service endpoint may change after restart
    state.endpoint_cache.invalidate(&app_name).await;

    Ok(Json(operation))
}

/// Queue an app update
#[utoipa::path(
    post,
    path = "/api/apps/{app_name}/update",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    request_body = UpdateAppRequest,
    responses((status = 200, body = serde_json::Value))
)]
async fn update_app(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authorized<AppsInstall>,
    body: Option<Json<UpdateAppRequest>>,
) -> Result<Json<AppOperationResponse>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    let operation = manager
        .enqueue_gpu_operation(
            &app_name,
            OP_UPDATE,
            HashMap::new(),
            body.and_then(|Json(request)| request.gpu),
            Some(auth.user_id()),
        )
        .await?;
    state.endpoint_cache.invalidate(&app_name).await;
    Ok(Json(operation))
}

/// List app operations
#[utoipa::path(
    get,
    path = "/api/apps/operations",
    tag = "Apps",
    responses((status = 200, body = serde_json::Value))
)]
async fn list_operations(
    State(state): State<AppState>,
    _auth: Authorized<AppsView>,
) -> Result<Json<Vec<AppOperationResponse>>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    Ok(Json(manager.list_operations().await?))
}

/// Get app operation
#[utoipa::path(
    get,
    path = "/api/apps/operations/{operation_id}",
    tag = "Apps",
    params(("operation_id" = String, Path, description = "Operation id")),
    responses((status = 200, body = serde_json::Value))
)]
async fn get_operation(
    State(state): State<AppState>,
    Path(operation_id): Path<String>,
    _auth: Authorized<AppsView>,
) -> Result<Json<AppOperationResponse>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    Ok(Json(manager.get_operation(&operation_id).await?))
}

// POST controls: 200 with updated operation (retry returns the new operation),
// 404 unknown id, 409 invalid transition. Running cancel requests a cooperative stop.
async fn control(
    state: AppState,
    id: String,
    auth: Authenticated,
    action: &str,
) -> Result<Json<AppOperationResponse>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db.clone(), state.k8s_client.clone(), state.catalog.clone());
    let operation = manager.get_operation(&id).await?;
    let needed = operation_control_permission(&operation.operation)?;
    let permissions = crate::endpoints::extractors::get_user_permissions(&db, auth.user_id()).await;
    if !permissions.iter().any(|p| p == needed) {
        return Err(AppError::Forbidden(format!(
            "Permission denied: {needed} required"
        )));
    }
    if operation.operation == OP_DELETE
        && state
            .catalog
            .read()
            .await
            .get_app(&operation.app_name)
            .is_some_and(|app| app.is_system)
        && action == "retry"
    {
        return Err(AppError::Forbidden("Cannot delete system app".into()));
    }
    let updated = if action == "retry" {
        manager.retry_operation(&id, auth.user_id()).await?
    } else {
        manager
            .control_operation(&id, action, auth.user_id())
            .await?
    };
    state.endpoint_cache.invalidate(&updated.app_name).await;
    Ok(Json(updated))
}

fn operation_control_permission(operation: &str) -> Result<&'static str> {
    Ok(match operation {
        OP_DELETE => "apps.delete",
        OP_RESTART => "apps.restart",
        OP_INSTALL | OP_UPDATE => "apps.install",
        _ => return Err(AppError::BadRequest("Unsupported operation type".into())),
    })
}

#[utoipa::path(post, path = "/api/apps/operations/{operation_id}/pause", tag = "Apps", params(("operation_id" = String, Path)), responses((status = 200, body = AppOperationResponse), (status = 409)))]
async fn pause_operation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: Authenticated,
) -> Result<Json<AppOperationResponse>> {
    control(state, id, auth, "pause").await
}
#[utoipa::path(post, path = "/api/apps/operations/{operation_id}/resume", tag = "Apps", params(("operation_id" = String, Path)), responses((status = 200, body = AppOperationResponse), (status = 409)))]
async fn resume_operation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: Authenticated,
) -> Result<Json<AppOperationResponse>> {
    control(state, id, auth, "resume").await
}
#[utoipa::path(post, path = "/api/apps/operations/{operation_id}/cancel", tag = "Apps", params(("operation_id" = String, Path)), responses((status = 200, body = AppOperationResponse), (status = 409)))]
async fn cancel_operation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: Authenticated,
) -> Result<Json<AppOperationResponse>> {
    control(state, id, auth, "cancel").await
}
#[utoipa::path(post, path = "/api/apps/operations/{operation_id}/retry", tag = "Apps", params(("operation_id" = String, Path)), responses((status = 200, body = AppOperationResponse), (status = 409)))]
async fn retry_operation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: Authenticated,
) -> Result<Json<AppOperationResponse>> {
    control(state, id, auth, "retry").await
}

/// List app states
#[utoipa::path(
    get,
    path = "/api/apps/states",
    tag = "Apps",
    responses((status = 200, body = serde_json::Value))
)]
async fn list_app_states(
    State(state): State<AppState>,
    _auth: Authorized<AppsView>,
) -> Result<Json<Vec<AppStateResponse>>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    let states = manager.list_states().await?;

    Ok(Json(states))
}

/// Get app state
#[utoipa::path(
    get,
    path = "/api/apps/{app_name}/state",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    responses((status = 200, body = serde_json::Value))
)]
async fn get_app_state(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<AppsView>,
) -> Result<Json<AppStateResponse>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    Ok(Json(manager.get_state(&app_name).await?))
}

/// List all categories
#[utoipa::path(
    get,
    path = "/api/apps/categories",
    tag = "Apps",
    responses((status = 200, body = Vec<String>))
)]
async fn list_categories(
    State(state): State<AppState>,
    _auth: Authorized<AppsView>,
) -> Result<Json<Vec<String>>> {
    let catalog = state.catalog.read().await;
    Ok(Json(catalog.get_categories()))
}

/// Get apps by category
#[utoipa::path(
    get,
    path = "/api/apps/category/{category}",
    tag = "Apps",
    params(("category" = String, Path, description = "Category name")),
    responses((status = 200, body = serde_json::Value))
)]
async fn get_apps_by_category(
    State(state): State<AppState>,
    Path(category): Path<String>,
    _auth: Authorized<AppsView>,
) -> Result<Json<Vec<AppConfig>>> {
    let catalog = state.catalog.read().await;
    let apps: Vec<AppConfig> = catalog
        .get_apps_by_category(&category)
        .into_iter()
        .cloned()
        .collect();
    Ok(Json(apps))
}

/// Check app health
#[utoipa::path(
    get,
    path = "/api/apps/{app_name}/health",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    responses((status = 200, body = serde_json::Value))
)]
async fn check_app_health(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<AppsView>,
) -> Result<Json<serde_json::Value>> {
    let k8s = state.k8s_client.read().await;
    let catalog = state.catalog.read().await;

    let client = k8s
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;

    let manager = DeploymentManager::new(client, &catalog);
    let health = manager.app_health(&app_name).await?;

    Ok(Json(health))
}

/// Check if app exists
#[utoipa::path(
    get,
    path = "/api/apps/{app_name}/exists",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    responses((status = 200, body = serde_json::Value))
)]
async fn check_app_exists(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<AppsView>,
) -> Result<Json<serde_json::Value>> {
    let k8s = state.k8s_client.read().await;
    let catalog = state.catalog.read().await;

    let client = k8s
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;

    let manager = DeploymentManager::new(client, &catalog);
    let exists = manager.check_app_deployed(&app_name).await;

    Ok(Json(serde_json::json!({"exists": exists})))
}

/// Get app status
#[utoipa::path(
    get,
    path = "/api/apps/{app_name}/status",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    responses((status = 200, body = serde_json::Value))
)]
async fn get_app_status(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<AppsView>,
) -> Result<Json<serde_json::Value>> {
    let k8s = state.k8s_client.read().await;
    let catalog = state.catalog.read().await;

    let client = match k8s.as_ref() {
        Some(c) => c,
        None => {
            return Ok(Json(serde_json::json!({
                "state": "error",
                "message": "Kubernetes client not available"
            })));
        }
    };

    let manager = DeploymentManager::new(client, &catalog);
    if !manager.check_app_deployed(&app_name).await {
        return Ok(Json(serde_json::json!({
            "state": "idle",
            "message": "Not installed"
        })));
    }

    // Check health
    match manager.app_health(&app_name).await {
        Ok(health) => {
            let status = health["status"].as_str().unwrap_or("unknown");
            match status {
                "healthy" => Ok(Json(serde_json::json!({
                    "state": "installed",
                    "message": "Running"
                }))),
                "no_deployments" => Ok(Json(serde_json::json!({
                    "state": "idle",
                    "message": "No deployments found"
                }))),
                _ => Ok(Json(serde_json::json!({
                    "state": "installing",
                    "message": health["message"].as_str().unwrap_or("Waiting for deployments to be ready")
                }))),
            }
        }
        Err(e) => Ok(Json(serde_json::json!({
            "state": "error",
            "message": e.to_string()
        }))),
    }
}

/// Trigger on-demand chart sync from OCI registry
#[utoipa::path(
    post,
    path = "/api/apps/sync",
    tag = "Apps",
    responses((status = 200, body = serde_json::Value))
)]
async fn sync_charts(
    State(state): State<AppState>,
    _auth: Authorized<AppsInstall>,
) -> Result<Json<serde_json::Value>> {
    state
        .chart_sync
        .sync()
        .await
        .map_err(|e| AppError::Internal(format!("Chart sync failed: {}", e)))?;

    if let Ok(db) = state.get_db().await {
        AppManager::new(db, state.k8s_client.clone(), state.catalog.clone())
            .refresh_available_chart_versions()
            .await?;
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Chart sync completed",
        "last_synced": state.chart_sync.last_synced().await,
    })))
}

/// When the app catalog was last synced from the chart registry
#[utoipa::path(
    get,
    path = "/api/apps/sync/status",
    tag = "Apps",
    responses((status = 200, body = serde_json::Value))
)]
async fn sync_status(
    State(state): State<AppState>,
    _auth: Authorized<AppsView>,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "last_synced": state.chart_sync.last_synced().await,
    })))
}

/// Log app access - called when user opens an app
#[utoipa::path(
    post,
    path = "/api/apps/{app_name}/access",
    tag = "Apps",
    params(("app_name" = String, Path, description = "App name")),
    responses((status = 200, body = serde_json::Value))
)]
async fn log_app_access(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authenticated,
) -> Result<Json<serde_json::Value>> {
    use crate::models::audit_log::ResourceType;

    if state.catalog.read().await.get_app(&app_name).is_none() {
        return Err(AppError::NotFound(format!("App '{}' not found", app_name)));
    }
    let db = state.get_db().await?;
    let permissions = crate::endpoints::extractors::get_user_permissions(&db, auth.user_id()).await;
    if !permissions
        .iter()
        .any(|permission| permission == "app.*" || permission == &format!("app.{app_name}"))
    {
        return Err(AppError::Forbidden("App access denied".into()));
    }
    // The client reports an intent to open the app, not proof that the proxy served it.
    state
        .audit
        .log(
            AuditAction::AppAccessed,
            ResourceType::App,
            Some(app_name.clone()),
            Some(auth.user_id()),
            Some(auth.user().username.clone()),
            Some(serde_json::json!({ "phase": "requested_access" })),
            None,
            None,
            true,
            None,
        )
        .await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Access logged"
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_query_deser_with_value() {
        let q: NamespaceQuery = serde_json::from_str(r#"{"namespace":"media"}"#).expect("deser");
        assert_eq!(q.namespace, Some("media".to_string()));
    }

    #[test]
    fn namespace_query_deser_empty() {
        let q: NamespaceQuery = serde_json::from_str("{}").expect("deser");
        assert_eq!(q.namespace, None);
    }

    #[test]
    fn update_gpu_distinguishes_omission_from_disable() {
        assert!(serde_json::from_str::<UpdateAppRequest>("{}")
            .unwrap()
            .gpu
            .is_none());
        assert_eq!(
            serde_json::from_str::<UpdateAppRequest>(r#"{"gpu":null}"#)
                .unwrap()
                .gpu,
            Some(None)
        );
        assert!(serde_json::from_str::<UpdateAppRequest>(
            r#"{"gpu":{"vendor":"unknown","node_name":"node"}}"#
        )
        .is_err());
    }

    #[test]
    fn operation_controls_require_the_permission_for_the_original_action() {
        for (operation, permission) in [
            (OP_INSTALL, "apps.install"),
            (OP_UPDATE, "apps.install"),
            (OP_DELETE, "apps.delete"),
            (OP_RESTART, "apps.restart"),
        ] {
            assert_eq!(operation_control_permission(operation).unwrap(), permission);
        }
        assert!(operation_control_permission("invalid").is_err());
    }
}
