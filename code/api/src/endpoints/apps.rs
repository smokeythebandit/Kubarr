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
use crate::services::app_storage;
use crate::services::gpu::{self, GpuNode, GpuSelection};
use crate::services::install_options::{self, InstallStorage};
use crate::services::vpn::AssignVpnRequest;
use crate::services::{
    AppConfig, AppManager, AppOperationResponse, AppStateResponse, DeploymentManager, OP_DELETE,
    OP_INSTALL, OP_RESTART, OP_UPDATE,
};
use crate::state::AppState;
use sea_orm::TransactionTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

/// Create apps routes
pub fn apps_routes(state: AppState) -> Router {
    Router::new()
        .route("/catalog", get(list_catalog))
        .route("/catalog/{app_name}", get(get_app_from_catalog))
        .route(
            "/catalog/{app_name}/install-options",
            get(get_install_options),
        )
        .route("/catalog/{app_name}/icon", get(get_app_icon))
        .route("/catalog/icons", get(list_app_icons))
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallRequest {
    app_name: String,
    #[serde(default)]
    _namespace: Option<String>,
    #[serde(default)]
    chart_version: Option<String>,
    #[serde(default)]
    storage: Option<InstallStorage>,
    // Outer option distinguishes omitted (legacy: retain existing assignment)
    // from null (explicit No VPN). A null needs vpn.manage if assigned.
    #[serde(default, deserialize_with = "vpn_choice")]
    vpn: Option<Option<AssignVpnRequest>>,
    #[serde(default)]
    custom_config: HashMap<String, String>,
    #[serde(default, deserialize_with = "gpu::gpu_field")]
    gpu: Option<Option<GpuSelection>>,
}

fn vpn_choice<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Option<AssignVpnRequest>>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    if value.is_null() {
        Ok(Some(None))
    } else {
        AssignVpnRequest::deserialize(value)
            .map(|vpn| Some(Some(vpn)))
            .map_err(serde::de::Error::custom)
    }
}

#[utoipa::path(get, path = "/api/apps/catalog/{app_name}/install-options", tag = "Apps", params(("app_name" = String, Path)), responses((status = 200, body = install_options::InstallOptions)))]
async fn get_install_options(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    _auth: Authorized<AppsView>,
) -> Result<Json<install_options::InstallOptions>> {
    let catalog = state.catalog.read().await;
    let mut options = install_options::load(&catalog, &app_name)?;
    drop(catalog);
    if let Some(saved) = app_storage::get(&state.get_db().await?, &options.app_name).await? {
        options.saved_storage = Some(saved.storage);
        options.saved_chart_version = Some(saved.chart_version);
    }
    Ok(Json(options))
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
) -> Result<impl IntoResponse> {
    let catalog = state.catalog.read().await;
    let apps: Vec<AppConfig> = catalog
        .get_all_apps()
        .into_iter()
        .filter(|app| !app.is_hidden)
        .cloned()
        .collect();
    // The catalog only changes when charts sync.
    Ok(([super::short_cache(60)], Json(apps)))
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

/// Read every chart icon in one pass.
///
/// Blocking file reads belong on the blocking pool: a dashboard asks for
/// twenty-nine icons at once, and on a CPU-limited runtime those reads
/// otherwise occupy threads that should be serving other requests.
async fn read_icon_bundle(
    dir: &std::path::Path,
) -> std::collections::BTreeMap<String, String> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut bundle = std::collections::BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return bundle;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains('/') || name.contains("..") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(entry.path().join("icon.svg")) {
                bundle.insert(name, content);
            }
        }
        bundle
    })
    .await
    .unwrap_or_default()
}

