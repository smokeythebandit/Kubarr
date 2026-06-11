//! Extended integration tests for `src/endpoints/setup.rs`
//!
//! Covers previously uncovered paths:
//! - `POST /api/setup/initialize`    — admin creation, forbidden after setup
//! - `POST /api/setup/validate-path` — valid/invalid/nonexistent paths
//! - `GET  /api/setup/browse`        — directory listing, error paths
//! - `POST /api/setup/bootstrap/start` — start bootstrap
//! - `POST /api/setup/bootstrap/retry/{component}` — retry component
//! - `GET  /api/setup/server`        — get server config (null before configure)
//! - `POST /api/setup/server`        — configure server, forbidden after setup
//! - `GET  /api/setup/required`      — database_pending field, no-DB path

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::util::ServiceExt;

mod common;
use common::{
    build_test_app_state, build_test_app_state_with_db, create_test_db_with_seed,
    create_test_user_with_role,
};

use kubarr::endpoints::create_router;
use kubarr::services::storage_config::{
    save_storage_config_to_db, PersistedStorageConfig, StorageMode, STORAGE_MOUNT_PATH,
};

// ============================================================================
// Helpers
// ============================================================================

async fn get(router: axum::Router, uri: &str) -> (StatusCode, String) {
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

async fn post_json(router: axum::Router, uri: &str, json: &str) -> (StatusCode, String) {
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

async fn post_empty(router: axum::Router, uri: &str) -> (StatusCode, String) {
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

async fn seed_validated_storage(db: &sea_orm::DatabaseConnection) {
    let config = PersistedStorageConfig {
        mode: StorageMode::ExternalNfs,
        mount_path: STORAGE_MOUNT_PATH.to_string(),
        uid: 1000,
        gid: 1000,
        fs_group: 1000,
        config_json: serde_json::json!({
            "server": "nas.local",
            "export_path": "/exports/media"
        }),
        validation_json: Some(serde_json::json!({
            "valid": true,
            "message": "ok",
            "checks": []
        })),
    };

    save_storage_config_to_db(db, &config)
        .await
        .expect("seed validated storage config");
}

async fn build_state_with_validated_storage() -> kubarr::state::AppState {
    let db = create_test_db_with_seed().await;
    seed_validated_storage(&db).await;
    build_test_app_state_with_db(db).await
}

// ============================================================================
// POST /api/setup/initialize
// ============================================================================

#[tokio::test]
async fn test_initialize_forbidden_when_no_server_config() {
    // initialize_setup requires a server_config row to exist first.
    // Without configure_server being called, it should return 400 Bad Request.
    let state = build_test_app_state().await;

    let body = serde_json::json!({
        "admin_username": "admin",
        "admin_email": "admin@example.com",
        "admin_password": "password123"
    })
    .to_string();

    let (status, response_body) =
        post_json(create_router(state), "/api/setup/initialize", &body).await;

    // Without server config it returns 400 (server must be configured first)
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "initialize without server config must return 400. Body: {}",
        response_body
    );
}

#[tokio::test]
async fn test_initialize_forbidden_after_admin_exists() {
    // If an admin user already exists the endpoint must return 403.
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "existing_admin", "ea@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "admin_username": "new_admin",
        "admin_email": "new@example.com",
        "admin_password": "password123"
    })
    .to_string();

    let (status, response_body) =
        post_json(create_router(state), "/api/setup/initialize", &body).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "initialize must return 403 after admin creation. Body: {}",
        response_body
    );
}

#[tokio::test]
async fn test_initialize_missing_fields_returns_422() {
    // Sending an incomplete JSON body triggers Axum's extractor validation.
    let state = build_test_app_state().await;

    let incomplete_body = serde_json::json!({
        "admin_username": "admin"
        // missing admin_email and admin_password
    })
    .to_string();

    let (status, _) = post_json(
        create_router(state),
        "/api/setup/initialize",
        &incomplete_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "initialize with missing fields must return 422"
    );
}

// ============================================================================
// POST /api/setup/validate-path
// ============================================================================

#[tokio::test]
async fn test_validate_path_existing_directory_returns_valid() {
    let state = build_test_app_state().await;

    // validate-path is a POST route with a Query extractor, so path goes in the URL
    let (status, body) =
        post_empty(create_router(state), "/api/setup/validate-path?path=/tmp").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json["exists"].as_bool().unwrap_or(false),
        "exists must be true for /tmp"
    );
    assert!(
        json["valid"].as_bool().unwrap_or(false),
        "/tmp must be a valid path"
    );
}

