//! Extended roles endpoint integration tests
//!
//! Covers endpoints NOT already tested in roles_endpoint_tests.rs:
//! - `GET /api/roles/permissions`        — list all available permissions (no params needed)
//! - `GET /api/roles/{id}/permissions`   — list permissions granted to a specific role
//! - `PUT /api/roles/{id}/permissions`   — set/replace permissions for a role
//! - `PUT /api/roles/{id}/apps`          — set app permissions for a role
//! Also adds coverage for edge cases on already-tested endpoints:
//! - Duplicate role name returns 400
//! - Renaming a system role is blocked
//! - Non-admin cannot access roles endpoints

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

#[allow(dead_code)]
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

async fn authenticated_put(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("PUT")
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
// GET /api/roles/permissions — list all available permissions
// ============================================================================

#[tokio::test]
async fn test_list_all_permissions_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "permlistadmin",
        "permlistadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "permlistadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/roles/permissions", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/roles/permissions must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_array(),
        "Permissions response must be a JSON array. Body: {}",
        body
    );

    let permissions = json.as_array().unwrap();
    assert!(
        !permissions.is_empty(),
        "Permissions list must not be empty"
    );

    // Verify each permission has the expected fields
    let first = &permissions[0];
    assert!(
        first.get("key").is_some(),
        "Each permission must have a 'key' field"
    );
    assert!(
        first.get("category").is_some(),
        "Each permission must have a 'category' field"
    );
    assert!(
        first.get("description").is_some(),
        "Each permission must have a 'description' field"
    );
}

#[tokio::test]
async fn test_list_all_permissions_contains_known_permissions() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "permcheckladmin",
        "permcheckladmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "permcheckladmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/roles/permissions", &cookie).await;

    assert_eq!(status, StatusCode::OK, "Must return 200. Body: {}", body);

    let permissions: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    let keys: Vec<&str> = permissions
        .iter()
        .filter_map(|p| p["key"].as_str())
        .collect();

    // Verify well-known permissions exist
    let expected_permissions = [
        "users.view",
        "users.manage",
        "roles.view",
        "roles.manage",
        "apps.view",
        "settings.view",
    ];
    for expected in expected_permissions {
        assert!(
            keys.contains(&expected),
            "Permissions list must contain '{}'. Got: {:?}",
            expected,
            keys
        );
    }
}

#[tokio::test]
async fn test_list_all_permissions_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/roles/permissions")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/roles/permissions without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_all_permissions_includes_app_permissions() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "apppermsadmin",
        "apppermsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "apppermsadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/roles/permissions", &cookie).await;

    assert_eq!(status, StatusCode::OK, "Must return 200. Body: {}", body);

    let permissions: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    let keys: Vec<&str> = permissions
        .iter()
        .filter_map(|p| p["key"].as_str())
        .collect();

    // App access permissions should be included
    assert!(
        keys.iter().any(|k| k.starts_with("app.")),
        "Permissions must include app.* access permissions. Got: {:?}",
        keys
    );
    assert!(
        keys.contains(&"app.sonarr"),
        "Permissions must contain 'app.sonarr'. Got: {:?}",
        keys
    );
    assert!(
        keys.contains(&"app.jellyfin"),
        "Permissions must contain 'app.jellyfin'. Got: {:?}",
        keys
    );
}

// ============================================================================
// GET /api/roles/{id}/permissions — list permissions for a role
// ============================================================================

#[tokio::test]
async fn test_get_role_permissions_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "getrolepermadmin",
        "getroleperm@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "getrolepermadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Find the admin role's ID
    let (_, list_body) =
        authenticated_get(create_router(state.clone()), "/api/roles", &cookie).await;
    let roles: Vec<serde_json::Value> = serde_json::from_str(&list_body).unwrap();
    let admin_role = roles
        .iter()
        .find(|r| r["name"] == "admin")
        .expect("admin role must exist");
    let admin_role_id = admin_role["id"].as_i64().unwrap();

    let uri = format!("/api/roles/{}/permissions", admin_role_id);
    let (status, body) = authenticated_get(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/roles/{{id}}/permissions must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_array(),
        "Role permissions response must be a JSON array"
    );

    let perms = json.as_array().unwrap();
    // Admin role should have at least some permissions
    assert!(
        !perms.is_empty(),
        "Admin role must have at least some permissions"
    );

    // All entries should be strings
    for perm in perms {
        assert!(
            perm.is_string(),
            "Each permission in the list must be a string, got: {}",
            perm
        );
    }
}

