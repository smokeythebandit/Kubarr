//! Roles endpoint integration tests
//!
//! Covers:
//! - `GET /api/roles` — list roles (requires roles.view)
//! - `GET /api/roles/{id}` — get role by ID (requires roles.view)
//! - `POST /api/roles` — create role (requires roles.manage)
//! - `PATCH /api/roles/{id}` — update role description (requires roles.manage)
//! - `DELETE /api/roles/{id}` — delete a custom role (requires roles.manage)
//! - Deleting a system role must fail
//! - Assigning/removing a role from a user via PATCH /api/users/{id}

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

/// Make an authenticated PATCH request and return (status, body).
async fn authenticated_patch(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("PATCH")
        .header("Cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(json_body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// Make an authenticated DELETE request and return (status, body).
async fn authenticated_delete(app: axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("DELETE")
        .header("Cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

// ============================================================================
// GET /api/roles — requires auth
// ============================================================================

#[tokio::test]
async fn test_list_roles_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/roles")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/roles without a session cookie must return 401"
    );
}

#[tokio::test]
async fn test_list_roles_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "rolesadmin",
        "rolesadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "rolesadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(create_router(state), "/api/roles", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/roles must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array(), "Roles response must be a JSON array");

    let roles = json.as_array().unwrap();
    assert!(
        !roles.is_empty(),
        "Roles list must not be empty (seeded roles exist)"
    );

    // The seeded roles admin, viewer, downloader must all be present
    let role_names: Vec<&str> = roles.iter().filter_map(|r| r["name"].as_str()).collect();

    assert!(
        role_names.contains(&"admin"),
        "Roles list must include the 'admin' role. Got: {:?}",
        role_names
    );
    assert!(
        role_names.contains(&"viewer"),
        "Roles list must include the 'viewer' role. Got: {:?}",
        role_names
    );
    assert!(
        role_names.contains(&"downloader"),
        "Roles list must include the 'downloader' role. Got: {:?}",
        role_names
    );
}

// ============================================================================
// GET /api/roles/{id}
// ============================================================================

#[tokio::test]
async fn test_get_role_by_id() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "getroleadmin",
        "getrole@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "getroleadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First get the full list to find a valid role ID
    let (_, list_body) =
        authenticated_get(create_router(state.clone()), "/api/roles", &cookie).await;
    let roles: Vec<serde_json::Value> = serde_json::from_str(&list_body).unwrap();
    let admin_role = roles
        .iter()
        .find(|r| r["name"] == "admin")
        .expect("Admin role must exist in the list");
    let admin_role_id = admin_role["id"].as_i64().unwrap();

    let uri = format!("/api/roles/{}", admin_role_id);
    let (status, body) = authenticated_get(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/roles/{{id}} must return 200 for a valid role. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["name"], "admin",
        "Returned role must have name 'admin'"
    );
    assert!(
        json.get("permissions").is_some(),
        "Role response must include permissions field"
    );
    assert!(
        json.get("is_system").is_some(),
        "Role response must include is_system field"
    );
    assert_eq!(json["is_system"], true, "Admin role must be a system role");
}

// ============================================================================
// POST /api/roles — create role
// ============================================================================

#[tokio::test]
async fn test_create_role() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "createrolead",
        "createrole@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "createrolead", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let new_role_body = serde_json::json!({
        "name": "custom_test_role",
        "description": "A custom role for testing",
        "requires_2fa": false
    })
    .to_string();

    let (status, body) =
        authenticated_post(create_router(state), "/api/roles", &cookie, &new_role_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/roles must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["name"], "custom_test_role",
        "Created role must have the requested name"
    );
    assert_eq!(
        json["description"], "A custom role for testing",
        "Created role must have the requested description"
    );
    assert_eq!(
        json["is_system"], false,
        "User-created roles must not be system roles"
    );
    assert!(json.get("id").is_some(), "Created role must have an id");
}

// ============================================================================
// PATCH /api/roles/{id} — update role
// ============================================================================

#[tokio::test]
async fn test_update_role() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "updateroleadmin",
        "updaterole@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "updateroleadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role to update
    let new_role_body = serde_json::json!({
        "name": "role_to_update",
        "description": "Original description"
    })
    .to_string();

    let (create_status, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "Role creation must succeed");

    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    // Update the role's description
    let update_body = serde_json::json!({
        "description": "Updated description"
    })
    .to_string();

    let uri = format!("/api/roles/{}", role_id);
    let (status, body) =
        authenticated_patch(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PATCH /api/roles/{{id}} must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["description"], "Updated description",
        "Role description must be updated"
    );
    assert_eq!(
        json["name"], "role_to_update",
        "Role name must remain unchanged"
    );
}

