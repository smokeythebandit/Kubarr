use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait,
    Set,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{AppError, Result};
use crate::models::prelude::*;
use crate::models::{role, user, user_role};
use crate::services::bootstrap::{self, BootstrapService, ComponentStatus};
use crate::services::storage_config as storage_cfg;
use crate::state::AppState;

/// System/virtual directories to hide from the setup directory browser
const FILTERED_DIRECTORIES: &[&str] = &["/proc", "/sys", "/dev", "/run", "/tmp", "/var/run"];

pub fn setup_routes(state: AppState) -> Router {
    Router::new()
        .route("/required", get(check_setup_required))
        .route("/status", get(get_setup_status))
        .route("/initialize", post(initialize_setup))
        .route("/generate-credentials", get(generate_credentials))
        .route("/validate-path", post(validate_path))
        .route("/browse", get(browse_setup_directory))
        // Storage-first setup endpoints
        .route("/storage", get(get_storage_config).post(configure_storage))
        .route("/storage/validate", post(validate_storage))
        .route("/storage/provision", post(provision_storage))
        // Bootstrap endpoints
        .route("/bootstrap/start", post(start_bootstrap))
        .route("/bootstrap/status", get(get_bootstrap_status))
        .route(
            "/bootstrap/retry/{component}",
            post(retry_bootstrap_component),
        )
        .route("/bootstrap/ws", get(bootstrap_ws_handler))
        // Server config endpoints
        .route("/server", get(get_server_config).post(configure_server))
        .with_state(state)
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SetupRequiredResponse {
    pub setup_required: bool,
    pub database_pending: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SetupStatusResponse {
    pub setup_required: bool,
    pub bootstrap_complete: bool,
    pub server_configured: bool,
    pub admin_user_exists: bool,
    pub storage_configured: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetupRequest {
    pub admin_username: String,
    pub admin_email: String,
    pub admin_password: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GeneratedCredentialsResponse {
    pub admin_username: String,
    pub admin_email: String,
    pub admin_password: String,
}

/// Check if any user with admin role exists (requires database)
async fn admin_user_exists(state: &AppState) -> Result<bool> {
    let db_guard = state.db.read().await;
    let db = match db_guard.as_ref() {
        Some(db) => db,
        None => return Ok(false), // No database = no admin
    };

    let admin_exists = UserRole::find()
        .join(JoinType::InnerJoin, user_role::Relation::Role.def())
        .filter(role::Column::Name.eq("admin"))
        .one(db)
        .await?;

    Ok(admin_exists.is_some())
}

/// Require setup to be incomplete (no admin user exists)
///
/// Returns 403 Forbidden if setup is already complete (admin user exists).
/// This is the self-disabling mechanism for setup endpoints - they should only
/// be accessible during initial setup before the first admin user is created.
async fn require_setup(state: &AppState) -> Result<()> {
    let admin_exists = admin_user_exists(state).await?;
    if admin_exists {
        return Err(AppError::Forbidden("Setup already complete".to_string()));
    }
    Ok(())
}

/// Check if setup is required (no admin user exists)
///
/// When the database is unavailable, we check Kubernetes to determine whether
/// PostgreSQL was previously installed. If it was, the system was already set up
/// and the DB is just not ready yet — we return `database_pending: true` instead
/// of incorrectly claiming setup is required.
#[utoipa::path(
    get,
    path = "/api/setup/required",
    tag = "Setup",
    responses(
        (status = 200, body = SetupRequiredResponse)
    )
)]
async fn check_setup_required(
    State(state): State<AppState>,
) -> Result<Json<SetupRequiredResponse>> {
    // Check if DB is connected
    let db_connected = state.is_db_connected().await;

    if !db_connected {
        // DB not available — check if PostgreSQL was previously installed
        let postgresql_exists = {
            let k8s_guard = state.k8s_client.read().await;
            if let Some(ref k8s) = *k8s_guard {
                use k8s_openapi::api::core::v1::Namespace;
                use kube::api::Api;
                let namespaces: Api<Namespace> = Api::all(k8s.client().clone());
                namespaces.get("postgresql").await.is_ok()
            } else {
                false
            }
        };

        if postgresql_exists {
            // PostgreSQL exists but DB not connected yet — not a fresh install
            return Ok(Json(SetupRequiredResponse {
                setup_required: false,
                database_pending: true,
            }));
        }

        // No PostgreSQL namespace — genuine first-time setup
        return Ok(Json(SetupRequiredResponse {
            setup_required: true,
            database_pending: false,
        }));
    }

    let admin_exists = admin_user_exists(&state).await?;

    Ok(Json(SetupRequiredResponse {
        setup_required: !admin_exists,
        database_pending: false,
    }))
}

/// Get detailed setup status
#[utoipa::path(
    get,
    path = "/api/setup/status",
    tag = "Setup",
    responses(
        (status = 200, body = SetupStatusResponse)
    )
)]
async fn get_setup_status(State(state): State<AppState>) -> Result<Json<SetupStatusResponse>> {
    // Check for admin user (user with admin role)
    let admin_exists = admin_user_exists(&state).await?;

    // Only accessible during setup
    if admin_exists {
        return Err(AppError::Forbidden(
            "Setup has already been completed".to_string(),
        ));
    }

    // Check bootstrap status
    let bootstrap_service = BootstrapService::new(
        state.db.clone(),
        state.k8s_client.clone(),
        state.catalog.clone(),
        state.bootstrap_tx.clone(),
    );
    let bootstrap_complete = bootstrap_service.is_complete().await;

    // Check for server configuration (requires database)
    let server_configured = {
        let db_guard = state.db.read().await;
        if let Some(ref db) = *db_guard {
            bootstrap::get_server_config(db).await?.is_some()
        } else {
            false
        }
    };

    let storage_configured = {
        let db_guard = state.db.read().await;
        let k8s_guard = state.k8s_client.read().await;
        storage_cfg::get_storage_config(db_guard.as_ref(), k8s_guard.as_ref())
            .await?
            .map(|c| c.validated)
            .unwrap_or(false)
    };

    Ok(Json(SetupStatusResponse {
        setup_required: !admin_exists,
        bootstrap_complete,
        server_configured,
        admin_user_exists: admin_exists,
        storage_configured,
    }))
}

