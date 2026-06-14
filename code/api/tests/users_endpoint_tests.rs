//! Users endpoint integration tests
//!
//! Covers:
//! - `GET /api/users` — list users (requires users.view)
//! - `GET /api/users/me` — current user info (any authenticated user)
//! - `POST /api/users` — create user (requires users.manage)
//! - `GET /api/users/{id}` — get user by ID (requires users.view)
//! - `GET /api/users/pending` — list pending users (requires users.view)
//! - `GET /api/users/me/2fa/status` — 2FA status (any authenticated user)
//! - `GET /api/users/invites` — list invites (requires users.manage)
//! - `POST /api/users/invites` — create invite (requires users.manage)
//! - `GET /api/users/me/preferences` — get preferences (any authenticated user)

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;

// ============================================================================
// JWT key initialization
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

/// POST /auth/login and return (status, Set-Cookie header value).
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

/// Make an authenticated GET request and return (status, body).
async fn authenticated_get(app: axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
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

/// Make an authenticated POST request and return (status, body).
async fn authenticated_post(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("POST")
        .header("Cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(json_body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

// ============================================================================
// GET /api/users — requires auth
// ============================================================================

#[tokio::test]
async fn test_list_users_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/users")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/users without a session cookie must return 401"
    );
}

#[tokio::test]
async fn test_list_users_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "adminuser",
        "admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "adminuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(create_router(state), "/api/users", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Admin must be able to list users. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_array(),
        "Response must be a JSON array. Body: {}",
        body
    );

    let users = json.as_array().unwrap();
    assert!(
        !users.is_empty(),
        "Users list must not be empty (at least the admin user exists)"
    );
}

// ============================================================================
// GET /api/users/me
// ============================================================================

#[tokio::test]
async fn test_get_current_user_info() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "meuser", "meuser@example.com", "mypassword", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "meuser", "mypassword").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(create_router(state), "/api/users/me", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/users/me must return 200 for authenticated user. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["username"], "meuser",
        "Response must include the caller's username"
    );
    assert_eq!(
        json["email"], "meuser@example.com",
        "Response must include the caller's email"
    );
    assert!(json.get("id").is_some(), "Response must include id field");
    assert!(
        json.get("roles").is_some(),
        "Response must include roles field"
    );
}

// ============================================================================
// POST /api/users — create user
// ============================================================================

#[tokio::test]
async fn test_create_user_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "createadmin",
        "createadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "createadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let new_user_body = serde_json::json!({
        "username": "newlyCreated",
        "email": "newlycreated@example.com",
        "password": "securepassword"
    })
    .to_string();

    let (status, body) =
        authenticated_post(create_router(state), "/api/users", &cookie, &new_user_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Admin creating a user must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["username"], "newlyCreated",
        "Created user must have the requested username"
    );
    assert_eq!(
        json["email"], "newlycreated@example.com",
        "Created user must have the requested email"
    );
    assert!(
        json.get("id").is_some(),
        "Created user response must include an id"
    );
}

#[tokio::test]
async fn test_create_user_duplicate_email() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "dupladmin",
        "dupladmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "dupladmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let first_body = serde_json::json!({
        "username": "uniqueuser1",
        "email": "duplicate@example.com",
        "password": "securepassword"
    })
    .to_string();

    // First creation should succeed
    let (status1, _) = authenticated_post(
        create_router(state.clone()),
        "/api/users",
        &cookie,
        &first_body,
    )
    .await;
    assert_eq!(status1, StatusCode::OK, "First user creation must succeed");

    let second_body = serde_json::json!({
        "username": "uniqueuser2",
        "email": "duplicate@example.com",
        "password": "securepassword"
    })
    .to_string();

    // Second creation with same email should fail
    let (status2, body2) =
        authenticated_post(create_router(state), "/api/users", &cookie, &second_body).await;

    assert!(
        status2 == StatusCode::BAD_REQUEST || status2 == StatusCode::CONFLICT,
        "Duplicate email must return 400 or 409. Got: {} Body: {}",
        status2,
        body2
    );
}

