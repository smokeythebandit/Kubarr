use axum::{
    extract::{Path, State},
    routing::{get, put},
    Json, Router,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::middleware::permissions::{Authorized, RolesManage, RolesView};
use crate::models::prelude::*;
use crate::models::{role, role_app_permission, role_permission};
use crate::state::AppState;

/// Create roles routes
pub fn roles_routes(state: AppState) -> Router {
    Router::new()
        .route("/", get(list_roles).post(create_role))
        .route("/permissions", get(list_all_permissions))
        .route(
            "/{role_id}",
            get(get_role).patch(update_role).delete(delete_role),
        )
        .route("/{role_id}/apps", put(set_role_apps))
        .route(
            "/{role_id}/permissions",
            get(get_role_permissions).put(set_role_permissions),
        )
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRoleRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub app_names: Vec<String>,
    #[serde(default)]
    pub requires_2fa: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub requires_2fa: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetRoleApps {
    pub app_names: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RoleWithAppsResponse {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    pub requires_2fa: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub app_names: Vec<String>,
    pub permissions: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetRolePermissions {
    pub permissions: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PermissionInfo {
    pub key: String,
    pub category: String,
    pub description: String,
}

// ============================================================================
// Helper Functions
// ============================================================================

async fn get_role_with_apps(state: &AppState, role_id: i64) -> Result<RoleWithAppsResponse> {
    let db = state.get_db().await?;
    let found_role = Role::find_by_id(role_id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound("Role not found".to_string()))?;

    let app_permissions = RoleAppPermission::find()
        .filter(role_app_permission::Column::RoleId.eq(role_id))
        .all(&db)
        .await?;

    let role_perms = RolePermission::find()
        .filter(role_permission::Column::RoleId.eq(role_id))
        .all(&db)
        .await?;

    // Convert app permissions to app.* format and merge with regular permissions
    let app_names: Vec<String> = app_permissions.iter().map(|p| p.app_name.clone()).collect();
    let mut permissions: Vec<String> = role_perms.into_iter().map(|p| p.permission).collect();

    // Add app.* permissions derived from role_app_permissions
    for app_name in &app_names {
        permissions.push(format!("app.{}", app_name));
    }

    Ok(RoleWithAppsResponse {
        id: found_role.id,
        name: found_role.name,
        description: found_role.description,
        is_system: found_role.is_system,
        requires_2fa: found_role.requires_2fa,
        created_at: found_role.created_at,
        app_names,
        permissions,
    })
}

// ============================================================================
// Endpoint Handlers
// ============================================================================

/// List all roles (requires roles.view permission)
#[utoipa::path(
    get,
    path = "/api/roles",
    tag = "Roles",
    responses((status = 200, body = Vec<RoleWithAppsResponse>))
)]
async fn list_roles(
    State(state): State<AppState>,
    _auth: Authorized<RolesView>,
) -> Result<Json<Vec<RoleWithAppsResponse>>> {
    let db = state.get_db().await?;
    let roles = Role::find().all(&db).await?;

    let mut responses = Vec::new();
    for r in roles {
        responses.push(get_role_with_apps(&state, r.id).await?);
    }

    Ok(Json(responses))
}

/// Get role by ID (requires roles.view permission)
#[utoipa::path(
    get,
    path = "/api/roles/{role_id}",
    tag = "Roles",
    params(("role_id" = i64, Path, description = "Role ID")),
    responses((status = 200, body = RoleWithAppsResponse))
)]
async fn get_role(
    State(state): State<AppState>,
    Path(role_id): Path<i64>,
    _auth: Authorized<RolesView>,
) -> Result<Json<RoleWithAppsResponse>> {
    let response = get_role_with_apps(&state, role_id).await?;
    Ok(Json(response))
}

/// Create a new role (requires roles.manage permission)
#[utoipa::path(
    post,
    path = "/api/roles",
    tag = "Roles",
    request_body = CreateRoleRequest,
    responses((status = 200, body = RoleWithAppsResponse))
)]
async fn create_role(
    State(state): State<AppState>,
    _auth: Authorized<RolesManage>,
    Json(data): Json<CreateRoleRequest>,
) -> Result<Json<RoleWithAppsResponse>> {
    let db = state.get_db().await?;
    // Check if role name exists
    let existing = Role::find()
        .filter(role::Column::Name.eq(&data.name))
        .one(&db)
        .await?;

    if existing.is_some() {
        return Err(AppError::BadRequest("Role name already exists".to_string()));
    }

    let now = Utc::now();

    // Create role
    let new_role = role::ActiveModel {
        name: Set(data.name),
        description: Set(data.description),
        is_system: Set(false),
        requires_2fa: Set(data.requires_2fa),
        created_at: Set(now),
        ..Default::default()
    };

    let created_role = new_role.insert(&db).await?;

    // Add app permissions
    for app_name in &data.app_names {
        let permission = role_app_permission::ActiveModel {
            role_id: Set(created_role.id),
            app_name: Set(app_name.clone()),
            ..Default::default()
        };
        permission.insert(&db).await?;
    }

    let response = get_role_with_apps(&state, created_role.id).await?;
    Ok(Json(response))
}

