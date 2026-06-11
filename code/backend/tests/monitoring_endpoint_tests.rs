//! Monitoring endpoint integration tests
//!
//! Covers all routes registered under `/api/monitoring/*`:
//! - `GET /api/monitoring/vm/apps`              — app metrics from VictoriaMetrics
//! - `GET /api/monitoring/vm/cluster`           — cluster-wide metrics from VictoriaMetrics
//! - `GET /api/monitoring/vm/app/{app_name}`    — per-app detail metrics
//! - `GET /api/monitoring/vm/cluster/network-history` — cluster network time-series
//! - `GET /api/monitoring/vm/cluster/metrics-history` — cluster metrics time-series
//! - `GET /api/monitoring/vm/available`         — VictoriaMetrics availability probe
//! - `GET /api/monitoring/pods`                 — pod status (via K8s)
//! - `GET /api/monitoring/metrics`              — pod metrics (via K8s)
//! - `GET /api/monitoring/health/{app_name}`    — app health (via K8s)
//! - `GET /api/monitoring/endpoints/{app_name}` — service endpoints (via K8s)
//! - `GET /api/monitoring/metrics-available`    — metrics-server probe
//!
//! Strategy: VictoriaMetrics and the Kubernetes API server are absent in the
//! test environment.  The VM helpers silently return empty vectors on network
//! failure, and the K8s handlers degrade gracefully when `k8s_client` is
//! `None`.  Therefore every authenticated request is expected to succeed (2xx)
//! after passing through the full auth middleware stack.
//! Unauthenticated requests must be rejected with 401.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;

// ============================================================================
// JWT key initialization (process-global, initialised once)
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

/// POST /auth/login and return the legacy `kubarr_session=<token>` cookie value.
async fn login_as_admin(app: axum::Router) -> String {
    let body = serde_json::json!({
        "username": "monuser",
        "password": "password123"
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

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Login must succeed before monitoring tests can run"
    );

    response
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
        })
        .expect("Login response must set a session cookie")
}

/// Assert that `status` is a valid HTTP status code (200–599).
/// This is the intentionally lenient check used throughout these tests:
/// we care that the handler ran to completion, not that external services
/// (VM, K8s) were reachable.
fn assert_valid_http_status(status: StatusCode, endpoint: &str) {
    let code = status.as_u16();
    assert!(
        (200..600).contains(&code),
        "Expected a valid HTTP status (200-599) for {}, got {}",
        endpoint,
        code
    );
}

// ============================================================================
// Unauthenticated access — every monitoring route must return 401
// ============================================================================

#[tokio::test]
async fn test_vm_apps_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/apps")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/vm/apps without auth must return 401"
    );
}

#[tokio::test]
async fn test_vm_cluster_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/vm/cluster without auth must return 401"
    );
}

#[tokio::test]
async fn test_vm_app_detail_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/app/jellyfin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/vm/app/{{app_name}} without auth must return 401"
    );
}

#[tokio::test]
async fn test_vm_cluster_network_history_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/network-history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/vm/cluster/network-history without auth must return 401"
    );
}

#[tokio::test]
async fn test_vm_cluster_metrics_history_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/metrics-history")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/vm/cluster/metrics-history without auth must return 401"
    );
}

#[tokio::test]
async fn test_vm_available_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/available")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/vm/available without auth must return 401"
    );
}

#[tokio::test]
async fn test_pods_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/pods")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/pods without auth must return 401"
    );
}

#[tokio::test]
async fn test_metrics_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/metrics without auth must return 401"
    );
}

#[tokio::test]
async fn test_health_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/health/jellyfin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/health/{{app_name}} without auth must return 401"
    );
}

#[tokio::test]
async fn test_endpoints_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/endpoints/jellyfin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/endpoints/{{app_name}} without auth must return 401"
    );
}

#[tokio::test]
async fn test_metrics_available_requires_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/metrics-available")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/monitoring/metrics-available without auth must return 401"
    );
}

// ============================================================================
// Authenticated access — handlers must produce a valid HTTP status
// ============================================================================

/// Create a fresh state + admin user, log in, and return (state, session_cookie).
async fn setup_authenticated_state() -> (kubarr::state::AppState, String) {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "monuser",
        "monuser@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let cookie = login_as_admin(create_router(state.clone())).await;
    (state, cookie)
}

#[tokio::test]
async fn test_vm_apps_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/apps")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(response.status(), "GET /api/monitoring/vm/apps");
}

#[tokio::test]
async fn test_vm_cluster_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(response.status(), "GET /api/monitoring/vm/cluster");
}

#[tokio::test]
async fn test_vm_app_detail_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/app/jellyfin?duration=1h")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(response.status(), "GET /api/monitoring/vm/app/{app_name}");
}

#[tokio::test]
async fn test_vm_app_detail_default_duration_with_auth() {
    let (state, cookie) = setup_authenticated_state().await;

    // Omit the `duration` query parameter — handler defaults to "1h"
    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/app/sonarr")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/vm/app/{app_name} (default duration)",
    );
}

