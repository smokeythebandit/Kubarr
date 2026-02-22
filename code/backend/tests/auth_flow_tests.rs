//! Auth flow integration tests
//!
//! Covers:
//! - `POST /auth/login` — valid credentials, invalid credentials, inactive/unapproved accounts
//! - `POST /auth/logout` — invalidates current session
//! - `GET /auth/sessions` — lists active sessions (requires auth)
//! - `DELETE /auth/sessions/:id` — revokes a specific session (requires auth)
//! - Permission enforcement — viewer vs admin access to permission-gated endpoints

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
use kubarr::state::AppState;

// ============================================================================
// JWT key initialization
// ============================================================================

/// Initialise JWT keys into the global in-memory cache exactly once per test binary.
///
/// `create_session_token` (called inside the login handler) needs RSA keys in the
/// process-global `PRIVATE_KEY` / `PUBLIC_KEY` statics. `init_jwt_keys` generates a
/// fresh key-pair, persists it to the supplied database, and caches it in those statics.
///
/// Using `tokio::sync::OnceCell` ensures the keys are generated at most once even when
/// multiple tests run concurrently, preventing a race where one test's keys overwrite
/// another test's in-flight tokens.
static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_jwt_keys() {
    JWT_INIT
        .get_or_init(|| async {
            // Use a throw-away database just for key generation.
            // The keys are stored in process-global statics and outlive the DB.
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

/// POST /auth/login and return (status, Set-Cookie header value for legacy cookie).
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

    let request = Request::builder()
        .uri("/auth/login")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();

    // Extract the legacy `kubarr_session=<token>` cookie from Set-Cookie headers.
    // The middleware's `extract_token` fallback accepts this format.
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            // Accept the legacy cookie (`kubarr_session=`) but not indexed ones
            // (`kubarr_session_0=`).
            if s.starts_with("kubarr_session=") && !s.contains("kubarr_session_") {
                // Trim attributes (HttpOnly, Path, ...) — keep only `name=value`
                Some(s.split(';').next().unwrap().to_string())
            } else {
                None
            }
        });

    (status, cookie)
}

/// Make a GET request with a session cookie and return (status, body).
async fn authenticated_get(state: AppState, uri: &str, cookie: &str) -> (StatusCode, String) {
    let app = create_router(state);

    let request = Request::builder()
        .uri(uri)
        .method("GET")
        .header("Cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

// ============================================================================
// POST /auth/login
// ============================================================================

#[tokio::test]
async fn test_login_valid_credentials_returns_200() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "testuser",
        "test@example.com",
        "correctpassword",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let app = create_router(state);
    let (status, cookie) = do_login(app, "testuser", "correctpassword").await;

    assert_eq!(status, StatusCode::OK, "Valid login must return 200");
    assert!(cookie.is_some(), "Login must set a session cookie");
}

#[tokio::test]
async fn test_login_valid_credentials_returns_user_info() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "myuser", "myuser@example.com", "mypassword", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "username": "myuser",
        "password": "mypassword"
    })
    .to_string();

    let response = create_router(state)
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

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(json["username"], "myuser");
    assert_eq!(json["email"], "myuser@example.com");
    assert!(
        json.get("user_id").is_some(),
        "Response must include user_id"
    );
    assert!(
        json.get("session_slot").is_some(),
        "Response must include session_slot"
    );
}

#[tokio::test]
async fn test_login_invalid_password_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "user2", "user2@example.com", "correct", "viewer").await;
    let state = build_test_app_state_with_db(db).await;

    let app = create_router(state);
    let (status, _) = do_login(app, "user2", "wrongpassword").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Wrong password must return 401"
    );
}

#[tokio::test]
async fn test_login_unknown_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;

    let app = create_router(state);
    let (status, _) = do_login(app, "nonexistent", "anypassword").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Unknown user must return 401"
    );
}

#[tokio::test]
async fn test_login_inactive_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Create an inactive user directly (is_active = false)
    {
        use kubarr::models::user;
        use kubarr::services::security::hash_password;

        let hashed = hash_password("password123").unwrap();
        let now = chrono::Utc::now();
        let inactive_user = user::ActiveModel {
            username: Set("inactive_user".to_string()),
            email: Set("inactive@example.com".to_string()),
            hashed_password: Set(hashed),
            is_active: Set(false), // <-- disabled
            is_approved: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        inactive_user.insert(&db).await.unwrap();
    }

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);
    let (status, _) = do_login(app, "inactive_user", "password123").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Inactive account must return 401"
    );
}