/// Update role (requires roles.manage permission)
#[utoipa::path(
    patch,
    path = "/api/roles/{role_id}",
    tag = "Roles",
    params(("role_id" = i64, Path, description = "Role ID")),
    request_body = UpdateRoleRequest,
    responses((status = 200, body = RoleWithAppsResponse))
)]
async fn update_role(
    State(state): State<AppState>,
    Path(role_id): Path<i64>,
    _auth: Authorized<RolesManage>,
    Json(data): Json<UpdateRoleRequest>,
) -> Result<Json<RoleWithAppsResponse>> {
    let db = state.get_db().await?;
    let existing_role = Role::find_by_id(role_id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound("Role not found".to_string()))?;

    // Prevent renaming system roles
    if existing_role.is_system
        && data.name.is_some()
        && data.name.as_ref() != Some(&existing_role.name)
    {
        return Err(AppError::BadRequest(
            "Cannot rename system roles".to_string(),
        ));
    }

    // Check for duplicate name
    if let Some(ref new_name) = data.name {
        if new_name != &existing_role.name {
            let existing = Role::find()
                .filter(role::Column::Name.eq(new_name))
                .one(&db)
                .await?;

            if existing.is_some() {
                return Err(AppError::BadRequest("Role name already exists".to_string()));
            }
        }
    }

    // Update fields
    let mut role_model: role::ActiveModel = existing_role.into();

    if let Some(name) = data.name {
        role_model.name = Set(name);
    }
    if let Some(description) = data.description {
        role_model.description = Set(Some(description));
    }
    if let Some(requires_2fa) = data.requires_2fa {
        role_model.requires_2fa = Set(requires_2fa);
    }

    role_model.update(&db).await?;

    let response = get_role_with_apps(&state, role_id).await?;
    Ok(Json(response))
}

/// Delete a role (requires roles.manage permission)
#[utoipa::path(
    delete,
    path = "/api/roles/{role_id}",
    tag = "Roles",
    params(("role_id" = i64, Path, description = "Role ID")),
    responses((status = 200, body = serde_json::Value))
)]
async fn delete_role(
    State(state): State<AppState>,
    Path(role_id): Path<i64>,
    _auth: Authorized<RolesManage>,
) -> Result<Json<serde_json::Value>> {
    let db = state.get_db().await?;
    let existing_role = Role::find_by_id(role_id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound("Role not found".to_string()))?;

    if existing_role.is_system {
        return Err(AppError::BadRequest(
            "Cannot delete system roles".to_string(),
        ));
    }

    existing_role.delete(&db).await?;

    Ok(Json(serde_json::json!({"message": "Role deleted"})))
}

