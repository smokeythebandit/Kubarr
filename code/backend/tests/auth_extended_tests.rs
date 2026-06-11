//! Extended auth endpoint integration tests
//!
//! The base auth flow (login, logout, sessions, revoke) is already covered by
//! `auth_flow_tests.rs`.  This file exercises the remaining endpoints that are
//! registered by `auth::auth_routes()`:
//!
//! - `POST /auth/switch/{slot}`  — switch active session slot
//! - `GET  /auth/accounts`       — list all signed-in accounts
//! - `POST /auth/2fa/recover`    — login via 2FA recovery code
//!
//! It also adds supplementary edge-case tests for the login endpoint that are
//! not present in `auth_flow_tests.rs`:
//! - Login with email instead of username
//! - Login returns all expected fields (`user_id`, `username`, `email`,
//!   `session_slot`)
//! - Login sets the indexed `kubarr_session_0=` cookie in addition to the
//!   legacy `kubarr_session=` cookie
//! - Logout clears the session cookie

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sea_orm::{ActiveModelTrait, Set};
use tower::util::ServiceExt;

mod common;
use common::{
    build_test_app_state_with_db, create_test_db_with_seed, create_test_user,
    create_test_user_with_role,
};

use kubarr::endpoints::create_router;

// ============================================================================
// JWT key initialization (process-global, initialised once)
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

/// POST /auth/login and return the full response (status + all Set-Cookie headers).
async fn do_login_full(
    app: axum::Router,
    username: &str,
    password: &str,
) -> axum::http::Response<axum::body::Body> {
    let body = serde_json::json!({
        "username": username,
        "password": password
    })
    .to_string();

    app.oneshot(
        Request::builder()
            .uri("/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
    .unwrap()
}

/// POST /auth/login and return (status, legacy session cookie string).
async fn do_login(
    app: axum::Router,
    username: &str,
    password: &str,
) -> (StatusCode, Option<String>) {
    let response = do_login_full(app, username, password).await;
    let status = response.status();

    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            // The legacy cookie is `kubarr_session=` (no trailing `_`).
            if s.starts_with("kubarr_session=") && !s.contains("kubarr_session_") {
                Some(s.split(';').next().unwrap().to_string())
            } else {
                None
            }
        });

    (status, cookie)
}

/// POST /auth/login and return (status, all session cookies joined as one Cookie header value).
/// This captures kubarr_session=, kubarr_session_0=, kubarr_session_active= etc.
/// Needed for endpoints that read indexed slot cookies (list_accounts, switch_session).
async fn do_login_all_cookies(
    app: axum::Router,
    username: &str,
    password: &str,
) -> (StatusCode, Option<String>) {
    let response = do_login_full(app, username, password).await;
    let status = response.status();

    let cookies: Vec<String> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| {
            let s = v.to_str().ok()?;
            // Only include kubarr_session* cookies, skip Max-Age=0 (logout) cookies
            if s.starts_with("kubarr_session") && !s.contains("Max-Age=0") {
                Some(s.split(';').next().unwrap().to_string())
            } else {
                None
            }
        })
        .collect();

    if cookies.is_empty() {
        (status, None)
    } else {
        (status, Some(cookies.join("; ")))
    }
}

/// Make an authenticated GET request and return (status, body string).
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
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// Make an authenticated POST request and return (status, body string).
async fn authenticated_post(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("POST")
                .header("Cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(json_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

// ============================================================================
// Additional login edge-case tests
// ============================================================================

#[tokio::test]
async fn test_login_with_email_returns_200() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "emailloginuser",
        "emaillogin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    // Login using email address instead of username — auth.rs supports both
    let (status, cookie) = do_login(
        create_router(state),
        "emaillogin@example.com",
        "password123",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Login with email address must return 200"
    );
    assert!(
        cookie.is_some(),
        "Login with email must set a session cookie"
    );
}

#[tokio::test]
async fn test_login_response_body_fields() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "fieldsuser",
        "fields@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let response = do_login_full(create_router(state), "fieldsuser", "password123").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("Login response must be valid JSON");

    assert_eq!(
        json["username"], "fieldsuser",
        "Login response must include username"
    );
    assert_eq!(
        json["email"], "fields@example.com",
        "Login response must include email"
    );
    assert!(
        json.get("user_id").is_some(),
        "Login response must include user_id"
    );
    assert!(
        json.get("session_slot").is_some(),
        "Login response must include session_slot"
    );
    // session_slot must be a non-negative integer
    assert!(
        json["session_slot"].as_u64().is_some(),
        "session_slot must be a non-negative integer"
    );
}

