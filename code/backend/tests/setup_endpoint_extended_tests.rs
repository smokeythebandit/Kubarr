//! Extended integration tests for `src/endpoints/setup.rs`
//!
//! Covers previously uncovered paths in the setup endpoints:
//!
//! - `GET /api/setup/status`                     — returns detailed setup status (all fields)
//! - `GET /api/setup/generate-credentials`       — generates random admin credentials
//! - `POST /api/setup/validate-path`             — validates a storage path with various inputs
//! - `GET /api/setup/browse`                     — directory browser
//! - `GET /api/setup/bootstrap/status`           — bootstrap status
//! - `POST /api/setup/bootstrap/start`           — starts bootstrap
//! - `POST /api/setup/bootstrap/retry/{component}` — retry a bootstrap component
//! - `GET /api/setup/server`                     — get server config
//! - `POST /api/setup/server`                    — configure server
//! - `POST /api/setup/initialize`                — create admin user
//!
//! All setup endpoints are public (no auth required). Many return 403 if setup
//! is already complete (admin user exists).

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::util::ServiceExt;

mod common;
use common::{
    build_test_app_state, build_test_app_state_no_db, build_test_app_state_with_db,
    create_test_db_with_seed, create_test_user_with_role,
};

use kubarr::endpoints::create_router;

// ============================================================================
// Helpers
// ============================================================================

