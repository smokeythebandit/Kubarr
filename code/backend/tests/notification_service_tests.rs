//! Tests for `kubarr::services::notification::NotificationService` (DB methods only).
//!
//! Covers:
//! - `get_unread_count` — returns 0 for a fresh user
//! - inserting a notification via Sea-ORM and retrieving it through the service
//! - `mark_as_read` — drops unread count to 0
//! - `mark_all_as_read` — marks every unread notification for a user
//! - `delete_notification` — removes a notification by id+user_id
//! - `delete_notification` with the wrong user_id — returns NotFound
//! - `get_user_notifications` pagination — limit / offset applied correctly

mod common;
use common::{create_test_db, create_test_user};

use kubarr::models::user_notification;
use kubarr::services::notification::NotificationService;
use sea_orm::{ActiveModelTrait, Set};

// ---------------------------------------------------------------------------
// Helper: build a NotificationService backed by `db`
// ---------------------------------------------------------------------------

async fn make_service(
    db: sea_orm::DatabaseConnection,
) -> (NotificationService, sea_orm::DatabaseConnection) {
    let svc = NotificationService::new();
    svc.set_db(db.clone()).await;
    (svc, db)
}

// ---------------------------------------------------------------------------
// Helper: insert a user_notification row directly so we can test the read-
// path of the service without relying on the private create_user_notification.
// ---------------------------------------------------------------------------

async fn insert_notification(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
    title: &str,
    message: &str,
    read: bool,
) -> user_notification::Model {
    let notif = user_notification::ActiveModel {
        user_id: Set(user_id),
        title: Set(title.to_string()),
        message: Set(message.to_string()),
        event_type: Set(None),
        severity: Set("info".to_string()),
        read: Set(read),
        created_at: Set(chrono::Utc::now()),
        ..Default::default()
    };
    notif.insert(db).await.unwrap()
}

// ---------------------------------------------------------------------------
// 1. A new user has 0 unread notifications
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unread_count_empty() {
    let db = create_test_db().await;
    // Use a seed db for create_test_user (needs the schema only — no roles needed here)
    let user = create_test_user(&db, "alice", "alice@example.com", "password", true).await;
    let (svc, _) = make_service(db).await;

    let count = svc
        .get_unread_count(user.id)
        .await
        .expect("get_unread_count should not fail");

    assert_eq!(count, 0, "A new user must have 0 unread notifications");
}

// ---------------------------------------------------------------------------
// 2. Insert a notification via Sea-ORM and retrieve it via the service
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_create_and_get_notifications() {
    let db = create_test_db().await;
    let user = create_test_user(&db, "bob", "bob@example.com", "password", true).await;

    // Insert directly since create_user_notification is private
    insert_notification(&db, user.id, "Hello", "World", false).await;

    let (svc, _) = make_service(db).await;

    let notifications = svc
        .get_user_notifications(user.id, 10, 0)
        .await
        .expect("get_user_notifications should not fail");

    assert_eq!(
        notifications.len(),
        1,
        "Service must return the inserted notification"
    );
    assert_eq!(notifications[0].title, "Hello");
    assert_eq!(notifications[0].message, "World");
    assert_eq!(notifications[0].user_id, user.id);
    assert!(!notifications[0].read, "Notification must start as unread");

    let count = svc.get_unread_count(user.id).await.unwrap();
    assert_eq!(
        count, 1,
        "Unread count must reflect the single unread notification"
    );
}

// ---------------------------------------------------------------------------
// 3. mark_as_read decrements unread count to 0
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mark_as_read() {
    let db = create_test_db().await;
    let user = create_test_user(&db, "carol", "carol@example.com", "password", true).await;

    let notif = insert_notification(&db, user.id, "Alert", "Something happened", false).await;

    let (svc, _) = make_service(db).await;

    // Confirm unread before
    assert_eq!(svc.get_unread_count(user.id).await.unwrap(), 1);

    svc.mark_as_read(notif.id, user.id)
        .await
        .expect("mark_as_read should succeed");

    // Unread count must drop to 0
    let count = svc.get_unread_count(user.id).await.unwrap();
    assert_eq!(count, 0, "Unread count must be 0 after marking as read");

    // The notification must still exist but with read=true
    let notifications = svc.get_user_notifications(user.id, 10, 0).await.unwrap();
    assert_eq!(notifications.len(), 1, "Notification must not be deleted");
    assert!(
        notifications[0].read,
        "Notification must be marked read=true"
    );
}

