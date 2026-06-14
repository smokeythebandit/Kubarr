//! Tests for `kubarr::services::audit::AuditService` and its standalone helpers.
//!
//! Covers:
//! - `AuditService::log_success` / `log_failure` — row insertion and field values
//! - `get_audit_logs` — empty table, pagination (limit/offset), and all three filter axes
//!   (user_id, action, success)
//! - `get_audit_stats` — correct counts for total/success/failure/today/week
//! - `clear_old_logs` — retention-policy deletion by age

mod common;
use common::create_test_db;

use kubarr::models::audit_log::{self, AuditAction, ResourceType};
use kubarr::services::audit::{
    clear_old_logs, get_audit_logs, get_audit_stats, AuditLogQuery, AuditService,
};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};

// ---------------------------------------------------------------------------
// Helper: build a minimal AuditService backed by `db`
// ---------------------------------------------------------------------------

async fn make_service(
    db: sea_orm::DatabaseConnection,
) -> (AuditService, sea_orm::DatabaseConnection) {
    let svc = AuditService::new();
    svc.set_db(db.clone()).await;
    (svc, db)
}

// ---------------------------------------------------------------------------
// Helper: insert a raw audit_log row directly, giving full control over the
// timestamp so pagination / stats tests are deterministic.
// ---------------------------------------------------------------------------

async fn insert_log(
    db: &sea_orm::DatabaseConnection,
    action: &str,
    resource_type: &str,
    user_id: Option<i64>,
    success: bool,
    timestamp: chrono::DateTime<chrono::Utc>,
) {
    let entry = audit_log::ActiveModel {
        timestamp: Set(timestamp),
        user_id: Set(user_id),
        username: Set(user_id.map(|id| format!("user_{}", id))),
        action: Set(action.to_string()),
        resource_type: Set(resource_type.to_string()),
        resource_id: Set(None),
        details: Set(None),
        ip_address: Set(None),
        user_agent: Set(None),
        success: Set(success),
        error_message: Set(if success {
            None
        } else {
            Some("test error".to_string())
        }),
        ..Default::default()
    };
    entry.insert(db).await.unwrap();
}

// ---------------------------------------------------------------------------
// 1. log_success inserts a row with success=true
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_log_success() {
    let db = create_test_db().await;
    let (svc, db) = make_service(db).await;

    svc.log_success(
        AuditAction::Login,
        ResourceType::Session,
        None,
        Some(1),
        Some("alice".to_string()),
        None,
        None,
        None,
    )
    .await
    .expect("log_success should not fail");

    let logs = audit_log::Entity::find().all(&db).await.unwrap();
    assert_eq!(logs.len(), 1, "Expected exactly one audit log row");

    let row = &logs[0];
    assert_eq!(row.action, "login");
    assert_eq!(row.resource_type, "session");
    assert_eq!(row.user_id, Some(1));
    assert_eq!(row.username.as_deref(), Some("alice"));
    assert!(
        row.success,
        "Row inserted by log_success must have success=true"
    );
    assert!(
        row.error_message.is_none(),
        "log_success must not store an error_message"
    );
}

// ---------------------------------------------------------------------------
// 2. log_failure inserts a row with success=false and an error message
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_log_failure() {
    let db = create_test_db().await;
    let (svc, db) = make_service(db).await;

    svc.log_failure(
        AuditAction::LoginFailed,
        ResourceType::User,
        Some("bob".to_string()),
        Some(2),
        Some("bob".to_string()),
        None,
        Some("127.0.0.1".to_string()),
        None,
        "invalid password",
    )
    .await
    .expect("log_failure should not fail");

    let logs = audit_log::Entity::find().all(&db).await.unwrap();
    assert_eq!(logs.len(), 1);

    let row = &logs[0];
    assert_eq!(row.action, "login_failed");
    assert!(!row.success, "log_failure must set success=false");
    assert_eq!(
        row.error_message.as_deref(),
        Some("invalid password"),
        "error_message must be stored"
    );
    assert_eq!(row.ip_address.as_deref(), Some("127.0.0.1"));
}

// ---------------------------------------------------------------------------
// 3. get_audit_logs on empty table returns empty list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_audit_logs_empty() {
    let db = create_test_db().await;

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
            search: None,
        },
    )
    .await
    .expect("get_audit_logs should succeed on empty table");

    assert_eq!(result.total, 0);
    assert!(result.logs.is_empty());
    assert_eq!(result.page, 1);
    assert_eq!(result.total_pages, 0);
}