#[tokio::test]
async fn test_get_role_permissions_contains_expected_perms() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "checkadminperms",
        "checkadminperms@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "checkadminperms",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Find admin role ID
    let (_, list_body) =
        authenticated_get(create_router(state.clone()), "/api/roles", &cookie).await;
    let roles: Vec<serde_json::Value> = serde_json::from_str(&list_body).unwrap();
    let admin_role = roles
        .iter()
        .find(|r| r["name"] == "admin")
        .expect("admin role must exist");
    let admin_role_id = admin_role["id"].as_i64().unwrap();

    let uri = format!("/api/roles/{}/permissions", admin_role_id);
    let (status, body) = authenticated_get(create_router(state), &uri, &cookie).await;

    assert_eq!(status, StatusCode::OK, "Must return 200. Body: {}", body);

    let perms: Vec<String> = serde_json::from_str(&body).unwrap();
    assert!(
        perms.contains(&"users.manage".to_string()),
        "Admin role must have 'users.manage' permission. Got: {:?}",
        perms
    );
    assert!(
        perms.contains(&"roles.manage".to_string()),
        "Admin role must have 'roles.manage' permission. Got: {:?}",
        perms
    );
}

#[tokio::test]
async fn test_get_role_permissions_nonexistent_role() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "nonexistroleperm",
        "nonexistroleperm@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "nonexistroleperm",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _body) = authenticated_get(
        create_router(state),
        "/api/roles/999999/permissions",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "GET /api/roles/{{id}}/permissions for nonexistent role must return 404"
    );
}

// ============================================================================
// PUT /api/roles/{id}/permissions — set permissions for a role
// ============================================================================

#[tokio::test]
async fn test_set_role_permissions() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "setpermadmin",
        "setpermadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "setpermadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role to set permissions on
    let new_role_body = serde_json::json!({
        "name": "perm_test_role",
        "description": "Role for permission testing"
    })
    .to_string();

    let (create_status, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "Role creation must succeed. Body: {}",
        create_body
    );

    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    // Set permissions on the role
    let perms_body = serde_json::json!({
        "permissions": ["apps.view", "logs.view", "storage.view"]
    })
    .to_string();

    let uri = format!("/api/roles/{}/permissions", role_id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &perms_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/roles/{{id}}/permissions must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // The response is the full RoleWithAppsResponse
    assert!(
        json.get("permissions").is_some(),
        "Response must include permissions field"
    );

    let perms_in_response: Vec<&str> = json["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p.as_str())
        .collect();

    assert!(
        perms_in_response.contains(&"apps.view"),
        "Role permissions must include 'apps.view'. Got: {:?}",
        perms_in_response
    );
    assert!(
        perms_in_response.contains(&"logs.view"),
        "Role permissions must include 'logs.view'. Got: {:?}",
        perms_in_response
    );
}

#[tokio::test]
async fn test_set_role_permissions_replaces_existing() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "replacepermsadmin",
        "replacepermsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "replacepermsadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role
    let new_role_body = serde_json::json!({
        "name": "replace_perm_role",
        "description": "Role for replace permission test"
    })
    .to_string();
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    // Set initial permissions
    let first_perms_body = serde_json::json!({
        "permissions": ["apps.view", "logs.view"]
    })
    .to_string();
    let uri = format!("/api/roles/{}/permissions", role_id);
    authenticated_put(
        create_router(state.clone()),
        &uri,
        &cookie,
        &first_perms_body,
    )
    .await;

    // Replace with a different set of permissions
    let second_perms_body = serde_json::json!({
        "permissions": ["storage.view"]
    })
    .to_string();
    let (status, body) = authenticated_put(
        create_router(state.clone()),
        &uri,
        &cookie,
        &second_perms_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Second PUT /api/roles/{{id}}/permissions must return 200. Body: {}",
        body
    );

    // Verify old permissions are gone, new ones are present
    let (get_status, get_body) = authenticated_get(create_router(state), &uri, &cookie).await;
    assert_eq!(get_status, StatusCode::OK);

    let perms: Vec<String> = serde_json::from_str(&get_body).unwrap();
    assert!(
        perms.contains(&"storage.view".to_string()),
        "New permission 'storage.view' must be present. Got: {:?}",
        perms
    );
    assert!(
        !perms.contains(&"apps.view".to_string()),
        "Old permission 'apps.view' must be removed. Got: {:?}",
        perms
    );
    assert!(
        !perms.contains(&"logs.view".to_string()),
        "Old permission 'logs.view' must be removed. Got: {:?}",
        perms
    );
}

