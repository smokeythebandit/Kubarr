use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::models::prelude::*;
use crate::models::{role_app_permission, role_permission, user_role};
use crate::state::DbConn;

/// Get all permissions for a user (from all their roles)
/// Includes app.* permissions based on role_app_permissions
pub async fn get_user_permissions(db: &DbConn, user_id: i64) -> Vec<String> {
    // Get all role IDs for this user
    let user_roles = UserRole::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .all(db)
        .await
        .unwrap_or_default();

    let role_ids: Vec<i64> = user_roles.iter().map(|ur| ur.role_id).collect();

    if role_ids.is_empty() {
        return vec![];
    }

    // Get all permissions from all roles
    let permissions = RolePermission::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids.clone()))
        .all(db)
        .await
        .unwrap_or_default();

    let mut unique_perms: Vec<String> = permissions.iter().map(|p| p.permission.clone()).collect();

    // Get app permissions and convert to app.{name} format
    let app_permissions = RoleAppPermission::find()
        .filter(role_app_permission::Column::RoleId.is_in(role_ids))
        .all(db)
        .await
        .unwrap_or_default();

    for app_perm in app_permissions {
        unique_perms.push(format!("app.{}", app_perm.app_name));
    }

    // Deduplicate and return
    unique_perms.sort();
    unique_perms.dedup();
    unique_perms
}