async fn get_setup(router: axum::Router, uri: &str) -> (StatusCode, String) {
    let response = router
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

async fn post_json_setup(router: axum::Router, uri: &str, json: &str) -> (StatusCode, String) {
    let response = router
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(json.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

async fn post_empty_setup(router: axum::Router, uri: &str) -> (StatusCode, String) {
    let response = router
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

// ============================================================================
// GET /api/setup/status — detailed setup status
// ============================================================================

#[tokio::test]
async fn test_setup_status_all_fields_present_before_setup() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/status").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/setup/status must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        json.get("setup_required").is_some(),
        "Must have setup_required. Body: {}",
        body
    );
    assert!(
        json.get("bootstrap_complete").is_some(),
        "Must have bootstrap_complete. Body: {}",
        body
    );
    assert!(
        json.get("server_configured").is_some(),
        "Must have server_configured. Body: {}",
        body
    );
    assert!(
        json.get("admin_user_exists").is_some(),
        "Must have admin_user_exists. Body: {}",
        body
    );
    assert!(
        json.get("storage_configured").is_some(),
        "Must have storage_configured. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_setup_status_values_correct_before_any_setup() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/status").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        json["setup_required"], true,
        "setup_required must be true before setup. Body: {}",
        body
    );
    assert_eq!(
        json["admin_user_exists"], false,
        "admin_user_exists must be false before setup. Body: {}",
        body
    );
    assert_eq!(
        json["bootstrap_complete"], false,
        "bootstrap_complete must be false initially. Body: {}",
        body
    );
    assert_eq!(
        json["server_configured"], false,
        "server_configured must be false initially. Body: {}",
        body
    );
    assert_eq!(
        json["storage_configured"], false,
        "storage_configured must be false initially. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_setup_status_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "status_admin_ext",
        "status_admin_ext@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get_setup(create_router(state), "/api/setup/status").await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "GET /api/setup/status must return 403 after admin creation. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_setup_status_after_server_config_shows_configured() {
    let state = build_test_app_state().await;

    // First configure the server
    let server_body = serde_json::json!({
        "name": "TestServer",
        "storage_path": "/tmp"
    })
    .to_string();

    let (config_status, config_body) = post_json_setup(
        create_router(state.clone()),
        "/api/setup/server",
        &server_body,
    )
    .await;
    assert_eq!(
        config_status,
        StatusCode::OK,
        "Server config must succeed. Body: {}",
        config_body
    );

    // Now check status — server_configured should be true
    let (status, body) = get_setup(create_router(state), "/api/setup/status").await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["server_configured"], true,
        "server_configured must be true after configure_server. Body: {}",
        body
    );
    assert_eq!(
        json["storage_configured"], true,
        "storage_configured must be true after configure_server. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/setup/generate-credentials — generates admin credentials
// ============================================================================

#[tokio::test]
async fn test_generate_credentials_returns_200_before_setup() {
    let state = build_test_app_state().await;
    let (status, _) = get_setup(create_router(state), "/api/setup/generate-credentials").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/setup/generate-credentials must return 200 before setup"
    );
}

#[tokio::test]
async fn test_generate_credentials_response_structure() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/generate-credentials").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        json.get("admin_username").is_some(),
        "Must have admin_username. Body: {}",
        body
    );
    assert!(
        json.get("admin_email").is_some(),
        "Must have admin_email. Body: {}",
        body
    );
    assert!(
        json.get("admin_password").is_some(),
        "Must have admin_password. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_generate_credentials_password_length_at_least_16() {
    let state = build_test_app_state().await;
    let (_, body) = get_setup(create_router(state), "/api/setup/generate-credentials").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let password = json["admin_password"].as_str().unwrap_or("");

    assert!(
        password.len() >= 16,
        "Generated password must be at least 16 characters long. Got: {} ('{}')",
        password.len(),
        password
    );
}

#[tokio::test]
async fn test_generate_credentials_username_is_admin() {
    let state = build_test_app_state().await;
    let (_, body) = get_setup(create_router(state), "/api/setup/generate-credentials").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["admin_username"], "admin",
        "Default username must be 'admin'"
    );
    assert_eq!(
        json["admin_email"], "admin@example.com",
        "Default email must be 'admin@example.com'"
    );
}

#[tokio::test]
async fn test_generate_credentials_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "gen_cred_admin",
        "gen_cred_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get_setup(create_router(state), "/api/setup/generate-credentials").await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "generate-credentials must return 403 after admin creation. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_generate_credentials_password_is_unique_across_calls() {
    // Two calls should generate different passwords (random generation)
    let state1 = build_test_app_state().await;
    let state2 = build_test_app_state().await;

    let (_, body1) = get_setup(create_router(state1), "/api/setup/generate-credentials").await;
    let (_, body2) = get_setup(create_router(state2), "/api/setup/generate-credentials").await;

    let json1: serde_json::Value = serde_json::from_str(&body1).unwrap();
    let json2: serde_json::Value = serde_json::from_str(&body2).unwrap();

    let pass1 = json1["admin_password"].as_str().unwrap_or("");
    let pass2 = json2["admin_password"].as_str().unwrap_or("");

    // With 16 char random passwords the collision probability is negligible
    assert_ne!(
        pass1, pass2,
        "Two generated passwords should be different: '{}' vs '{}'",
        pass1, pass2
    );
}

// ============================================================================
// POST /api/setup/validate-path — path validation
// ============================================================================

#[tokio::test]
async fn test_validate_path_existing_directory_returns_valid_true() {
    let state = build_test_app_state().await;

    let (status, body) =
        post_empty_setup(create_router(state), "/api/setup/validate-path?path=/tmp").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "validate-path must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["valid"], true, "/tmp must be valid. Body: {}", body);
    assert_eq!(json["exists"], true, "/tmp must exist. Body: {}", body);
    assert_eq!(
        json["writable"], true,
        "/tmp must be writable. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_validate_path_response_has_all_required_fields() {
    let state = build_test_app_state().await;

    let (status, body) =
        post_empty_setup(create_router(state), "/api/setup/validate-path?path=/tmp").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        json.get("valid").is_some(),
        "Response must have 'valid'. Body: {}",
        body
    );
    assert!(
        json.get("exists").is_some(),
        "Response must have 'exists'. Body: {}",
        body
    );
    assert!(
        json.get("writable").is_some(),
        "Response must have 'writable'. Body: {}",
        body
    );
    assert!(
        json.get("message").is_some(),
        "Response must have 'message'. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_validate_path_nonexistent_with_existing_parent_is_valid() {
    let state = build_test_app_state().await;

    // /tmp exists (parent) but this subdirectory does not
    let uri = "/api/setup/validate-path?path=/tmp/kubarr_ext_test_nonexistent_xyz_12345";
    let (status, body) = post_empty_setup(create_router(state), uri).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        !json["exists"].as_bool().unwrap_or(true),
        "Path must not exist. Body: {}",
        body
    );
    assert!(
        json["valid"].as_bool().unwrap_or(false),
        "Path with existing parent must be valid (can be created). Body: {}",
        body
    );
}

#[tokio::test]
async fn test_validate_path_deeply_nested_nonexistent_is_invalid() {
    let state = build_test_app_state().await;

    let uri = "/api/setup/validate-path?path=/nonexistent_root_xyz_ext/a/b/c";
    let (status, body) = post_empty_setup(create_router(state), uri).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        !json["valid"].as_bool().unwrap_or(true),
        "Deeply nested path must be invalid. Body: {}",
        body
    );
    assert!(
        !json["exists"].as_bool().unwrap_or(true),
        "Deeply nested path must not exist. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_validate_path_message_describes_status() {
    let state = build_test_app_state().await;

    let (_, body) =
        post_empty_setup(create_router(state), "/api/setup/validate-path?path=/tmp").await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    let message = json["message"].as_str().unwrap_or("");
    assert!(
        !message.is_empty(),
        "Validate-path message must not be empty"
    );
}

// ============================================================================
// GET /api/setup/browse — directory browser
// ============================================================================

#[tokio::test]
async fn test_browse_tmp_returns_ok_before_setup() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/browse?path=/tmp").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "browse /tmp must return 200 before setup. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_browse_response_structure() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/browse?path=/tmp").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        json.get("path").is_some(),
        "browse must have 'path'. Body: {}",
        body
    );
    assert!(
        json.get("directories").is_some(),
        "browse must have 'directories'. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_browse_path_in_response_matches_requested() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/browse?path=/tmp").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    let returned_path = json["path"].as_str().unwrap_or("");
    // The returned path should be /tmp or its canonical form
    assert!(
        returned_path.contains("tmp"),
        "browse path must match requested /tmp. Got: {}",
        returned_path
    );
}

