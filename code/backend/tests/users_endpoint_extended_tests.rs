//! Extended users endpoint integration tests
//!
//! Covers endpoints NOT already tested in users_endpoint_tests.rs:
//! - `PATCH /api/users/me`           — update own profile (username, email)
//! - `DELETE /api/users/me`          — delete own account (requires password)
//! - `PATCH /api/users/me/preferences` — update preferences (theme)
//! - `POST /api/users/{id}/approve`  — approve pending user
//! - `POST /api/users/{id}/reject`   — reject/delete pending user
//! - `DELETE /api/users/{id}`        — admin delete user
//! - `PATCH /api/users/{id}`         — admin update user info
//! - `DELETE /api/users/invites/{id}` — delete invite
//! - `PATCH /api/users/me/password`  — change own password
//! - `PATCH /api/users/{id}/password` — admin reset user password
//! - `GET /api/users/me/2fa/status`  — 2FA status (extended cases)

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
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

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

async fn authenticated_patch(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("PATCH")
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

async fn authenticated_delete(app: axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("DELETE")
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

/// DELETE with a JSON body (used for delete-own-account which takes a password).
async fn authenticated_delete_with_body(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("DELETE")
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
// PATCH /api/users/me — update own profile
// ============================================================================

#[tokio::test]
async fn test_update_own_profile_username() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "patchmeuser",
        "patchme@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "patchmeuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({
        "username": "patchedusername"
    })
    .to_string();

    let (status, body) =
        authenticated_patch(create_router(state), "/api/users/me", &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PATCH /api/users/me must return 200 when updating username. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["username"], "patchedusername",
        "Username must be updated in response"
    );
}

#[tokio::test]
async fn test_update_own_profile_email() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "emailpatchuser",
        "emailpatch@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "emailpatchuser",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({
        "email": "newemail@example.com"
    })
    .to_string();

    let (status, body) =
        authenticated_patch(create_router(state), "/api/users/me", &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PATCH /api/users/me must return 200 when updating email. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["email"], "newemail@example.com",
        "Email must be updated in response"
    );
}

#[tokio::test]
async fn test_update_own_profile_username_too_short() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "shortuser1",
        "shortuser1@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "shortuser1", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({
        "username": "ab"  // too short (< 3 chars)
    })
    .to_string();

    let (status, _body) =
        authenticated_patch(create_router(state), "/api/users/me", &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Username shorter than 3 characters must return 400"
    );
}

#[tokio::test]
async fn test_update_own_profile_duplicate_username() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    // Create two users; try to rename one to the other's username
    create_test_user_with_role(
        &db,
        "user_alpha",
        "alpha@example.com",
        "password123",
        "admin",
    )
    .await;
    create_test_user_with_role(
        &db,
        "user_beta",
        "beta@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "user_alpha", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({
        "username": "user_beta"   // already taken by another user
    })
    .to_string();

    let (status, _body) =
        authenticated_patch(create_router(state), "/api/users/me", &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Updating to an existing username must return 400"
    );
}

#[tokio::test]
async fn test_update_own_profile_invalid_email() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "invalidemail1",
        "invalidemail1@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "invalidemail1", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({
        "email": "notanemail"   // no @ sign
    })
    .to_string();

    let (status, _body) =
        authenticated_patch(create_router(state), "/api/users/me", &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Invalid email format must return 400"
    );
}

// ============================================================================
// DELETE /api/users/me — delete own account
// ============================================================================

#[tokio::test]
async fn test_delete_own_account_success() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    // Need a second admin so the single-admin guard doesn't fire
    create_test_user_with_role(
        &db,
        "admin_secondary",
        "admin2@example.com",
        "password123",
        "admin",
    )
    .await;
    create_test_user_with_role(
        &db,
        "selfdelete_user",
        "selfdelete@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "selfdelete_user",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let delete_body = serde_json::json!({
        "password": "password123"
    })
    .to_string();

    let (status, body) = authenticated_delete_with_body(
        create_router(state),
        "/api/users/me",
        &cookie,
        &delete_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "DELETE /api/users/me must return 200 with correct password. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "Delete response must include a message field"
    );
}

