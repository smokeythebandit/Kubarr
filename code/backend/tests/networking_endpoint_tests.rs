//! Networking endpoint integration tests
//!
//! Covers endpoints under `/api/networking`:
//! - `GET /api/networking/topology`  — network topology (requires networking.view)
//! - `GET /api/networking/stats`     — per-app stats (requires networking.view)
//!
//! When K8s client is None (which it always is in tests), both topology and
//! stats degrade gracefully, returning empty collections. This lets us cover
//! the "happy path" handler code with a real DB but no K8s.

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
                .expect("Failed to init JWT keys");
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
    let body = serde_json::json!({"username": username, "password": password}).to_string();
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
            if (s.starts_with("kubarr_session_0=")
                || (s.starts_with("kubarr_session=") && !s.contains("kubarr_session_")))
                && !s.contains("Max-Age=0")
            {
                Some(s.split(';').next().unwrap().to_string())
            } else {
                None
            }
        });
    (status, cookie)
}

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

// ============================================================================
// Unauthenticated access tests
// ============================================================================

#[tokio::test]
async fn test_topology_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/networking/topology")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_stats_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/networking/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Permission tests (viewer role has no networking.view)
// ============================================================================

#[tokio::test]
async fn test_topology_requires_networking_view_permission() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "netviewer1",
        "netviewer1@test.com",
        "pass123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "netviewer1", "pass123").await;
    let cookie = cookie.expect("viewer login must succeed");

    let (status, _) =
        authenticated_get(create_router(state), "/api/networking/topology", &cookie).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer role has no networking.view"
    );
}

#[tokio::test]
async fn test_stats_requires_networking_view_permission() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "netviewer2",
        "netviewer2@test.com",
        "pass123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "netviewer2", "pass123").await;
    let cookie = cookie.expect("viewer login must succeed");

    let (status, _) =
        authenticated_get(create_router(state), "/api/networking/stats", &cookie).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer role has no networking.view"
    );
}

// ============================================================================
// Admin access - K8s is None, should degrade gracefully
// ============================================================================

async fn create_admin_with_networking_permission(
    username: &str,
    email: &str,
) -> (axum::Router, String) {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    // Add networking.view permission to admin role
    use kubarr::models::prelude::*;
    use kubarr::models::{role, role_permission};
    use sea_orm::{ActiveModelTrait, Set};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(&db)
        .await
        .unwrap()
        .expect("admin role must exist");

    let perm = role_permission::ActiveModel {
        role_id: Set(admin_role.id),
        permission: Set("networking.view".to_string()),
        ..Default::default()
    };
    let _ = perm.insert(&db).await;

    create_test_user_with_role(&db, username, email, "pass123", "admin").await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (_, cookie) = do_login(app.clone(), username, "pass123").await;
    let cookie = cookie.expect("admin login must succeed");
    (app, cookie)
}

#[tokio::test]
async fn test_topology_returns_empty_when_k8s_unavailable() {
    let (app, cookie) =
        create_admin_with_networking_permission("netadmin1", "netadmin1@test.com").await;

    let (status, body) = authenticated_get(app, "/api/networking/topology", &cookie).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "topology must return 200 when K8s is unavailable (empty result)"
    );

    let json: serde_json::Value = serde_json::from_str(&body).expect("must be valid JSON");
    assert!(
        json.get("nodes").is_some(),
        "response must have 'nodes' field"
    );
    assert!(
        json.get("edges").is_some(),
        "response must have 'edges' field"
    );
    assert!(
        json["nodes"].as_array().unwrap().is_empty(),
        "nodes must be empty when K8s unavailable"
    );
    assert!(
        json["edges"].as_array().unwrap().is_empty(),
        "edges must be empty when K8s unavailable"
    );
}

#[tokio::test]
async fn test_stats_returns_empty_when_k8s_unavailable() {
    let (app, cookie) =
        create_admin_with_networking_permission("netadmin2", "netadmin2@test.com").await;

    let (status, body) = authenticated_get(app, "/api/networking/stats", &cookie).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "stats must return 200 when K8s is unavailable (empty result)"
    );

    let json: serde_json::Value = serde_json::from_str(&body).expect("must be valid JSON");
    assert!(json.is_array(), "stats response must be an array");
    assert!(
        json.as_array().unwrap().is_empty(),
        "stats must be empty when K8s unavailable"
    );
}

#[tokio::test]
async fn test_topology_response_structure_with_empty_k8s() {
    let (app, cookie) =
        create_admin_with_networking_permission("netadmin3", "netadmin3@test.com").await;

    let (status, body) = authenticated_get(app, "/api/networking/topology", &cookie).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Verify response structure
    assert!(json.is_object(), "topology must be an object");
    assert!(json["nodes"].is_array(), "nodes must be an array");
    assert!(json["edges"].is_array(), "edges must be an array");
}

#[tokio::test]
async fn test_stats_response_is_array_with_empty_k8s() {
    let (app, cookie) =
        create_admin_with_networking_permission("netadmin4", "netadmin4@test.com").await;

    let (status, body) = authenticated_get(app, "/api/networking/stats", &cookie).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array(), "stats must be an array");
}

// ============================================================================
// Auth enforcement for all networking routes
// ============================================================================

#[tokio::test]
async fn test_networking_ws_endpoint_exists() {
    // The /api/networking/ws WebSocket endpoint should return 400 or 426 (Upgrade Required)
    // when accessed without a WebSocket upgrade header.
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/networking/ws")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Without auth, should return 401
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