// ---------------------------------------------------------------------------
// 4. mark_all_as_read marks every unread notification for a user
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mark_all_as_read() {
    let db = create_test_db().await;
    let user = create_test_user(&db, "dave", "dave@example.com", "password", true).await;

    // Insert 3 unread + 1 already-read notification
    for i in 0..3 {
        insert_notification(&db, user.id, &format!("Notif {}", i), "body", false).await;
    }
    insert_notification(&db, user.id, "Already read", "body", true).await;

    let (svc, _) = make_service(db).await;

    // Sanity-check: 3 unread before the call
    assert_eq!(svc.get_unread_count(user.id).await.unwrap(), 3);

    svc.mark_all_as_read(user.id)
        .await
        .expect("mark_all_as_read should succeed");

    let count = svc.get_unread_count(user.id).await.unwrap();
    assert_eq!(count, 0, "All 3 unread notifications must be marked read");

    // All 4 notifications must still exist
    let all = svc.get_user_notifications(user.id, 20, 0).await.unwrap();
    assert_eq!(all.len(), 4);
    assert!(
        all.iter().all(|n| n.read),
        "Every notification must have read=true"
    );
}

// ---------------------------------------------------------------------------
// 5. delete_notification removes the row
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_delete_notification() {
    let db = create_test_db().await;
    let user = create_test_user(&db, "eve", "eve@example.com", "password", true).await;

    let notif = insert_notification(&db, user.id, "To delete", "bye", false).await;

    let (svc, _) = make_service(db).await;

    svc.delete_notification(notif.id, user.id)
        .await
        .expect("delete_notification should succeed");

    let remaining = svc.get_user_notifications(user.id, 10, 0).await.unwrap();
    assert!(
        remaining.is_empty(),
        "Notification must be gone after deletion"
    );
}

// ---------------------------------------------------------------------------
// 6. delete_notification with the wrong user_id returns NotFound
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_delete_notification_wrong_user() {
    let db = create_test_db().await;
    let owner = create_test_user(&db, "frank", "frank@example.com", "password", true).await;
    let intruder = create_test_user(&db, "grace", "grace@example.com", "password", true).await;

    let notif = insert_notification(&db, owner.id, "Private", "secret", false).await;

    let (svc, _) = make_service(db).await;

    let result = svc.delete_notification(notif.id, intruder.id).await;

    assert!(
        result.is_err(),
        "Deleting another user's notification must return an error"
    );

    // Verify the error is AppError::NotFound
    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("Not found") || err_str.contains("not found"),
        "Error must be a NotFound variant, got: {}",
        err_str
    );

    // The notification must still be present in the database
    let remaining = svc.get_user_notifications(owner.id, 10, 0).await.unwrap();
    assert_eq!(remaining.len(), 1, "Owner's notification must be untouched");
}

// ---------------------------------------------------------------------------
// 7. get_user_notifications pagination — limit and offset are respected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_user_notifications_pagination() {
    let db = create_test_db().await;
    let user = create_test_user(&db, "henry", "henry@example.com", "password", true).await;

    // Insert 5 notifications with slightly different timestamps so ordering
    // is deterministic (the service orders by created_at DESC).
    let now = chrono::Utc::now();
    for i in 0..5u64 {
        let notif = user_notification::ActiveModel {
            user_id: Set(user.id),
            title: Set(format!("Notif {}", i)),
            message: Set("body".to_string()),
            event_type: Set(None),
            severity: Set("info".to_string()),
            read: Set(false),
            // Decreasing offset so that i=0 is the newest
            created_at: Set(now - chrono::Duration::seconds(i as i64 * 5)),
            ..Default::default()
        };
        notif.insert(&db).await.unwrap();
    }

    let (svc, _) = make_service(db).await;

    // First page: limit=2, offset=0 → 2 results
    let page1 = svc
        .get_user_notifications(user.id, 2, 0)
        .await
        .expect("Pagination page 1 should not fail");
    assert_eq!(page1.len(), 2, "Page 1 must return 2 notifications");

    // Second page: limit=2, offset=2 → next 2 results
    let page2 = svc
        .get_user_notifications(user.id, 2, 2)
        .await
        .expect("Pagination page 2 should not fail");
    assert_eq!(page2.len(), 2, "Page 2 must return 2 notifications");

    // Third page: limit=2, offset=4 → last 1 result
    let page3 = svc
        .get_user_notifications(user.id, 2, 4)
        .await
        .expect("Pagination page 3 should not fail");
    assert_eq!(
        page3.len(),
        1,
        "Page 3 must return the remaining 1 notification"
    );

    // The two pages must not overlap (IDs must be disjoint)
    let ids1: std::collections::HashSet<i64> = page1.iter().map(|n| n.id).collect();
    let ids2: std::collections::HashSet<i64> = page2.iter().map(|n| n.id).collect();
    assert!(
        ids1.is_disjoint(&ids2),
        "Page 1 and page 2 must not contain overlapping notifications"
    );
}
