//! Tests that exercise the `Related<>` trait implementations in all model files.
//!
//! Sea-ORM's `Related<>` trait methods (`to()` and `via()`) are only called when
//! a JOIN query is built.  These tests execute the minimum JOIN queries needed to
//! cover every `impl Related<…>` block that shows up as uncovered in tarpaulin.
//!
//! Each test uses an in-memory SQLite database with fully migrated schema so the
//! queries compile and run without errors (they simply return empty result sets).

mod common;
use common::create_test_db_with_seed;

use kubarr::models::prelude::*;
use sea_orm::EntityTrait;

// ============================================================================
// session::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_session_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = Session::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("session → user JOIN");
}

// ============================================================================
// user::Related<role::Entity>  (via user_role)
// user::Related<user_role::Entity>
// user::Related<user_preferences::Entity>
// ============================================================================

#[tokio::test]
async fn test_user_related_role_via_user_role() {
    let db = create_test_db_with_seed().await;
    let _ = User::find()
        .find_also_related(Role)
        .all(&db)
        .await
        .expect("user → role via user_role JOIN");
}

#[tokio::test]
async fn test_user_related_user_role_join() {
    let db = create_test_db_with_seed().await;
    let _ = User::find()
        .find_also_related(UserRole)
        .all(&db)
        .await
        .expect("user → user_role JOIN");
}

#[tokio::test]
async fn test_user_related_user_preferences_join() {
    let db = create_test_db_with_seed().await;
    let _ = User::find()
        .find_also_related(UserPreferences)
        .all(&db)
        .await
        .expect("user → user_preferences JOIN");
}

// ============================================================================
// role::Related<user::Entity>  (via user_role)
// role::Related<user_role::Entity>
// role::Related<role_app_permission::Entity>
// ============================================================================

#[tokio::test]
async fn test_role_related_user_via_user_role() {
    let db = create_test_db_with_seed().await;
    let _ = Role::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("role → user via user_role JOIN");
}

#[tokio::test]
async fn test_role_related_user_role_join() {
    let db = create_test_db_with_seed().await;
    let _ = Role::find()
        .find_also_related(UserRole)
        .all(&db)
        .await
        .expect("role → user_role JOIN");
}

#[tokio::test]
async fn test_role_related_role_app_permission_join() {
    let db = create_test_db_with_seed().await;
    let _ = Role::find()
        .find_also_related(RoleAppPermission)
        .all(&db)
        .await
        .expect("role → role_app_permission JOIN");
}

// ============================================================================
// user_role::Related<role::Entity>
// user_role::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_user_role_related_role_join() {
    let db = create_test_db_with_seed().await;
    let _ = UserRole::find()
        .find_also_related(Role)
        .all(&db)
        .await
        .expect("user_role → role JOIN");
}

#[tokio::test]
async fn test_user_role_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = UserRole::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("user_role → user JOIN");
}

// ============================================================================
// role_app_permission::Related<role::Entity>
// ============================================================================

#[tokio::test]
async fn test_role_app_permission_related_role_join() {
    let db = create_test_db_with_seed().await;
    let _ = RoleAppPermission::find()
        .find_also_related(Role)
        .all(&db)
        .await
        .expect("role_app_permission → role JOIN");
}

// ============================================================================
// role_permission::Related<role::Entity>
// ============================================================================

#[tokio::test]
async fn test_role_permission_related_role_join() {
    let db = create_test_db_with_seed().await;
    let _ = RolePermission::find()
        .find_also_related(Role)
        .all(&db)
        .await
        .expect("role_permission → role JOIN");
}

// ============================================================================
// pending_2fa_challenge::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_pending_2fa_challenge_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = Pending2faChallenge::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("pending_2fa_challenge → user JOIN");
}

// ============================================================================
// notification_log::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_notification_log_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = NotificationLog::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("notification_log → user JOIN");
}

// ============================================================================
// oauth_account::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_oauth_account_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = OauthAccount::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("oauth_account → user JOIN");
}

// ============================================================================
// two_factor_recovery_code::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_two_factor_recovery_code_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = TwoFactorRecoveryCode::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("two_factor_recovery_code → user JOIN");
}

// ============================================================================
// user_notification::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_user_notification_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = UserNotification::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("user_notification → user JOIN");
}

// ============================================================================
// user_notification_pref::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_user_notification_pref_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = UserNotificationPref::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("user_notification_pref → user JOIN");
}

// ============================================================================
// user_preferences::Related<user::Entity>
// ============================================================================

#[tokio::test]
async fn test_user_preferences_related_user_join() {
    let db = create_test_db_with_seed().await;
    let _ = UserPreferences::find()
        .find_also_related(User)
        .all(&db)
        .await
        .expect("user_preferences → user JOIN");
}

// ============================================================================
// app_vpn_config::Related<vpn_provider::Entity>
// ============================================================================

#[tokio::test]
async fn test_app_vpn_config_related_vpn_provider_join() {
    let db = create_test_db_with_seed().await;
    let _ = AppVpnConfig::find()
        .find_also_related(VpnProvider)
        .all(&db)
        .await
        .expect("app_vpn_config → vpn_provider JOIN");
}