#[tokio::test]
async fn test_browse_directories_is_array() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/browse?path=/tmp").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        json["directories"].is_array(),
        "browse directories must be an array. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_browse_directory_entries_have_name_and_path() {
    let state = build_test_app_state().await;
    // Use / to get some directories
    let (status, body) = get_setup(create_router(state), "/api/setup/browse?path=/").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    let directories = json["directories"].as_array().unwrap();
    for dir in directories {
        assert!(
            dir.get("name").is_some(),
            "Each directory entry must have 'name'. Got: {:?}",
            dir
        );
        assert!(
            dir.get("path").is_some(),
            "Each directory entry must have 'path'. Got: {:?}",
            dir
        );
    }
}

#[tokio::test]
async fn test_browse_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "browse_admin_ext",
        "browse_admin_ext@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get_setup(create_router(state), "/api/setup/browse?path=/tmp").await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "browse must return 403 after admin creation. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_browse_relative_path_returns_bad_request() {
    let state = build_test_app_state().await;
    let (status, _) = get_setup(create_router(state), "/api/setup/browse?path=relative/path").await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "browse with relative path must return 400"
    );
}

#[tokio::test]
async fn test_browse_nonexistent_path_returns_not_found() {
    let state = build_test_app_state().await;
    let (status, _) = get_setup(
        create_router(state),
        "/api/setup/browse?path=/nonexistent_ext_xyz_abc_12345",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "browse with nonexistent path must return 404"
    );
}

#[tokio::test]
async fn test_browse_default_path_no_crash() {
    // When path param is omitted, default is "/" — must not panic
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/browse").await;

    assert!(
        status == StatusCode::OK || status == StatusCode::FORBIDDEN,
        "browse without path must return 200 or 403, not crash. Got: {}. Body: {}",
        status,
        body
    );
}

#[tokio::test]
async fn test_browse_parent_is_none_at_root() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/browse?path=/").await;

    // Root might be forbidden depending on system, but if 200 we check the parent
    if status == StatusCode::OK {
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        // At "/", parent should be null/None
        let parent = &json["parent"];
        assert!(
            parent.is_null() || parent == &serde_json::Value::Null,
            "At '/', parent must be null. Got: {:?}. Body: {}",
            parent,
            body
        );
    }
}

// ============================================================================
// GET /api/setup/bootstrap/status
// ============================================================================

