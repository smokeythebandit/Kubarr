//! Test helpers and utilities for unit and integration testing.
//!
//! This module provides common utilities for setting up test environments,
//! creating mock data, and testing database operations.

#![allow(dead_code)]

use std::sync::Arc;
use tokio::sync::RwLock;

use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;

use kubarr::migrations::Migrator;
use kubarr::services::audit::AuditService;
use kubarr::services::catalog::AppCatalog;
use kubarr::services::chart_sync::ChartSyncService;
use kubarr::services::notification::NotificationService;
use kubarr::state::{AppState, SharedCatalog, SharedK8sClient};

/// Build a test AppState from an existing DatabaseConnection.
///
/// Use this when you need direct control over the database (e.g., to seed
/// specific users before constructing the state). Also initialises the
/// `NotificationService` and `AuditService` internal DB connections so that
/// their methods (e.g. `get_unread_count`, `log`) work in integration tests —
/// mirroring what the production bootstrapper does.
pub async fn build_test_app_state_with_db(db: DatabaseConnection) -> AppState {
    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog: SharedCatalog = Arc::new(RwLock::new(AppCatalog::default()));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    // Mirror what the bootstrapper does: give both services their own DB handle.
    audit.set_db(db.clone()).await;
    notification.set_db(db.clone()).await;

    AppState::new(
        Some(db),
        k8s_client,
        catalog,
        chart_sync,
        audit,
        notification,
    )
}

/// Build a test AppState with a seeded database.
pub async fn build_test_app_state() -> AppState {
    let db = create_test_db_with_seed().await;
    build_test_app_state_with_db(db).await
}

/// Build a test AppState with NO database connection (db = None).
///
/// Use this to test code paths that execute when the database is unavailable,
/// such as `admin_user_exists` returning `Ok(false)` on line 85 of setup.rs.
pub async fn build_test_app_state_no_db() -> AppState {
    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog: SharedCatalog = Arc::new(RwLock::new(AppCatalog::default()));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    AppState::new(None, k8s_client, catalog, chart_sync, audit, notification)
}

/// Create an in-memory SQLite database for testing
pub async fn create_test_db() -> DatabaseConnection {
    // Use simple in-memory SQLite - each connection gets its own database
    let db_url = "sqlite::memory:";

    let db = Database::connect(db_url)
        .await
        .expect("Failed to create test database");

    // Run migrations using the Migrator
    Migrator::up(&db, None)
        .await
        .expect("Failed to run test migrations");

    db
}

/// Create a test database with seeded default data (roles, permissions)
pub async fn create_test_db_with_seed() -> DatabaseConnection {
    let db = create_test_db().await;
    seed_test_data(&db).await;
    db
}