#[tokio::test]
async fn test_delete_own_account_wrong_password() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "wrongpwuser",
        "wrongpw@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "wrongpwuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let delete_body = serde_json::json!({
        "password": "wrongpassword"
    })
    .to_string();

    let (status, _body) = authenticated_delete_with_body(
        create_router(state),
        "/api/users/me",
        &cookie,
        &delete_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "DELETE /api/users/me with wrong password must return 400"
    );
}

#[tokio::test]
async fn test_delete_own_account_last_admin_blocked() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    // Only one admin - should be blocked
    create_test_user_with_role(
        &db,
        "only_admin",
        "only_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "only_admin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let delete_body = serde_json::json!({
        "password": "password123"
    })
    .to_string();

    let (status, _body) = authenticated_delete_with_body(
        create_router(state),
        "/api/users/me",
        &cookie,
        &delete_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Last admin must not be able to delete their own account"
    );
}

// ============================================================================
// PATCH /api/users/me/preferences — update preferences
// ============================================================================

#[tokio::test]
async fn test_update_preferences_to_dark() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "darkthemeuser",
        "dark@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "darkthemeuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({ "theme": "dark" }).to_string();

    let (status, body) = authenticated_patch(
        create_router(state),
        "/api/users/me/preferences",
        &cookie,
        &patch_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PATCH /api/users/me/preferences must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["theme"], "dark", "Theme must be updated to 'dark'");
}

#[tokio::test]
async fn test_update_preferences_to_light() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "lightthemeuser",
        "light@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "lightthemeuser",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({ "theme": "light" }).to_string();

    let (status, body) = authenticated_patch(
        create_router(state),
        "/api/users/me/preferences",
        &cookie,
        &patch_body,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "Must return 200. Body: {}", body);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["theme"], "light", "Theme must be updated to 'light'");
}

#[tokio::test]
async fn test_update_preferences_invalid_theme() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "badthemeuser",
        "badtheme@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "badthemeuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({ "theme": "pink" }).to_string(); // invalid value

    let (status, _body) = authenticated_patch(
        create_router(state),
        "/api/users/me/preferences",
        &cookie,
        &patch_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Invalid theme value must return 400"
    );
}

#[tokio::test]
async fn test_update_preferences_persists_on_subsequent_get() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "persistprefs",
        "persistprefs@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "persistprefs", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Set theme to dark
    let patch_body = serde_json::json!({ "theme": "dark" }).to_string();
    authenticated_patch(
        create_router(state.clone()),
        "/api/users/me/preferences",
        &cookie,
        &patch_body,
    )
    .await;

    // Get preferences and verify they persisted
    let (status, body) =
        authenticated_get(create_router(state), "/api/users/me/preferences", &cookie).await;

    assert_eq!(status, StatusCode::OK, "GET preferences must return 200");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["theme"], "dark",
        "Theme preference must persist after update"
    );
}

// ============================================================================
// POST /api/users/{id}/approve
// ============================================================================

#[tokio::test]
async fn test_approve_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Create a pending (unapproved) user directly in the DB
    let pending_user = {
        use kubarr::models::user;
        use kubarr::services::security::hash_password;
        use sea_orm::{ActiveModelTrait, Set};

        let hashed = hash_password("userpassword").unwrap();
        let now = chrono::Utc::now();
        let pending = user::ActiveModel {
            username: Set("approve_me".to_string()),
            email: Set("approveme@example.com".to_string()),
            hashed_password: Set(hashed),
            is_active: Set(true),
            is_approved: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        pending.insert(&db).await.unwrap()
    };

    create_test_user_with_role(
        &db,
        "approveadmin",
        "approveadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "approveadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let uri = format!("/api/users/{}/approve", pending_user.id);
    let (status, body) = authenticated_post(create_router(state), &uri, &cookie, "{}").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/users/{{id}}/approve must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["is_approved"], true,
        "Approved user must have is_approved = true"
    );
    assert_eq!(
        json["is_active"], true,
        "Approved user must have is_active = true"
    );
}