// ============================================================================
// DELETE /api/roles/{id} — delete custom role
// ============================================================================

#[tokio::test]
async fn test_delete_role() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "deleteroleadmin",
        "deleterole@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "deleteroleadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role to delete
    let new_role_body = serde_json::json!({
        "name": "role_to_delete",
        "description": "This role will be deleted"
    })
    .to_string();

    let (create_status, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "Role creation must succeed");

    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    let uri = format!("/api/roles/{}", role_id);
    let (status, body) = authenticated_delete(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "DELETE /api/roles/{{id}} on a custom role must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "Delete response must include a message field"
    );
}

// ============================================================================
// DELETE /api/roles/{id} — system roles cannot be deleted
// ============================================================================

#[tokio::test]
async fn test_delete_system_role_fails() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "delsysrole",
        "delsysrole@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "delsysrole", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Find the viewer system role ID
    let (_, list_body) =
        authenticated_get(create_router(state.clone()), "/api/roles", &cookie).await;
    let roles: Vec<serde_json::Value> = serde_json::from_str(&list_body).unwrap();
    let viewer_role = roles
        .iter()
        .find(|r| r["name"] == "viewer" && r["is_system"] == true)
        .expect("viewer system role must exist");
    let viewer_role_id = viewer_role["id"].as_i64().unwrap();

    let uri = format!("/api/roles/{}", viewer_role_id);
    let (status, body) = authenticated_delete(create_router(state), &uri, &cookie).await;

    assert!(
        status == StatusCode::BAD_REQUEST
            || status == StatusCode::UNPROCESSABLE_ENTITY
            || status == StatusCode::FORBIDDEN,
        "Deleting a system role must return an error (400, 422, or 403). Got: {} Body: {}",
        status,
        body
    );
}

// ============================================================================
// Assign role to user — via PATCH /api/users/{user_id}
// ============================================================================

#[tokio::test]
async fn test_assign_role_to_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "assignroleadmin",
        "assignrole@example.com",
        "password123",
        "admin",
    )
    .await;
    // Create a plain user with no role to assign one to
    let target_user =
        common::create_test_user(&db, "noroleuser", "norole@example.com", "pass123", true).await;

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "assignroleadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Find the viewer role ID
    let (_, list_body) =
        authenticated_get(create_router(state.clone()), "/api/roles", &cookie).await;
    let roles: Vec<serde_json::Value> = serde_json::from_str(&list_body).unwrap();
    let viewer_role = roles
        .iter()
        .find(|r| r["name"] == "viewer")
        .expect("viewer role must exist");
    let viewer_role_id = viewer_role["id"].as_i64().unwrap();

    // Assign the viewer role to the target user
    let update_body = serde_json::json!({
        "role_ids": [viewer_role_id]
    })
    .to_string();

    let uri = format!("/api/users/{}", target_user.id);
    let (status, body) =
        authenticated_patch(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Assigning a role to a user must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let roles_in_response: Vec<serde_json::Value> =
        json["roles"].as_array().cloned().unwrap_or_default();
    let role_names: Vec<&str> = roles_in_response
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();

    assert!(
        role_names.contains(&"viewer"),
        "Updated user must have the viewer role assigned. Got: {:?}",
        role_names
    );
}

// ============================================================================
// Remove role from user — via PATCH /api/users/{user_id} with empty role_ids
// ============================================================================

#[tokio::test]
async fn test_remove_role_from_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "removeroleadmin",
        "removerole@example.com",
        "password123",
        "admin",
    )
    .await;
    // Create a user that has the viewer role
    let target_user = create_test_user_with_role(
        &db,
        "hadroleuser",
        "hadrole@example.com",
        "pass123",
        "viewer",
    )
    .await;

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "removeroleadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Remove all roles from the target user by passing empty role_ids
    let update_body = serde_json::json!({
        "role_ids": []
    })
    .to_string();

    let uri = format!("/api/users/{}", target_user.id);
    let (status, body) =
        authenticated_patch(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Removing roles from a user must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let roles_in_response: Vec<serde_json::Value> =
        json["roles"].as_array().cloned().unwrap_or_default();

    assert!(
        roles_in_response.is_empty(),
        "User must have no roles after removal. Got: {:?}",
        roles_in_response
    );
}