/// Get all app names a user has access to
/// Returns vec!["*"] if user has app.* permission (all apps access)
pub async fn get_user_app_access(db: &DbConn, user_id: i64) -> Vec<String> {
    // Get all role IDs for this user
    let user_roles = UserRole::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .all(db)
        .await
        .unwrap_or_default();

    let role_ids: Vec<i64> = user_roles.iter().map(|ur| ur.role_id).collect();

    if role_ids.is_empty() {
        return vec![];
    }

    // Check for app.* wildcard permission
    let has_wildcard = RolePermission::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids.clone()))
        .filter(role_permission::Column::Permission.eq("app.*"))
        .one(db)
        .await
        .map(|r| r.is_some())
        .unwrap_or(false);

    if has_wildcard {
        return vec!["*".to_string()];
    }

    // Get all app permissions from all roles
    let app_permissions = RoleAppPermission::find()
        .filter(role_app_permission::Column::RoleId.is_in(role_ids))
        .all(db)
        .await
        .unwrap_or_default();

    // Deduplicate and return
    let mut unique_apps: Vec<String> = app_permissions.iter().map(|p| p.app_name.clone()).collect();
    unique_apps.sort();
    unique_apps.dedup();
    unique_apps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{role, role_app_permission, role_permission, user, user_role};
    use chrono::Utc;
    use sea_orm::{ActiveModelTrait, Set};

    async fn make_db() -> DbConn {
        crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory SQLite")
    }

    async fn insert_user(db: &DbConn) -> user::Model {
        let now = Utc::now();
        user::ActiveModel {
            username: Set(format!("user_{}", now.timestamp_nanos_opt().unwrap_or(0))),
            email: Set(format!(
                "u{}@test.com",
                now.timestamp_nanos_opt().unwrap_or(0)
            )),
            hashed_password: Set("hash".to_string()),
            is_active: Set(true),
            is_approved: Set(true),
            totp_secret: Set(None),
            totp_enabled: Set(false),
            totp_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert user")
    }

    async fn insert_role(db: &DbConn, name: &str) -> role::Model {
        role::ActiveModel {
            name: Set(name.to_string()),
            description: Set(None),
            is_system: Set(false),
            requires_2fa: Set(false),
            created_at: Set(Utc::now()),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert role")
    }

    async fn assign_role(db: &DbConn, user_id: i64, role_id: i64) {
        user_role::ActiveModel {
            user_id: Set(user_id),
            role_id: Set(role_id),
        }
        .insert(db)
        .await
        .expect("assign role");
    }

    async fn add_permission(db: &DbConn, role_id: i64, permission: &str) {
        role_permission::ActiveModel {
            role_id: Set(role_id),
            permission: Set(permission.to_string()),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("add permission");
    }

    async fn add_app_permission(db: &DbConn, role_id: i64, app_name: &str) {
        role_app_permission::ActiveModel {
            role_id: Set(role_id),
            app_name: Set(app_name.to_string()),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("add app permission");
    }

    // -----------------------------------------------------------------------
    // get_user_permissions
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_user_permissions_no_roles_returns_empty() {
        let db = make_db().await;
        let user = insert_user(&db).await;
        let perms = get_user_permissions(&db, user.id).await;
        assert!(perms.is_empty());
    }

    #[tokio::test]
    async fn get_user_permissions_with_role_and_permissions() {
        let db = make_db().await;
        let user = insert_user(&db).await;
        let role = insert_role(&db, "test_perm_role").await;
        assign_role(&db, user.id, role.id).await;
        add_permission(&db, role.id, "apps.view").await;
        add_permission(&db, role.id, "apps.install").await;

        let mut perms = get_user_permissions(&db, user.id).await;
        perms.sort();
        assert!(perms.contains(&"apps.install".to_string()));
        assert!(perms.contains(&"apps.view".to_string()));
    }

    #[tokio::test]
    async fn get_user_permissions_with_app_permissions() {
        let db = make_db().await;
        let user = insert_user(&db).await;
        let role = insert_role(&db, "test_app_perm_role").await;
        assign_role(&db, user.id, role.id).await;
        add_app_permission(&db, role.id, "sonarr").await;
        add_app_permission(&db, role.id, "radarr").await;

        let perms = get_user_permissions(&db, user.id).await;
        assert!(perms.contains(&"app.sonarr".to_string()));
        assert!(perms.contains(&"app.radarr".to_string()));
    }

    #[tokio::test]
    async fn get_user_permissions_deduplication() {
        let db = make_db().await;
        let user = insert_user(&db).await;
        let role1 = insert_role(&db, "dedup_role1").await;
        let role2 = insert_role(&db, "dedup_role2").await;
        assign_role(&db, user.id, role1.id).await;
        assign_role(&db, user.id, role2.id).await;
        // Same permission on both roles — should deduplicate
        add_permission(&db, role1.id, "apps.view").await;
        add_permission(&db, role2.id, "apps.view").await;

        let perms = get_user_permissions(&db, user.id).await;
        assert_eq!(perms.iter().filter(|p| *p == "apps.view").count(), 1);
    }

    // -----------------------------------------------------------------------
    // get_user_app_access
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_user_app_access_no_roles_returns_empty() {
        let db = make_db().await;
        let user = insert_user(&db).await;
        let apps = get_user_app_access(&db, user.id).await;
        assert!(apps.is_empty());
    }

    #[tokio::test]
    async fn get_user_app_access_wildcard_returns_star() {
        let db = make_db().await;
        let user = insert_user(&db).await;
        let role = insert_role(&db, "wildcard_role").await;
        assign_role(&db, user.id, role.id).await;
        add_permission(&db, role.id, "app.*").await;

        let apps = get_user_app_access(&db, user.id).await;
        assert_eq!(apps, vec!["*".to_string()]);
    }

    #[tokio::test]
    async fn get_user_app_access_specific_apps() {
        let db = make_db().await;
        let user = insert_user(&db).await;
        let role = insert_role(&db, "specific_apps_role").await;
        assign_role(&db, user.id, role.id).await;
        add_app_permission(&db, role.id, "sonarr").await;
        add_app_permission(&db, role.id, "radarr").await;

        let mut apps = get_user_app_access(&db, user.id).await;
        apps.sort();
        assert_eq!(apps, vec!["radarr".to_string(), "sonarr".to_string()]);
    }

    #[tokio::test]
    async fn get_user_app_access_with_role_but_no_app_perms() {
        let db = make_db().await;
        let user = insert_user(&db).await;
        let role = insert_role(&db, "no_app_perms_role").await;
        assign_role(&db, user.id, role.id).await;
        add_permission(&db, role.id, "apps.view").await; // non-wildcard, non-app permission

        let apps = get_user_app_access(&db, user.id).await;
        assert!(apps.is_empty());
    }
}