#[tokio::test]
async fn test_approve_nonexistent_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "approveadmin2",
        "approveadmin2@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "approveadmin2", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _body) = authenticated_post(
        create_router(state),
        "/api/users/999999/approve",
        &cookie,
        "{}",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Approving a nonexistent user must return 404"
    );
}

// ============================================================================
// POST /api/users/{id}/reject
// ============================================================================

#[tokio::test]
async fn test_reject_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Create a pending (unapproved) user
    let pending_user = {
        use kubarr::models::user;
        use kubarr::services::security::hash_password;
        use sea_orm::{ActiveModelTrait, Set};

        let hashed = hash_password("userpassword").unwrap();
        let now = chrono::Utc::now();
        let pending = user::ActiveModel {
            username: Set("reject_me".to_string()),
            email: Set("rejectme@example.com".to_string()),
            hashed_password: Set(hashed),
            is_active: Set(true),
            is_approved: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        pending.insert(&db).await.unwrap()
    };

    create_test_user_with_role(
        &db,
        "rejectadmin",
        "rejectadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "rejectadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let uri = format!("/api/users/{}/reject", pending_user.id);
    let (status, body) = authenticated_post(create_router(state), &uri, &cookie, "{}").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/users/{{id}}/reject must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "Reject response must include a message field"
    );
}

#[tokio::test]
async fn test_reject_nonexistent_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "rejectadmin2",
        "rejectadmin2@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "rejectadmin2", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _body) = authenticated_post(
        create_router(state),
        "/api/users/999999/reject",
        &cookie,
        "{}",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Rejecting a nonexistent user must return 404"
    );
}

// ============================================================================
// DELETE /api/users/{id} — admin delete user
// ============================================================================

#[tokio::test]
async fn test_admin_delete_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "deleteadmin",
        "deleteadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let target = create_test_user_with_role(
        &db,
        "tobe_deleted",
        "tobe_deleted@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "deleteadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let uri = format!("/api/users/{}", target.id);
    let (status, body) = authenticated_delete(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "DELETE /api/users/{{id}} must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "Delete user response must include a message field"
    );
}

#[tokio::test]
async fn test_admin_delete_self_fails() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let admin = create_test_user_with_role(
        &db,
        "selfdeleteadmin",
        "selfdeleteadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "selfdeleteadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let uri = format!("/api/users/{}", admin.id);
    let (status, _body) = authenticated_delete(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "An admin must not be able to delete themselves via DELETE /api/users/{{id}}"
    );
}

#[tokio::test]
async fn test_admin_delete_nonexistent_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "deleteadmin3",
        "deleteadmin3@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "deleteadmin3", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _body) =
        authenticated_delete(create_router(state), "/api/users/999999", &cookie).await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Deleting a nonexistent user must return 404"
    );
}

// ============================================================================
// PATCH /api/users/{id} — admin update user info
// ============================================================================

#[tokio::test]
async fn test_admin_update_user_email() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "updateadmin",
        "updateadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let target = create_test_user_with_role(
        &db,
        "updatetarget",
        "updatetarget@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "updateadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({
        "email": "updated_target@example.com"
    })
    .to_string();

    let uri = format!("/api/users/{}", target.id);
    let (status, body) =
        authenticated_patch(create_router(state), &uri, &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Admin PATCH /api/users/{{id}} must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["email"], "updated_target@example.com",
        "User email must be updated"
    );
}

#[tokio::test]
async fn test_admin_update_user_is_active() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "activateadmin",
        "activateadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let target = create_test_user_with_role(
        &db,
        "activatetarget",
        "activatetarget@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "activateadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({
        "is_active": false
    })
    .to_string();

    let uri = format!("/api/users/{}", target.id);
    let (status, body) =
        authenticated_patch(create_router(state), &uri, &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Admin PATCH is_active must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["is_active"], false,
        "User is_active must be updated to false"
    );
}