/// Seed default test data into the database
/// Note: Most seed data is now inserted by migrations, so we only add
/// data that isn't already present
pub async fn seed_test_data(db: &DatabaseConnection) {
    use kubarr::models::prelude::*;
    use kubarr::models::{role, role_app_permission, role_permission, system_setting};
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    let now = chrono::Utc::now();

    // Create system settings (only if not already present from migrations)
    let default_settings = [
        (
            "registration_enabled",
            "true",
            "Allow new user registration",
        ),
        (
            "registration_require_approval",
            "true",
            "Require admin approval for new registrations",
        ),
    ];

    for (key, value, description) in default_settings {
        // Check if setting already exists
        if SystemSetting::find_by_id(key)
            .one(db)
            .await
            .unwrap()
            .is_none()
        {
            let setting = system_setting::ActiveModel {
                key: Set(key.to_string()),
                value: Set(value.to_string()),
                description: Set(Some(description.to_string())),
                updated_at: Set(now),
            };
            setting.insert(db).await.unwrap();
        }
    }

    // Create default roles (only if not already present from migrations)
    use sea_orm::ColumnTrait;
    use sea_orm::QueryFilter;

    // Check if admin role exists
    if Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(db)
        .await
        .unwrap()
        .is_some()
    {
        // Seed data already exists from migrations
        return;
    }

    let admin_role = role::ActiveModel {
        name: Set("admin".to_string()),
        description: Set(Some("Full administrator access".to_string())),
        is_system: Set(true),
        requires_2fa: Set(false),
        created_at: Set(now),
        ..Default::default()
    };
    let admin = admin_role.insert(db).await.unwrap();

    let viewer_role = role::ActiveModel {
        name: Set("viewer".to_string()),
        description: Set(Some("View-only access".to_string())),
        is_system: Set(true),
        requires_2fa: Set(false),
        created_at: Set(now),
        ..Default::default()
    };
    let viewer = viewer_role.insert(db).await.unwrap();

    let downloader_role = role::ActiveModel {
        name: Set("downloader".to_string()),
        description: Set(Some("Download client access".to_string())),
        is_system: Set(true),
        requires_2fa: Set(false),
        created_at: Set(now),
        ..Default::default()
    };
    let downloader = downloader_role.insert(db).await.unwrap();

    // Add admin permissions
    let admin_permissions = [
        "apps.view",
        "apps.install",
        "apps.delete",
        "apps.restart",
        "storage.view",
        "storage.write",
        "storage.delete",
        "storage.download",
        "logs.view",
        "monitoring.view",
        "users.view",
        "users.manage",
        "roles.view",
        "roles.manage",
        "settings.view",
        "settings.manage",
    ];
    for perm in admin_permissions {
        let permission = role_permission::ActiveModel {
            role_id: Set(admin.id),
            permission: Set(perm.to_string()),
            ..Default::default()
        };
        permission.insert(db).await.unwrap();
    }

    // Add viewer permissions
    let viewer_permissions = [
        "apps.view",
        "logs.view",
        "monitoring.view",
        "storage.view",
        "storage.download",
    ];
    for perm in viewer_permissions {
        let permission = role_permission::ActiveModel {
            role_id: Set(viewer.id),
            permission: Set(perm.to_string()),
            ..Default::default()
        };
        permission.insert(db).await.unwrap();
    }

    // Add viewer app permissions
    for app in ["jellyfin", "jellyseerr"] {
        let app_perm = role_app_permission::ActiveModel {
            role_id: Set(viewer.id),
            app_name: Set(app.to_string()),
            ..Default::default()
        };
        app_perm.insert(db).await.unwrap();
    }

    // Add downloader permissions
    let downloader_permissions = [
        "apps.view",
        "apps.restart",
        "storage.view",
        "storage.download",
    ];
    for perm in downloader_permissions {
        let permission = role_permission::ActiveModel {
            role_id: Set(downloader.id),
            permission: Set(perm.to_string()),
            ..Default::default()
        };
        permission.insert(db).await.unwrap();
    }

    // Add downloader app permissions
    for app in ["qbittorrent", "transmission", "deluge"] {
        let app_perm = role_app_permission::ActiveModel {
            role_id: Set(downloader.id),
            app_name: Set(app.to_string()),
            ..Default::default()
        };
        app_perm.insert(db).await.unwrap();
    }
}

/// Create a test user and return the user model
pub async fn create_test_user(
    db: &DatabaseConnection,
    username: &str,
    email: &str,
    password: &str,
    is_approved: bool,
) -> kubarr::models::user::Model {
    use kubarr::models::user;
    use kubarr::services::security::hash_password;
    use sea_orm::{ActiveModelTrait, Set};

    let hashed = hash_password(password).unwrap();
    let now = chrono::Utc::now();

    let new_user = user::ActiveModel {
        username: Set(username.to_string()),
        email: Set(email.to_string()),
        hashed_password: Set(hashed),
        is_active: Set(true),
        is_approved: Set(is_approved),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };

    new_user.insert(db).await.unwrap()
}

/// Create a test user with a specific role
pub async fn create_test_user_with_role(
    db: &DatabaseConnection,
    username: &str,
    email: &str,
    password: &str,
    role_name: &str,
) -> kubarr::models::user::Model {
    use kubarr::models::{role, user_role};
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

    let user = create_test_user(db, username, email, password, true).await;

    // Find the role
    let role = kubarr::models::prelude::Role::find()
        .filter(role::Column::Name.eq(role_name))
        .one(db)
        .await
        .unwrap()
        .expect("Role not found");

    // Assign role to user
    let user_role = user_role::ActiveModel {
        user_id: Set(user.id),
        role_id: Set(role.id),
    };
    user_role.insert(db).await.unwrap();

    user
}