/// Initialize the dashboard (create admin user)
#[utoipa::path(
    post,
    path = "/api/setup/initialize",
    tag = "Setup",
    request_body = SetupRequest,
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn initialize_setup(
    State(state): State<AppState>,
    Json(request): Json<SetupRequest>,
) -> Result<Json<serde_json::Value>> {
    // Get database connection (required for this step)
    let db = state.get_db().await?;

    // Check if setup is required (user with admin role exists)
    let admin_exists = admin_user_exists(&state).await?;

    if admin_exists {
        return Err(AppError::Forbidden(
            "Setup has already been completed".to_string(),
        ));
    }

    // Get server config for display/name settings.
    let server_config = bootstrap::get_server_config(&db).await?.ok_or_else(|| {
        AppError::BadRequest("Server must be configured before creating admin user".to_string())
    })?;

    let storage_ready = {
        let k8s_guard = state.k8s_client.read().await;
        storage_cfg::storage_ready(Some(&db), k8s_guard.as_ref()).await?
    };
    if !storage_ready {
        return Err(AppError::BadRequest(
            "Storage must be configured and validated before creating admin user".to_string(),
        ));
    }

    // Hash the password
    let hashed_password = crate::services::security::hash_password(&request.admin_password)?;

    // Create admin user
    let now = Utc::now();
    let new_user = user::ActiveModel {
        username: Set(request.admin_username.clone()),
        email: Set(request.admin_email.clone()),
        hashed_password: Set(hashed_password),
        is_active: Set(true),
        is_approved: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };

    let created_user = new_user
        .insert(&db)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create admin user: {}", e)))?;

    // Check if admin role exists
    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(&db)
        .await?;

    let admin_role = match admin_role {
        Some(r) => r,
        None => {
            // Create admin role
            let new_role = role::ActiveModel {
                name: Set("admin".to_string()),
                description: Set(Some("Full system access".to_string())),
                is_system: Set(true),
                created_at: Set(now),
                ..Default::default()
            };
            new_role
                .insert(&db)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to create admin role: {}", e)))?
        }
    };

    // Assign admin role to user
    let user_role_model = user_role::ActiveModel {
        user_id: Set(created_user.id),
        role_id: Set(admin_role.id),
    };
    user_role_model
        .insert(&db)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to assign admin role: {}", e)))?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Setup completed successfully",
        "data": {
            "admin_user": {
                "username": request.admin_username,
                "email": request.admin_email
            },
            "server": {
                "name": server_config.name,
                "storage_path": storage_cfg::STORAGE_MOUNT_PATH
            }
        }
    })))
}