#[tokio::test]
async fn test_bootstrap_status_returns_200_before_setup() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/bootstrap/status").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "bootstrap/status must return 200. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_status_response_has_all_fields() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/bootstrap/status").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        json.get("components").is_some(),
        "Must have 'components'. Body: {}",
        body
    );
    assert!(
        json.get("complete").is_some(),
        "Must have 'complete'. Body: {}",
        body
    );
    assert!(
        json.get("started").is_some(),
        "Must have 'started'. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_status_components_array_not_empty() {
    let state = build_test_app_state().await;
    let (_, body) = get_setup(create_router(state), "/api/setup/bootstrap/status").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let components = json["components"]
        .as_array()
        .expect("components must be array");

    assert!(
        !components.is_empty(),
        "Components list must not be empty before bootstrap starts"
    );
}

#[tokio::test]
async fn test_bootstrap_status_complete_false_initially() {
    let state = build_test_app_state().await;
    let (_, body) = get_setup(create_router(state), "/api/setup/bootstrap/status").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["complete"], false,
        "Bootstrap must not be complete initially. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_status_started_false_initially() {
    let state = build_test_app_state().await;
    let (_, body) = get_setup(create_router(state), "/api/setup/bootstrap/status").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["started"], false,
        "Bootstrap must not be started initially. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_status_components_have_status_pending() {
    let state = build_test_app_state().await;
    let (_, body) = get_setup(create_router(state), "/api/setup/bootstrap/status").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let components = json["components"].as_array().unwrap();

    for component in components {
        let component_status = component["status"].as_str().unwrap_or("unknown");
        assert_eq!(
            component_status,
            "pending",
            "Component '{:?}' must have status 'pending' before bootstrap starts",
            component.get("component")
        );
    }
}

#[tokio::test]
async fn test_bootstrap_status_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "bootstrap_status_admin",
        "bootstrap_status_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get_setup(create_router(state), "/api/setup/bootstrap/status").await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bootstrap/status must return 403 after admin creation. Body: {}",
        body
    );
}

// ============================================================================
// POST /api/setup/bootstrap/start — starts bootstrap
// ============================================================================

#[tokio::test]
async fn test_bootstrap_start_returns_200() {
    let state = build_test_app_state().await;
    let (status, body) = post_empty_setup(create_router(state), "/api/setup/bootstrap/start").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "bootstrap/start must return 200. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_start_response_has_message_and_started() {
    let state = build_test_app_state().await;
    let (status, body) = post_empty_setup(create_router(state), "/api/setup/bootstrap/start").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        json.get("message").is_some(),
        "bootstrap/start must have 'message'. Body: {}",
        body
    );
    assert!(
        json.get("started").is_some(),
        "bootstrap/start must have 'started'. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_start_started_is_true() {
    let state = build_test_app_state().await;
    let (_, body) = post_empty_setup(create_router(state), "/api/setup/bootstrap/start").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["started"], true,
        "bootstrap/start response must have started=true on first call. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_start_message_is_not_empty() {
    let state = build_test_app_state().await;
    let (_, body) = post_empty_setup(create_router(state), "/api/setup/bootstrap/start").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let message = json["message"].as_str().unwrap_or("");
    assert!(
        !message.is_empty(),
        "bootstrap/start message must not be empty"
    );
}

// ============================================================================
// POST /api/setup/bootstrap/retry/{component} — retry a bootstrap component
// ============================================================================