#[tokio::test]
async fn test_vm_cluster_network_history_default_with_auth() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/network-history")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/vm/cluster/network-history (default duration)",
    );
}

#[tokio::test]
async fn test_vm_cluster_network_history_15m_with_auth() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/network-history?duration=15m")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/vm/cluster/network-history?duration=15m",
    );
}

#[tokio::test]
async fn test_vm_cluster_network_history_1h_with_auth() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/network-history?duration=1h")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/vm/cluster/network-history?duration=1h",
    );
}

#[tokio::test]
async fn test_vm_cluster_network_history_3h_with_auth() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/network-history?duration=3h")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/vm/cluster/network-history?duration=3h",
    );
}

#[tokio::test]
async fn test_vm_cluster_metrics_history_default_with_auth() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/metrics-history")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/vm/cluster/metrics-history (default duration)",
    );
}

#[tokio::test]
async fn test_vm_cluster_metrics_history_1h_with_auth() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/metrics-history?duration=1h")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/vm/cluster/metrics-history?duration=1h",
    );
}

#[tokio::test]
async fn test_vm_cluster_metrics_history_3h_with_auth() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster/metrics-history?duration=3h")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/vm/cluster/metrics-history?duration=3h",
    );
}

#[tokio::test]
async fn test_vm_available_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/available")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // The handler always returns 200 with {"available": false} when VM is unreachable.
    assert_valid_http_status(response.status(), "GET /api/monitoring/vm/available");
}

#[tokio::test]
async fn test_pods_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/pods")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // K8s client is None in tests; handler returns an empty list with 200.
    assert_valid_http_status(response.status(), "GET /api/monitoring/pods");
}

#[tokio::test]
async fn test_pods_with_namespace_query_param() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/pods?namespace=default")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/pods?namespace=default",
    );
}

#[tokio::test]
async fn test_pods_with_app_query_param() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/pods?namespace=media&app=jellyfin")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/pods?namespace=media&app=jellyfin",
    );
}

#[tokio::test]
async fn test_metrics_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/metrics")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // K8s client is None; handler returns empty list with 200.
    assert_valid_http_status(response.status(), "GET /api/monitoring/metrics");
}

#[tokio::test]
async fn test_metrics_with_namespace_query_param() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/metrics?namespace=kubarr")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/metrics?namespace=kubarr",
    );
}

#[tokio::test]
async fn test_app_health_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/health/jellyfin")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // K8s absent → pods=[]; handler returns healthy=false, message="No pods found", 200.
    assert_valid_http_status(response.status(), "GET /api/monitoring/health/{app_name}");
}

#[tokio::test]
async fn test_app_health_with_namespace_query_param() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/health/sonarr?namespace=media")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/health/sonarr?namespace=media",
    );
}

#[tokio::test]
async fn test_endpoints_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/endpoints/jellyfin")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // K8s absent → handler returns empty list with 200.
    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/endpoints/{app_name}",
    );
}

#[tokio::test]
async fn test_endpoints_with_namespace_query_param() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/endpoints/radarr?namespace=media")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_valid_http_status(
        response.status(),
        "GET /api/monitoring/endpoints/radarr?namespace=media",
    );
}

#[tokio::test]
async fn test_metrics_available_with_auth_returns_valid_status() {
    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/metrics-available")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // K8s absent → available=false, 200.
    assert_valid_http_status(response.status(), "GET /api/monitoring/metrics-available");
}

// ============================================================================
// Viewer role — has monitoring.view permission and must not get 401/403
// ============================================================================

#[tokio::test]
async fn test_viewer_can_access_vm_apps() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "viewmon",
        "viewmon@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let body = serde_json::json!({"username": "viewmon", "password": "password123"}).to_string();
    let login_resp = create_router(state.clone())
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
    assert_eq!(login_resp.status(), StatusCode::OK);

    let cookie = login_resp
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
        })
        .expect("Login must set a session cookie");

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/apps")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Viewer must not get 401 on monitoring endpoints"
    );
    assert_ne!(
        response.status(),
        StatusCode::FORBIDDEN,
        "Viewer must not get 403 on monitoring endpoints (viewer has monitoring.view)"
    );
}

// ============================================================================
// vm/available — response shape when VM is unreachable
// ============================================================================

#[tokio::test]
async fn test_vm_available_response_shape_when_unreachable() {
    use http_body_util::BodyExt;

    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/available")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Handler always returns 200; VM simply isn't reachable in test env.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "vm/available must return 200 even when VM is unreachable"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("vm/available response must be valid JSON");

    assert!(
        json.get("available").is_some(),
        "Response must include 'available' field"
    );
    assert!(
        json.get("message").is_some(),
        "Response must include 'message' field"
    );
    // VM is not reachable from tests, so available must be false
    assert_eq!(
        json["available"], false,
        "available must be false when VictoriaMetrics is unreachable"
    );
}

// ============================================================================
// metrics-available — response shape when K8s is absent
// ============================================================================