#[tokio::test]
async fn test_login_sets_indexed_session_cookie() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "cookieuser",
        "cookie@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let response = do_login_full(create_router(state), "cookieuser", "password123").await;
    assert_eq!(response.status(), StatusCode::OK);

    // The login handler sets three Set-Cookie headers:
    //   1. kubarr_session_<slot>=<token>   (indexed)
    //   2. kubarr_active_session=<slot>     (active slot indicator)
    //   3. kubarr_session=<token>           (legacy)
    let all_cookies: Vec<String> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect();

    // An indexed session cookie looks like `kubarr_session_0=...` (digit after underscore).
    let has_indexed = all_cookies.iter().any(|c| {
        c.starts_with("kubarr_session_")
            && c.chars()
                .nth("kubarr_session_".len())
                .map_or(false, |ch| ch.is_ascii_digit())
    });
    assert!(
        has_indexed,
        "Login must set an indexed session cookie (kubarr_session_N=). Got: {:?}",
        all_cookies
    );

    let has_legacy = all_cookies
        .iter()
        .any(|c| c.starts_with("kubarr_session=") && !c.contains("kubarr_session_"));
    assert!(
        has_legacy,
        "Login must set a legacy kubarr_session= cookie for backwards compatibility. Got: {:?}",
        all_cookies
    );
}

#[tokio::test]
async fn test_login_wrong_password_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "wrongpw_user",
        "wrongpw@example.com",
        "correctpassword",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, _) = do_login(create_router(state), "wrongpw_user", "wrongpassword").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Login with wrong password must return 401"
    );
}

#[tokio::test]
async fn test_login_nonexistent_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    let (status, _) = do_login(create_router(state), "ghost_user_xyz", "anything").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Login with non-existent username must return 401"
    );
}

#[tokio::test]
async fn test_login_inactive_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    {
        use kubarr::models::user;
        use kubarr::services::security::hash_password;

        let hashed = hash_password("password123").unwrap();
        let now = chrono::Utc::now();
        let inactive = user::ActiveModel {
            username: Set("ext_inactive".to_string()),
            email: Set("ext_inactive@example.com".to_string()),
            hashed_password: Set(hashed),
            is_active: Set(false),
            is_approved: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        inactive.insert(&db).await.unwrap();
    }

    let state = build_test_app_state_with_db(db).await;
    let (status, _) = do_login(create_router(state), "ext_inactive", "password123").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Login with an inactive account must return 401"
    );
}

#[tokio::test]
async fn test_login_unapproved_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user(
        &db,
        "ext_pending",
        "ext_pending@example.com",
        "password123",
        false,
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, _) = do_login(create_router(state), "ext_pending", "password123").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Login with an unapproved (pending) account must return 401"
    );
}

// ============================================================================
// POST /auth/logout — supplementary coverage
// ============================================================================

#[tokio::test]
async fn test_logout_clears_session_cookie() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "ext_logout",
        "ext_logout@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "ext_logout", "password123").await;
    let cookie = cookie.unwrap();

    let logout_response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/logout")
                .method("POST")
                .header("Cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        logout_response.status(),
        StatusCode::OK,
        "Logout must return 200"
    );

    // The Set-Cookie header must clear the session cookie (Max-Age=0)
    let set_cookie_headers: Vec<String> = logout_response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect();

    let has_clearing_cookie = set_cookie_headers
        .iter()
        .any(|c| c.contains("kubarr_session=") && c.contains("Max-Age=0"));
    assert!(
        has_clearing_cookie,
        "Logout must set a clearing cookie (Max-Age=0). Got: {:?}",
        set_cookie_headers
    );
}

#[tokio::test]
async fn test_logout_response_body() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/logout")
                .method("POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("Logout response must be valid JSON");

    assert!(
        json.get("message").is_some(),
        "Logout response must include a 'message' field"
    );
}

// ============================================================================
// GET /auth/accounts
// ============================================================================