// ============================================================================
// GET /api/users/{id}
// ============================================================================

#[tokio::test]
async fn test_get_user_by_id_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let target_user = create_test_user_with_role(
        &db,
        "targetuser",
        "target@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "targetuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let uri = format!("/api/users/{}", target_user.id);
    let (status, body) = authenticated_get(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/users/{{id}} must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["id"], target_user.id,
        "Returned user must match requested ID"
    );
    assert_eq!(
        json["username"], "targetuser",
        "Returned user must have the correct username"
    );
}

// ============================================================================
// GET /api/users/pending
// ============================================================================

#[tokio::test]
async fn test_list_pending_users() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Create an unapproved user directly
    {
        use kubarr::models::user;
        use kubarr::services::security::hash_password;
        use sea_orm::{ActiveModelTrait, Set};

        let hashed = hash_password("password").unwrap();
        let now = chrono::Utc::now();
        let pending = user::ActiveModel {
            username: Set("pendinguser".to_string()),
            email: Set("pending@example.com".to_string()),
            hashed_password: Set(hashed),
            is_active: Set(true),
            is_approved: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        pending.insert(&db).await.unwrap();
    }

    create_test_user_with_role(
        &db,
        "pendingadmin",
        "pendingadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "pendingadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/users/pending", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/users/pending must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_array(),
        "Pending users response must be a JSON array"
    );

    let pending_list = json.as_array().unwrap();
    assert!(
        !pending_list.is_empty(),
        "Pending users list must contain the unapproved user"
    );

    // Verify all returned users are unapproved
    for user in pending_list {
        assert_eq!(
            user["is_approved"], false,
            "All pending users must have is_approved = false"
        );
    }
}

// ============================================================================
// GET /api/users/me/2fa/status
// ============================================================================

#[tokio::test]
async fn test_2fa_status() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "twofauser",
        "twofa@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "twofauser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/users/me/2fa/status", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/users/me/2fa/status must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("enabled").is_some(),
        "2FA status must include enabled field"
    );
    assert_eq!(
        json["enabled"], false,
        "Newly created user must have 2FA disabled"
    );
    assert!(
        json.get("required_by_role").is_some(),
        "2FA status must include required_by_role field"
    );
}

// ============================================================================
// GET /api/users/invites
// ============================================================================

#[tokio::test]
async fn test_list_invites() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "inviteadmin",
        "inviteadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "inviteadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/users/invites", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/users/invites must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_array(),
        "Invites response must be a JSON array. Body: {}",
        body
    );

    // Fresh DB should have no invites
    let invites = json.as_array().unwrap();
    assert!(
        invites.is_empty(),
        "Invites list must be empty for a fresh database"
    );
}

// ============================================================================
// POST /api/users/invites
// ============================================================================

#[tokio::test]
async fn test_create_invite() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "createinvite",
        "createinvite@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "createinvite", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let invite_body = serde_json::json!({
        "expires_in_days": 7
    })
    .to_string();

    let (status, body) = authenticated_post(
        create_router(state),
        "/api/users/invites",
        &cookie,
        &invite_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/users/invites must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.get("id").is_some(), "Created invite must have an id");
    assert!(
        json.get("code").is_some(),
        "Created invite must have a code"
    );
    assert_eq!(
        json["is_used"], false,
        "Newly created invite must not be used"
    );
    assert_eq!(
        json["created_by_username"], "createinvite",
        "Invite must record the creating user's username"
    );
}

// ============================================================================
// GET /api/users/me/preferences
// ============================================================================

#[tokio::test]
async fn test_get_my_preferences() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "prefsuser",
        "prefs@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "prefsuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/users/me/preferences", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/users/me/preferences must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("theme").is_some(),
        "Preferences response must include a theme field"
    );
    // Default theme should be "system"
    assert_eq!(
        json["theme"], "system",
        "Default theme for a new user must be 'system'"
    );
}
