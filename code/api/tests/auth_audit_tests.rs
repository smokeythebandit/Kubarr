use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{header, Request, StatusCode},
};
use kubarr::models::{two_factor_recovery_code, user};
use kubarr::services::security::{
    generate_recovery_codes, generate_totp_secret, hash_recovery_code,
};
use kubarr::{
    endpoints::create_router,
    models::prelude::*,
    services::{decode_session_token, init_jwt_keys},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Set,
    Statement,
};
use std::net::SocketAddr;
use tower::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn setup_keys() {
    JWT_INIT
        .get_or_init(|| async {
            let db = create_test_db_with_seed().await;
            init_jwt_keys(&db).await.unwrap();
        })
        .await;
}

#[tokio::test]
async fn recovery_and_missing_totp_audit() {
    setup_keys().await;
    let db = create_test_db_with_seed().await;
    let actor = create_test_user_with_role(
        &db,
        "recoverer",
        "recover@example.com",
        "safe-password",
        "viewer",
    )
    .await;
    let now = chrono::Utc::now();
    user::ActiveModel {
        id: Set(actor.id),
        totp_enabled: Set(true),
        totp_secret: Set(Some(generate_totp_secret())),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let code = generate_recovery_codes().remove(0);
    two_factor_recovery_code::ActiveModel {
        user_id: Set(actor.id),
        code_hash: Set(hash_recovery_code(&code).unwrap()),
        used_at: Set(None),
        created_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let app = create_router(build_test_app_state_with_db(db.clone()).await);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"username":"recoverer", "password":"safe-password"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    for (recovery_code, expected) in [
        ("bad-code-SENTINEL", StatusCode::UNAUTHORIZED),
        (code.as_str(), StatusCode::OK),
    ] {
        let response = app.clone().oneshot(Request::builder().uri("/auth/2fa/recover").method("POST")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({"username":"recoverer", "password":"safe-password", "recovery_code":recovery_code}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), expected);
    }
    let rows = AuditLog::find().all(&db).await.unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(),
        ["2fa_failed", "2fa_failed", "2fa_verified", "login"]
    );
    assert_eq!(
        rows[0].details.as_deref(),
        Some(r#"{"reason":"totp_required"}"#)
    );
    assert_eq!(
        rows[1].details.as_deref(),
        Some(r#"{"reason":"invalid_totp"}"#)
    );
    assert_eq!(
        rows[2].details.as_deref(),
        Some(r#"{"method":"recovery_code"}"#)
    );
    assert_eq!(
        rows[3].details.as_deref(),
        Some(r#"{"method":"recovery_code"}"#)
    );
    assert!(rows.iter().all(|r| r.user_id == Some(actor.id)));
    let serialized = serde_json::to_string(&rows).unwrap();
    assert!(!serialized.contains(&code));
    assert!(!serialized.contains("bad-code-SENTINEL"));
    assert!(!serialized.contains("safe-password"));
}

#[tokio::test]
async fn valid_totp_emits_verified_and_login_but_failed_totp_does_not() {
    setup_keys().await;
    let db = create_test_db_with_seed().await;
    let actor = create_test_user_with_role(
        &db,
        "totpactor",
        "totp@example.com",
        "totp-password-SENTINEL",
        "viewer",
    )
    .await;
    let secret = generate_totp_secret();
    user::ActiveModel {
        id: Set(actor.id),
        totp_enabled: Set(true),
        totp_secret: Set(Some(secret.clone())),
        updated_at: Set(chrono::Utc::now()),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let app = create_router(build_test_app_state_with_db(db.clone()).await);
    let failed = app.clone().oneshot(Request::builder().uri("/auth/login").method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::json!({"username":"totpactor", "password":"totp-password-SENTINEL", "totp_code":"invalid-SENTINEL"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(failed.status(), StatusCode::UNAUTHORIZED);
    let prior = AuditLog::find().all(&db).await.unwrap();
    assert_eq!(prior.len(), 1);
    assert_eq!(prior[0].action, "2fa_failed");

    use totp_rs::{Algorithm, Secret, TOTP};
    let code = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        Secret::Encoded(secret.clone()).to_bytes().unwrap(),
        Some("Kubarr".to_string()),
        "totp@example.com".to_string(),
    )
    .unwrap()
    .generate_current()
    .unwrap();
    let success = app.oneshot(Request::builder().uri("/auth/login").method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::json!({"username":"totpactor", "password":"totp-password-SENTINEL", "totp_code":code}).to_string())).unwrap()).await.unwrap();
    assert_eq!(success.status(), StatusCode::OK);
    assert!(success
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .is_some());
    let rows = AuditLog::find().all(&db).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(),
        ["2fa_failed", "2fa_verified", "login"]
    );
    assert_eq!(rows[1].details.as_deref(), Some(r#"{"method":"totp"}"#));
    assert_eq!(rows[1].user_id, Some(actor.id));
    assert_eq!(rows[1].resource_id.as_deref(), Some(&*actor.id.to_string()));
    assert_eq!(rows[1].username.as_deref(), Some("totpactor"));
    assert!(rows[1].success);
    let serialized = serde_json::to_string(&rows).unwrap();
    for secret_value in [
        secret.as_str(),
        code.as_str(),
        "totp-password-SENTINEL",
        "invalid-SENTINEL",
    ] {
        assert!(
            !serialized.contains(secret_value),
            "secret leaked into audit row"
        );
    }
}

#[tokio::test]
async fn recovery_verification_audit_failure_does_not_consume_code_or_create_session() {
    setup_keys().await;
    let db = create_test_db_with_seed().await;
    let actor = create_test_user_with_role(
        &db,
        "rollback2fa",
        "rollback2fa@example.com",
        "correct-password",
        "viewer",
    )
    .await;
    let now = chrono::Utc::now();
    user::ActiveModel {
        id: Set(actor.id),
        totp_enabled: Set(true),
        totp_secret: Set(Some(generate_totp_secret())),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let code = generate_recovery_codes().remove(0);
    let recovery = two_factor_recovery_code::ActiveModel {
        user_id: Set(actor.id),
        code_hash: Set(hash_recovery_code(&code).unwrap()),
        used_at: Set(None),
        created_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let app = create_router(build_test_app_state_with_db(db.clone()).await);
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "DROP TABLE audit_logs".to_string(),
    ))
    .await
    .unwrap();
    let response = app.oneshot(Request::builder().uri("/auth/2fa/recover").method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::json!({"username":"rollback2fa", "password":"correct-password", "recovery_code":code}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .is_none());
    let stored = TwoFactorRecoveryCode::find_by_id(recovery.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(stored.used_at.is_none());
    assert!(
        User::find_by_id(actor.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .totp_enabled
    );
    assert!(Session::find()
        .filter(kubarr::models::session::Column::UserId.eq(actor.id))
        .all(&db)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn login_failure_success_and_logout_audit_without_credentials() {
    setup_keys().await;
    let db = create_test_db_with_seed().await;
    let actor = create_test_user_with_role(
        &db,
        "auditactor",
        "audit@example.com",
        "password_SENTINEL",
        "viewer",
    )
    .await;
    let app = create_router(build_test_app_state_with_db(db.clone()).await);

    for (name, password, expected) in [
        (
            "unknown_SENTINEL",
            "password_SENTINEL",
            StatusCode::UNAUTHORIZED,
        ),
        ("auditactor", "wrong_SENTINEL", StatusCode::UNAUTHORIZED),
        ("auditactor", "password_SENTINEL", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/auth/login")
                    .method("POST")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("x-forwarded-for", "untrusted_SENTINEL")
                    .header(header::USER_AGENT, "test-agent")
                    .body(Body::from(
                        serde_json::json!({"username": name, "password": password}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let cookie = response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .find_map(|v| v.to_str().ok().filter(|v| v.starts_with("kubarr_session=")))
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned();
            let logout = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/auth/logout")
                        .method("POST")
                        .header(header::COOKIE, cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(logout.status(), StatusCode::OK);
        }
    }

    let rows = AuditLog::find().all(&db).await.unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(),
        ["login_failed", "login_failed", "login", "logout"]
    );
    assert_eq!(rows[0].user_id, None);
    assert_eq!(rows[0].username, None);
    assert_eq!(rows[0].resource_id, None);
    for row in &rows[1..] {
        assert_eq!(row.user_id, Some(actor.id));
        assert_eq!(row.username.as_deref(), Some("auditactor"));
        assert_eq!(row.resource_id.as_deref(), Some(&*actor.id.to_string()));
    }
    assert_eq!(
        rows[0].details.as_deref(),
        Some(r#"{"reason":"invalid_credentials"}"#)
    );
    assert_eq!(
        rows[1].details.as_deref(),
        Some(r#"{"reason":"invalid_credentials"}"#)
    );
    assert!(rows.iter().all(|r| r.ip_address.is_none()));
    let serialized = serde_json::to_string(&rows).unwrap();
    for secret in [
        "password_SENTINEL",
        "wrong_SENTINEL",
        "unknown_SENTINEL",
        "untrusted_SENTINEL",
        "kubarr_session",
    ] {
        assert!(!serialized.contains(secret), "audit leaked {secret}");
    }
}

#[tokio::test]
async fn socket_peer_wins_over_forwarded_headers_and_agent_is_bounded() {
    setup_keys().await;
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "peeruser",
        "peer@example.com",
        "socket-password",
        "viewer",
    )
    .await;
    let app = create_router(build_test_app_state_with_db(db.clone()).await);
    let peer: SocketAddr = "192.0.2.42:54321".parse().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-forwarded-for", "198.51.100.98")
                .header("x-real-ip", "198.51.100.99")
                .header(header::USER_AGENT, "a".repeat(300))
                .extension(ConnectInfo(peer))
                .body(Body::from(
                    serde_json::json!({"username":"peeruser", "password":"socket-password"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let rows = AuditLog::find().all(&db).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].ip_address.as_deref(), Some("192.0.2.42"));
    // The audit service discards untrusted User-Agent contents entirely.
    assert_eq!(rows[0].user_agent.as_deref(), Some("[REDACTED]"));
    let text = serde_json::to_string(&rows).unwrap();
    assert!(!text.contains("socket-password"));
    assert!(!text.contains("198.51.100"));

    let app = create_router(build_test_app_state_with_db(db.clone()).await);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::USER_AGENT, "password=AGENT_SECRET_SENTINEL")
                .header(header::COOKIE, "unrelated_cookie=COOKIE_SECRET_SENTINEL")
                .extension(ConnectInfo(peer))
                .body(Body::from(
                    serde_json::json!({"username":"peeruser", "password":"wrong"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let rows = AuditLog::find().all(&db).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].user_agent.as_deref(), Some("[REDACTED]"));
    assert_eq!(rows[1].ip_address.as_deref(), Some("192.0.2.42"));
    let text = serde_json::to_string(&rows).unwrap();
    assert!(!text.contains("AGENT_SECRET_SENTINEL"));
    assert!(!text.contains("COOKIE_SECRET_SENTINEL"));
}

async fn login_cookie(app: &axum::Router, username: &str, password: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"username":username,"password":password}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| v.to_str().ok().filter(|v| v.starts_with("kubarr_session=")))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn revoke_session_audits_only_authorized_deletions_and_rolls_back_on_audit_failure() {
    setup_keys().await;
    let db = create_test_db_with_seed().await;
    let owner =
        create_test_user_with_role(&db, "owner", "owner@example.com", "owner-pass", "viewer").await;
    create_test_user_with_role(&db, "other", "other@example.com", "other-pass", "viewer").await;
    let app = create_router(build_test_app_state_with_db(db.clone()).await);
    let owner_cookie = login_cookie(&app, "owner", "owner-pass").await;
    let other_cookie = login_cookie(&app, "other", "other-pass").await;
    let second_owner_cookie = login_cookie(&app, "owner", "owner-pass").await;
    let target_id = decode_session_token(owner_cookie.split('=').nth(1).unwrap())
        .unwrap()
        .sid;
    let baseline = AuditLog::find().all(&db).await.unwrap().len();
    for (cookie, id, expected) in [
        (&other_cookie, &target_id, StatusCode::FORBIDDEN),
        (&owner_cookie, &target_id, StatusCode::BAD_REQUEST),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/auth/sessions/{id}"))
                    .method("DELETE")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    assert_eq!(AuditLog::find().all(&db).await.unwrap().len(), baseline);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/auth/sessions/{target_id}"))
                .method("DELETE")
                .header(header::COOKIE, &second_owner_cookie)
                .header("x-forwarded-for", "spoofed-SENTINEL")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(Session::find_by_id(&target_id)
        .one(&db)
        .await
        .unwrap()
        .is_none());
    let rows = AuditLog::find().all(&db).await.unwrap();
    assert_eq!(rows.len(), baseline + 1);
    let row = rows.last().unwrap();
    assert_eq!(row.action, "logout");
    assert_eq!(row.user_id, Some(owner.id));
    assert_eq!(row.resource_id.as_deref(), Some(&*owner.id.to_string()));
    assert_eq!(
        row.details.as_deref(),
        Some(
            &*serde_json::json!({"operation":"session_revoked", "target_user_id":owner.id})
                .to_string()
        )
    );
    assert!(!serde_json::to_string(row).unwrap().contains(&target_id));
    assert!(!serde_json::to_string(row)
        .unwrap()
        .contains("spoofed-SENTINEL"));

    // Removing the audit table forces the audit insert to fail. The preceding
    // session deletion must roll back and must not report successful revocation.
    let newest_owner_cookie = login_cookie(&app, "owner", "owner-pass").await;
    let survivor_id = decode_session_token(second_owner_cookie.split('=').nth(1).unwrap())
        .unwrap()
        .sid;
    db.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "DROP TABLE audit_logs".to_string(),
    ))
    .await
    .unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/auth/sessions/{survivor_id}"))
                .method("DELETE")
                .header(header::COOKIE, &newest_owner_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(Session::find_by_id(&survivor_id)
        .one(&db)
        .await
        .unwrap()
        .is_some());
}
