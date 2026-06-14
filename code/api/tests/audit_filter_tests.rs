//! Additional audit tests to cover uncovered filter paths in `get_audit_logs`
//!
//! The existing `audit_tests.rs` covers user_id, action, and success filters.
//! This file covers: resource_type, from, to, and search filters.

mod common;
use common::create_test_db;

use chrono::{Duration, Utc};
use kubarr::models::audit_log;
use kubarr::services::audit::{get_audit_logs, AuditLogQuery};
use sea_orm::{ActiveModelTrait, Set};

// ============================================================================
// Helper: insert a minimal audit log row
// ============================================================================

async fn insert_log(
    db: &sea_orm::DatabaseConnection,
    action: &str,
    resource_type: &str,
    username: Option<&str>,
    resource_id: Option<&str>,
    success: bool,
    timestamp: chrono::DateTime<Utc>,
) {
    let row = audit_log::ActiveModel {
        timestamp: Set(timestamp),
        action: Set(action.to_string()),
        resource_type: Set(resource_type.to_string()),
        resource_id: Set(resource_id.map(|s| s.to_string())),
        user_id: Set(None),
        username: Set(username.map(|s| s.to_string())),
        details: Set(None),
        ip_address: Set(None),
        user_agent: Set(None),
        success: Set(success),
        error_message: Set(None),
        ..Default::default()
    };
    row.insert(db).await.expect("insert audit log");
}

// ============================================================================
// filter by resource_type
// ============================================================================

#[tokio::test]
async fn test_get_audit_logs_filter_by_resource_type() {
    let db = create_test_db().await;
    let now = Utc::now();

    insert_log(&db, "login", "session", Some("alice"), None, true, now).await;
    insert_log(&db, "user_created", "user", Some("admin"), None, true, now).await;
    insert_log(&db, "role_assigned", "role", Some("admin"), None, true, now).await;

    // Filter by resource_type = "user"
    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: Some("user".to_string()),
            success: None,
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .expect("get_audit_logs must succeed");

    assert_eq!(result.logs.len(), 1);
    assert_eq!(result.logs[0].resource_type, "user");
    assert_eq!(result.logs[0].action, "user_created");
}

// ============================================================================
// filter by from (timestamp >=)
// ============================================================================

#[tokio::test]
async fn test_get_audit_logs_filter_by_from() {
    let db = create_test_db().await;
    let now = Utc::now();
    let yesterday = now - Duration::days(1);
    let last_week = now - Duration::days(7);

    insert_log(&db, "login", "session", Some("u1"), None, true, last_week).await;
    insert_log(&db, "login", "session", Some("u2"), None, true, yesterday).await;
    insert_log(&db, "login", "session", Some("u3"), None, true, now).await;

    // Filter: from = now - 2 hours (should include only recent + yesterday)
    let two_days_ago = now - Duration::hours(48);
    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: Some(two_days_ago),
            to: None,
            search: None,
        },
    )
    .await
    .expect("get_audit_logs with from must succeed");

    // Should include yesterday and now, but not last_week
    assert_eq!(result.logs.len(), 2, "must include yesterday and now only");
}

// ============================================================================
// filter by to (timestamp <=)
// ============================================================================

#[tokio::test]
async fn test_get_audit_logs_filter_by_to() {
    let db = create_test_db().await;
    let now = Utc::now();
    let yesterday = now - Duration::days(1);
    let last_week = now - Duration::days(7);

    insert_log(&db, "logout", "session", Some("u1"), None, true, last_week).await;
    insert_log(&db, "logout", "session", Some("u2"), None, true, yesterday).await;
    insert_log(&db, "logout", "session", Some("u3"), None, true, now).await;

    // Filter: to = 2 days ago (should include only last_week)
    let two_days_ago = now - Duration::hours(48);
    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: Some(two_days_ago),
            search: None,
        },
    )
    .await
    .expect("get_audit_logs with to must succeed");

    assert_eq!(result.logs.len(), 1, "must include only last_week");
}