#[tokio::test]
async fn test_set_role_permissions_with_app_permissions() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "apppermbyadmin",
        "apppermbyadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "apppermbyadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role
    let new_role_body = serde_json::json!({
        "name": "app_perm_role",
        "description": "Role with app permissions"
    })
    .to_string();
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    // Set permissions including app.* permissions
    let perms_body = serde_json::json!({
        "permissions": ["apps.view", "app.sonarr", "app.radarr"]
    })
    .to_string();

    let uri = format!("/api/roles/{}/permissions", role_id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &perms_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT with app.* permissions must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // app_names should be synced
    let empty_vec = vec![];
    let app_names: Vec<&str> = json["app_names"]
        .as_array()
        .unwrap_or(&empty_vec)
        .iter()
        .filter_map(|a| a.as_str())
        .collect();
    assert!(
        app_names.contains(&"sonarr"),
        "app_names must include 'sonarr'. Got: {:?}",
        app_names
    );
    assert!(
        app_names.contains(&"radarr"),
        "app_names must include 'radarr'. Got: {:?}",
        app_names
    );
}

#[tokio::test]
async fn test_set_role_permissions_clear_all() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "clearpermsadmin",
        "clearpermsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "clearpermsadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role with some permissions
    let new_role_body = serde_json::json!({
        "name": "clear_perm_role",
        "description": "Role for clear permission test"
    })
    .to_string();
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    let uri = format!("/api/roles/{}/permissions", role_id);

    // First set some permissions
    let initial_perms = serde_json::json!({ "permissions": ["apps.view"] }).to_string();
    authenticated_put(create_router(state.clone()), &uri, &cookie, &initial_perms).await;

    // Now clear all permissions
    let clear_body = serde_json::json!({ "permissions": [] }).to_string();
    let (status, body) =
        authenticated_put(create_router(state.clone()), &uri, &cookie, &clear_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT with empty permissions must return 200. Body: {}",
        body
    );

    // Verify permissions are now empty
    let (get_status, get_body) = authenticated_get(create_router(state), &uri, &cookie).await;
    assert_eq!(get_status, StatusCode::OK);
    let perms: Vec<String> = serde_json::from_str(&get_body).unwrap();
    assert!(
        perms.is_empty(),
        "All permissions must be cleared. Got: {:?}",
        perms
    );
}

#[tokio::test]
async fn test_set_role_permissions_nonexistent_role() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "setnonexistperm",
        "setnonexistperm@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "setnonexistperm",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let perms_body = serde_json::json!({ "permissions": ["apps.view"] }).to_string();
    let (status, _body) = authenticated_put(
        create_router(state),
        "/api/roles/999999/permissions",
        &cookie,
        &perms_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Setting permissions on a nonexistent role must return 404"
    );
}

// ============================================================================
// PUT /api/roles/{id}/apps — set app permissions for a role
// ============================================================================

#[tokio::test]
async fn test_set_role_apps() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "setappsadmin",
        "setappsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "setappsadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role
    let new_role_body = serde_json::json!({
        "name": "apps_test_role",
        "description": "Role for apps test"
    })
    .to_string();
    let (create_status, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "Role creation must succeed. Body: {}",
        create_body
    );
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    // Set app permissions
    let apps_body = serde_json::json!({
        "app_names": ["sonarr", "radarr", "jellyfin"]
    })
    .to_string();

    let uri = format!("/api/roles/{}/apps", role_id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &apps_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/roles/{{id}}/apps must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let empty_vec = vec![];
    let app_names: Vec<&str> = json["app_names"]
        .as_array()
        .unwrap_or(&empty_vec)
        .iter()
        .filter_map(|a| a.as_str())
        .collect();

    assert!(
        app_names.contains(&"sonarr"),
        "app_names must include 'sonarr'. Got: {:?}",
        app_names
    );
    assert!(
        app_names.contains(&"radarr"),
        "app_names must include 'radarr'. Got: {:?}",
        app_names
    );
    assert!(
        app_names.contains(&"jellyfin"),
        "app_names must include 'jellyfin'. Got: {:?}",
        app_names
    );
}

