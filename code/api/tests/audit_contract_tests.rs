mod common;

use chrono::{Duration, Utc};
use kubarr::models::audit_log::{self, AuditAction, ResourceType};
use kubarr::services::audit::{
    clear_old_logs, clear_old_logs_audited, clear_old_logs_with, get_audit_logs, get_audit_stats,
    log_on_transaction, AuditLogQuery, AuditService,
};
use sea_orm::{ActiveModelTrait, EntityTrait, Set, TransactionTrait};

#[tokio::test]
async fn created_user_audit_uses_target_id_not_posted_username() {
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use kubarr::endpoints::create_router;
    use tower::ServiceExt;

    static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    JWT_INIT
        .get_or_init(|| async {
            let db = common::create_test_db_with_seed().await;
            kubarr::services::init_jwt_keys(&db).await.unwrap();
        })
        .await;
    let db = common::create_test_db_with_seed().await;
    let actor = common::create_test_user_with_role(
        &db,
        "auditadmin",
        "admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = common::build_test_app_state_with_db(db.clone()).await;
    let response = create_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"username":"auditadmin","password":"password123"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("kubarr_session=") && !v.contains("kubarr_session_"))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let sentinel = "pasted-secret-123";
    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/users")
                .method("POST")
                .header("Cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"username": sentinel, "email": "target@example.com",
            "password": "safe-test-password", "role_ids": []})
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let rows = audit_log::Entity::find().all(&db).await.unwrap();
    let created = rows
        .iter()
        .find(|row| row.action == "user_created")
        .unwrap();
    assert_eq!(created.user_id, Some(actor.id));
    assert_eq!(created.username.as_deref(), Some("auditadmin"));
    assert!(created.resource_id.as_ref().unwrap().parse::<i64>().is_ok());
    assert_eq!(created.details.as_deref(), Some(r#"{"role_ids":[]}"#));
    assert!(!format!("{created:?}").contains(sentinel));
}

#[tokio::test]
async fn missing_db_is_an_error_and_details_are_redacted_and_bounded() {
    let service = AuditService::new();
    assert!(service
        .log_success(
            AuditAction::Login,
            ResourceType::User,
            None,
            None,
            None,
            None,
            None,
            None
        )
        .await
        .is_err());

    let db = common::create_test_db().await;
    service.set_db(db.clone()).await;
    service.log(AuditAction::Login, ResourceType::User, None, None, None,
        Some(serde_json::json!({"nested": [{"PASSWORD": "never-store", "api-Key": "also-secret",
            "profile": {"ToTp": "otp-secret", "recovery_code": "backup-secret", "inviteCode": "invite-secret",
            "credentials": {"username": "hidden"}, "cookie": "session-secret", "private_key": "key-secret",
            "token": "token-secret", "secret": "secret-secret"}}], "safe": "visible"})),
        None, Some("agent".repeat(1000)), false, Some("error".repeat(1000)))
        .await.unwrap();
    let log = audit_log::Entity::find().one(&db).await.unwrap().unwrap();
    let details = log.details.unwrap();
    for secret in [
        "never-store",
        "also-secret",
        "otp-secret",
        "backup-secret",
        "invite-secret",
        "hidden",
        "session-secret",
        "key-secret",
        "token-secret",
        "secret-secret",
    ] {
        assert!(!details.contains(secret));
    }
    assert!(!details.contains("visible"));
    assert!(details.len() <= 8192);
    assert!(log.user_agent.unwrap().len() <= 1024);
    assert_eq!(log.error_message.as_deref(), Some("[REDACTED]"));
}

#[tokio::test]
async fn only_typed_details_and_fixed_labels_survive_untrusted_values() {
    let db = common::create_test_db().await;
    let service = AuditService::new();
    service.set_db(db.clone()).await;
    let sentinel = "pasted-secret-123";
    service
        .log(
            AuditAction::UserCreated,
            ResourceType::User,
            Some(sentinel.into()),
            Some(7),
            Some("actor".into()),
            Some(
                serde_json::json!({"username": sentinel, "name": sentinel, "role_ids": [1, 2],
            "operation_id": "ca37db03-0dd1-4d5e-881a-74b3717179b7",
            "fields": ["email", sentinel], "reason": sentinel, "unknown": {"value": sentinel},
            "provider_id": 4, "operation": "audit_clear", (sentinel): "value"}),
            ),
            None,
            Some(sentinel.into()),
            false,
            Some(sentinel.into()),
        )
        .await
        .unwrap();
    let row = audit_log::Entity::find().one(&db).await.unwrap().unwrap();
    assert_eq!(row.username.as_deref(), Some("actor"));
    assert_eq!(row.resource_id.as_deref(), Some("[REDACTED]"));
    assert_eq!(row.user_agent.as_deref(), Some("[REDACTED]"));
    assert_eq!(row.error_message.as_deref(), Some("[REDACTED]"));
    let details: serde_json::Value = serde_json::from_str(row.details.as_ref().unwrap()).unwrap();
    assert!(!row.details.unwrap().contains(sentinel));
    assert!(details.get("username").is_none());
    assert_eq!(details["role_ids"], serde_json::json!([1, 2]));
    assert_eq!(details["provider_id"], 4);
    assert_eq!(
        details["fields"],
        serde_json::json!(["email", "[REDACTED]"])
    );
    assert_eq!(details["operation"], "audit_clear");
}

#[tokio::test]
async fn fixed_failure_labels_survive_but_freeform_errors_do_not() {
    let db = common::create_test_db().await;
    let service = AuditService::new();
    service.set_db(db.clone()).await;
    for label in [
        "invalid_credentials",
        "account_disabled",
        "approval_required",
        "totp_required",
        "invalid_totp",
        "setup_required",
        "verification_failed",
        "app_operation_failed",
        "outcome_indeterminate_after_worker_interruption",
        "invalid password",
        "unknown exception: secret",
    ] {
        service
            .log_failure(
                AuditAction::LoginFailed,
                ResourceType::User,
                None,
                None,
                None,
                None,
                None,
                None,
                label,
            )
            .await
            .unwrap();
    }
    let rows = audit_log::Entity::find().all(&db).await.unwrap();
    for (row, label) in rows.iter().zip([
        "invalid_credentials",
        "account_disabled",
        "approval_required",
        "totp_required",
        "invalid_totp",
        "setup_required",
        "verification_failed",
        "app_operation_failed",
        "outcome_indeterminate_after_worker_interruption",
        "[REDACTED]",
        "[REDACTED]",
    ]) {
        assert_eq!(row.error_message.as_deref(), Some(label));
    }
}

#[tokio::test]
async fn transaction_writer_rolls_back_and_query_limits_and_stats_are_correct() {
    let db = common::create_test_db().await;
    let transaction = db.begin().await.unwrap();
    log_on_transaction(
        &transaction,
        AuditAction::Logout,
        ResourceType::User,
        None,
        None,
        None,
        None,
        None,
        None,
        true,
        None,
    )
    .await
    .unwrap();
    transaction.rollback().await.unwrap();
    assert!(audit_log::Entity::find().one(&db).await.unwrap().is_none());

    let service = AuditService::new();
    service.set_db(db.clone()).await;
    for action in [AuditAction::Login, AuditAction::Login, AuditAction::Logout] {
        service
            .log_success(
                action,
                ResourceType::User,
                None,
                None,
                Some("alice".into()),
                None,
                None,
                None,
            )
            .await
            .unwrap();
    }
    let stats = get_audit_stats(&db).await.unwrap();
    assert_eq!(stats.total_events, 3);
    assert_eq!(stats.top_actions[0].action, "login");
    assert_eq!(stats.top_actions[0].count, 2);
    assert_eq!(stats.top_actions[1].count, 1);

    let empty = get_audit_logs(
        &db,
        AuditLogQuery {
            search: Some("not-found".into()),
            page: Some(0),
            per_page: Some(0),
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
        },
    )
    .await
    .unwrap();
    assert_eq!((empty.page, empty.per_page, empty.total_pages), (1, 1, 0));
    let huge = get_audit_logs(
        &db,
        AuditLogQuery {
            page: Some(u64::MAX),
            per_page: Some(100),
            search: None,
            user_id: None,
            action: None,
            resource_type: None,
            success: None,
            from: None,
            to: None,
        },
    )
    .await;
    assert!(huge.is_err());
    let query = AuditLogQuery {
        page: None,
        per_page: Some(u64::MAX),
        search: Some("alice".into()),
        user_id: None,
        action: None,
        resource_type: None,
        success: None,
        from: None,
        to: None,
    };
    let result = get_audit_logs(&db, query).await.unwrap();
    assert_eq!((result.total, result.per_page), (3, 100));
}

#[tokio::test]
async fn retention_is_validated_before_mutation_and_clear_is_attributed() {
    let db = common::create_test_db().await;
    for (days, action) in [(40, "old"), (2, "recent")] {
        audit_log::ActiveModel {
            timestamp: Set(Utc::now() - Duration::days(days)),
            action: Set(action.into()),
            resource_type: Set("system".into()),
            success: Set(true),
            ..Default::default()
        }
        .insert(&db)
        .await
        .unwrap();
    }
    for days in [-1, 0, 3651] {
        assert!(clear_old_logs(&db, days).await.is_err());
        assert!(clear_old_logs_audited(&db, days, 12, "admin".into())
            .await
            .is_err());
        assert_eq!(audit_log::Entity::find().all(&db).await.unwrap().len(), 2);
    }
    let (deleted, id) = clear_old_logs_audited(&db, 30, 12, "admin".into())
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    let logs = audit_log::Entity::find().all(&db).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert!(logs.iter().any(|row| row.action == "recent"));
    let event = logs.iter().find(|row| row.id == id).unwrap();
    assert!(event.success);
    assert_eq!(event.user_id, Some(12));
    assert_eq!(event.username.as_deref(), Some("admin"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(event.details.as_ref().unwrap()).unwrap(),
        serde_json::json!({"operation": "audit_clear", "days": 30, "deleted": 1})
    );
}

#[tokio::test]
async fn retention_delete_on_transaction_rolls_back_with_audit_write() {
    let db = common::create_test_db().await;
    audit_log::ActiveModel {
        timestamp: Set(Utc::now() - Duration::days(100)),
        action: Set("old".into()),
        resource_type: Set("system".into()),
        success: Set(true),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();

    let txn = db.begin().await.unwrap();
    assert_eq!(clear_old_logs_with(&txn, 90).await.unwrap(), 1);
    log_on_transaction(
        &txn,
        AuditAction::SystemSettingChanged,
        ResourceType::System,
        None,
        None,
        None,
        Some(
            serde_json::json!({"operation": "automatic_audit_retention", "days": 90, "deleted": 1}),
        ),
        None,
        None,
        true,
        None,
    )
    .await
    .unwrap();
    txn.rollback().await.unwrap();
    let logs = audit_log::Entity::find().all(&db).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action, "old");
}