/// Set app permissions for a role (requires roles.manage permission)
#[utoipa::path(
    put,
    path = "/api/roles/{role_id}/apps",
    tag = "Roles",
    params(("role_id" = i64, Path, description = "Role ID")),
    request_body = SetRoleApps,
    responses((status = 200, body = RoleWithAppsResponse))
)]
async fn set_role_apps(
    State(state): State<AppState>,
    Path(role_id): Path<i64>,
    _auth: Authorized<RolesManage>,
    Json(data): Json<SetRoleApps>,
) -> Result<Json<RoleWithAppsResponse>> {
    let db = state.get_db().await?;
    // Verify role exists
    let _ = Role::find_by_id(role_id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound("Role not found".to_string()))?;

    // Delete existing permissions
    RoleAppPermission::delete_many()
        .filter(role_app_permission::Column::RoleId.eq(role_id))
        .exec(&db)
        .await?;

    // Add new permissions
    for app_name in &data.app_names {
        let permission = role_app_permission::ActiveModel {
            role_id: Set(role_id),
            app_name: Set(app_name.clone()),
            ..Default::default()
        };
        permission.insert(&db).await?;
    }

    let response = get_role_with_apps(&state, role_id).await?;
    Ok(Json(response))
}

/// Get all available permissions with descriptions
#[utoipa::path(
    get,
    path = "/api/roles/permissions",
    tag = "Roles",
    responses((status = 200, body = Vec<PermissionInfo>))
)]
async fn list_all_permissions(_auth: Authorized<RolesView>) -> Result<Json<Vec<PermissionInfo>>> {
    let mut permissions = vec![
        // Apps permissions
        PermissionInfo {
            key: "apps.view".to_string(),
            category: "Apps".to_string(),
            description: "View app catalog and installed apps".to_string(),
        },
        PermissionInfo {
            key: "apps.install".to_string(),
            category: "Apps".to_string(),
            description: "Install new applications".to_string(),
        },
        PermissionInfo {
            key: "apps.delete".to_string(),
            category: "Apps".to_string(),
            description: "Delete installed applications".to_string(),
        },
        PermissionInfo {
            key: "apps.restart".to_string(),
            category: "Apps".to_string(),
            description: "Restart application pods".to_string(),
        },
        // Storage permissions
        PermissionInfo {
            key: "storage.view".to_string(),
            category: "Storage".to_string(),
            description: "Browse storage and files".to_string(),
        },
        PermissionInfo {
            key: "storage.write".to_string(),
            category: "Storage".to_string(),
            description: "Create directories".to_string(),
        },
        PermissionInfo {
            key: "storage.delete".to_string(),
            category: "Storage".to_string(),
            description: "Delete files and directories".to_string(),
        },
        PermissionInfo {
            key: "storage.download".to_string(),
            category: "Storage".to_string(),
            description: "Download files".to_string(),
        },
        // Logs permissions
        PermissionInfo {
            key: "logs.view".to_string(),
            category: "Logs".to_string(),
            description: "View pod and application logs".to_string(),
        },
        // Monitoring permissions
        PermissionInfo {
            key: "monitoring.view".to_string(),
            category: "Monitoring".to_string(),
            description: "View metrics and monitoring data".to_string(),
        },
        // Users permissions
        PermissionInfo {
            key: "users.view".to_string(),
            category: "Users".to_string(),
            description: "View user list".to_string(),
        },
        PermissionInfo {
            key: "users.manage".to_string(),
            category: "Users".to_string(),
            description: "Create, edit, and delete users".to_string(),
        },
        // Roles permissions
        PermissionInfo {
            key: "roles.view".to_string(),
            category: "Roles".to_string(),
            description: "View roles".to_string(),
        },
        PermissionInfo {
            key: "roles.manage".to_string(),
            category: "Roles".to_string(),
            description: "Create, edit, and delete roles".to_string(),
        },
        // Settings permissions
        PermissionInfo {
            key: "settings.view".to_string(),
            category: "Settings".to_string(),
            description: "View system settings".to_string(),
        },
        PermissionInfo {
            key: "settings.manage".to_string(),
            category: "Settings".to_string(),
            description: "Modify system settings".to_string(),
        },
        // VPN permissions
        PermissionInfo {
            key: "vpn.view".to_string(),
            category: "VPN".to_string(),
            description: "View VPN providers and app VPN configurations".to_string(),
        },
        PermissionInfo {
            key: "vpn.manage".to_string(),
            category: "VPN".to_string(),
            description: "Manage VPN providers and assign VPN to apps".to_string(),
        },
    ];

    // Add app access permissions
    // These are the apps that the backend can proxy to
    let app_permissions = vec![
        ("sonarr", "Access Sonarr TV show manager"),
        ("radarr", "Access Radarr movie manager"),
        ("qbittorrent", "Access qBittorrent download client"),
        ("transmission", "Access Transmission download client"),
        ("deluge", "Access Deluge download client"),
        ("rutorrent", "Access ruTorrent web UI"),
        ("jellyfin", "Access Jellyfin media server"),
        ("plex", "Access Plex media server"),
        ("jackett", "Access Jackett indexer proxy"),
        ("jellyseerr", "Access Jellyseerr request manager"),
        ("sabnzbd", "Access SABnzbd Usenet client"),
        ("grafana", "Access Grafana dashboards"),
        ("victoriametrics", "Access VictoriaMetrics"),
        ("victorialogs", "Access VictoriaLogs log storage"),
        ("kubernetes-dashboard", "Access Kubernetes Dashboard"),
    ];

    for (app_name, description) in app_permissions {
        permissions.push(PermissionInfo {
            key: format!("app.{}", app_name),
            category: "App Access".to_string(),
            description: description.to_string(),
        });
    }

    Ok(Json(permissions))
}