#[tokio::test]
async fn test_set_role_apps_replaces_existing() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "replaceappsadmin",
        "replaceappsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "replaceappsadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role with initial apps
    let new_role_body = serde_json::json!({
        "name": "replace_apps_role",
        "description": "For replace apps test"
    })
    .to_string();
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    let uri = format!("/api/roles/{}/apps", role_id);

    // Set initial apps
    let first_apps = serde_json::json!({ "app_names": ["sonarr", "radarr"] }).to_string();
    authenticated_put(create_router(state.clone()), &uri, &cookie, &first_apps).await;

    // Replace with a different set
    let second_apps = serde_json::json!({ "app_names": ["jellyfin"] }).to_string();
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &second_apps).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Second PUT /api/roles/{{id}}/apps must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let empty_vec = vec![];
    let app_names: Vec<&str> = json["app_names"]
        .as_array()
        .unwrap_or(&empty_vec)
        .iter()
        .filter_map(|a| a.as_str())
        .collect();

    assert!(
        app_names.contains(&"jellyfin"),
        "New app 'jellyfin' must be present. Got: {:?}",
        app_names
    );
    assert!(
        !app_names.contains(&"sonarr"),
        "Old app 'sonarr' must be removed. Got: {:?}",
        app_names
    );
    assert!(
        !app_names.contains(&"radarr"),
        "Old app 'radarr' must be removed. Got: {:?}",
        app_names
    );
}

#[tokio::test]
async fn test_set_role_apps_clear_all() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "clearappsadmin",
        "clearappsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "clearappsadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a role with apps
    let new_role_body = serde_json::json!({
        "name": "clear_apps_role",
        "description": "For clear apps test",
        "app_names": ["sonarr"]
    })
    .to_string();
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &new_role_body,
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    // Clear all apps
    let clear_body = serde_json::json!({ "app_names": [] }).to_string();
    let uri = format!("/api/roles/{}/apps", role_id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &clear_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT with empty app_names must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let empty_vec = vec![];
    let app_names: Vec<&str> = json["app_names"]
        .as_array()
        .unwrap_or(&empty_vec)
        .iter()
        .filter_map(|a| a.as_str())
        .collect();

    assert!(
        app_names.is_empty(),
        "All app permissions must be cleared. Got: {:?}",
        app_names
    );
}

#[tokio::test]
async fn test_set_role_apps_nonexistent_role() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "setappsnonexist",
        "setappsnonexist@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "setappsnonexist",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let apps_body = serde_json::json!({ "app_names": ["sonarr"] }).to_string();
    let (status, _body) = authenticated_put(
        create_router(state),
        "/api/roles/999999/apps",
        &cookie,
        &apps_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Setting apps on a nonexistent role must return 404"
    );
}

// ============================================================================
// Edge cases for existing endpoints
// ============================================================================

#[tokio::test]
async fn test_create_role_duplicate_name_fails() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "duprolead",
        "duprole@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "duprolead", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let role_body = serde_json::json!({
        "name": "duplicate_role",
        "description": "First role"
    })
    .to_string();

    // First creation must succeed
    let (status1, _body1) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &role_body,
    )
    .await;
    assert_eq!(status1, StatusCode::OK, "First role creation must succeed");

    // Second creation with same name must fail
    let (status2, body2) =
        authenticated_post(create_router(state), "/api/roles", &cookie, &role_body).await;

    assert_eq!(
        status2,
        StatusCode::BAD_REQUEST,
        "Creating a role with a duplicate name must return 400. Body: {}",
        body2
    );
}

#[tokio::test]
async fn test_rename_system_role_fails() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "renamead",
        "renamead@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "renamead", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Get admin role ID
    let (_, list_body) =
        authenticated_get(create_router(state.clone()), "/api/roles", &cookie).await;
    let roles: Vec<serde_json::Value> = serde_json::from_str(&list_body).unwrap();
    let admin_role = roles
        .iter()
        .find(|r| r["name"] == "admin" && r["is_system"] == true)
        .expect("admin system role must exist");
    let admin_role_id = admin_role["id"].as_i64().unwrap();

    let rename_body = serde_json::json!({
        "name": "not_admin_anymore"
    })
    .to_string();

    let uri = format!("/api/roles/{}", admin_role_id);
    let (status, _body) =
        authenticated_patch(create_router(state), &uri, &cookie, &rename_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Renaming a system role must return 400"
    );
}

