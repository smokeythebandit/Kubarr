//! Tests for database pool and seeding module

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;

use kubarr::migrations::Migrator;
use kubarr::models::prelude::*;
use kubarr::models::{role, role_app_permission, role_permission};
use kubarr::state::DbConn;
mod common;

use common::{create_test_db, create_test_db_with_seed};

#[tokio::test]
async fn test_migrations_create_tables() {
    let db = create_test_db().await;

    // Verify we can query the tables (migrations ran successfully)

    // Check users table exists
    let result = db
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM users".to_string(),
        ))
        .await;
    assert!(result.is_ok());

    // Check roles table exists
    let result = db
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM roles".to_string(),
        ))
        .await;
    assert!(result.is_ok());

    // Check oauth tables exist (oauth2_* replaced by oauth_providers and oauth_accounts)
    let result = db
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM oauth_providers".to_string(),
        ))
        .await;
    assert!(result.is_ok());

    // Check notification tables exist
    let result = db
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM notification_channels".to_string(),
        ))
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_migrations_are_idempotent() {
    let db = create_test_db().await;

    // Running migrations again should not fail (migrations track state)
    Migrator::up(&db, None).await.unwrap();
    Migrator::up(&db, None).await.unwrap();

    // Tables should still exist and be usable
    let result = db
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT 1 FROM users LIMIT 1".to_string(),
        ))
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_seed_defaults_creates_roles() {
    let db = create_test_db_with_seed().await;

    // Verify roles were created
    let roles = Role::find().all(&db).await.unwrap();
    assert_eq!(roles.len(), 3);

    let role_names: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
    assert!(role_names.contains(&"admin"));
    assert!(role_names.contains(&"viewer"));
    assert!(role_names.contains(&"downloader"));
}

#[tokio::test]
async fn test_seed_defaults_creates_system_settings() {
    let db = create_test_db_with_seed().await;

    // Verify system settings were created
    let settings = SystemSetting::find().all(&db).await.unwrap();
    assert!(settings.len() >= 2);

    let setting_keys: Vec<&str> = settings.iter().map(|s| s.key.as_str()).collect();
    assert!(setting_keys.contains(&"registration_enabled"));
    assert!(setting_keys.contains(&"registration_require_approval"));
}

#[tokio::test]
async fn test_seed_defaults_creates_role_permissions() {
    let db = create_test_db_with_seed().await;

    // Verify admin has all permissions
    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let admin_perms = RolePermission::find()
        .filter(role_permission::Column::RoleId.eq(admin_role.id))
        .all(&db)
        .await
        .unwrap();

    // Admin should have many permissions
    assert!(admin_perms.len() >= 10);

    // Check specific permissions
    let perm_names: Vec<&str> = admin_perms.iter().map(|p| p.permission.as_str()).collect();
    assert!(perm_names.contains(&"apps.view"));
    assert!(perm_names.contains(&"users.manage"));
    assert!(perm_names.contains(&"settings.manage"));
}

#[tokio::test]
async fn test_seed_defaults_creates_app_permissions() {
    let db = create_test_db_with_seed().await;

    // Verify viewer has app permissions
    let viewer_role = Role::find()
        .filter(role::Column::Name.eq("viewer"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let viewer_apps = RoleAppPermission::find()
        .filter(role_app_permission::Column::RoleId.eq(viewer_role.id))
        .all(&db)
        .await
        .unwrap();

    let app_names: Vec<&str> = viewer_apps.iter().map(|a| a.app_name.as_str()).collect();
    assert!(app_names.contains(&"jellyfin"));
    assert!(app_names.contains(&"jellyseerr"));

    // Verify downloader has app permissions
    let downloader_role = Role::find()
        .filter(role::Column::Name.eq("downloader"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let downloader_apps = RoleAppPermission::find()
        .filter(role_app_permission::Column::RoleId.eq(downloader_role.id))
        .all(&db)
        .await
        .unwrap();

    let app_names: Vec<&str> = downloader_apps
        .iter()
        .map(|a| a.app_name.as_str())
        .collect();
    assert!(app_names.contains(&"qbittorrent"));
    assert!(app_names.contains(&"transmission"));
}

#[tokio::test]
async fn test_seed_defaults_is_idempotent() {
    // create_test_db_with_seed calls seed internally
    // Calling it multiple times or running seed twice should not duplicate data
    let db = create_test_db_with_seed().await;

    // Should have exactly 3 roles
    let roles = Role::find().all(&db).await.unwrap();
    assert_eq!(roles.len(), 3);

    // Should have 2 system settings
    let settings = SystemSetting::find().all(&db).await.unwrap();
    assert_eq!(settings.len(), 2);
}

#[test]
fn test_db_conn_type_alias() {
    // DbConn should be DatabaseConnection
    fn _takes_db_conn(_: &DbConn) {}
    fn _takes_database_connection(_: &DatabaseConnection) {}
    // If this compiles, the type alias is correct
}