/// Get permissions for a specific role
#[utoipa::path(
    get,
    path = "/api/roles/{role_id}/permissions",
    tag = "Roles",
    params(("role_id" = i64, Path, description = "Role ID")),
    responses((status = 200, body = Vec<String>))
)]
async fn get_role_permissions(
    State(state): State<AppState>,
    Path(role_id): Path<i64>,
    _auth: Authorized<RolesView>,
) -> Result<Json<Vec<String>>> {
    let db = state.get_db().await?;
    // Verify role exists
    let _ = Role::find_by_id(role_id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound("Role not found".to_string()))?;

    let role_perms = RolePermission::find()
        .filter(role_permission::Column::RoleId.eq(role_id))
        .all(&db)
        .await?;

    let permissions: Vec<String> = role_perms.into_iter().map(|p| p.permission).collect();
    Ok(Json(permissions))
}

/// Set permissions for a role (requires roles.manage permission)
/// Handles both regular permissions and app.* permissions
/// App permissions (app.sonarr, app.radarr, etc.) are synced with role_app_permissions table
#[utoipa::path(
    put,
    path = "/api/roles/{role_id}/permissions",
    tag = "Roles",
    params(("role_id" = i64, Path, description = "Role ID")),
    request_body = SetRolePermissions,
    responses((status = 200, body = RoleWithAppsResponse))
)]
async fn set_role_permissions(
    State(state): State<AppState>,
    Path(role_id): Path<i64>,
    _auth: Authorized<RolesManage>,
    Json(data): Json<SetRolePermissions>,
) -> Result<Json<RoleWithAppsResponse>> {
    let db = state.get_db().await?;
    // Verify role exists
    let _ = Role::find_by_id(role_id)
        .one(&db)
        .await?
        .ok_or_else(|| AppError::NotFound("Role not found".to_string()))?;

    // Separate app.* permissions from regular permissions
    let mut regular_permissions = Vec::new();
    let mut app_names = Vec::new();

    for permission in &data.permissions {
        if let Some(app_name) = permission.strip_prefix("app.") {
            app_names.push(app_name.to_string());
        } else {
            regular_permissions.push(permission.clone());
        }
    }

    // Delete existing regular permissions
    RolePermission::delete_many()
        .filter(role_permission::Column::RoleId.eq(role_id))
        .exec(&db)
        .await?;

    // Add new regular permissions
    for permission in &regular_permissions {
        let perm = role_permission::ActiveModel {
            role_id: Set(role_id),
            permission: Set(permission.clone()),
            ..Default::default()
        };
        perm.insert(&db).await?;
    }

    // Delete existing app permissions
    RoleAppPermission::delete_many()
        .filter(role_app_permission::Column::RoleId.eq(role_id))
        .exec(&db)
        .await?;

    // Add new app permissions
    for app_name in &app_names {
        let app_perm = role_app_permission::ActiveModel {
            role_id: Set(role_id),
            app_name: Set(app_name.clone()),
            ..Default::default()
        };
        app_perm.insert(&db).await?;
    }

    let response = get_role_with_apps(&state, role_id).await?;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_role_request_deser_minimal() {
        let r: CreateRoleRequest = serde_json::from_str(r#"{"name":"viewer"}"#).expect("deser");
        assert_eq!(r.name, "viewer");
        assert_eq!(r.description, None);
        assert_eq!(r.app_names, Vec::<String>::new());
        assert!(!r.requires_2fa);
    }

    #[test]
    fn create_role_request_deser_full() {
        let r: CreateRoleRequest = serde_json::from_str(
            r#"{"name":"power","description":"Power user","app_names":["sonarr"],"requires_2fa":true}"#,
        )
        .expect("deser");
        assert_eq!(r.name, "power");
        assert_eq!(r.description, Some("Power user".to_string()));
        assert_eq!(r.app_names, vec!["sonarr"]);
        assert!(r.requires_2fa);
    }

    #[test]
    fn update_role_request_deser_empty() {
        let r: UpdateRoleRequest = serde_json::from_str("{}").expect("deser");
        assert_eq!(r.name, None);
        assert_eq!(r.description, None);
        assert_eq!(r.requires_2fa, None);
    }

    #[test]
    fn update_role_request_deser_partial() {
        let r: UpdateRoleRequest = serde_json::from_str(r#"{"name":"admin"}"#).expect("deser");
        assert_eq!(r.name, Some("admin".to_string()));
        assert_eq!(r.description, None);
    }

    #[test]
    fn set_role_apps_deser() {
        let r: SetRoleApps =
            serde_json::from_str(r#"{"app_names":["radarr","sonarr"]}"#).expect("deser");
        assert_eq!(r.app_names, vec!["radarr", "sonarr"]);
    }

    #[test]
    fn set_role_apps_deser_empty() {
        let r: SetRoleApps = serde_json::from_str(r#"{"app_names":[]}"#).expect("deser");
        assert!(r.app_names.is_empty());
    }

    #[test]
    fn role_with_apps_response_ser() {
        let r = RoleWithAppsResponse {
            id: 1,
            name: "admin".to_string(),
            description: Some("Admin role".to_string()),
            is_system: true,
            requires_2fa: false,
            created_at: chrono::Utc::now(),
            app_names: vec!["sonarr".to_string()],
            permissions: vec!["apps.view".to_string()],
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"name\":\"admin\""));
        assert!(json.contains("\"is_system\":true"));
        assert!(json.contains("\"app_names\""));
        assert!(json.contains("\"permissions\""));
    }

    #[test]
    fn set_role_permissions_deser() {
        let r: SetRolePermissions =
            serde_json::from_str(r#"{"permissions":["apps.view","logs.view"]}"#).expect("deser");
        assert_eq!(r.permissions, vec!["apps.view", "logs.view"]);
    }

    #[test]
    fn permission_info_ser() {
        let p = PermissionInfo {
            key: "apps.view".to_string(),
            category: "Apps".to_string(),
            description: "View app catalog".to_string(),
        };
        let json = serde_json::to_string(&p).expect("ser");
        assert!(json.contains("\"key\":\"apps.view\""));
        assert!(json.contains("\"category\":\"Apps\""));
        assert!(json.contains("\"description\""));
    }
}