/// Generate random credentials for setup
#[utoipa::path(
    get,
    path = "/api/setup/generate-credentials",
    tag = "Setup",
    responses(
        (status = 200, body = GeneratedCredentialsResponse)
    )
)]
async fn generate_credentials(
    State(state): State<AppState>,
) -> Result<Json<GeneratedCredentialsResponse>> {
    // Check if setup is required (user with admin role exists)
    let admin_exists = admin_user_exists(&state).await?;

    if admin_exists {
        return Err(AppError::Forbidden(
            "Setup has already been completed".to_string(),
        ));
    }

    Ok(Json(GeneratedCredentialsResponse {
        admin_username: "admin".to_string(),
        admin_email: "admin@example.com".to_string(),
        admin_password: crate::services::security::generate_random_string(16),
    }))
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct ValidatePathQuery {
    pub path: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ValidatePathResponse {
    pub valid: bool,
    pub exists: bool,
    pub writable: bool,
    pub message: String,
}

/// Validate a storage path
#[utoipa::path(
    post,
    path = "/api/setup/validate-path",
    tag = "Setup",
    params(ValidatePathQuery),
    responses(
        (status = 200, body = ValidatePathResponse)
    )
)]
async fn validate_path(
    Query(query): Query<ValidatePathQuery>,
) -> Result<Json<ValidatePathResponse>> {
    let path = Path::new(&query.path);

    // Check if path exists
    let exists = path.exists();

    // Check if path is a directory and writable
    let (valid, writable, message) = if exists {
        if path.is_dir() {
            // Try to check if writable by checking metadata
            match std::fs::metadata(path) {
                Ok(_) => (true, true, "Path is valid and accessible".to_string()),
                Err(e) => (
                    false,
                    false,
                    format!("Path exists but is not accessible: {}", e),
                ),
            }
        } else {
            (
                false,
                false,
                "Path exists but is not a directory".to_string(),
            )
        }
    } else {
        // Path doesn't exist, check if parent exists and is writable
        if let Some(parent) = path.parent() {
            if parent.exists() && parent.is_dir() {
                (
                    true,
                    true,
                    "Path does not exist but can be created".to_string(),
                )
            } else {
                (false, false, "Parent directory does not exist".to_string())
            }
        } else {
            (false, false, "Invalid path".to_string())
        }
    };

    Ok(Json(ValidatePathResponse {
        valid,
        exists,
        writable,
        message,
    }))
}

// ============================================================================
// Directory Browsing for Setup
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct BrowseSetupQuery {
    #[serde(default = "default_browse_path")]
    pub path: String,
}

