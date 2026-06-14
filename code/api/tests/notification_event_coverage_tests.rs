//! Tests to cover `format_event_title` and `format_event_body` in the notification service.
//!
//! These private functions are only exercised when `notify_event` is called with an
//! event type that is configured as enabled in the notification_events table.
//!
//! This test file sets up notification event records for every AuditAction variant,
//! then calls `notify_event` for each one to exercise all match arms.

mod common;
use common::{create_test_db_with_seed, create_test_user};

use kubarr::models::{audit_log::AuditAction, notification_event};
use kubarr::services::notification::NotificationService;
use sea_orm::{ActiveModelTrait, Set};

// ============================================================================
// Helper: insert a notification event record for an action type
// ============================================================================

async fn insert_notification_event(db: &sea_orm::DatabaseConnection, event_type: &str) {
    let event = notification_event::ActiveModel {
        event_type: Set(event_type.to_string()),
        enabled: Set(true),
        severity: Set("info".to_string()),
        ..Default::default()
    };
    let _ = event.insert(db).await; // Ignore error if already exists
}

// ============================================================================
// Setup: create a DB with all AuditAction types enabled for notifications
// ============================================================================

async fn setup_all_events(db: &sea_orm::DatabaseConnection) {
    let all_actions = [
        "login",
        "login_failed",
        "logout",
        "token_refresh",
        "2fa_enabled",
        "2fa_disabled",
        "2fa_verified",
        "2fa_failed",
        "password_changed",
        "user_created",
        "user_updated",
        "user_deleted",
        "user_approved",
        "user_deactivated",
        "user_activated",
        "role_created",
        "role_updated",
        "role_deleted",
        "role_assigned",
        "role_unassigned",
        "app_installed",
        "app_uninstalled",
        "app_started",
        "app_stopped",
        "app_restarted",
        "app_configured",
        "app_accessed",
        "system_setting_changed",
        "invite_created",
        "invite_used",
        "invite_deleted",
        "api_access",
    ];

    for action in &all_actions {
        insert_notification_event(db, action).await;
    }
}

// ============================================================================
// Tests that exercise format_event_title + format_event_body via notify_event
// ============================================================================

/// Calls notify_event for all AuditAction variants without details (covers no-detail branches)
#[tokio::test]
async fn test_notify_event_all_actions_without_details() {
    let db = create_test_db_with_seed().await;
    let user = create_test_user(&db, "notifuser1", "notifuser1@test.com", "pass123", true).await;

    setup_all_events(&db).await;

    let svc = NotificationService::new();
    svc.set_db(db.clone()).await;

    let all_actions = vec![
        AuditAction::Login,
        AuditAction::LoginFailed,
        AuditAction::Logout,
        AuditAction::TokenRefresh,
        AuditAction::TwoFactorEnabled,
        AuditAction::TwoFactorDisabled,
        AuditAction::TwoFactorVerified,
        AuditAction::TwoFactorFailed,
        AuditAction::PasswordChanged,
        AuditAction::UserCreated,
        AuditAction::UserUpdated,
        AuditAction::UserDeleted,
        AuditAction::UserApproved,
        AuditAction::UserDeactivated,
        AuditAction::UserActivated,
        AuditAction::RoleCreated,
        AuditAction::RoleUpdated,
        AuditAction::RoleDeleted,
        AuditAction::RoleAssigned,
        AuditAction::RoleUnassigned,
        AuditAction::AppInstalled,
        AuditAction::AppUninstalled,
        AuditAction::AppStarted,
        AuditAction::AppStopped,
        AuditAction::AppRestarted,
        AuditAction::AppConfigured,
        AuditAction::AppAccessed,
        AuditAction::SystemSettingChanged,
        AuditAction::InviteCreated,
        AuditAction::InviteUsed,
        AuditAction::InviteDeleted,
        AuditAction::ApiAccess,
    ];

    for action in &all_actions {
        let result = svc
            .notify_event(action, Some(user.id), Some("testuser"), None)
            .await;
        assert!(
            result.is_ok(),
            "notify_event({:?}) without details must succeed",
            action
        );
    }
}

/// Calls notify_event for all AuditAction variants WITH details (covers detail branches)
#[tokio::test]
async fn test_notify_event_all_actions_with_details() {
    let db = create_test_db_with_seed().await;
    let user = create_test_user(&db, "notifuser2", "notifuser2@test.com", "pass123", true).await;

    setup_all_events(&db).await;

    let svc = NotificationService::new();
    svc.set_db(db.clone()).await;

    let all_actions = vec![
        AuditAction::Login,
        AuditAction::LoginFailed,
        AuditAction::Logout,
        AuditAction::TokenRefresh,
        AuditAction::TwoFactorEnabled,
        AuditAction::TwoFactorDisabled,
        AuditAction::TwoFactorVerified,
        AuditAction::TwoFactorFailed,
        AuditAction::PasswordChanged,
        AuditAction::UserCreated,
        AuditAction::UserUpdated,
        AuditAction::UserDeleted,
        AuditAction::UserApproved,
        AuditAction::UserDeactivated,
        AuditAction::UserActivated,
        AuditAction::RoleCreated,
        AuditAction::RoleUpdated,
        AuditAction::RoleDeleted,
        AuditAction::RoleAssigned,
        AuditAction::RoleUnassigned,
        AuditAction::AppInstalled,
        AuditAction::AppUninstalled,
        AuditAction::AppStarted,
        AuditAction::AppStopped,
        AuditAction::AppRestarted,
        AuditAction::AppConfigured,
        AuditAction::AppAccessed,
        AuditAction::SystemSettingChanged,
        AuditAction::InviteCreated,
        AuditAction::InviteUsed,
        AuditAction::InviteDeleted,
        AuditAction::ApiAccess,
    ];

    for action in &all_actions {
        let result = svc
            .notify_event(
                action,
                Some(user.id),
                Some("testuser"),
                Some("extra-detail"),
            )
            .await;
        assert!(
            result.is_ok(),
            "notify_event({:?}) with details must succeed",
            action
        );
    }
}

/// Test system-wide notify_event (user_id = None) covers the tracing::debug! path
#[tokio::test]
async fn test_notify_event_system_wide_all_actions() {
    let db = create_test_db_with_seed().await;
    setup_all_events(&db).await;

    let svc = NotificationService::new();
    svc.set_db(db.clone()).await;

    // Only test a subset to cover the system-wide branch
    let result = svc
        .notify_event(
            &AuditAction::AppInstalled,
            None,
            Some("admin"),
            Some("myapp"),
        )
        .await;
    assert!(result.is_ok(), "system-wide notify_event must succeed");

    let result = svc
        .notify_event(&AuditAction::UserCreated, None, None, None)
        .await;
    assert!(
        result.is_ok(),
        "system-wide notify_event without user must succeed"
    );
}

/// Test that notify_event with an unconfigured event type still returns Ok
#[tokio::test]
async fn test_notify_event_unconfigured_action_returns_ok() {
    let db = create_test_db_with_seed().await;
    let user = create_test_user(&db, "notifuser3", "notifuser3@test.com", "pass123", true).await;

    // Do NOT set up notification events — all actions are unconfigured
    let svc = NotificationService::new();
    svc.set_db(db.clone()).await;

    let result = svc
        .notify_event(&AuditAction::Login, Some(user.id), Some("notifuser3"), None)
        .await;
    assert!(result.is_ok(), "unconfigured event must return Ok(())");
}