#[tokio::test]
async fn test_bootstrap_retry_postgresql_returns_200() {
    let state = build_test_app_state().await;
    let (status, body) = post_empty_setup(
        create_router(state),
        "/api/setup/bootstrap/retry/postgresql",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "bootstrap/retry/postgresql must return 200. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_retry_response_has_message_and_started() {
    let state = build_test_app_state().await;
    let (_, body) = post_empty_setup(
        create_router(state),
        "/api/setup/bootstrap/retry/postgresql",
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "retry must have 'message'. Body: {}",
        body
    );
    assert!(
        json.get("started").is_some(),
        "retry must have 'started'. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_retry_message_contains_component_name() {
    let state = build_test_app_state().await;
    let (_, body) = post_empty_setup(
        create_router(state),
        "/api/setup/bootstrap/retry/my-component",
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let message = json["message"].as_str().unwrap_or("");
    assert!(
        message.contains("my-component"),
        "retry message must contain component name. Got: '{}'",
        message
    );
}

#[tokio::test]
async fn test_bootstrap_retry_different_components_all_succeed() {
    let state = build_test_app_state().await;

    for component in ["postgresql", "helm", "kubeconfig", "cert-manager"] {
        let uri = format!("/api/setup/bootstrap/retry/{}", component);
        let (status, body) = post_empty_setup(create_router(state.clone()), &uri).await;

        assert_eq!(
            status,
            StatusCode::OK,
            "bootstrap/retry/{} must return 200. Body: {}",
            component,
            body
        );
    }
}

#[tokio::test]
async fn test_bootstrap_retry_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "retry_admin_ext",
        "retry_admin_ext@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = post_empty_setup(
        create_router(state),
        "/api/setup/bootstrap/retry/postgresql",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bootstrap/retry must return 403 after admin creation. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/setup/server — get server config
// ============================================================================

#[tokio::test]
async fn test_get_server_config_returns_null_before_configuration() {
    let state = build_test_app_state().await;
    let (status, body) = get_setup(create_router(state), "/api/setup/server").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/setup/server must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_null(),
        "Server config must be null before configuration. Got: {}",
        body
    );
}

#[tokio::test]
async fn test_get_server_config_returns_config_after_configure() {
    let state = build_test_app_state().await;

    // Configure server first
    let config_body = serde_json::json!({
        "name": "MyServer",
        "storage_path": "/tmp"
    })
    .to_string();

    let (config_status, _) = post_json_setup(
        create_router(state.clone()),
        "/api/setup/server",
        &config_body,
    )
    .await;
    assert_eq!(
        config_status,
        StatusCode::OK,
        "Server configuration must succeed"
    );

    // Now get server config
    let (status, body) = get_setup(create_router(state), "/api/setup/server").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        !json.is_null(),
        "Server config must not be null after configuration. Body: {}",
        body
    );
    assert_eq!(json["name"], "MyServer", "Server name must match");
    assert_eq!(json["storage_path"], "/tmp", "Storage path must match");
}

// ============================================================================
// POST /api/setup/server — configure server
// ============================================================================

#[tokio::test]
async fn test_configure_server_with_valid_path_returns_ok() {
    let state = build_test_app_state().await;

    let body = serde_json::json!({
        "name": "MyServer",
        "storage_path": "/tmp"
    })
    .to_string();

    let (status, response_body) =
        post_json_setup(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "configure_server with /tmp must return 200. Body: {}",
        response_body
    );

    let json: serde_json::Value = serde_json::from_str(&response_body).unwrap();
    assert_eq!(json["name"], "MyServer");
    assert_eq!(json["storage_path"], "/tmp");
}

#[tokio::test]
async fn test_configure_server_response_has_name_and_storage_path() {
    let state = build_test_app_state().await;

    let body = serde_json::json!({
        "name": "StorageServer",
        "storage_path": "/tmp"
    })
    .to_string();

    let (status, response_body) =
        post_json_setup(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&response_body).unwrap();

    assert!(
        json.get("name").is_some(),
        "Response must have 'name'. Body: {}",
        response_body
    );
    assert!(
        json.get("storage_path").is_some(),
        "Response must have 'storage_path'. Body: {}",
        response_body
    );
}

#[tokio::test]
async fn test_configure_server_invalid_path_returns_400() {
    let state = build_test_app_state().await;

    let body = serde_json::json!({
        "name": "BadServer",
        "storage_path": "/absolutely_nonexistent_xyz_ext_test/deep"
    })
    .to_string();

    let (status, _) = post_json_setup(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "configure_server with invalid path must return 400"
    );
}

#[tokio::test]
async fn test_configure_server_missing_fields_returns_422() {
    let state = build_test_app_state().await;

    // Missing storage_path
    let body = serde_json::json!({"name": "MyServer"}).to_string();

    let (status, _) = post_json_setup(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "configure_server with missing fields must return 422"
    );
}

#[tokio::test]
async fn test_configure_server_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "server_admin_ext",
        "server_admin_ext@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "name": "TestServer",
        "storage_path": "/tmp"
    })
    .to_string();

    let (status, response_body) =
        post_json_setup(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "configure_server must return 403 after admin creation. Body: {}",
        response_body
    );
}