#[tokio::test]
async fn test_admin_update_nonexistent_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "updateadmin2",
        "updateadmin2@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "updateadmin2", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({ "email": "x@x.com" }).to_string();

    let (status, _body) = authenticated_patch(
        create_router(state),
        "/api/users/999999",
        &cookie,
        &patch_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Updating a nonexistent user must return 404"
    );
}

// ============================================================================
// DELETE /api/users/invites/{id} — delete invite
// ============================================================================

#[tokio::test]
async fn test_delete_invite() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "delinviteadmin",
        "delinviteadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "delinviteadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First, create an invite so we have an ID to delete
    let invite_body = serde_json::json!({ "expires_in_days": 7 }).to_string();
    let (create_status, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/users/invites",
        &cookie,
        &invite_body,
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "Invite creation must succeed. Body: {}",
        create_body
    );
    let created_invite: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let invite_id = created_invite["id"].as_i64().unwrap();

    // Delete the invite
    let uri = format!("/api/users/invites/{}", invite_id);
    let (status, body) = authenticated_delete(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "DELETE /api/users/invites/{{id}} must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "Delete invite response must include a message field"
    );
}

#[tokio::test]
async fn test_delete_nonexistent_invite() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "delinviteadmin2",
        "delinviteadmin2@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "delinviteadmin2",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _body) =
        authenticated_delete(create_router(state), "/api/users/invites/999999", &cookie).await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Deleting a nonexistent invite must return 404"
    );
}

// ============================================================================
// PATCH /api/users/me/password — change own password
// ============================================================================

#[tokio::test]
async fn test_change_own_password_success() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "changepwuser",
        "changepw@example.com",
        "oldpassword",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "changepwuser", "oldpassword").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let pw_body = serde_json::json!({
        "current_password": "oldpassword",
        "new_password": "newpassword123"
    })
    .to_string();

    let (status, body) = authenticated_patch(
        create_router(state),
        "/api/users/me/password",
        &cookie,
        &pw_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PATCH /api/users/me/password must return 200 when password is correct. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "Password change response must include a message field"
    );
}

#[tokio::test]
async fn test_change_own_password_wrong_current() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "wrongcurrpw",
        "wrongcurrpw@example.com",
        "correctpassword",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "wrongcurrpw",
        "correctpassword",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let pw_body = serde_json::json!({
        "current_password": "wrongpassword",
        "new_password": "newpassword123"
    })
    .to_string();

    let (status, _body) = authenticated_patch(
        create_router(state),
        "/api/users/me/password",
        &cookie,
        &pw_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Wrong current password must return 400"
    );
}

#[tokio::test]
async fn test_change_own_password_too_short() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "shortpwuser",
        "shortpw@example.com",
        "correctpassword",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "shortpwuser",
        "correctpassword",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let pw_body = serde_json::json!({
        "current_password": "correctpassword",
        "new_password": "short"  // less than 8 chars
    })
    .to_string();

    let (status, _body) = authenticated_patch(
        create_router(state),
        "/api/users/me/password",
        &cookie,
        &pw_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "New password shorter than 8 characters must return 400"
    );
}

// ============================================================================
// PATCH /api/users/{id}/password — admin reset user password
// ============================================================================

#[tokio::test]
async fn test_admin_reset_password_success() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "resetpwadmin",
        "resetpwadmin@example.com",
        "adminpassword",
        "admin",
    )
    .await;
    let target = create_test_user_with_role(
        &db,
        "resetpwtarget",
        "resetpwtarget@example.com",
        "oldpassword",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "resetpwadmin",
        "adminpassword",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let pw_body = serde_json::json!({
        "new_password": "newlyset_password"
    })
    .to_string();

    let uri = format!("/api/users/{}/password", target.id);
    let (status, body) = authenticated_patch(create_router(state), &uri, &cookie, &pw_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Admin PATCH /api/users/{{id}}/password must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "Admin password reset response must include a message field"
    );
}