#[tokio::test]
async fn test_login_unapproved_user_returns_401() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // create_test_user with is_approved = false
    create_test_user(
        &db,
        "pending_user",
        "pending@example.com",
        "password123",
        false,
    )
    .await;

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);
    let (status, _) = do_login(app, "pending_user", "password123").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "Unapproved account must return 401"
    );
}

// ============================================================================
// POST /auth/logout
// ============================================================================

#[tokio::test]
async fn test_logout_without_session_returns_200() {
    // Logout is graceful — it clears the cookie even without a valid session
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

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Logout must return 200 even without a session"
    );
}

#[tokio::test]
async fn test_logout_with_session_returns_200() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "logoutuser", "logout@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    // Login first
    let (login_status, cookie) =
        do_login(create_router(state.clone()), "logoutuser", "password").await;
    assert_eq!(login_status, StatusCode::OK);
    let cookie = cookie.unwrap();

    // Then logout
    let response = create_router(state)
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

    assert_eq!(response.status(), StatusCode::OK);
}

// ============================================================================
// GET /auth/sessions
// ============================================================================

#[tokio::test]
async fn test_list_sessions_without_auth_returns_401() {
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

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_list_sessions_with_valid_session_returns_200() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "sessuser", "sess@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    // Login to get a session cookie
    let (_, cookie) = do_login(create_router(state.clone()), "sessuser", "password").await;
    let cookie = cookie.unwrap();

    // Use session to list sessions
    let (status, body) = authenticated_get(state, "/auth/sessions", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Listing sessions with valid auth must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array(), "Sessions response must be a JSON array");

    // There should be at least one session (the one we just created)
    let sessions = json.as_array().unwrap();
    assert!(
        !sessions.is_empty(),
        "Sessions list must not be empty after login"
    );
}

#[tokio::test]
async fn test_list_sessions_shows_current_session() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "currsess", "currsess@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "currsess", "password").await;
    let cookie = cookie.unwrap();

    let (status, body) = authenticated_get(state, "/auth/sessions", &cookie).await;
    assert_eq!(status, StatusCode::OK);

    let sessions: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    let current = sessions.iter().find(|s| s["is_current"] == true);
    assert!(
        current.is_some(),
        "Session list must contain the current session (is_current = true)"
    );
}

// ============================================================================
// DELETE /auth/sessions/:id
// ============================================================================

#[tokio::test]
async fn test_revoke_current_session_returns_bad_request() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "revokeuser", "revoke@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    // Login to get current session
    let (_, cookie) = do_login(create_router(state.clone()), "revokeuser", "password").await;
    let cookie = cookie.unwrap();

    // Get current session ID from the sessions list
    let (_, sessions_body) = authenticated_get(state.clone(), "/auth/sessions", &cookie).await;
    let sessions: Vec<serde_json::Value> = serde_json::from_str(&sessions_body).unwrap();
    let current_id = sessions
        .iter()
        .find(|s| s["is_current"] == true)
        .and_then(|s| s["id"].as_str())
        .expect("Current session must appear in the list");

    // Attempt to revoke the current session — should be rejected
    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri(&format!("/auth/sessions/{}", current_id))
                .method("DELETE")
                .header("Cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Revoking the current session must return 400 (use logout instead)"
    );
}

#[tokio::test]
async fn test_revoke_nonexistent_session_returns_not_found() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "revokeuser2",
        "revoke2@example.com",
        "password",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "revokeuser2", "password").await;
    let cookie = cookie.unwrap();

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/auth/sessions/00000000-0000-0000-0000-000000000000")
                .method("DELETE")
                .header("Cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Revoking a nonexistent session must return 404"
    );
}

// ============================================================================
// Permission enforcement (viewer vs admin)
// ============================================================================

#[tokio::test]
async fn test_viewer_cannot_access_settings() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vieweruser",
        "viewer@example.com",
        "password",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vieweruser", "password").await;
    let cookie = cookie.unwrap();

    // Viewer lacks settings.view — should be 403
    let (status, _) = authenticated_get(state, "/api/settings", &cookie).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer must be denied access to /api/settings (lacks settings.view)"
    );
}