// ============================================================================
// POST /api/setup/initialize — full workflow: configure server then initialize
// ============================================================================

#[tokio::test]
async fn test_initialize_without_server_config_returns_400() {
    // initialize_setup requires server config to exist first
    let state = build_test_app_state().await;

    let body = serde_json::json!({
        "admin_username": "admin",
        "admin_email": "admin@example.com",
        "admin_password": "supersecure123"
    })
    .to_string();

    let (status, response_body) =
        post_json_setup(create_router(state), "/api/setup/initialize", &body).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "initialize without server config must return 400. Body: {}",
        response_body
    );
}

#[tokio::test]
async fn test_initialize_forbidden_when_admin_already_exists() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "existing_admin_ext",
        "existing_admin_ext@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "admin_username": "another_admin",
        "admin_email": "another@example.com",
        "admin_password": "password123"
    })
    .to_string();

    let (status, response_body) =
        post_json_setup(create_router(state), "/api/setup/initialize", &body).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "initialize must return 403 when admin already exists. Body: {}",
        response_body
    );
}

#[tokio::test]
async fn test_initialize_missing_fields_returns_422() {
    let state = build_test_app_state().await;

    // Missing admin_password
    let body = serde_json::json!({
        "admin_username": "admin",
        "admin_email": "admin@example.com"
    })
    .to_string();

    let (status, _) = post_json_setup(create_router(state), "/api/setup/initialize", &body).await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "initialize with missing fields must return 422"
    );
}

#[tokio::test]
async fn test_initialize_with_server_config_creates_admin_user() {
    let state = build_test_app_state().await;

    // Step 1: Configure server
    let server_body = serde_json::json!({
        "name": "InitTestServer",
        "storage_path": "/tmp"
    })
    .to_string();

    let (server_status, server_response) = post_json_setup(
        create_router(state.clone()),
        "/api/setup/server",
        &server_body,
    )
    .await;
    assert_eq!(
        server_status,
        StatusCode::OK,
        "Server config must succeed. Body: {}",
        server_response
    );

    // Step 2: Initialize setup (create admin user)
    let init_body = serde_json::json!({
        "admin_username": "testadminuser",
        "admin_email": "testadmin@example.com",
        "admin_password": "SuperSecret123!"
    })
    .to_string();

    let (status, response_body) =
        post_json_setup(create_router(state), "/api/setup/initialize", &init_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "initialize with valid data must return 200. Body: {}",
        response_body
    );

    let json: serde_json::Value = serde_json::from_str(&response_body).unwrap();
    assert_eq!(
        json["success"], true,
        "initialize must return success=true. Body: {}",
        response_body
    );
    assert!(
        json.get("data").is_some(),
        "Response must have data field. Body: {}",
        response_body
    );
}

#[tokio::test]
async fn test_initialize_response_contains_admin_user_info() {
    let state = build_test_app_state().await;

    // Configure server first
    let server_body = serde_json::json!({
        "name": "InfoTestServer",
        "storage_path": "/tmp"
    })
    .to_string();
    post_json_setup(
        create_router(state.clone()),
        "/api/setup/server",
        &server_body,
    )
    .await;

    // Initialize
    let init_body = serde_json::json!({
        "admin_username": "info_test_admin",
        "admin_email": "info_test@example.com",
        "admin_password": "SuperSecret456!"
    })
    .to_string();

    let (status, response_body) =
        post_json_setup(create_router(state), "/api/setup/initialize", &init_body).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&response_body).unwrap();

    let admin_user = &json["data"]["admin_user"];
    assert_eq!(
        admin_user["username"], "info_test_admin",
        "Response must include username. Body: {}",
        response_body
    );
    assert_eq!(
        admin_user["email"], "info_test@example.com",
        "Response must include email. Body: {}",
        response_body
    );
}

