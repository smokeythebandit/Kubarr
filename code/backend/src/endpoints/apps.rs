use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use sea_orm::EntityTrait;
use serde::Deserialize;

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::middleware::permissions::{
    AppsDelete, AppsInstall, AppsRestart, AppsView, Authenticated, Authorized,
};
use crate::models::audit_log::AuditAction;
use crate::models::prelude::*;
use crate::services::{AppConfig, DeploymentManager, DeploymentRequest, DeploymentStatus};
use crate::state::AppState;

/// Create apps routes
pub fn apps_routes(state: AppState) -> Router {
    Router::new()
        .route("/catalog", get(list_catalog))
        .route("/catalog/{app_name}", get(get_app_from_catalog))
        .route("/catalog/{app_name}/icon", get(get_app_icon))
        .route("/installed", get(list_installed_apps))
        .route("/install", post(install_app))
        .route("/sync", post(sync_charts))
        .route("/categories", get(list_categories))
        .route("/category/{category}", get(get_apps_by_category))
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
    _auth: Authorized<AppsInstall>,
    Json(request): Json<DeploymentRequest>,
) -> Result<Json<DeploymentStatus>> {
    let db = state.get_db().await?;
    let k8s = state.k8s_client.read().await;
    let catalog = state.catalog.read().await;

    let client = k8s
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;

    // Get storage config from server_config
    let server_cfg = ServerConfig::find().one(&db).await?;
    let nfs_server = server_cfg.as_ref().and_then(|c| c.nfs_server.clone());
    let storage_path = server_cfg.as_ref().map(|c| c.storage_path.clone());

    // Use with_db to enable VPN support
    let manager = DeploymentManager::with_db(client, &catalog, &db);
    let status = manager
        .deploy_app(&request, storage_path.as_deref(), nfs_server.as_deref())
        .await?;

    // Invalidate cache to ensure fresh lookup when app becomes ready
    state.endpoint_cache.invalidate(&request.app_name).await;

    Ok(Json(status))
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
    _auth: Authorized<AppsDelete>,
) -> Result<Json<serde_json::Value>> {
    let k8s = state.k8s_client.read().await;
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

    let client = k8s
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;

    let manager = DeploymentManager::new(client, &catalog);
    manager.remove_app(&app_name).await?;

    // Invalidate endpoint cache for deleted app
    state.endpoint_cache.invalidate(&app_name).await;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": format!("App '{}' deletion initiated", app_name),
        "status": "deleting"
    })))
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
    _auth: Authorized<AppsRestart>,
) -> Result<Json<serde_json::Value>> {
    let namespace = query.namespace.unwrap_or_else(|| app_name.clone());

    let k8s = state.k8s_client.read().await;
    let client = k8s
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;

    // Get pods with app label
    let pods = client.get_pod_status(&namespace, Some(&app_name)).await?;

    // Delete each pod
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::{Api, DeleteParams};

    let pod_api: Api<Pod> = Api::namespaced(client.client().clone(), &namespace);
    let mut deleted_count = 0;

    for pod in &pods {
        if pod_api
            .delete(&pod.name, &DeleteParams::default())
            .await
            .is_ok()
        {
            deleted_count += 1;
        }
    }

    // Invalidate endpoint cache since service endpoint may change after restart
    state.endpoint_cache.invalidate(&app_name).await;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": format!("Restarted {} pod(s) for app '{}'", deleted_count, app_name)
    })))
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
