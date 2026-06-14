use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::middleware::permissions::{
    AppsDelete, AppsInstall, AppsRestart, AppsView, Authenticated, Authorized,
};
use crate::models::audit_log::AuditAction;
use crate::services::{
    catalog::{kubarr_system_component, kubarr_system_component_deployment},
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
        .route("/install", post(install_app))
        .route("/operations", get(list_operations))
        .route("/operations/{operation_id}", get(get_operation))
        .route("/states", get(list_app_states))
        .route("/sync", post(sync_charts))
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
        .enqueue_operation(
            &request.app_name,
            OP_INSTALL,
            request.custom_config,
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
    responses((status = 200, body = serde_json::Value))
)]
async fn update_app(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authorized<AppsInstall>,
) -> Result<Json<AppOperationResponse>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db, state.k8s_client.clone(), state.catalog.clone());
    let operation = manager
        .enqueue_operation(
            &app_name,
            OP_UPDATE,
            std::collections::HashMap::new(),
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
    let mut states = manager.list_states().await?;

    if let Some(client) = state.k8s_client.read().await.as_ref() {
        let catalog = state.catalog.read().await;
        let deployment_manager = DeploymentManager::new(client, &catalog);
        for app in catalog
            .get_all_apps()
            .into_iter()
            .filter(|app| app.is_system && !app.is_hidden)
        {
            if kubarr_system_component_deployment(&app.name).is_some() {
                states.retain(|state| state.app_name != app.name);
                states.push(system_component_state(&deployment_manager, &app.name).await);
            }
        }
    }

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
    if kubarr_system_component_deployment(&app_name).is_some() {
        let k8s = state.k8s_client.read().await;
        let catalog = state.catalog.read().await;
        let client = k8s
            .as_ref()
            .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;
        let deployment_manager = DeploymentManager::new(client, &catalog);
        return Ok(Json(
            system_component_state(&deployment_manager, &app_name).await,
        ));
    }

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
    if kubarr_system_component_deployment(&app_name).is_some() {
        let healthy = manager.check_kubarr_system_component(&app_name).await;
        return Ok(Json(serde_json::json!({
            "status": if healthy { "healthy" } else { "unhealthy" },
            "healthy": healthy,
            "message": if healthy { "System component is running" } else { "System component is not ready" }
        })));
    }

    let health = manager.check_namespace_health(&app_name).await?;

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
    if kubarr_system_component_deployment(&app_name).is_some() {
        return Ok(Json(serde_json::json!({
            "exists": manager.check_kubarr_system_component(&app_name).await
        })));
    }

    let exists = manager.check_namespace_exists(&app_name).await;

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
    if kubarr_system_component_deployment(&app_name).is_some() {
        let healthy = manager.check_kubarr_system_component(&app_name).await;
        return Ok(Json(serde_json::json!({
            "state": if healthy { "installed" } else { "installing" },
            "message": if healthy { "Running" } else { "Waiting for component to be ready" }
        })));
    }

    // Check if namespace exists
    if !manager.check_namespace_exists(&app_name).await {
        return Ok(Json(serde_json::json!({
            "state": "idle",
            "message": "Not installed"
        })));
    }

    // Check health
    match manager.check_namespace_health(&app_name).await {
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

async fn system_component_state(
    manager: &DeploymentManager<'_>,
    app_name: &str,
) -> AppStateResponse {
    let healthy = manager.check_kubarr_system_component(app_name).await;
    let now = Utc::now();
    AppStateResponse {
        app_name: app_name.to_string(),
        namespace: kubarr_system_component(app_name)
            .map(|component| component.namespace)
            .unwrap_or("kubarr-system")
            .to_string(),
        desired_state: "installed".to_string(),
        observed_state: if healthy { "installed" } else { "unhealthy" }.to_string(),
        healthy,
        message: Some(if healthy {
            "System component is running".to_string()
        } else {
            "System component is not ready".to_string()
        }),
        installed_chart_version: None,
        available_chart_version: None,
        update_available: false,
        last_operation_id: None,
        last_checked_at: Some(now),
        updated_at: now,
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

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Chart sync completed"
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

    // Log the access in audit trail
    let _ = state
        .audit
        .log(
            AuditAction::AppAccessed,
            ResourceType::App,
            Some(app_name.clone()),
            Some(auth.user_id()),
            Some(auth.user().username.clone()),
            Some(serde_json::json!({ "app": app_name })),
            None,
            None,
            true,
            None,
        )
        .await;

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
}