#[tokio::test]
async fn test_list_accounts_without_session_returns_empty_array() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    // No cookie supplied — accounts handler inspects cookies but does NOT
    // require auth middleware.  With no session cookies it returns an empty list.
    let (status, body) = authenticated_get(create_router(state), "/auth/accounts", "").await;

    // The endpoint is public (not behind require_auth); it returns [] when there
    // are no recognisable session cookies.
    assert_eq!(
        status,
        StatusCode::OK,
        "GET /auth/accounts without cookies must return 200 with empty list. Body: {}",
        body
    );

    let json: serde_json::Value =
        serde_json::from_str(&body).expect("accounts response must be valid JSON");
    assert!(json.is_array(), "accounts response must be a JSON array");
    assert!(
        json.as_array().unwrap().is_empty(),
        "accounts list must be empty when no session cookies are present"
    );
}

#[tokio::test]
async fn test_list_accounts_with_session_returns_current_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "accountsuser",
        "accounts@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) =
        do_login_all_cookies(create_router(state.clone()), "accountsuser", "password123").await;
    let cookie = cookie.unwrap();

    let (status, body) = authenticated_get(create_router(state), "/auth/accounts", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /auth/accounts with a valid session must return 200. Body: {}",
        body
    );

    let json: serde_json::Value =
        serde_json::from_str(&body).expect("accounts response must be valid JSON");
    assert!(json.is_array(), "accounts response must be a JSON array");

    let accounts = json.as_array().unwrap();
    assert!(
        !accounts.is_empty(),
        "Accounts list must contain the logged-in user"
    );

    let account = &accounts[0];
    assert_eq!(
        account["username"], "accountsuser",
        "Account entry must match the logged-in user"
    );
    assert!(
        account.get("slot").is_some(),
        "Account entry must include a slot field"
    );
    assert!(
        account.get("user_id").is_some(),
        "Account entry must include a user_id field"
    );
    assert!(
        account.get("email").is_some(),
        "Account entry must include an email field"
    );
    assert!(
        account.get("is_active").is_some(),
        "Account entry must include an is_active field"
    );
}

#[tokio::test]
async fn test_list_accounts_shows_slot_number() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "slotcheckuser",
        "slotcheck@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) =
        do_login_all_cookies(create_router(state.clone()), "slotcheckuser", "password123").await;
    let cookie = cookie.unwrap();

    let (status, body) = authenticated_get(create_router(state), "/auth/accounts", &cookie).await;
    assert_eq!(status, StatusCode::OK);

    let accounts: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("accounts response must be a JSON array");

    // The first login always uses slot 0 (first available slot)
    let slot = accounts[0]["slot"].as_u64().expect("slot must be a number");
    assert_eq!(slot, 0, "First login must be assigned to slot 0");
}

// ============================================================================
// POST /auth/switch/{slot}
// ============================================================================

#[tokio::test]
async fn test_switch_to_invalid_slot_returns_error() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "switchuser",
        "switch@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "switchuser", "password123").await;
    let cookie = cookie.unwrap();

    // Slot 99 does not exist — should return 400 or 404
    let (status, _body) =
        authenticated_post(create_router(state), "/auth/switch/99", &cookie, "").await;

    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
        "Switching to a nonexistent slot must return 400 or 404, got {}",
        status
    );
}

#[tokio::test]
async fn test_switch_to_empty_slot_returns_not_found() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "switchuser2",
        "switch2@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "switchuser2", "password123").await;
    let cookie = cookie.unwrap();

    // Slot 1 has no session (only slot 0 was used for login)
    let (status, _body) =
        authenticated_post(create_router(state), "/auth/switch/1", &cookie, "").await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Switching to an empty slot must return 404"
    );
}

#[tokio::test]
async fn test_switch_to_current_slot_returns_200() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "switchcurrent",
        "switchcurrent@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) =
        do_login_all_cookies(create_router(state.clone()), "switchcurrent", "password123").await;
    let cookie = cookie.unwrap();

    // Slot 0 is the currently active slot — switching to it is idempotent and must succeed
    let (status, body) =
        authenticated_post(create_router(state), "/auth/switch/0", &cookie, "").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Switching to the current active slot must return 200. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_switch_response_body_contains_slot() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "switchbody",
        "switchbody@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) =
        do_login_all_cookies(create_router(state.clone()), "switchbody", "password123").await;
    let cookie = cookie.unwrap();

    let (status, body) =
        authenticated_post(create_router(state), "/auth/switch/0", &cookie, "").await;

    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value =
        serde_json::from_str(&body).expect("switch response must be valid JSON");
    assert!(
        json.get("slot").is_some(),
        "Switch response must include 'slot' field"
    );
    assert!(
        json.get("message").is_some(),
        "Switch response must include 'message' field"
    );
}