#[tokio::test]
async fn test_admin_reset_own_password_blocked() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let admin = create_test_user_with_role(
        &db,
        "selfpwadmin",
        "selfpwadmin@example.com",
        "adminpassword",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "selfpwadmin", "adminpassword").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let pw_body = serde_json::json!({
        "new_password": "anotherpassword"
    })
    .to_string();

    // Trying to reset own password via the admin endpoint
    let uri = format!("/api/users/{}/password", admin.id);
    let (status, _body) = authenticated_patch(create_router(state), &uri, &cookie, &pw_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Admin must not reset their own password via the admin endpoint"
    );
}

#[tokio::test]
async fn test_admin_reset_password_too_short() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "shortresetadmin",
        "shortresetadmin@example.com",
        "adminpassword",
        "admin",
    )
    .await;
    let target = create_test_user_with_role(
        &db,
        "shortresettarget",
        "shortresettarget@example.com",
        "oldpassword",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "shortresetadmin",
        "adminpassword",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let pw_body = serde_json::json!({
        "new_password": "short"  // less than 8 chars
    })
    .to_string();

    let uri = format!("/api/users/{}/password", target.id);
    let (status, _body) = authenticated_patch(create_router(state), &uri, &cookie, &pw_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Admin reset with password shorter than 8 characters must return 400"
    );
}

#[tokio::test]
async fn test_admin_reset_password_nonexistent_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "resetnonexistadmin",
        "resetnonexistadmin@example.com",
        "adminpassword",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "resetnonexistadmin",
        "adminpassword",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let pw_body = serde_json::json!({
        "new_password": "validpassword123"
    })
    .to_string();

    let (status, _body) = authenticated_patch(
        create_router(state),
        "/api/users/999999/password",
        &cookie,
        &pw_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Admin resetting password for nonexistent user must return 404"
    );
}

// ============================================================================
// GET /api/users/me/2fa/status — additional cases
// ============================================================================

#[tokio::test]
async fn test_2fa_status_unauthenticated() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/users/me/2fa/status")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/users/me/2fa/status without auth must return 401"
    );
}

// ============================================================================
// Auth checks — non-admin users cannot manage users
// ============================================================================

#[tokio::test]
async fn test_viewer_cannot_delete_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vieweronly",
        "vieweronly@example.com",
        "password123",
        "viewer",
    )
    .await;
    let target = create_test_user_with_role(
        &db,
        "viewertarget",
        "viewertarget@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vieweronly", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let uri = format!("/api/users/{}", target.id);
    let (status, _body) = authenticated_delete(create_router(state), &uri, &cookie).await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
        "Viewer must not be able to delete users. Got: {}",
        status
    );
}

#[tokio::test]
async fn test_viewer_cannot_approve_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "viewerapprove",
        "viewerapprove@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "viewerapprove", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _body) =
        authenticated_post(create_router(state), "/api/users/1/approve", &cookie, "{}").await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
        "Viewer must not be able to approve users. Got: {}",
        status
    );
}

// ============================================================================
// PATCH /api/users/me — empty username/email, email taken (lines 323,347,359)
// ============================================================================

#[tokio::test]
async fn test_update_own_profile_empty_username_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "emptyusername1",
        "emptyusername1@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "emptyusername1",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Whitespace-only username trims to empty string
    let patch_body = serde_json::json!({ "username": "   " }).to_string();
    let (status, _body) =
        authenticated_patch(create_router(state), "/api/users/me", &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Whitespace-only username must return 400"
    );
}

#[tokio::test]
async fn test_update_own_profile_empty_email_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "emptyemail1",
        "emptyemail1@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "emptyemail1", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Whitespace-only email trims to empty string
    let patch_body = serde_json::json!({ "email": "   " }).to_string();
    let (status, _body) =
        authenticated_patch(create_router(state), "/api/users/me", &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Whitespace-only email must return 400"
    );
}