// ---------------------------------------------------------------------------
// 4. get_audit_logs pagination — limit/offset work correctly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_audit_logs_pagination() {
    let db = create_test_db().await;
    let now = chrono::Utc::now();

    // Insert 5 logs with distinct timestamps so ordering is deterministic.
    for i in 0..5u64 {
        insert_log(
            &db,
            "login",
            "session",
            Some(1),
            true,
            now - chrono::Duration::seconds(i as i64 * 10),
        )
        .await;
    }

    // Page 1: first 2 results
    let page1 = get_audit_logs(
        &db,
        AuditLogQuery {
            page: Some(1),
            per_page: Some(2),
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(page1.total, 5, "Total must reflect all 5 rows");
    assert_eq!(page1.logs.len(), 2, "Page 1 must return 2 rows");
    assert_eq!(page1.per_page, 2);
    assert_eq!(page1.total_pages, 3); // ceil(5/2)

    // Page 2: next 2 results
    let page2 = get_audit_logs(
        &db,
        AuditLogQuery {
            page: Some(2),
            per_page: Some(2),
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(page2.logs.len(), 2, "Page 2 must return 2 rows");

    // Page 3: last 1 result
    let page3 = get_audit_logs(
        &db,
        AuditLogQuery {
            page: Some(3),
            per_page: Some(2),
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        page3.logs.len(),
        1,
        "Last page must return the remaining row"
    );
}

// ---------------------------------------------------------------------------
// 5. get_audit_logs filter by user_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_audit_logs_filter_by_user() {
    let db = create_test_db().await;
    let now = chrono::Utc::now();

    // 3 logs for user 10, 2 logs for user 20
    for _ in 0..3 {
        insert_log(&db, "login", "session", Some(10), true, now).await;
    }
    for _ in 0..2 {
        insert_log(&db, "login", "session", Some(20), true, now).await;
    }

    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: Some(10),
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.total, 3, "Filter by user_id=10 must return 3 rows");
    assert!(
        result.logs.iter().all(|l| l.user_id == Some(10)),
        "All returned logs must belong to user 10"
    );
}

// ---------------------------------------------------------------------------
// 6. get_audit_logs filter by action
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_audit_logs_filter_by_action() {
    let db = create_test_db().await;
    let now = chrono::Utc::now();

    insert_log(&db, "login", "session", None, true, now).await;
    insert_log(&db, "login", "session", None, true, now).await;
    insert_log(&db, "logout", "session", None, true, now).await;
    insert_log(&db, "app_installed", "app", None, true, now).await;

    let result = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: Some("login".to_string()),
            resource_type: None,
            success: None,
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.total, 2, "Filter action='login' must return 2 rows");
    assert!(
        result.logs.iter().all(|l| l.action == "login"),
        "All returned logs must have action='login'"
    );
}

// ---------------------------------------------------------------------------
// 7. get_audit_logs filter by success=true / false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_audit_logs_filter_by_success() {
    let db = create_test_db().await;
    let now = chrono::Utc::now();

    // 3 successes, 2 failures
    for _ in 0..3 {
        insert_log(&db, "login", "session", None, true, now).await;
    }
    for _ in 0..2 {
        insert_log(&db, "login_failed", "session", None, false, now).await;
    }

    // Filter success=true
    let successes = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: Some(true),
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(successes.total, 3, "Filter success=true must return 3 rows");
    assert!(
        successes.logs.iter().all(|l| l.success),
        "All returned logs must have success=true"
    );

    // Filter success=false
    let failures = get_audit_logs(
        &db,
        AuditLogQuery {
            page: None,
            per_page: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: Some(false),
            from: None,
            to: None,
            search: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(failures.total, 2, "Filter success=false must return 2 rows");
    assert!(
        failures.logs.iter().all(|l| !l.success),
        "All returned logs must have success=false"
    );
}

// ---------------------------------------------------------------------------
// 8. get_audit_stats — correct totals and time-window counts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_audit_stats() {
    let db = create_test_db().await;
    let now = chrono::Utc::now();

    // 2 successes today
    insert_log(&db, "login", "session", None, true, now).await;
    insert_log(&db, "app_installed", "app", None, true, now).await;

    // 1 failure today
    insert_log(&db, "login_failed", "session", None, false, now).await;

    // 1 success that is old (90 days ago — outside current week but still total)
    let old = now - chrono::Duration::days(90);
    insert_log(&db, "login", "session", None, true, old).await;

    let stats = get_audit_stats(&db)
        .await
        .expect("get_audit_stats should succeed");

    assert_eq!(stats.total_events, 4, "Total must count all 4 rows");
    assert_eq!(stats.successful_events, 3, "3 rows are successful");
    assert_eq!(stats.failed_events, 1, "1 row is a failure");
    assert_eq!(
        stats.events_today, 3,
        "3 events were inserted with today's timestamp"
    );
    assert_eq!(
        stats.events_this_week, 3,
        "3 events are within the last 7 days; the 90-day-old one is excluded"
    );
    assert_eq!(
        stats.recent_failures.len(),
        1,
        "There is 1 failure, so recent_failures must have 1 entry"
    );

    // top_actions must contain at least "login" (appears twice across all time)
    let login_action = stats.top_actions.iter().find(|a| a.action == "login");
    assert!(
        login_action.is_some(),
        "top_actions must include the 'login' action"
    );
    assert_eq!(
        login_action.unwrap().count,
        2,
        "'login' must appear with count=2"
    );
}

// ---------------------------------------------------------------------------
// 9. clear_old_logs removes rows older than the given day threshold
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_clear_old_logs() {
    let db = create_test_db().await;
    let now = chrono::Utc::now();

    // 2 recent rows (within retention window)
    insert_log(&db, "login", "session", None, true, now).await;
    insert_log(
        &db,
        "login",
        "session",
        None,
        true,
        now - chrono::Duration::days(5),
    )
    .await;

    // 3 old rows that should be purged (older than 30 days)
    let old_ts = now - chrono::Duration::days(60);
    insert_log(&db, "login", "session", None, true, old_ts).await;
    insert_log(&db, "logout", "session", None, true, old_ts).await;
    insert_log(&db, "login_failed", "session", None, false, old_ts).await;

    // Run retention with a 30-day window
    let deleted = clear_old_logs(&db, 30)
        .await
        .expect("clear_old_logs should succeed");

    assert_eq!(deleted, 3, "clear_old_logs must report 3 rows affected");

    // Verify remaining rows in the database
    let remaining = audit_log::Entity::find().all(&db).await.unwrap();
    assert_eq!(
        remaining.len(),
        2,
        "Only the 2 recent rows must survive retention"
    );
    for row in &remaining {
        assert!(
            row.timestamp > now - chrono::Duration::days(30),
            "Surviving rows must be within the retention window"
        );
    }
}