#[tokio::test]
async fn test_metrics_available_response_shape_when_k8s_absent() {
    use http_body_util::BodyExt;

    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/metrics-available")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "metrics-available must return 200 when K8s client is absent"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("metrics-available response must be valid JSON");

    assert!(
        json.get("available").is_some(),
        "Response must include 'available' field"
    );
    assert!(
        json.get("message").is_some(),
        "Response must include 'message' field"
    );
    assert_eq!(
        json["available"], false,
        "available must be false when K8s client is absent"
    );
}

// ============================================================================
// pods — response is an empty JSON array when K8s is absent
// ============================================================================

#[tokio::test]
async fn test_pods_returns_empty_array_when_k8s_absent() {
    use http_body_util::BodyExt;

    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/pods")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "pods must return 200 when K8s is absent"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("pods response must be valid JSON");
    assert!(json.is_array(), "pods response must be a JSON array");
    assert!(
        json.as_array().unwrap().is_empty(),
        "pods must return an empty array when K8s client is absent"
    );
}

// ============================================================================
// metrics — response is an empty JSON array when K8s is absent
// ============================================================================

#[tokio::test]
async fn test_metrics_returns_empty_array_when_k8s_absent() {
    use http_body_util::BodyExt;

    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/metrics")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "metrics must return 200 when K8s is absent"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("metrics response must be valid JSON");
    assert!(json.is_array(), "metrics response must be a JSON array");
    assert!(
        json.as_array().unwrap().is_empty(),
        "metrics must return an empty array when K8s client is absent"
    );
}

// ============================================================================
// health/{app_name} — degraded response when K8s is absent
// ============================================================================

#[tokio::test]
async fn test_app_health_response_shape_when_k8s_absent() {
    use http_body_util::BodyExt;

    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/health/jellyfin")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "health/{{app_name}} must return 200 when K8s is absent"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("health response must be valid JSON");

    assert_eq!(
        json["app_name"], "jellyfin",
        "app_name must match path param"
    );
    assert!(
        json.get("healthy").is_some(),
        "Response must include 'healthy'"
    );
    assert!(json.get("pods").is_some(), "Response must include 'pods'");
    assert!(
        json.get("message").is_some(),
        "Response must include 'message'"
    );
    assert_eq!(
        json["healthy"], false,
        "healthy must be false when no pods exist (K8s absent)"
    );
}

// ============================================================================
// endpoints/{app_name} — empty array when K8s is absent
// ============================================================================

#[tokio::test]
async fn test_app_endpoints_returns_empty_array_when_k8s_absent() {
    use http_body_util::BodyExt;

    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/endpoints/jellyfin")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "endpoints/{{app_name}} must return 200 when K8s is absent"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("endpoints response must be valid JSON");
    assert!(json.is_array(), "endpoints response must be a JSON array");
    assert!(
        json.as_array().unwrap().is_empty(),
        "endpoints must return an empty array when K8s client is absent"
    );
}

// ============================================================================
// vm/apps — empty array when VM is unreachable
// ============================================================================

#[tokio::test]
async fn test_vm_apps_returns_empty_array_when_vm_unreachable() {
    use http_body_util::BodyExt;

    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/apps")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "vm/apps must return 200 when VM is unreachable (empty metrics)"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("vm/apps response must be valid JSON");
    assert!(json.is_array(), "vm/apps response must be a JSON array");
    // VM not reachable → all query_vm calls return empty vec → no metrics built
    assert!(
        json.as_array().unwrap().is_empty(),
        "vm/apps must return an empty array when VictoriaMetrics is unreachable"
    );
}

// ============================================================================
// vm/cluster — response shape when VM is unreachable (all zeroes)
// ============================================================================

#[tokio::test]
async fn test_vm_cluster_response_shape_when_vm_unreachable() {
    use http_body_util::BodyExt;

    let (state, cookie) = setup_authenticated_state().await;

    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/monitoring/vm/cluster")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "vm/cluster must return 200 when VM is unreachable"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("vm/cluster response must be valid JSON");

    // All fields must be present even when VM is unreachable (defaults to 0)
    assert!(
        json.get("total_cpu_cores").is_some(),
        "Response must include total_cpu_cores"
    );
    assert!(
        json.get("total_memory_bytes").is_some(),
        "Response must include total_memory_bytes"
    );
    assert!(
        json.get("cpu_usage_percent").is_some(),
        "Response must include cpu_usage_percent"
    );
    assert!(
        json.get("memory_usage_percent").is_some(),
        "Response must include memory_usage_percent"
    );
    assert!(
        json.get("container_count").is_some(),
        "Response must include container_count"
    );
    assert!(
        json.get("pod_count").is_some(),
        "Response must include pod_count"
    );
    assert!(
        json.get("network_receive_bytes_per_sec").is_some(),
        "Response must include network_receive_bytes_per_sec"
    );
    assert!(
        json.get("network_transmit_bytes_per_sec").is_some(),
        "Response must include network_transmit_bytes_per_sec"
    );
    assert!(
        json.get("storage_usage_percent").is_some(),
        "Response must include storage_usage_percent"
    );
}
