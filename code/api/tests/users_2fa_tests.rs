//! Tests for the 2FA user endpoints
//!
//! Covers:
//! - `POST /api/users/me/2fa/setup`         — success, already-enabled error
//! - `POST /api/users/me/2fa/enable`        — no-setup error, wrong-code error
//! - `POST /api/users/me/2fa/disable`       — not-enabled error, wrong-password error
//! - `GET  /api/users/me/2fa/recovery-codes` — returns remaining count
//!
//! Note: The full TOTP enable/disable success paths require a live TOTP code,
//! so the tests exercise the error paths that cover most of the handler code.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sea_orm::{ActiveModelTrait, Set};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;
use kubarr::models::user;

// ============================================================================
// JWT key initialization (one-time per test binary)
// ============================================================================

static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_jwt_keys() {
    JWT_INIT
        .get_or_init(|| async {
            let db = create_test_db_with_seed().await;
            kubarr::services::init_jwt_keys(&db)
                .await
                .expect("Failed to initialise test JWT keys");
        })
        .await;
}

// ============================================================================
// Helpers
// ============================================================================

async fn do_login(
    app: axum::Router,
    username: &str,
    password: &str,
) -> (StatusCode, Option<String>) {
    let body = serde_json::json!({
        "username": username,
        "password": password
    })
    .to_string();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            if s.starts_with("kubarr_session=") && !s.contains("kubarr_session_") {
                Some(s.split(';').next().unwrap().to_string())
            } else {
                None
            }
        });

    (status, cookie)
}

async fn authenticated_get(app: axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("GET")
                .header("Cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes).to_string();

    (status, body)
}

async fn authenticated_post(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("POST")
                .header("content-type", "application/json")
                .header("Cookie", cookie)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes).to_string();

    (status, body_str)
}

// ============================================================================
// POST /api/users/me/2fa/setup — success
// ============================================================================

#[tokio::test]
async fn test_setup_2fa_success_returns_secret_and_uri() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "setup2fauser",
        "setup2fa@test.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, cookie) =
        do_login(create_router(state.clone()), "setup2fauser", "password123").await;
    assert_eq!(status, StatusCode::OK, "Login must succeed");
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_post(create_router(state), "/api/users/me/2fa/setup", &cookie, "").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/users/me/2fa/setup must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("secret").is_some(),
        "Response must contain 'secret' field"
    );
    assert!(
        json.get("provisioning_uri").is_some(),
        "Response must contain 'provisioning_uri' field"
    );
    assert!(
        json["secret"].as_str().unwrap_or("").len() > 10,
        "Secret must be non-empty"
    );
    assert!(
        json["provisioning_uri"]
            .as_str()
            .unwrap_or("")
            .starts_with("otpauth://"),
        "Provisioning URI must be an otpauth:// URL"
    );
}

// ============================================================================
// POST /api/users/me/2fa/setup — already enabled error
// ============================================================================

#[tokio::test]
async fn test_setup_2fa_already_enabled_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let target_user = create_test_user_with_role(
        &db,
        "setup2faenabled",
        "setup2faenabled@test.com",
        "password123",
        "admin",
    )
    .await;

    let state = build_test_app_state_with_db(db.clone()).await;

    // Login first (user has 2FA disabled at this point)
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "setup2faenabled",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Set totp_enabled=true in the DB after login so the session is still valid
    let now = chrono::Utc::now();
    let user_model = user::ActiveModel {
        id: Set(target_user.id),
        totp_enabled: Set(true),
        totp_secret: Set(Some("JBSWY3DPEHPK3PXP".to_string())),
        totp_verified_at: Set(Some(now)),
        updated_at: Set(now),
        ..Default::default()
    };
    user_model.update(&db).await.unwrap();

    // Now call setup_2fa — should fail since 2FA is already enabled
    let (status, body) =
        authenticated_post(create_router(state), "/api/users/me/2fa/setup", &cookie, "").await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "setup_2fa when already enabled must return 400. Body: {}",
        body
    );
}

// ============================================================================
// POST /api/users/me/2fa/enable — no setup (no secret stored)
// ============================================================================

#[tokio::test]
async fn test_enable_2fa_no_setup_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "enable2fauser",
        "enable2fa@test.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "enable2fauser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let body = serde_json::json!({"code": "123456"}).to_string();
    let (status, resp_body) = authenticated_post(
        create_router(state),
        "/api/users/me/2fa/enable",
        &cookie,
        &body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "enable_2fa without setup must return 400. Body: {}",
        resp_body
    );
}

// ============================================================================
// POST /api/users/me/2fa/enable — wrong TOTP code after setup
// ============================================================================