#[tokio::test]
async fn test_switch_without_session_cookies_returns_not_found() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    // No session cookies at all — get_existing_sessions returns [] so slot is missing
    let (status, _body) = authenticated_post(create_router(state), "/auth/switch/0", "", "").await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Switching without any session cookies must return 404"
    );
}

// ============================================================================
// POST /auth/2fa/recover
// ============================================================================

#[tokio::test]
async fn test_recovery_without_2fa_enabled_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "no2fauser",
        "no2fa@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "username": "no2fauser",
        "password": "password123",
        "recovery_code": "AABB-CCDD-EEFF-0011"
    })
    .to_string();

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/2fa/recover")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    // User does not have 2FA enabled → the handler returns 400
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Recovery for a user without 2FA enabled must return 400"
    );
}

#[tokio::test]
async fn test_recovery_with_wrong_password_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "rec_wrongpw",
        "rec_wrongpw@example.com",
        "correctpassword",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "username": "rec_wrongpw",
        "password": "wrongpassword",
        "recovery_code": "AABB-CCDD-EEFF-0011"
    })
    .to_string();

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/2fa/recover")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Recovery with wrong password must return 401"
    );
}

#[tokio::test]
async fn test_recovery_with_nonexistent_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "username": "ghost_user_for_recovery",
        "password": "somepassword",
        "recovery_code": "AABB-CCDD-EEFF-0011"
    })
    .to_string();

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/2fa/recover")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Recovery for a non-existent user must return 401"
    );
}

#[tokio::test]
async fn test_recovery_with_inactive_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    {
        use kubarr::models::user;
        use kubarr::services::security::hash_password;

        let hashed = hash_password("password123").unwrap();
        let now = chrono::Utc::now();
        let inactive = user::ActiveModel {
            username: Set("rec_inactive".to_string()),
            email: Set("rec_inactive@example.com".to_string()),
            hashed_password: Set(hashed),
            is_active: Set(false),
            is_approved: Set(true),
            totp_enabled: Set(false),
            totp_secret: Set(None),
            totp_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        inactive.insert(&db).await.unwrap();
    }

    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "username": "rec_inactive",
        "password": "password123",
        "recovery_code": "AABB-CCDD-EEFF-0011"
    })
    .to_string();

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/2fa/recover")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Recovery for an inactive account must return 401"
    );
}

#[tokio::test]
async fn test_recovery_missing_fields_returns_422() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    // Send an incomplete body (missing recovery_code)
    let body = serde_json::json!({
        "username": "someuser",
        "password": "somepassword"
    })
    .to_string();

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/2fa/recover")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    // Axum's JSON extractor returns 422 Unprocessable Entity for missing fields
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Recovery with a missing required field must return 422"
    );
}

// ============================================================================
// Session cookie auth edge cases
// ============================================================================

#[tokio::test]
async fn test_sessions_endpoint_with_invalid_token_returns_401() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    // Craft a syntactically valid cookie value that contains a bogus JWT
    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/sessions")
                .method("GET")
                .header("Cookie", "kubarr_session=not.a.valid.jwt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "A request with a malformed JWT cookie must return 401"
    );
}

#[tokio::test]
async fn test_sessions_after_logout_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "postlogoutuser",
        "postlogout@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    // Step 1: login
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "postlogoutuser",
        "password123",
    )
    .await;
    let cookie = cookie.unwrap();

    // Step 2: logout (revokes the session in the DB)
    let _logout = create_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/auth/logout")
                .method("POST")
                .header("Cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Step 3: try to list sessions using the old (now-revoked) token
    // The session was deleted from the DB, so this must return 401
    let (status, _) = authenticated_get(create_router(state), "/auth/sessions", &cookie).await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Using a revoked session token after logout must return 401"
    );
}

// ============================================================================
// Unauthenticated access to auth-internal routes
// ============================================================================

#[tokio::test]
async fn test_accounts_without_cookies_returns_empty_list() {
    // /auth/accounts is NOT behind the auth middleware — it reads cookies
    // directly and returns whatever sessions it can decode.
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/accounts")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "GET /auth/accounts without cookies must return 200"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("accounts response must be valid JSON");
    assert!(json.is_array(), "accounts response must be a JSON array");
    assert!(
        json.as_array().unwrap().is_empty(),
        "accounts list must be empty with no session cookies"
    );
}

#[tokio::test]
async fn test_sessions_without_auth_returns_401() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/sessions")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /auth/sessions without auth must return 401"
    );
}