#[tokio::test]
async fn test_initialize_second_call_returns_forbidden() {
    let state = build_test_app_state().await;

    // Configure server
    let server_body = serde_json::json!({
        "name": "DoubleInitServer",
        "storage_path": "/tmp"
    })
    .to_string();
    post_json_setup(
        create_router(state.clone()),
        "/api/setup/server",
        &server_body,
    )
    .await;

    // First initialize
    let init_body = serde_json::json!({
        "admin_username": "first_admin",
        "admin_email": "first@example.com",
        "admin_password": "FirstPass123!"
    })
    .to_string();
    let (first_status, first_body) = post_json_setup(
        create_router(state.clone()),
        "/api/setup/initialize",
        &init_body,
    )
    .await;
    assert_eq!(
        first_status,
        StatusCode::OK,
        "First init must succeed. Body: {}",
        first_body
    );

    // Second initialize attempt (should be forbidden)
    let init_body2 = serde_json::json!({
        "admin_username": "second_admin",
        "admin_email": "second@example.com",
        "admin_password": "SecondPass123!"
    })
    .to_string();
    let (second_status, second_body) =
        post_json_setup(create_router(state), "/api/setup/initialize", &init_body2).await;

    assert_eq!(
        second_status,
        StatusCode::FORBIDDEN,
        "Second initialize must return 403. Body: {}",
        second_body
    );
}

// ============================================================================
// GET /api/setup/required — edge cases
// ============================================================================

#[tokio::test]
async fn test_setup_required_returns_setup_required_false_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "req_admin_ext",
        "req_admin_ext@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get_setup(create_router(state), "/api/setup/required").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["setup_required"], false,
        "setup_required must be false after admin creation. Body: {}",
        body
    );
    assert_eq!(
        json["database_pending"], false,
        "database_pending must be false when DB is connected. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_setup_required_accessible_always_even_after_setup() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "req_always_admin",
        "req_always_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (status, _) = get_setup(create_router(state), "/api/setup/required").await;

    // Must never return 403 — this endpoint is always accessible
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "GET /api/setup/required must never return 403"
    );
    assert_eq!(status, StatusCode::OK);
}

// ============================================================================
// validate-path: file exists but is not a directory (line 416 in setup.rs)
// ============================================================================

#[tokio::test]
async fn test_validate_path_file_not_directory_returns_invalid() {
    let state = build_test_app_state().await;

    // /etc/hostname exists on every Linux system as a regular file, not a dir
    let uri = "/api/setup/validate-path?path=/etc/hostname";
    let (status, body) = post_empty_setup(create_router(state), uri).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "validate-path must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["exists"], true,
        "/etc/hostname must exist. Body: {}",
        body
    );
    assert_eq!(
        json["valid"], false,
        "/etc/hostname is a file, not a directory — must be invalid. Body: {}",
        body
    );
    assert_eq!(
        json["writable"], false,
        "File path must not be writable. Body: {}",
        body
    );
    let message = json["message"].as_str().unwrap_or("");
    assert!(
        message.contains("not a directory"),
        "Message must explain the path is not a directory. Got: {}",
        message
    );
}

// ============================================================================
// validate-path: absolute path with no parent (line 432 in setup.rs)
// ============================================================================

#[tokio::test]
async fn test_validate_path_root_slash_that_exists_is_valid_dir() {
    let state = build_test_app_state().await;

    // "/" itself exists and IS a directory — should report valid
    let uri = "/api/setup/validate-path?path=/";
    let (status, body) = post_empty_setup(create_router(state), uri).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "validate-path must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // "/" exists and is a directory, so valid = true
    assert_eq!(json["exists"], true);
    assert_eq!(json["valid"], true);
}

// ============================================================================
// admin_user_exists: no-database path (setup.rs line 85)
// ============================================================================

#[tokio::test]
async fn test_generate_credentials_with_no_db_succeeds() {
    // Build a state with NO database connection.
    // admin_user_exists will hit the `None => return Ok(false)` branch (line 85
    // in setup.rs) and treat setup as incomplete, allowing credentials to be
    // generated without any DB.
    let state = build_test_app_state_no_db().await;

    let (status, body) = get_setup(create_router(state), "/api/setup/generate-credentials").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "generate-credentials must return 200 when db is absent (setup not complete). Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("admin_username").is_some(),
        "Response must include 'admin_username'. Body: {}",
        body
    );
    assert!(
        json.get("admin_email").is_some(),
        "Response must include 'admin_email'. Body: {}",
        body
    );
    assert!(
        json.get("admin_password").is_some(),
        "Response must include 'admin_password'. Body: {}",
        body
    );
}