#[tokio::test]
async fn test_validate_path_nonexistent_path_with_existing_parent() {
    let state = build_test_app_state().await;

    // /tmp/kubarr_test_nonexistent_dir probably doesn't exist but /tmp does
    let uri = "/api/setup/validate-path?path=/tmp/kubarr_test_nonexistent_dir_xyz_abc";
    let (status, body) = post_empty(create_router(state), uri).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // exists must be false
    assert!(
        !json["exists"].as_bool().unwrap_or(true),
        "exists must be false"
    );
    // But valid=true because parent (/tmp) exists
    assert!(
        json["valid"].as_bool().unwrap_or(false),
        "Path with existing parent must be valid (can be created). Body: {}",
        body
    );
}

#[tokio::test]
async fn test_validate_path_invalid_deeply_nested_returns_not_valid() {
    let state = build_test_app_state().await;

    // Neither /nonexistent_root nor its parent exist
    let uri = "/api/setup/validate-path?path=/nonexistent_root_xyz/nested/deep";
    let (status, body) = post_empty(create_router(state), uri).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        !json["valid"].as_bool().unwrap_or(true),
        "Deeply nested nonexistent path must be invalid. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_validate_path_response_has_required_fields() {
    let state = build_test_app_state().await;

    let (status, body) =
        post_empty(create_router(state), "/api/setup/validate-path?path=/tmp").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.get("valid").is_some(), "Response must include 'valid'");
    assert!(
        json.get("exists").is_some(),
        "Response must include 'exists'"
    );
    assert!(
        json.get("writable").is_some(),
        "Response must include 'writable'"
    );
    assert!(
        json.get("message").is_some(),
        "Response must include 'message'"
    );
}

// ============================================================================
// GET /api/setup/browse
// ============================================================================

#[tokio::test]
async fn test_browse_root_returns_ok_before_setup() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/browse?path=/tmp").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "browse must return 200 before setup. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_browse_response_has_required_fields() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/browse?path=/tmp").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("path").is_some(),
        "browse must include 'path'. Body: {}",
        body
    );
    assert!(
        json.get("directories").is_some(),
        "browse must include 'directories'. Body: {}",
        body
    );
    // parent is optional (None for /)
}

#[tokio::test]
async fn test_browse_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin_browse", "ab@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get(create_router(state), "/api/setup/browse?path=/tmp").await;

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
    let (status, body) = get(create_router(state), "/api/setup/browse?path=relative/path").await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "browse with relative path must return 400. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_browse_nonexistent_path_returns_not_found() {
    let state = build_test_app_state().await;
    let (status, body) = get(
        create_router(state),
        "/api/setup/browse?path=/nonexistent_path_xyz_abc_123",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "browse with nonexistent path must return 404. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_browse_default_path_is_root() {
    // When the path query param is omitted, the server should browse "/" (default_browse_path)
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/browse").await;

    // If / is accessible, we get 200. The default path must not cause a crash.
    assert!(
        status == StatusCode::OK || status == StatusCode::FORBIDDEN,
        "browse without path param must return 200 or 403 (not a crash). Got: {}. Body: {}",
        status,
        body
    );
}

// ============================================================================
// POST /api/setup/bootstrap/start
// ============================================================================

#[tokio::test]
async fn test_bootstrap_start_returns_ok_before_setup() {
    let state = build_state_with_validated_storage().await;
    let (status, body) = post_empty(create_router(state), "/api/setup/bootstrap/start").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "bootstrap/start must return 200. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_start_response_has_started_field() {
    let state = build_state_with_validated_storage().await;
    let (status, body) = post_empty(create_router(state), "/api/setup/bootstrap/start").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("started").is_some(),
        "bootstrap/start response must include 'started'. Body: {}",
        body
    );
    assert!(
        json.get("message").is_some(),
        "bootstrap/start response must include 'message'. Body: {}",
        body
    );
}

// ============================================================================
// POST /api/setup/bootstrap/retry/{component}
// ============================================================================

#[tokio::test]
async fn test_bootstrap_retry_returns_ok_before_setup() {
    let state = build_test_app_state().await;
    let (status, body) = post_empty(
        create_router(state),
        "/api/setup/bootstrap/retry/postgresql",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "bootstrap/retry must return 200 before setup. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_retry_response_has_message_and_started() {
    let state = build_test_app_state().await;
    let (status, body) = post_empty(
        create_router(state),
        "/api/setup/bootstrap/retry/postgresql",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("message").is_some(),
        "retry response must include 'message'. Body: {}",
        body
    );
    assert!(
        json.get("started").is_some(),
        "retry response must include 'started'. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_retry_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin_retry", "ar@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = post_empty(
        create_router(state),
        "/api/setup/bootstrap/retry/postgresql",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "retry must return 403 after admin creation. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/setup/server
// ============================================================================

#[tokio::test]
async fn test_get_server_config_returns_null_when_not_configured() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/server").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/setup/server must return 200. Body: {}",
        body
    );
    // When no server config has been saved, the response is JSON null
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_null(),
        "server config must be null before configuration. Body: {}",
        body
    );
}

// ============================================================================
// POST /api/setup/server
// ============================================================================

