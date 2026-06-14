//! Tests for settings and extractor helper functions
//!
//! Covers:
//! - `get_setting_value()` — DB-backed setting with default fallback
//! - `get_setting_bool()` — boolean coercion helper
//! - `get_user_permissions()` — aggregate permissions from user roles
//! - `get_user_app_access()` — app access list and wildcard handling

mod common;
use common::{
    create_test_db, create_test_db_with_seed, create_test_user, create_test_user_with_role,
};

use kubarr::endpoints::extractors::{get_user_app_access, get_user_permissions};
use kubarr::endpoints::settings::{get_setting_bool, get_setting_value};
use kubarr::models::{role_permission, user_role};
use sea_orm::{ActiveModelTrait, Set};

// ============================================================================
// get_setting_value
// ============================================================================

#[tokio::test]
async fn test_get_setting_value_returns_default_when_not_in_db() {
    let db = create_test_db_with_seed().await;
    // "registration_enabled" is a known default ("true")
    let result = get_setting_value(&db, "registration_enabled")
        .await
        .expect("get_setting_value must succeed");

    // Either from DB seed or from default — value must be "true"
    assert!(
        result.is_some(),
        "registration_enabled must return Some value"
    );
    assert_eq!(
        result.unwrap(),
        "true",
        "registration_enabled default must be 'true'"
    );
}

#[tokio::test]
async fn test_get_setting_value_returns_none_for_unknown_key() {
    let db = create_test_db_with_seed().await;
    let result = get_setting_value(&db, "nonexistent_setting_xyz")
        .await
        .expect("get_setting_value must succeed for unknown key");

    assert!(result.is_none(), "Unknown setting key must return None");
}

#[tokio::test]
async fn test_get_setting_value_returns_db_value_over_default() {
    use kubarr::models::system_setting;

    let db = create_test_db_with_seed().await;

    // Override the default in the DB
    let now = chrono::Utc::now();
    let setting = system_setting::ActiveModel {
        key: Set("registration_enabled".to_string()),
        value: Set("false".to_string()),
        description: Set(Some("test override".to_string())),
        updated_at: Set(now),
    };
    // Use upsert pattern: delete then insert
    use kubarr::models::prelude::SystemSetting;
    use sea_orm::EntityTrait;
    if let Some(existing) = SystemSetting::find_by_id("registration_enabled")
        .one(&db)
        .await
        .unwrap()
    {
        use sea_orm::ModelTrait;
        existing.delete(&db).await.unwrap();
    }
    setting.insert(&db).await.unwrap();

    let result = get_setting_value(&db, "registration_enabled")
        .await
        .expect("get_setting_value must succeed");

    assert_eq!(
        result,
        Some("false".to_string()),
        "DB value must override default"
    );
}

// ============================================================================
// get_setting_bool
// ============================================================================

#[tokio::test]
async fn test_get_setting_bool_true_for_default_registration_enabled() {
    let db = create_test_db_with_seed().await;
    let result = get_setting_bool(&db, "registration_enabled")
        .await
        .expect("get_setting_bool must succeed");

    assert!(result, "registration_enabled default must return true");
}

#[tokio::test]
async fn test_get_setting_bool_false_for_unknown_key() {
    let db = create_test_db_with_seed().await;
    let result = get_setting_bool(&db, "completely_unknown_setting_abc")
        .await
        .expect("get_setting_bool must succeed for unknown key");

    assert!(!result, "Unknown setting must default to false");
}

#[tokio::test]
async fn test_get_setting_bool_true_for_yes_value() {
    use kubarr::models::{prelude::SystemSetting, system_setting};

    let db = create_test_db_with_seed().await;

    // Insert a setting with value "yes"
    let now = chrono::Utc::now();
    use sea_orm::EntityTrait;
    if let Some(existing) = SystemSetting::find_by_id("registration_enabled")
        .one(&db)
        .await
        .unwrap()
    {
        use sea_orm::ModelTrait;
        existing.delete(&db).await.unwrap();
    }
    let setting = system_setting::ActiveModel {
        key: Set("registration_enabled".to_string()),
        value: Set("yes".to_string()),
        description: Set(None),
        updated_at: Set(now),
    };
    setting.insert(&db).await.unwrap();

    let result = get_setting_bool(&db, "registration_enabled")
        .await
        .expect("get_setting_bool must succeed");

    assert!(result, "'yes' must evaluate to true");
}

#[tokio::test]
async fn test_get_setting_bool_true_for_1_value() {
    use kubarr::models::{prelude::SystemSetting, system_setting};

    let db = create_test_db_with_seed().await;

    // Insert a setting with value "1"
    let now = chrono::Utc::now();
    use sea_orm::EntityTrait;
    if let Some(existing) = SystemSetting::find_by_id("registration_enabled")
        .one(&db)
        .await
        .unwrap()
    {
        use sea_orm::ModelTrait;
        existing.delete(&db).await.unwrap();
    }
    let setting = system_setting::ActiveModel {
        key: Set("registration_enabled".to_string()),
        value: Set("1".to_string()),
        description: Set(None),
        updated_at: Set(now),
    };
    setting.insert(&db).await.unwrap();

    let result = get_setting_bool(&db, "registration_enabled")
        .await
        .expect("get_setting_bool must succeed");

    assert!(result, "'1' must evaluate to true");
}