#[tokio::test]
async fn test_enable_2fa_wrong_code_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "enable2fawrong",
        "enable2fawrong@test.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    // First, call setup to generate and store a TOTP secret
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "enable2fawrong",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must succeed");

    let (setup_status, _) = authenticated_post(
        create_router(state.clone()),
        "/api/users/me/2fa/setup",
        &cookie,
        "",
    )
    .await;
    assert_eq!(setup_status, StatusCode::OK, "Setup must succeed first");

    // Now try to enable with a deliberately wrong code
    let body = serde_json::json!({"code": "000000"}).to_string();
    let (status, resp_body) = authenticated_post(
        create_router(state),
        "/api/users/me/2fa/enable",
        &cookie,
        &body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "enable_2fa with wrong code must return 400. Body: {}",
        resp_body
    );
}

// ============================================================================
// POST /api/users/me/2fa/enable — already enabled
// ============================================================================

#[tokio::test]
async fn test_enable_2fa_already_enabled_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let target_user = create_test_user_with_role(
        &db,
        "enable2faalready",
        "enable2faalready@test.com",
        "password123",
        "admin",
    )
    .await;

    let state = build_test_app_state_with_db(db.clone()).await;

    // Login first (2FA disabled)
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "enable2faalready",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Set totp_enabled=true in DB after login
    let now = chrono::Utc::now();
    let user_model = user::ActiveModel {
        id: Set(target_user.id),
        totp_enabled: Set(true),
        totp_secret: Set(Some("JBSWY3DPEHPK3PXP".to_string())),
        totp_verified_at: Set(Some(now)),
        updated_at: Set(now),
        ..Default::default()
    };
    user_model.update(&db).await.unwrap();

    let body = serde_json::json!({"code": "123456"}).to_string();
    let (status, resp_body) = authenticated_post(
        create_router(state),
        "/api/users/me/2fa/enable",
        &cookie,
        &body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "enable_2fa when already enabled must return 400. Body: {}",
        resp_body
    );
}

// ============================================================================
// POST /api/users/me/2fa/disable — 2FA not enabled
// ============================================================================

#[tokio::test]
async fn test_disable_2fa_not_enabled_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "disable2fanone",
        "disable2fanone@test.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "disable2fanone",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let body = serde_json::json!({"password": "password123"}).to_string();
    let (status, resp_body) = authenticated_post(
        create_router(state),
        "/api/users/me/2fa/disable",
        &cookie,
        &body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "disable_2fa when not enabled must return 400. Body: {}",
        resp_body
    );
}

// ============================================================================
// POST /api/users/me/2fa/disable — wrong password
// ============================================================================

#[tokio::test]
async fn test_disable_2fa_wrong_password_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let target_user = create_test_user_with_role(
        &db,
        "disable2fawrong",
        "disable2fawrong@test.com",
        "correct_password",
        "admin",
    )
    .await;

    let state = build_test_app_state_with_db(db.clone()).await;

    // Login first (2FA disabled), then enable in DB
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "disable2fawrong",
        "correct_password",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Set totp_enabled=true in DB after login
    let now = chrono::Utc::now();
    let user_model = user::ActiveModel {
        id: Set(target_user.id),
        totp_enabled: Set(true),
        totp_secret: Set(Some("JBSWY3DPEHPK3PXP".to_string())),
        totp_verified_at: Set(Some(now)),
        updated_at: Set(now),
        ..Default::default()
    };
    user_model.update(&db).await.unwrap();

    let body = serde_json::json!({"password": "wrong_password"}).to_string();
    let (status, resp_body) = authenticated_post(
        create_router(state),
        "/api/users/me/2fa/disable",
        &cookie,
        &body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "disable_2fa with wrong password must return 400. Body: {}",
        resp_body
    );
}

// ============================================================================
// GET /api/users/me/2fa/recovery-codes — returns remaining count
// ============================================================================

#[tokio::test]
async fn test_get_recovery_code_count_returns_zero_for_new_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "recoveryuser",
        "recovery@test.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "recoveryuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/users/me/2fa/recovery-codes",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/users/me/2fa/recovery-codes must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["remaining"], 0,
        "New user with no 2FA must have 0 remaining recovery codes"
    );
}

// ============================================================================
// GET /api/users/me/2fa/recovery-codes — unauthenticated
// ============================================================================

#[tokio::test]
async fn test_get_recovery_code_count_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/users/me/2fa/recovery-codes")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/users/me/2fa/recovery-codes without auth must return 401"
    );
}

// ============================================================================
// POST /api/users/me/2fa/setup — unauthenticated
// ============================================================================

#[tokio::test]
async fn test_setup_2fa_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/users/me/2fa/setup")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/users/me/2fa/setup without auth must return 401"
    );
}

// ============================================================================
// POST /api/users/me/2fa/enable — unauthenticated
// ============================================================================

#[tokio::test]
async fn test_enable_2fa_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/users/me/2fa/enable")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"code":"123456"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/users/me/2fa/enable without auth must return 401"
    );
}

// ============================================================================
// POST /api/users/me/2fa/disable — unauthenticated
// ============================================================================

#[tokio::test]
async fn test_disable_2fa_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/users/me/2fa/disable")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/users/me/2fa/disable without auth must return 401"
    );
}
