//! Setup and bootstrap flow integration tests
//!
//! Covers:
//! - `GET /api/setup/required` — always accessible; reports whether setup is needed
//! - `GET /api/setup/status` — accessible before setup, 403 after admin created
//! - `GET /api/setup/generate-credentials` — accessible before setup, 403 after
//! - `GET /api/setup/bootstrap/status` — accessible before setup, 403 after
//! - Self-disabling: all protected setup endpoints return 403 after first admin creation

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

// ============================================================================
// GET /api/setup/required
// ============================================================================

#[tokio::test]
async fn test_setup_required_returns_200() {
    let state = build_test_app_state().await;
    let (status, _) = get(create_router(state), "/api/setup/required").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/setup/required must return 200"
    );
}

#[tokio::test]
async fn test_setup_required_true_when_no_admin() {
    let state = build_test_app_state().await;
    let (status, body) = get(create_router(state), "/api/setup/required").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["setup_required"], true,
        "setup_required must be true when no admin user exists. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_setup_required_false_when_admin_exists() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin", "admin@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get(create_router(state), "/api/setup/required").await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["setup_required"], false,
        "setup_required must be false when admin user exists. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_setup_required_accessible_after_admin_creation() {
    // /api/setup/required must ALWAYS be accessible — it is intentionally exempt from
    // the self-disabling guard so the frontend can redirect to the dashboard.
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin2", "admin2@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (status, _) = get(create_router(state), "/api/setup/required").await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "/api/setup/required must remain accessible (not 403) after admin creation"
    );
    assert_eq!(status, StatusCode::OK);
}

// ============================================================================
// GET /api/setup/status
// ============================================================================

#[tokio::test]
async fn test_setup_status_accessible_before_admin_creation() {
    let state = build_test_app_state().await;
    let (status, _) = get(create_router(state), "/api/setup/status").await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "GET /api/setup/status must not return 403 before admin exists"
    );
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_setup_status_returns_setup_required_true_before_admin() {
    let state = build_test_app_state().await;
    let (_, body) = get(create_router(state), "/api/setup/status").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["setup_required"], true,
        "setup_required must be true before admin creation. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_setup_status_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin3", "admin3@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get(create_router(state), "/api/setup/status").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "GET /api/setup/status must return 403 after admin creation. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/setup/generate-credentials
// ============================================================================

#[tokio::test]
async fn test_generate_credentials_accessible_before_admin_creation() {
    let state = build_test_app_state().await;
    let (status, _) = get(create_router(state), "/api/setup/generate-credentials").await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "GET /api/setup/generate-credentials must not return 403 before admin exists"
    );
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_generate_credentials_returns_credentials() {
    let state = build_test_app_state().await;
    let (_, body) = get(create_router(state), "/api/setup/generate-credentials").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("admin_username").is_some(),
        "generate-credentials must include admin_username. Body: {}",
        body
    );
    assert!(
        json.get("admin_password").is_some(),
        "generate-credentials must include admin_password. Body: {}",
        body
    );
    assert!(
        json.get("admin_email").is_some(),
        "generate-credentials must include admin_email. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_generate_credentials_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin4", "admin4@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get(create_router(state), "/api/setup/generate-credentials").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "GET /api/setup/generate-credentials must return 403 after admin creation. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/setup/bootstrap/status
// ============================================================================

#[tokio::test]
async fn test_bootstrap_status_accessible_before_admin_creation() {
    let state = build_test_app_state().await;
    let (status, _) = get(create_router(state), "/api/setup/bootstrap/status").await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "GET /api/setup/bootstrap/status must not return 403 before admin exists"
    );
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_bootstrap_status_returns_components_list() {
    let state = build_test_app_state().await;
    let (_, body) = get(create_router(state), "/api/setup/bootstrap/status").await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("components").is_some(),
        "bootstrap/status must include components. Body: {}",
        body
    );
    assert!(
        json.get("complete").is_some(),
        "bootstrap/status must include complete. Body: {}",
        body
    );
    assert!(
        json.get("started").is_some(),
        "bootstrap/status must include started. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_bootstrap_status_forbidden_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin5", "admin5@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    let (status, body) = get(create_router(state), "/api/setup/bootstrap/status").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "GET /api/setup/bootstrap/status must return 403 after admin creation. Body: {}",
        body
    );
}

// ============================================================================
// Self-disabling: multiple setup endpoints return 403 together after admin creation
// ============================================================================

#[tokio::test]
async fn test_all_protected_setup_endpoints_return_403_after_admin_creation() {
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "admin6", "admin6@example.com", "password", "admin").await;
    let state = build_test_app_state_with_db(db).await;

    // All of these are self-disabling: they return 403 once an admin user exists.
    let protected_setup_endpoints = vec![
        "/api/setup/status",
        "/api/setup/generate-credentials",
        "/api/setup/bootstrap/status",
    ];

    for endpoint in protected_setup_endpoints {
        let (status, body) = get(create_router(state.clone()), endpoint).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Setup endpoint {} must return 403 after admin creation. Body: {}",
            endpoint,
            body
        );
        // The error body must mention setup completion
        assert!(
            body.contains("Setup")
                || body.contains("setup")
                || body.contains("completed")
                || body.contains("complete"),
            "Error message for {} must reference setup completion. Body: {}",
            endpoint,
            body
        );
    }
}