/// Every catalog icon in one response, so a dashboard load fetches them once.
#[utoipa::path(
    get,
    path = "/api/apps/catalog/icons",
    tag = "Apps",
    responses((status = 200, description = "App name to SVG"))
)]
async fn list_app_icons(_auth: Authorized<AppsView>) -> Result<impl IntoResponse> {
    let bundle = read_icon_bundle(&CONFIG.charts.dir).await;
    Ok(([super::short_cache(600)], Json(bundle)))
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

    let content = tokio::task::spawn_blocking(move || std::fs::read(&icon_path))
        .await
        .map_err(|e| AppError::Internal(format!("Icon read cancelled: {}", e)))?
        .map_err(|_| AppError::NotFound(format!("Icon not found for app '{}'", app_name)))?;

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
) -> Result<impl IntoResponse> {
    let k8s = state.k8s_client.read().await;
    let catalog = state.catalog.read().await;

    let apps = if let Some(ref client) = *k8s {
        let manager = DeploymentManager::new(client, &catalog);
        manager.get_deployed_apps().await
    } else {
        Vec::new()
    };

    Ok(([super::short_cache(5)], Json(apps)))
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
    Json(request): Json<InstallRequest>,
) -> Result<Json<AppOperationResponse>> {
    let db = state.get_db().await?;
    let manager = AppManager::new(db.clone(), state.k8s_client.clone(), state.catalog.clone());
    let has_choices =
        request.chart_version.is_some() || request.storage.is_some() || request.vpn.is_some();
    if has_choices && !request.custom_config.is_empty() {
        return Err(AppError::BadRequest(
            "Wizard choices cannot be mixed with custom_config".into(),
        ));
    }
    let operation = if has_choices {
        let options = {
            let catalog = state.catalog.read().await;
            install_options::load(&catalog, &request.app_name)?
        };
        if request.chart_version.as_deref() != Some(options.chart_version.as_str()) {
            return Err(AppError::Conflict(
                "Chart changed; reload install options and review again".into(),
            ));
        }
        let requested_storage = request.storage.unwrap_or_default();
        install_options::validate(&options, &requested_storage)?;
        if request.gpu.is_some() && !options.gpu_supported {
            return Err(AppError::BadRequest(
                "GPU is unsupported for this chart".into(),
            ));
        }
        if request.vpn.as_ref().is_some_and(|v| v.is_some()) && !options.vpn_supported {
            return Err(AppError::BadRequest(
                "VPN is unsupported for this chart".into(),
            ));
        }
        let permissions =
            crate::endpoints::extractors::get_user_permissions(&db, auth.user_id()).await;
        let can_manage_vpn = permissions.iter().any(|p| p == "vpn.manage");
        if request.vpn.as_ref().is_some_and(|v| v.is_some()) && !can_manage_vpn {
            return Err(AppError::Forbidden(
                "vpn.manage required for install-time VPN".into(),
            ));
        }
        let tx = db.begin().await?;
        let active = crate::models::app_operation::Entity::find()
            .filter(crate::models::app_operation::Column::AppName.eq(&request.app_name))
            .filter(
                crate::models::app_operation::Column::Status.is_in(["queued", "running", "paused"]),
            )
            .one(&tx)
            .await?;
        if active.is_some() {
            return Err(AppError::Conflict(
                "An app operation is already in progress".into(),
            ));
        }
        let saved = app_storage::get(&tx, &request.app_name).await?;
        let storage = app_storage::materialize(
            &options,
            &requested_storage,
            saved.as_ref().map(|s| &s.storage),
        )?;
        let fresh_private_config = saved
            .as_ref()
            .is_some_and(|s| s.fresh_private_config && s.storage.config == storage.config);
        let existing = crate::models::app_vpn_config::Entity::find_by_id(&request.app_name)
            .one(&tx)
            .await?;
        if existing.is_some() && !options.vpn_supported && !matches!(request.vpn, Some(None)) {
            return Err(AppError::BadRequest("This chart cannot install with its existing VPN assignment; remove it explicitly with vpn.manage".into()));
        }
        if matches!(request.vpn, Some(None)) && existing.is_some() && !can_manage_vpn {
            return Err(AppError::Forbidden(
                "vpn.manage required to remove an existing VPN assignment".into(),
            ));
        }
        if let Some(vpn) = request.vpn {
            if let Some(vpn) = vpn {
                crate::services::vpn::assign_vpn_to_app(&tx, &request.app_name, vpn).await?;
            } else {
                crate::services::vpn::remove_vpn_from_app(&tx, &request.app_name).await?;
            }
        }
        let result = manager
            .enqueue_install_in_transaction(
                &tx,
                &request.app_name,
                &options.chart_version,
                storage.clone(),
                fresh_private_config,
                request.gpu,
                auth.user_id(),
            )
            .await?;
        // Queue, VPN assignment and durable desired storage are all-or-nothing.
        app_storage::save(&tx, &request.app_name, &options.chart_version, &storage).await?;
        tx.commit().await?;
        result
    } else {
        manager
            .enqueue_gpu_operation(
                &request.app_name,
                OP_INSTALL,
                request.custom_config,
                request.gpu,
                Some(auth.user_id()),
            )
            .await?
    };
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

    #[tokio::test]
    async fn icon_bundle_reads_every_chart_icon_once() {
        let charts = tempfile::tempdir().expect("tmpdir");
        for app in ["sonarr", "radarr"] {
            let dir = charts.path().join(app);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("icon.svg"), format!("<svg id=\"{app}\"/>")).expect("write");
        }
        std::fs::create_dir_all(charts.path().join("no-icon")).expect("mkdir");

        let bundle = read_icon_bundle(charts.path()).await;
        assert_eq!(bundle.len(), 2, "charts without an icon are skipped");
        assert_eq!(bundle["sonarr"], "<svg id=\"sonarr\"/>");
        assert!(!bundle.contains_key("no-icon"));
    }

    #[tokio::test]
    async fn icon_bundle_is_empty_when_no_charts_are_synced() {
        let empty = tempfile::tempdir().expect("tmpdir");
        assert!(read_icon_bundle(empty.path()).await.is_empty());
        // A missing directory must not panic a request thread.
        assert!(read_icon_bundle(&empty.path().join("absent")).await.is_empty());
    }

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