fn default_browse_path() -> String {
    "/".to_string()
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BrowseSetupResponse {
    pub path: String,
    pub parent: Option<String>,
    pub directories: Vec<SetupDirectoryEntry>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SetupDirectoryEntry {
    pub name: String,
    pub path: String,
}

/// Browse directories on the host filesystem during setup
///
/// Lists only directories (not files) at the given absolute path.
/// Filters out system/virtual directories. Only accessible before
/// admin user creation (same gate as other setup endpoints).
#[utoipa::path(
    get,
    path = "/api/setup/browse",
    tag = "Setup",
    params(BrowseSetupQuery),
    responses(
        (status = 200, body = BrowseSetupResponse)
    )
)]
async fn browse_setup_directory(
    State(state): State<AppState>,
    Query(query): Query<BrowseSetupQuery>,
) -> Result<Json<BrowseSetupResponse>> {
    // Only accessible during setup
    require_setup(&state).await?;

    // The host filesystem may be mounted at a prefix (e.g. /host) inside the container.
    // KUBARR_HOST_BROWSE_PREFIX maps the container mount point so we can translate
    // between host paths (what the user sees) and container paths (what we read).
    // When not set, we browse the container's own filesystem directly.
    let host_prefix = std::env::var("KUBARR_HOST_BROWSE_PREFIX").unwrap_or_default();

    let requested = Path::new(&query.path);
    if !requested.is_absolute() {
        return Err(AppError::BadRequest("Path must be absolute".to_string()));
    }

    // Map host path to container path: /mnt/data -> /host/mnt/data
    let container_path = if host_prefix.is_empty() {
        query.path.clone()
    } else {
        format!("{}{}", host_prefix.trim_end_matches('/'), query.path)
    };

    let canonical = Path::new(&container_path)
        .canonicalize()
        .map_err(|_| AppError::NotFound(format!("Path not found: {}", query.path)))?;

    // Ensure the resolved path stays within the host prefix (prevent traversal)
    if !host_prefix.is_empty() {
        let prefix_canonical = Path::new(&host_prefix)
            .canonicalize()
            .map_err(|_| AppError::Internal("Host browse prefix not found".to_string()))?;
        if !canonical.starts_with(&prefix_canonical) {
            return Err(AppError::Forbidden(
                "Access denied: path traversal attempt detected".to_string(),
            ));
        }
    }

    if !canonical.is_dir() {
        return Err(AppError::BadRequest(format!(
            "Path is not a directory: {}",
            query.path
        )));
    }

    // Helper to convert a container path back to a host path
    let to_host_path = |container: &str| -> String {
        if host_prefix.is_empty() {
            container.to_string()
        } else {
            let trimmed_prefix = host_prefix.trim_end_matches('/');
            let stripped = container.strip_prefix(trimmed_prefix).unwrap_or(container);
            if stripped.is_empty() {
                "/".to_string()
            } else {
                stripped.to_string()
            }
        }
    };

    let canonical_str = canonical.to_string_lossy().to_string();
    let host_path = to_host_path(&canonical_str);

    // Compute parent path (None if at "/")
    let parent = if host_path == "/" {
        None
    } else {
        let p = Path::new(&host_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string());
        // Ensure parent is at least "/"
        p.map(|s| if s.is_empty() { "/".to_string() } else { s })
    };

    // List only directories, filtering out system/virtual ones
    let mut directories: Vec<SetupDirectoryEntry> = Vec::new();

    let entries = std::fs::read_dir(&canonical)
        .map_err(|e| AppError::Forbidden(format!("Cannot read directory: {}", e)))?;

    for entry in entries.filter_map(|e| e.ok()) {
        let entry_path = entry.path();

        // Only include directories
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }

        let container_full = entry_path.to_string_lossy().to_string();
        let host_full = to_host_path(&container_full);

        // Filter out system/virtual directories (based on host path)
        if FILTERED_DIRECTORIES.iter().any(|&filtered| {
            host_full == filtered || host_full.starts_with(&format!("{}/", filtered))
        }) {
            continue;
        }

        let name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Skip hidden directories (starting with .)
        if name.starts_with('.') {
            continue;
        }

        directories.push(SetupDirectoryEntry {
            name,
            path: host_full,
        });
    }

    // Sort alphabetically
    directories.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    Ok(Json(BrowseSetupResponse {
        path: host_path,
        parent,
        directories,
    }))
}

// ============================================================================
// Storage Setup Endpoints
// ============================================================================

/// Get storage configuration selected during setup.
#[utoipa::path(
    get,
    path = "/api/setup/storage",
    tag = "Setup",
    responses(
        (status = 200, body = Option<storage_cfg::StorageConfigResponse>)
    )
)]
async fn get_storage_config(
    State(state): State<AppState>,
) -> Result<Json<Option<storage_cfg::StorageConfigResponse>>> {
    require_setup(&state).await?;

    let db_guard = state.db.read().await;
    let k8s_guard = state.k8s_client.read().await;
    Ok(Json(
        storage_cfg::get_storage_config(db_guard.as_ref(), k8s_guard.as_ref()).await?,
    ))
}

