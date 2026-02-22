//! Health endpoint integration tests
//!
//! Covers:
//! - `GET /api/health` — always returns HTTP 200 "OK"
//! - `GET /api/system/health` — returns JSON with `status`, `ready`, `setup_required`
//! - `GET /api/system/version` — returns version metadata

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
// GET /api/health
// ============================================================================

#[tokio::test]
async fn test_health_check_returns_200() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_health_check_returns_ok_body() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(std::str::from_utf8(&body).unwrap(), "OK");
}

// ============================================================================
// GET /api/system/health
// ============================================================================

#[tokio::test]
async fn test_system_health_returns_200() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/system/health")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_system_health_returns_status_ok() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/system/health")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok", "System health status must be 'ok'");
}

#[tokio::test]
async fn test_system_health_setup_required_when_no_admin() {
    // Fresh seeded DB has no admin user — setup is required
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/system/health")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["setup_required"], true,
        "setup_required must be true when no admin user exists"
    );
    assert_eq!(
        json["ready"], false,
        "ready must be false when setup is required"
    );
}

#[tokio::test]
async fn test_system_health_not_setup_required_when_admin_exists() {
    let db = create_test_db_with_seed().await;

    // Create an admin user to mark setup as complete
    create_test_user_with_role(&db, "admin", "admin@example.com", "password", "admin").await;

    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/system/health")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        json["setup_required"], false,
        "setup_required must be false when admin user exists"
    );
    assert_eq!(
        json["ready"], true,
        "ready must be true when admin user exists"
    );
}

// ============================================================================
// GET /api/system/version
// ============================================================================

#[tokio::test]
async fn test_version_endpoint_returns_200() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/system/version")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_version_endpoint_returns_json_with_version_field() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/system/version")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(
        json.get("version").is_some(),
        "Response must have a 'version' field"
    );
    assert!(
        json.get("backend").is_some(),
        "Response must have a 'backend' field"
    );
    assert_eq!(json["backend"], "rust");
}

// ============================================================================
// GET /api/openapi.json
// ============================================================================

#[tokio::test]
async fn test_openapi_json_returns_200() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/openapi.json")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_openapi_json_contains_openapi_field() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/openapi.json")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json.get("openapi").is_some(),
        "OpenAPI JSON must have an 'openapi' field"
    );
}

// ============================================================================
// GET /api/docs
// ============================================================================

#[tokio::test]
async fn test_swagger_ui_returns_200() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/docs")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_swagger_ui_returns_html() {
    let state = build_test_app_state().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/docs")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/html"),
        "Expected HTML content type"
    );
}

// ============================================================================
// GET /api/system/health — Err(_) branch when db is unavailable (mod.rs line 292)
// ============================================================================

#[tokio::test]
async fn test_system_health_no_db_returns_setup_required() {
    let state = build_test_app_state_no_db().await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/system/health")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "GET /api/system/health must return 200 even without DB"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&body)).unwrap();

    assert_eq!(json["status"], "ok", "status must be 'ok' even without DB");
    assert_eq!(
        json["ready"], false,
        "ready must be false when no DB (no admin)"
    );
    assert_eq!(
        json["setup_required"], true,
        "setup_required must be true when no DB"
    );
}