#[tokio::test]
async fn test_update_own_profile_email_already_taken_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "emailtaken_a",
        "emailtaken_a@example.com",
        "password123",
        "admin",
    )
    .await;
    create_test_user_with_role(
        &db,
        "emailtaken_b",
        "emailtaken_b@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    // Log in as user A and try to steal user B's email
    let (_, cookie) = do_login(create_router(state.clone()), "emailtaken_a", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let patch_body = serde_json::json!({ "email": "emailtaken_b@example.com" }).to_string();
    let (status, _body) =
        authenticated_patch(create_router(state), "/api/users/me", &cookie, &patch_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Attempting to take another user's email must return 400"
    );
}

// ============================================================================
// PATCH /api/users/me/preferences — update twice (lines 504-508)
// ============================================================================

#[tokio::test]
async fn test_update_preferences_twice_exercises_update_path() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "prefsupdatetwice",
        "prefsupdatetwice@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "prefsupdatetwice",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First PATCH: inserts new preferences row (theme = dark)
    let patch1 = serde_json::json!({ "theme": "dark" }).to_string();
    let (status1, _) = authenticated_patch(
        create_router(state.clone()),
        "/api/users/me/preferences",
        &cookie,
        &patch1,
    )
    .await;
    assert_eq!(
        status1,
        StatusCode::OK,
        "First preference update must succeed"
    );

    // Second PATCH: updates the existing row (theme = light)
    let patch2 = serde_json::json!({ "theme": "light" }).to_string();
    let (status2, body2) = authenticated_patch(
        create_router(state.clone()),
        "/api/users/me/preferences",
        &cookie,
        &patch2,
    )
    .await;
    assert_eq!(
        status2,
        StatusCode::OK,
        "Second preference update must succeed. Body: {}",
        body2
    );

    let json: serde_json::Value = serde_json::from_str(&body2).unwrap();
    assert_eq!(
        json["theme"], "light",
        "Theme should be 'light' after second update"
    );
}

// ============================================================================
// POST /api/users — create user with role_ids (lines 615-619)
// ============================================================================

#[tokio::test]
async fn test_create_user_with_role_ids_assigns_roles() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "createadmin1",
        "createadmin1@example.com",
        "password123",
        "admin",
    )
    .await;

    // Look up the viewer role ID so we can pass it in role_ids
    let viewer_role_id = {
        use kubarr::models::{prelude::Role, role};
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let role = Role::find()
            .filter(role::Column::Name.eq("viewer"))
            .one(&db)
            .await
            .unwrap()
            .expect("viewer role must exist");
        role.id
    };

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "createadmin1", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let create_body = serde_json::json!({
        "username": "newuserwithrole",
        "email": "newuserwithrole@example.com",
        "password": "securepass123",
        "role_ids": [viewer_role_id]
    })
    .to_string();

    let (status, body) =
        authenticated_post(create_router(state), "/api/users", &cookie, &create_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/users must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["username"], "newuserwithrole");
    // User should have the viewer role
    let roles = json["roles"].as_array().expect("roles must be an array");
    assert!(
        !roles.is_empty(),
        "Created user must have assigned roles. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/users/invites — list invites (lines 826-844)
// ============================================================================

#[tokio::test]
async fn test_list_invites_with_created_invite() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "invitelistadmin",
        "invitelistadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "invitelistadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create an invite first
    let create_body = serde_json::json!({ "expires_in_days": 7 }).to_string();
    let (create_status, create_body_str) = authenticated_post(
        create_router(state.clone()),
        "/api/users/invites",
        &cookie,
        &create_body,
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "Creating invite must return 200. Body: {}",
        create_body_str
    );

    // Now list invites
    let (list_status, list_body) =
        authenticated_get(create_router(state), "/api/users/invites", &cookie).await;

    assert_eq!(
        list_status,
        StatusCode::OK,
        "GET /api/users/invites must return 200. Body: {}",
        list_body
    );

    let invites: serde_json::Value = serde_json::from_str(&list_body).unwrap();
    let arr = invites.as_array().expect("Invites must be an array");
    assert!(
        !arr.is_empty(),
        "Invite list must contain the created invite. Body: {}",
        list_body
    );

    // Verify the invite structure has expected fields
    let invite = &arr[0];
    assert!(invite.get("id").is_some(), "Invite must have an id");
    assert!(invite.get("code").is_some(), "Invite must have a code");
    assert!(
        invite.get("created_by_username").is_some(),
        "Invite must have created_by_username"
    );
    assert!(
        invite.get("is_used").is_some(),
        "Invite must have is_used field"
    );
}