/// Validate storage by mounting temporary PVCs and running read/write checks.
#[utoipa::path(
    post,
    path = "/api/setup/storage/validate",
    tag = "Setup",
    request_body = storage_cfg::StorageConfigRequest,
    responses(
        (status = 200, body = storage_cfg::StorageValidationResponse)
    )
)]
async fn validate_storage(
    State(state): State<AppState>,
    Json(request): Json<storage_cfg::StorageConfigRequest>,
) -> Result<Json<storage_cfg::StorageValidationResponse>> {
    require_setup(&state).await?;

    let k8s_guard = state.k8s_client.read().await;
    let k8s = k8s_guard.as_ref().ok_or_else(|| {
        AppError::ServiceUnavailable(
            "Kubernetes client is required for storage validation".to_string(),
        )
    })?;

    let mut persisted = storage_cfg::build_persisted_config(&request)?;
    let validation = storage_cfg::validate_storage(k8s, &persisted).await?;
    persisted.validation_json = Some(serde_json::to_value(&validation)?);

    let db_guard = state.db.read().await;
    if let Some(db) = db_guard.as_ref() {
        storage_cfg::save_storage_config_to_db(db, &persisted).await?;
    } else {
        storage_cfg::save_pre_db_secret(k8s, &persisted).await?;
    }

    Ok(Json(validation))
}

/// Save storage configuration selected during setup.
#[utoipa::path(
    post,
    path = "/api/setup/storage",
    tag = "Setup",
    request_body = storage_cfg::StorageConfigRequest,
    responses(
        (status = 200, body = storage_cfg::StorageConfigResponse)
    )
)]
async fn configure_storage(
    State(state): State<AppState>,
    Json(request): Json<storage_cfg::StorageConfigRequest>,
) -> Result<Json<storage_cfg::StorageConfigResponse>> {
    require_setup(&state).await?;

    let k8s_guard = state.k8s_client.read().await;
    let mut persisted = storage_cfg::build_persisted_config(&request)?;
    let existing = load_persisted_storage_config(&state).await?;
    if let Some(existing) = existing {
        if existing.clone().without_validation() == persisted {
            persisted.validation_json = existing.validation_json;
        }
    }

    let db_guard = state.db.read().await;
    let response = if let Some(db) = db_guard.as_ref() {
        let model = storage_cfg::save_storage_config_to_db(db, &persisted).await?;
        storage_cfg::response_from_persisted(persisted, Some(&model))
    } else {
        let k8s = k8s_guard.as_ref().ok_or_else(|| {
            AppError::ServiceUnavailable(
                "Kubernetes client is required before the database exists".to_string(),
            )
        })?;
        storage_cfg::save_pre_db_secret(k8s, &persisted).await?;
        storage_cfg::response_from_persisted(persisted, None)
    };

    Ok(Json(response))
}

/// Provision stable media PV/PVC resources for the Kubarr backend namespace.
#[utoipa::path(
    post,
    path = "/api/setup/storage/provision",
    tag = "Setup",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn provision_storage(State(state): State<AppState>) -> Result<Json<serde_json::Value>> {
    require_setup(&state).await?;

    let k8s_guard = state.k8s_client.read().await;
    let k8s = k8s_guard.as_ref().ok_or_else(|| {
        AppError::ServiceUnavailable(
            "Kubernetes client is required for storage provisioning".to_string(),
        )
    })?;
    let config = load_persisted_storage_config(&state)
        .await?
        .ok_or_else(|| AppError::BadRequest("Storage is not configured".to_string()))?;

    if !config.validated() {
        return Err(AppError::BadRequest(
            "Storage must be validated before provisioning".to_string(),
        ));
    }

    storage_cfg::provision_storage(k8s, &config).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Storage provisioned",
        "mount_path": storage_cfg::STORAGE_MOUNT_PATH,
        "claim": storage_cfg::MEDIA_PVC_NAME
    })))
}

async fn load_persisted_storage_config(
    state: &AppState,
) -> Result<Option<storage_cfg::PersistedStorageConfig>> {
    let db_guard = state.db.read().await;
    if let Some(db) = db_guard.as_ref() {
        if let Some((config, _)) = storage_cfg::get_storage_config_from_db(db).await? {
            return Ok(Some(config));
        }
    }
    drop(db_guard);

    let k8s_guard = state.k8s_client.read().await;
    if let Some(k8s) = k8s_guard.as_ref() {
        return storage_cfg::load_pre_db_secret(k8s).await;
    }

    Ok(None)
}

// ============================================================================
// Bootstrap Endpoints
// ============================================================================

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BootstrapStatusResponse {
    #[schema(inline)]
    pub components: Vec<ComponentStatus>,
    pub complete: bool,
    pub started: bool,
}