#[tokio::test]
async fn test_get_nonexistent_role_returns_404() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "rolenotfoundad",
        "rolenotfound@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "rolenotfoundad",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _body) =
        authenticated_get(create_router(state), "/api/roles/999999", &cookie).await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "GET /api/roles/{{id}} for nonexistent role must return 404"
    );
}

#[tokio::test]
async fn test_viewer_cannot_create_role() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "viewercreate",
        "viewercreate@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "viewercreate", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let role_body = serde_json::json!({
        "name": "hacked_role",
        "description": "Viewer should not be able to create this"
    })
    .to_string();

    let (status, _body) =
        authenticated_post(create_router(state), "/api/roles", &cookie, &role_body).await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
        "Viewer must not be able to create roles. Got: {}",
        status
    );
}

#[tokio::test]
async fn test_create_role_with_requires_2fa() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "create2farolead",
        "create2farole@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "create2farolead",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let role_body = serde_json::json!({
        "name": "secure_role_2fa",
        "description": "A role that requires 2FA",
        "requires_2fa": true
    })
    .to_string();

    let (status, body) =
        authenticated_post(create_router(state), "/api/roles", &cookie, &role_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Creating a role with requires_2fa must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["requires_2fa"], true,
        "Created role must have requires_2fa = true"
    );
}

// ============================================================================
// PATCH /api/roles/{id} — name update and requires_2fa paths (lines 253-275)
// ============================================================================

#[tokio::test]
async fn test_update_role_rename_to_new_name_succeeds() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "renameroleadmin1",
        "renamerole1@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "renameroleadmin1",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a role to rename
    let create_body = serde_json::json!({
        "name": "rename_me_role",
        "description": "Role to rename"
    })
    .to_string();
    let (cs, cb) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &create_body,
    )
    .await;
    assert_eq!(cs, StatusCode::OK, "Create role must succeed. Body: {}", cb);
    let created: serde_json::Value = serde_json::from_str(&cb).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    // Rename to a brand-new unique name — exercises the "not a duplicate" path
    let update_body = serde_json::json!({ "name": "rename_me_role_updated" }).to_string();
    let uri = format!("/api/roles/{}", role_id);
    let (status, body) =
        authenticated_patch(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Renaming to a new unique name must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["name"], "rename_me_role_updated",
        "Role name must be updated in response"
    );
}

#[tokio::test]
async fn test_update_role_rename_to_duplicate_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "renameduproleadmin",
        "renameduplicate@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "renameduproleadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create two roles
    let body_a = serde_json::json!({ "name": "dup_role_a" }).to_string();
    let body_b = serde_json::json!({ "name": "dup_role_b" }).to_string();

    let (_, ca) =
        authenticated_post(create_router(state.clone()), "/api/roles", &cookie, &body_a).await;
    let (_, cb) =
        authenticated_post(create_router(state.clone()), "/api/roles", &cookie, &body_b).await;

    let role_a: serde_json::Value = serde_json::from_str(&ca).unwrap();
    let role_a_id = role_a["id"].as_i64().unwrap();

    // Try to rename role A to role B's name — must return 400
    let update_body = serde_json::json!({ "name": "dup_role_b" }).to_string();
    let uri = format!("/api/roles/{}", role_a_id);
    let (status, _) = authenticated_patch(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Renaming to an existing role name must return 400"
    );
}

#[tokio::test]
async fn test_update_role_requires_2fa_field() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "requires2faadmin",
        "requires2fa@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "requires2faadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a custom role without requires_2fa
    let create_body =
        serde_json::json!({ "name": "requires2fa_test_role", "requires_2fa": false }).to_string();
    let (cs, cb) = authenticated_post(
        create_router(state.clone()),
        "/api/roles",
        &cookie,
        &create_body,
    )
    .await;
    assert_eq!(cs, StatusCode::OK, "Create role must succeed. Body: {}", cb);
    let created: serde_json::Value = serde_json::from_str(&cb).unwrap();
    let role_id = created["id"].as_i64().unwrap();

    // Update the role, setting requires_2fa = true — exercises line 275
    let update_body = serde_json::json!({ "requires_2fa": true }).to_string();
    let uri = format!("/api/roles/{}", role_id);
    let (status, body) =
        authenticated_patch(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PATCH with requires_2fa must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["requires_2fa"], true,
        "requires_2fa must be updated to true"
    );
}