// ============================================================================
// POST /api/users/me/2fa/setup + enable — enable 2FA success path (lines 1141-1170)
// POST /api/users/me/2fa/disable — disable 2FA success path (lines 1209-1228)
// ============================================================================

#[tokio::test]
async fn test_enable_2fa_success_generates_recovery_codes() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "enable2fauser",
        "enable2fauser@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "enable2fauser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Step 1: call setup to get the TOTP secret
    let (setup_status, setup_body) = authenticated_post(
        create_router(state.clone()),
        "/api/users/me/2fa/setup",
        &cookie,
        "{}",
    )
    .await;
    assert_eq!(
        setup_status,
        StatusCode::OK,
        "2FA setup must return 200. Body: {}",
        setup_body
    );
    let setup_json: serde_json::Value = serde_json::from_str(&setup_body).unwrap();
    let secret = setup_json["secret"]
        .as_str()
        .expect("setup must return a secret");

    // Step 2: generate the current TOTP code for this secret
    let code = {
        use totp_rs::{Algorithm, Secret, TOTP};

        let secret_bytes = Secret::Encoded(secret.to_string())
            .to_bytes()
            .expect("decode TOTP secret");
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret_bytes,
            Some("Kubarr".to_string()),
            "enable2fauser@example.com".to_string(),
        )
        .expect("create TOTP");
        totp.generate_current().expect("generate TOTP code")
    };

    // Step 3: enable 2FA with the just-generated code
    let enable_body = serde_json::json!({ "code": code }).to_string();
    let (enable_status, enable_resp) = authenticated_post(
        create_router(state),
        "/api/users/me/2fa/enable",
        &cookie,
        &enable_body,
    )
    .await;

    assert_eq!(
        enable_status,
        StatusCode::OK,
        "2FA enable must return 200. Body: {}",
        enable_resp
    );
    let enable_json: serde_json::Value = serde_json::from_str(&enable_resp).unwrap();
    let recovery_codes = enable_json["recovery_codes"]
        .as_array()
        .expect("enable_2fa must return recovery_codes");
    assert_eq!(
        recovery_codes.len(),
        8,
        "Must generate exactly 8 recovery codes"
    );
}

#[tokio::test]
async fn test_disable_2fa_success() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "disable2fauser",
        "disable2fauser@example.com",
        "password123",
        "viewer", // viewer role does NOT require 2FA
    )
    .await;

    // Log in BEFORE enabling 2FA (so the session cookie works without a TOTP challenge)
    let state_pre = build_test_app_state_with_db(db.clone()).await;
    let (_, cookie) = do_login(create_router(state_pre), "disable2fauser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Directly enable 2FA in the DB so we can test disable without going through enable
    {
        use kubarr::models::prelude::User;
        use kubarr::services::security::generate_totp_secret;
        use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

        let user: kubarr::models::user::Model = User::find()
            .filter(kubarr::models::user::Column::Username.eq("disable2fauser"))
            .one(&db)
            .await
            .unwrap()
            .unwrap();

        let secret = generate_totp_secret();
        let now = chrono::Utc::now();
        let update = kubarr::models::user::ActiveModel {
            id: Set(user.id),
            totp_enabled: Set(true),
            totp_secret: Set(Some(secret)),
            totp_verified_at: Set(Some(now)),
            updated_at: Set(now),
            ..Default::default()
        };
        update.update(&db).await.expect("enable 2FA in DB");
    }

    let state = build_test_app_state_with_db(db).await;

    // Now call disable — session cookie is still valid
    let disable_body = serde_json::json!({ "password": "password123" }).to_string();
    let (status, body) = authenticated_post(
        create_router(state),
        "/api/users/me/2fa/disable",
        &cookie,
        &disable_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "2FA disable must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "Disable response must include a message"
    );
}