/// Get bootstrap status
///
/// Returns the current status of bootstrap components during initial setup.
/// Protected by require_setup() to prevent information disclosure after
/// admin user creation.
#[utoipa::path(
    get,
    path = "/api/setup/bootstrap/status",
    tag = "Setup",
    responses(
        (status = 200, body = BootstrapStatusResponse)
    )
)]
async fn get_bootstrap_status(
    State(state): State<AppState>,
) -> Result<Json<BootstrapStatusResponse>> {
    // Return 403 if setup is already complete
    require_setup(&state).await?;

    let bootstrap_service = BootstrapService::new(
        state.db.clone(),
        state.k8s_client.clone(),
        state.catalog.clone(),
        state.bootstrap_tx.clone(),
    );

    let components = bootstrap_service.get_status().await?;
    let complete = bootstrap_service.is_complete().await;
    let started = bootstrap_service.has_started().await;

    Ok(Json(BootstrapStatusResponse {
        components,
        complete,
        started,
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BootstrapStartResponse {
    pub message: String,
    pub started: bool,
}

/// Start the bootstrap process
#[utoipa::path(
    post,
    path = "/api/setup/bootstrap/start",
    tag = "Setup",
    responses(
        (status = 200, body = BootstrapStartResponse)
    )
)]
async fn start_bootstrap(State(state): State<AppState>) -> Result<Json<BootstrapStartResponse>> {
    require_setup(&state).await?;

    let storage_ready = {
        let db_guard = state.db.read().await;
        let k8s_guard = state.k8s_client.read().await;
        storage_cfg::storage_ready(db_guard.as_ref(), k8s_guard.as_ref()).await?
    };
    if !storage_ready {
        return Err(AppError::BadRequest(
            "Storage must be configured and validated before bootstrap can start".to_string(),
        ));
    }

    let bootstrap_service = Arc::new(RwLock::new(BootstrapService::new(
        state.db.clone(),
        state.k8s_client.clone(),
        state.catalog.clone(),
        state.bootstrap_tx.clone(),
    )));

    // Check if already complete
    {
        let service = bootstrap_service.read().await;
        if service.is_complete().await {
            return Ok(Json(BootstrapStartResponse {
                message: "Bootstrap already complete".to_string(),
                started: false,
            }));
        }
    }

    // Start bootstrap in background task
    let service_clone = bootstrap_service.clone();
    tokio::spawn(async move {
        let service = service_clone.read().await;
        if let Err(e) = service.start_bootstrap().await {
            tracing::error!("Bootstrap failed: {}", e);
        }
    });

    Ok(Json(BootstrapStartResponse {
        message: "Bootstrap started".to_string(),
        started: true,
    }))
}

/// Retry a failed bootstrap component
///
/// This endpoint allows retrying individual bootstrap steps if they fail during
/// the initial setup process. Protected by require_setup() to ensure it's only
/// accessible before admin user creation.
#[utoipa::path(
    post,
    path = "/api/setup/bootstrap/retry/{component}",
    tag = "Setup",
    params(
        ("component" = String, Path, description = "Component name to retry")
    ),
    responses(
        (status = 200, body = BootstrapStartResponse)
    )
)]
async fn retry_bootstrap_component(
    State(state): State<AppState>,
    axum::extract::Path(component): axum::extract::Path<String>,
) -> Result<Json<BootstrapStartResponse>> {
    // Return 403 if setup is already complete
    require_setup(&state).await?;

    let bootstrap_service = Arc::new(RwLock::new(BootstrapService::new(
        state.db.clone(),
        state.k8s_client.clone(),
        state.catalog.clone(),
        state.bootstrap_tx.clone(),
    )));

    // Retry in background task
    let component_clone = component.clone();
    tokio::spawn(async move {
        let service = bootstrap_service.read().await;
        if let Err(e) = service.retry_component(&component_clone).await {
            tracing::error!("Retry failed for {}: {}", component_clone, e);
        }
    });

    Ok(Json(BootstrapStartResponse {
        message: format!("Retrying {}", component),
        started: true,
    }))
}

/// WebSocket handler for bootstrap progress updates
async fn bootstrap_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    tracing::info!("Bootstrap WebSocket upgrade request received");
    ws.on_upgrade(move |socket| handle_bootstrap_socket(socket, state))
}