#[tokio::test]
async fn test_configure_server_with_valid_path_returns_ok() {
    let state = build_test_app_state().await;

    // /tmp always exists as a directory
    let body = serde_json::json!({
        "name": "TestServer",
        "storage_path": "/tmp"
    })
    .to_string();

    let (status, response_body) = post_json(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "configure_server with valid /tmp must return 200. Body: {}",
        response_body
    );

    let json: serde_json::Value = serde_json::from_str(&response_body).unwrap();
    assert_eq!(json["name"], "TestServer");
    assert_eq!(json["storage_path"], "/data");
}

#[tokio::test]
async fn test_configure_server_ignores_legacy_invalid_storage_path() {
    let state = build_test_app_state().await;

    let body = serde_json::json!({
        "name": "TestServer",
        "storage_path": "/absolutely_nonexistent_root_xyz/nested"
    })
    .to_string();

    let (status, response_body) = post_json(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "configure_server ignores legacy storage_path. Body: {}",
        response_body
    );

    let json: serde_json::Value = serde_json::from_str(&response_body).unwrap();
    assert_eq!(json["storage_path"], "/data");
}

#[tokio::test]
async fn test_configure_server_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin_server", "as@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({
        "name": "TestServer",
        "storage_path": "/tmp"
    })
    .to_string();

    let (status, response_body) = post_json(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "configure_server must return 403 after admin creation. Body: {}",
        response_body
    );
}

#[tokio::test]
async fn test_configure_server_missing_fields_returns_422() {
    let state = build_test_app_state().await;

    // Missing name
    let body = serde_json::json!({
        "storage_path": "/tmp"
    })
    .to_string();

    let (status, _) = post_json(create_router(state), "/api/setup/server", &body).await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "configure_server with missing fields must return 422"
    );
}

// ============================================================================
// GET /api/setup/required — no-DB path (k8s not set, db not connected)
// ============================================================================

#[tokio::test]
async fn test_setup_required_no_db_returns_setup_required_true() {
    // When no DB is available and no K8s is configured, setup_required must be true
    use kubarr::services::audit::AuditService;
    use kubarr::services::catalog::AppCatalog;
    use kubarr::services::chart_sync::ChartSyncService;
    use kubarr::services::notification::NotificationService;
    use kubarr::state::{AppState, SharedCatalog, SharedK8sClient};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog: SharedCatalog = Arc::new(RwLock::new(AppCatalog::default()));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    // Pass None for DB — simulates pre-database state
    let state = AppState::new(None, k8s_client, catalog, chart_sync, audit, notification);

    let (status, body) = get(create_router(state), "/api/setup/required").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["setup_required"], true,
        "setup_required must be true when DB not connected. Body: {}",
        body
    );
    assert_eq!(
        json["database_pending"], false,
        "database_pending must be false when no K8s namespace exists. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/setup/status — detailed response fields
// ============================================================================

#[tokio::test]
async fn test_setup_status_response_has_all_fields() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/status").await;

    assert_eq!(status, StatusCode::OK);
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
async fn test_setup_status_admin_user_exists_false_before_setup() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/status").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["admin_user_exists"], false,
        "admin_user_exists must be false before setup. Body: {}",
        body
    );
    assert_eq!(
        json["setup_required"], true,
        "setup_required must be true before setup. Body: {}",
        body
    );
}

// ============================================================================
// Bootstrap status — detailed response structure
// ============================================================================

#[tokio::test]
async fn test_bootstrap_status_components_are_pending() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/bootstrap/status").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    let components = json["components"]
        .as_array()
        .expect("components must be an array");
    assert!(
        !components.is_empty(),
        "components list must not be empty. Body: {}",
        body
    );

    // Before bootstrap starts, all components must have a status of "pending"
    for component in components {
        let component_status = component["status"].as_str().unwrap_or("");
        assert_eq!(
            component_status, "pending",
            "Component {:?} must be pending before bootstrap starts",
            component["component"]
        );
    }
}

#[tokio::test]
async fn test_bootstrap_status_complete_false_before_start() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/bootstrap/status").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["complete"], false,
        "bootstrap must not be complete before start. Body: {}",
        body
    );
}

// ============================================================================
// Generate credentials — field format validation
// ============================================================================

#[tokio::test]
async fn test_generate_credentials_password_has_minimum_length() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/generate-credentials").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    let password = json["admin_password"].as_str().unwrap_or("");
    assert!(
        password.len() >= 16,
        "Generated password must be at least 16 characters, got: {}",
        password.len()
    );
}

#[tokio::test]
async fn test_generate_credentials_returns_expected_defaults() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/generate-credentials").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Based on the source: admin_username is always "admin"
    assert_eq!(
        json["admin_username"], "admin",
        "Default username must be 'admin'"
    );
    // admin_email is always "admin@example.com"
    assert_eq!(
        json["admin_email"], "admin@example.com",
        "Default email must be 'admin@example.com'"
    );
}