// ============================================================================
// get_user_permissions
// ============================================================================

#[tokio::test]
async fn test_get_user_permissions_returns_empty_for_user_with_no_roles() {
    let db = create_test_db_with_seed().await;
    // Create a user but don't assign any role
    let user = create_test_user(&db, "nopermuser", "noperm@test.com", "pass", true).await;

    let perms = get_user_permissions(&db, user.id).await;
    assert!(
        perms.is_empty(),
        "User with no roles must have no permissions"
    );
}

#[tokio::test]
async fn test_get_user_permissions_returns_role_permissions() {
    let db = create_test_db_with_seed().await;
    let user =
        create_test_user_with_role(&db, "adminpermuser", "adminperm@test.com", "pass", "admin")
            .await;

    let perms = get_user_permissions(&db, user.id).await;
    assert!(!perms.is_empty(), "Admin user must have permissions");
    assert!(
        perms.contains(&"apps.view".to_string()),
        "Admin must have apps.view permission"
    );
    assert!(
        perms.contains(&"users.manage".to_string()),
        "Admin must have users.manage permission"
    );
}

#[tokio::test]
async fn test_get_user_permissions_deduplicates() {
    let db = create_test_db_with_seed().await;
    let user =
        create_test_user_with_role(&db, "dedupuser", "dedup@test.com", "pass", "viewer").await;

    let perms = get_user_permissions(&db, user.id).await;

    // No duplicates (sorted + deduped)
    let mut sorted = perms.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(perms, sorted, "Permissions must be deduplicated and sorted");
}

#[tokio::test]
async fn test_get_user_permissions_includes_app_permissions() {
    let db = create_test_db_with_seed().await;
    // viewer role has app permissions for jellyfin and jellyseerr
    let user =
        create_test_user_with_role(&db, "viewerapps", "viewerapps@test.com", "pass", "viewer")
            .await;

    let perms = get_user_permissions(&db, user.id).await;

    // Should include app.jellyfin and app.jellyseerr from the viewer role
    assert!(
        perms.iter().any(|p| p.starts_with("app.")),
        "Viewer must have app permissions"
    );
}

// ============================================================================
// get_user_app_access
// ============================================================================

#[tokio::test]
async fn test_get_user_app_access_returns_empty_for_no_roles() {
    let db = create_test_db_with_seed().await;
    let user = create_test_user(&db, "noappuser", "noapp@test.com", "pass", true).await;

    let apps = get_user_app_access(&db, user.id).await;
    assert!(
        apps.is_empty(),
        "User with no roles must have no app access"
    );
}

#[tokio::test]
async fn test_get_user_app_access_returns_specific_apps_for_viewer() {
    let db = create_test_db_with_seed().await;
    let user =
        create_test_user_with_role(&db, "viewerapps2", "viewerapps2@test.com", "pass", "viewer")
            .await;

    let apps = get_user_app_access(&db, user.id).await;

    // Viewer has jellyfin and jellyseerr
    assert!(
        apps.contains(&"jellyfin".to_string()) || apps.contains(&"jellyseerr".to_string()),
        "Viewer must have access to at least one app"
    );
}

#[tokio::test]
async fn test_get_user_app_access_wildcard_returns_star() {
    use kubarr::models::role;

    let db = create_test_db_with_seed().await;

    // Create a fresh role with app.* permission (avoid conflicts with existing roles)
    let now = chrono::Utc::now();
    let wildcard_role = role::ActiveModel {
        name: Set("wildcard_role".to_string()),
        description: Set(Some("Role with wildcard app access".to_string())),
        is_system: Set(false),
        requires_2fa: Set(false),
        created_at: Set(now),
        ..Default::default()
    };
    let created_role = wildcard_role.insert(&db).await.unwrap();

    // Add app.* permission to this new role
    let wildcard_perm = role_permission::ActiveModel {
        role_id: Set(created_role.id),
        permission: Set("app.*".to_string()),
        ..Default::default()
    };
    wildcard_perm.insert(&db).await.unwrap();

    // Create a user and assign the wildcard role
    let user = create_test_user(&db, "wildcarduser", "wildcard@test.com", "pass", true).await;
    let user_role_model = user_role::ActiveModel {
        user_id: Set(user.id),
        role_id: Set(created_role.id),
    };
    user_role_model.insert(&db).await.unwrap();

    let apps = get_user_app_access(&db, user.id).await;

    assert_eq!(
        apps,
        vec!["*".to_string()],
        "User with app.* permission must get wildcard access"
    );
}