#[tokio::test]
async fn test_viewer_cannot_manage_users() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vieweruser2",
        "viewer2@example.com",
        "password",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vieweruser2", "password").await;
    let cookie = cookie.unwrap();

    // Viewer lacks users.view — should be 403
    let (status, _) = authenticated_get(state, "/api/users", &cookie).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer must be denied access to /api/users (lacks users.view)"
    );
}

#[tokio::test]
async fn test_admin_can_access_settings() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "adminuser", "admin@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "adminuser", "password").await;
    let cookie = cookie.unwrap();

    // Admin has settings.view — should NOT be 401 or 403
    let (status, _) = authenticated_get(state, "/api/settings", &cookie).await;
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "Admin must not get 401 on /api/settings"
    );
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "Admin must not get 403 on /api/settings"
    );
}

// ============================================================================
// DELETE /auth/sessions/{id} — success and cross-user forbidden paths
// ============================================================================

/// Revoke one of the current user's *other* sessions successfully (auth.rs lines 476-480).
#[tokio::test]
async fn test_revoke_own_non_current_session_succeeds() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "revoke_own_user",
        "revoke_own@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    // First login — session A
    let (_, cookie_a) = do_login(
        create_router(state.clone()),
        "revoke_own_user",
        "password123",
    )
    .await;
    let cookie_a = cookie_a.expect("First login must succeed");

    // Second login — session B (used as the "current" session for DELETE)
    let (_, cookie_b) = do_login(
        create_router(state.clone()),
        "revoke_own_user",
        "password123",
    )
    .await;
    let cookie_b = cookie_b.expect("Second login must succeed");

    // Get all sessions via session B and find session A's ID
    let (_, sessions_body) = authenticated_get(state.clone(), "/auth/sessions", &cookie_b).await;
    let sessions: Vec<serde_json::Value> = serde_json::from_str(&sessions_body).unwrap();

    // Find a session that is NOT the current one (is_current == false)
    let other_id = sessions
        .iter()
        .find(|s| s["is_current"] == false)
        .and_then(|s| s["id"].as_str())
        .expect("Must have at least one non-current session");

    // Delete the other session using session B
    let response = create_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(&format!("/auth/sessions/{}", other_id))
                .method("DELETE")
                .header("Cookie", &cookie_b)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Revoking own non-current session must return 200"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&body)).unwrap();
    assert!(
        json.get("message").is_some(),
        "Revoke response must include a message"
    );

    // Verify the first cookie no longer works
    let (verify_status, _) = authenticated_get(state.clone(), "/auth/sessions", &cookie_a).await;
    assert_eq!(
        verify_status,
        StatusCode::UNAUTHORIZED,
        "Revoked session cookie must no longer authenticate"
    );
}

/// Revoke another user's session is forbidden (auth.rs lines 463-464).
#[tokio::test]
async fn test_revoke_another_users_session_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "attacker_user",
        "attacker@example.com",
        "password123",
        "viewer",
    )
    .await;
    create_test_user_with_role(
        &db,
        "victim_user",
        "victim@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    // Attacker logs in
    let (_, attacker_cookie) =
        do_login(create_router(state.clone()), "attacker_user", "password123").await;
    let attacker_cookie = attacker_cookie.expect("Attacker login must succeed");

    // Victim logs in
    let (_, victim_cookie) =
        do_login(create_router(state.clone()), "victim_user", "password123").await;
    let victim_cookie = victim_cookie.expect("Victim login must succeed");

    // Victim: list own sessions to get session ID
    let (_, victim_sessions_body) =
        authenticated_get(state.clone(), "/auth/sessions", &victim_cookie).await;
    let victim_sessions: Vec<serde_json::Value> =
        serde_json::from_str(&victim_sessions_body).unwrap();
    let victim_session_id = victim_sessions
        .first()
        .and_then(|s| s["id"].as_str())
        .expect("Victim must have at least one session");

    // Attacker tries to revoke victim's session
    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri(&format!("/auth/sessions/{}", victim_session_id))
                .method("DELETE")
                .header("Cookie", &attacker_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "Revoking another user's session must return 403"
    );
}