/// Handle bootstrap WebSocket connection
async fn handle_bootstrap_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to the shared bootstrap broadcast channel
    let mut rx = state.bootstrap_tx.subscribe();

    // Create a bootstrap service to get the initial status
    let bootstrap_service = BootstrapService::new(
        state.db.clone(),
        state.k8s_client.clone(),
        state.catalog.clone(),
        state.bootstrap_tx.clone(),
    );

    tracing::info!(
        "New WebSocket client connected for bootstrap updates, subscribers: {}",
        state.bootstrap_tx.receiver_count()
    );

    // Send initial status
    if let Ok(components) = bootstrap_service.get_status().await {
        let initial_status = serde_json::json!({
            "type": "initial_status",
            "components": components,
            "complete": bootstrap_service.is_complete().await,
        });
        if let Ok(json) = serde_json::to_string(&initial_status) {
            let _ = sender.send(Message::Text(json.into())).await;
        }
    }

    // Spawn task to forward broadcast messages to WebSocket
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages (ping/pong, close)
    let recv_task = tokio::spawn(async move {
        while let Some(result) = receiver.next().await {
            match result {
                Ok(Message::Ping(_)) => {
                    tracing::debug!("Received ping from WebSocket client");
                }
                Ok(Message::Close(_)) => {
                    tracing::debug!("WebSocket client requested close");
                    break;
                }
                Err(e) => {
                    tracing::debug!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    tracing::info!("Bootstrap WebSocket client disconnected");
}

// ============================================================================
// Server Config Endpoints
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ServerConfigRequest {
    pub name: String,
    pub storage_path: Option<String>,
    pub nfs_server: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ServerConfigResponse {
    pub name: String,
    pub storage_path: String,
    pub nfs_server: Option<String>,
}

/// Get server configuration
#[utoipa::path(
    get,
    path = "/api/setup/server",
    tag = "Setup",
    responses(
        (status = 200, body = Option<ServerConfigResponse>)
    )
)]
async fn get_server_config(
    State(state): State<AppState>,
) -> Result<Json<Option<ServerConfigResponse>>> {
    let db_guard = state.db.read().await;
    let config = if let Some(ref db) = *db_guard {
        bootstrap::get_server_config(db).await?
    } else {
        None
    };

    Ok(Json(config.map(|c| ServerConfigResponse {
        name: c.name,
        storage_path: c.storage_path,
        nfs_server: c.nfs_server,
    })))
}

/// Configure server name.
#[utoipa::path(
    post,
    path = "/api/setup/server",
    tag = "Setup",
    request_body = ServerConfigRequest,
    responses(
        (status = 200, body = ServerConfigResponse)
    )
)]
async fn configure_server(
    State(state): State<AppState>,
    Json(request): Json<ServerConfigRequest>,
) -> Result<Json<ServerConfigResponse>> {
    // Get database connection (required for this step)
    let db = state.get_db().await?;

    // Check if setup is required
    let admin_exists = admin_user_exists(&state).await?;
    if admin_exists {
        return Err(AppError::Forbidden(
            "Setup has already been completed".to_string(),
        ));
    }

    if request.name.trim().is_empty() {
        return Err(AppError::BadRequest("Server name is required".to_string()));
    }

    // Keep server_config.storage_path populated for older clients/tests, but the
    // storage contract is now represented by storage_config and mounted at /data.
    let config = bootstrap::save_server_config(
        &db,
        request.name.trim(),
        storage_cfg::STORAGE_MOUNT_PATH,
        None,
    )
    .await?;

    Ok(Json(ServerConfigResponse {
        name: config.name,
        storage_path: config.storage_path,
        nfs_server: config.nfs_server,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::bootstrap::ComponentStatus;

    #[test]
    fn setup_required_response_ser() {
        let r = SetupRequiredResponse {
            setup_required: true,
            database_pending: false,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"setup_required\":true"));
        assert!(json.contains("\"database_pending\":false"));
    }

    #[test]
    fn setup_status_response_ser() {
        let r = SetupStatusResponse {
            setup_required: true,
            bootstrap_complete: false,
            server_configured: false,
            admin_user_exists: false,
            storage_configured: false,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"setup_required\":true"));
        assert!(json.contains("\"bootstrap_complete\":false"));
        assert!(json.contains("\"admin_user_exists\":false"));
    }

    #[test]
    fn setup_request_deser() {
        let r: SetupRequest = serde_json::from_str(
            r#"{"admin_username":"admin","admin_email":"admin@example.com","admin_password":"secret"}"#,
        )
        .expect("deser");
        assert_eq!(r.admin_username, "admin");
        assert_eq!(r.admin_email, "admin@example.com");
        assert_eq!(r.admin_password, "secret");
    }

    #[test]
    fn generated_credentials_response_ser() {
        let r = GeneratedCredentialsResponse {
            admin_username: "admin".to_string(),
            admin_email: "admin@example.com".to_string(),
            admin_password: "generated_pass".to_string(),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"admin_username\":\"admin\""));
        assert!(json.contains("\"admin_email\""));
        assert!(json.contains("\"admin_password\""));
    }

    #[test]
    fn validate_path_query_deser() {
        let q: ValidatePathQuery =
            serde_json::from_str(r#"{"path":"/mnt/storage"}"#).expect("deser");
        assert_eq!(q.path, "/mnt/storage");
    }

    #[test]
    fn validate_path_response_ser() {
        let r = ValidatePathResponse {
            valid: true,
            exists: true,
            writable: true,
            message: "Path is valid and accessible".to_string(),
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"valid\":true"));
        assert!(json.contains("\"exists\":true"));
        assert!(json.contains("\"writable\":true"));
    }

    #[test]
    fn browse_setup_query_default_path() {
        let q: BrowseSetupQuery = serde_json::from_str("{}").expect("deser");
        assert_eq!(q.path, "/");
    }

    #[test]
    fn browse_setup_query_custom_path() {
        let q: BrowseSetupQuery = serde_json::from_str(r#"{"path":"/mnt"}"#).expect("deser");
        assert_eq!(q.path, "/mnt");
    }

    #[test]
    fn setup_directory_entry_ser() {
        let e = SetupDirectoryEntry {
            name: "storage".to_string(),
            path: "/mnt/storage".to_string(),
        };
        let json = serde_json::to_string(&e).expect("ser");
        assert!(json.contains("\"name\":\"storage\""));
        assert!(json.contains("\"path\":\"/mnt/storage\""));
    }

    #[test]
    fn browse_setup_response_ser() {
        let r = BrowseSetupResponse {
            path: "/mnt".to_string(),
            parent: Some("/".to_string()),
            directories: vec![SetupDirectoryEntry {
                name: "storage".to_string(),
                path: "/mnt/storage".to_string(),
            }],
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"path\":\"/mnt\""));
        assert!(json.contains("\"parent\":\"/\""));
        assert!(json.contains("\"directories\""));
    }

    #[test]
    fn browse_setup_response_null_parent() {
        let r = BrowseSetupResponse {
            path: "/".to_string(),
            parent: None,
            directories: vec![],
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"parent\":null"));
    }

    #[test]
    fn bootstrap_start_response_ser() {
        let r = BootstrapStartResponse {
            message: "Bootstrap started".to_string(),
            started: true,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"message\":\"Bootstrap started\""));
        assert!(json.contains("\"started\":true"));
    }

    #[test]
    fn bootstrap_status_response_ser() {
        let component = ComponentStatus {
            component: "postgresql".to_string(),
            display_name: "PostgreSQL".to_string(),
            status: "healthy".to_string(),
            message: Some("Running".to_string()),
            error: None,
        };
        let r = BootstrapStatusResponse {
            components: vec![component],
            complete: true,
            started: true,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"complete\":true"));
        assert!(json.contains("\"started\":true"));
        assert!(json.contains("\"components\""));
    }

    #[test]
    fn server_config_request_deser() {
        let r: ServerConfigRequest =
            serde_json::from_str(r#"{"name":"my-server","storage_path":"/mnt/storage"}"#)
                .expect("deser");
        assert_eq!(r.name, "my-server");
        assert_eq!(r.storage_path.as_deref(), Some("/mnt/storage"));
    }

    #[test]
    fn server_config_response_ser() {
        let r = ServerConfigResponse {
            name: "my-server".to_string(),
            storage_path: "/mnt/storage".to_string(),
            nfs_server: None,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"name\":\"my-server\""));
        assert!(json.contains("\"storage_path\":\"/mnt/storage\""));
    }
}