// ============================================================================
// filter by from AND to
// ============================================================================

#[tokio::test]
async fn test_get_audit_logs_filter_by_from_and_to() {
    let db = create_test_db().await;
    let now = Utc::now();
    let three_days_ago = now - Duration::days(3);
    let last_week = now - Duration::days(7);

    // Insert rows: last_week, three_days_ago, now
    insert_log(&db, "login", "session", Some("u1"), None, true, last_week).await;
    insert_log(
        &db,
        "login",
        "session",
        Some("u2"),
        None,
        true,
        three_days_ago,
    )
    .await;
    insert_log(&db, "login", "session", Some("u3"), None, true, now).await;

    // Filter: from=5 days ago, to=2 days ago → only three_days_ago
    let five_days_ago = now - Duration::days(5);
    let two_days_ago = now - Duration::days(2);
    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: Some(five_days_ago),
            to: Some(two_days_ago),
            search: None,
        },
    )
    .await
    .expect("get_audit_logs with from+to must succeed");

    // Should only include three_days_ago (between 5 days ago and 2 days ago)
    assert_eq!(result.logs.len(), 1, "must include only three_days_ago");
    assert_eq!(result.logs[0].username, Some("u2".to_string()));
}

// ============================================================================
// filter by search (username / action / resource_id / details)
// ============================================================================

#[tokio::test]
async fn test_get_audit_logs_filter_by_search_matches_username() {
    let db = create_test_db().await;
    let now = Utc::now();

    insert_log(
        &db,
        "login",
        "session",
        Some("alice_admin"),
        None,
        true,
        now,
    )
    .await;
    insert_log(&db, "login", "session", Some("bob_user"), None, true, now).await;
    insert_log(&db, "login", "session", Some("charlie"), None, true, now).await;

    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: Some("alice".to_string()),
        },
    )
    .await
    .expect("get_audit_logs with search must succeed");

    assert_eq!(result.logs.len(), 1);
    assert_eq!(result.logs[0].username, Some("alice_admin".to_string()));
}

#[tokio::test]
async fn test_get_audit_logs_filter_by_search_matches_action() {
    let db = create_test_db().await;
    let now = Utc::now();

    insert_log(&db, "app_installed", "app", None, None, true, now).await;
    insert_log(&db, "login", "session", None, None, true, now).await;
    insert_log(&db, "app_uninstalled", "app", None, None, true, now).await;

    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: Some("app_install".to_string()),
        },
    )
    .await
    .expect("get_audit_logs with search on action must succeed");

    assert_eq!(result.logs.len(), 1);
    assert_eq!(result.logs[0].action, "app_installed");
}

#[tokio::test]
async fn test_get_audit_logs_filter_by_search_matches_resource_id() {
    let db = create_test_db().await;
    let now = Utc::now();

    insert_log(
        &db,
        "login",
        "session",
        None,
        Some("resource-abc-123"),
        true,
        now,
    )
    .await;
    insert_log(
        &db,
        "login",
        "session",
        None,
        Some("resource-xyz-456"),
        true,
        now,
    )
    .await;

    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: Some("abc-123".to_string()),
        },
    )
    .await
    .expect("get_audit_logs search by resource_id must succeed");

    assert_eq!(result.logs.len(), 1);
    assert_eq!(
        result.logs[0].resource_id,
        Some("resource-abc-123".to_string())
    );
}

#[tokio::test]
async fn test_get_audit_logs_search_no_match_returns_empty() {
    let db = create_test_db().await;
    let now = Utc::now();

    insert_log(&db, "login", "session", Some("alice"), None, true, now).await;

    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: Some("zzz-nonexistent-zzz".to_string()),
        },
    )
    .await
    .expect("get_audit_logs with no-match search must succeed");

    assert_eq!(result.logs.len(), 0);
    assert_eq!(result.total, 0);
}
