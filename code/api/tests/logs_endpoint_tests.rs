//! Integration tests for `src/endpoints/logs.rs`
//!
//! Tests all log endpoints for:
//! - 401 Unauthorized for unauthenticated requests
//! - 403 Forbidden for authenticated users without `logs.view` permission
//! - 2xx or 5xx (not 401/403) for authenticated users with proper permissions
//!
//! Routes tested:
//! - `GET /api/logs/{pod_name}`                     — requires logs.view
//! - `GET /api/logs/app/{app_name}`                 — requires logs.view
//! - `GET /api/logs/raw/{pod_name}`                 — requires logs.view
//! - `GET /api/logs/vlogs/namespaces`               — requires logs.view (makes HTTP to VictoriaLogs)
//! - `GET /api/logs/vlogs/labels`                   — requires logs.view (makes HTTP to VictoriaLogs)
//! - `GET /api/logs/vlogs/label/{label}/values`     — requires logs.view (makes HTTP to VictoriaLogs)
//! - `GET /api/logs/vlogs/query`                    — requires logs.view (makes HTTP to VictoriaLogs)
//!
//! K8s-dependent endpoints return 500 when k8s_client is None.
//! VictoriaLogs endpoints return 503 when VictoriaLogs is not reachable.

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

/// Make an authenticated GET request and return (status, body_string).
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

/// Make an unauthenticated GET request and return (status, body_string).
async fn unauthenticated_get(app: axum::Router, uri: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

// ============================================================================
// GET /api/logs/{pod_name} — pod logs (requires K8s)
// ============================================================================

#[tokio::test]
async fn test_get_pod_logs_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/my-pod-123").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/{{pod_name}} without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_pod_logs_viewer_without_logs_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    // "downloader" role does NOT have logs.view permission
    create_test_user_with_role(
        &db,
        "logs_no_perm_user",
        "logs_no_perm@example.com",
        "password123",
        "downloader",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "logs_no_perm_user",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/my-pod-123", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User without logs.view must get 403 on GET /api/logs/{{pod_name}}"
    );
}

#[tokio::test]
async fn test_get_pod_logs_with_logs_view_returns_error_without_k8s() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    // "admin" role has logs.view (seeded in common)
    create_test_user_with_role(
        &db,
        "logs_admin_user",
        "logs_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "logs_admin_user",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/my-pod-123", &cookie).await;

    // Without K8s client, handler returns 500 (Internal Server Error)
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "Authenticated user with logs.view must not get 401"
    );
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "Authenticated user with logs.view must not get 403"
    );
    // Should be 500 because K8s is not available
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "GET /api/logs/{{pod_name}} without K8s must return 500"
    );
}

#[tokio::test]
async fn test_get_pod_logs_viewer_with_logs_view_returns_error_without_k8s() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    // "viewer" role has logs.view (seeded in common)
    create_test_user_with_role(
        &db,
        "logs_viewer_user",
        "logs_viewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "logs_viewer_user",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(create_router(state), "/api/logs/test-pod", &cookie).await;

    // Viewer has logs.view — auth succeeds, K8s not available returns 500
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Viewer with logs.view should get 500 without K8s, not 401/403"
    );
}

// ============================================================================
// GET /api/logs/app/{app_name} — app logs (requires K8s)
// ============================================================================

#[tokio::test]
async fn test_get_app_logs_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/app/jellyfin").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/app/{{app_name}} without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_app_logs_without_logs_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "app_logs_no_perm",
        "app_logs_no_perm@example.com",
        "password123",
        "downloader",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "app_logs_no_perm",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/app/jellyfin", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User without logs.view must get 403 on GET /api/logs/app/{{app_name}}"
    );
}

#[tokio::test]
async fn test_get_app_logs_with_logs_view_returns_error_without_k8s() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "app_logs_admin",
        "app_logs_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "app_logs_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/app/jellyfin", &cookie).await;

    // K8s not available → 500
    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "GET /api/logs/app/{{app_name}} without K8s must return 500"
    );
}

// ============================================================================
// GET /api/logs/raw/{pod_name} — raw pod logs (requires K8s)
// ============================================================================

#[tokio::test]
async fn test_get_raw_pod_logs_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/raw/my-pod-456").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/raw/{{pod_name}} without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_raw_pod_logs_without_logs_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "raw_logs_no_perm",
        "raw_logs_no_perm@example.com",
        "password123",
        "downloader",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "raw_logs_no_perm",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/raw/my-pod-456", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User without logs.view must get 403 on GET /api/logs/raw/{{pod_name}}"
    );
}

#[tokio::test]
async fn test_get_raw_pod_logs_with_logs_view_returns_error_without_k8s() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "raw_logs_admin",
        "raw_logs_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "raw_logs_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/raw/my-pod-456", &cookie).await;

    // K8s not available → 500
    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "GET /api/logs/raw/{{pod_name}} without K8s must return 500"
    );
}

// ============================================================================
// GET /api/logs/vlogs/namespaces — VictoriaLogs namespaces
// ============================================================================

#[tokio::test]
async fn test_vlogs_namespaces_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/vlogs/namespaces").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/vlogs/namespaces without auth must return 401"
    );
}

#[tokio::test]
async fn test_vlogs_namespaces_without_logs_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_ns_no_perm",
        "vlogs_ns_no_perm@example.com",
        "password123",
        "downloader",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_ns_no_perm",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/vlogs/namespaces", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User without logs.view must get 403 on GET /api/logs/vlogs/namespaces"
    );
}

#[tokio::test]
async fn test_vlogs_namespaces_with_logs_view_returns_503_when_victorialogs_unavailable() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_ns_admin",
        "vlogs_ns_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_ns_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/vlogs/namespaces", &cookie).await;

    // VictoriaLogs is not running in tests — expect 503 ServiceUnavailable
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "Authenticated user with logs.view must not get 401"
    );
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "Authenticated user with logs.view must not get 403"
    );
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "GET /api/logs/vlogs/namespaces without VictoriaLogs must return 503"
    );
}

// ============================================================================
// GET /api/logs/vlogs/labels — VictoriaLogs labels
// ============================================================================

#[tokio::test]
async fn test_vlogs_labels_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/vlogs/labels").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/vlogs/labels without auth must return 401"
    );
}

#[tokio::test]
async fn test_vlogs_labels_without_logs_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_labels_no_perm",
        "vlogs_labels_no_perm@example.com",
        "password123",
        "downloader",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_labels_no_perm",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/vlogs/labels", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User without logs.view must get 403 on GET /api/logs/vlogs/labels"
    );
}

#[tokio::test]
async fn test_vlogs_labels_with_logs_view_returns_503_when_victorialogs_unavailable() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_labels_admin",
        "vlogs_labels_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_labels_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/vlogs/labels", &cookie).await;

    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "GET /api/logs/vlogs/labels without VictoriaLogs must return 503"
    );
}

// ============================================================================
// GET /api/logs/vlogs/label/{label}/values — VictoriaLogs label values
// ============================================================================

#[tokio::test]
async fn test_vlogs_label_values_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/vlogs/label/namespace/values").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/vlogs/label/{{label}}/values without auth must return 401"
    );
}

#[tokio::test]
async fn test_vlogs_label_values_without_logs_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_lv_no_perm",
        "vlogs_lv_no_perm@example.com",
        "password123",
        "downloader",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_lv_no_perm",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/logs/vlogs/label/namespace/values",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User without logs.view must get 403 on GET /api/logs/vlogs/label/{{label}}/values"
    );
}

#[tokio::test]
async fn test_vlogs_label_values_with_logs_view_returns_503_when_victorialogs_unavailable() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_lv_admin",
        "vlogs_lv_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_lv_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/logs/vlogs/label/namespace/values",
        &cookie,
    )
    .await;

    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "GET /api/logs/vlogs/label/{{label}}/values without VictoriaLogs must return 503"
    );
}

#[tokio::test]
async fn test_vlogs_label_values_different_label_names() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_lv_labels_admin",
        "vlogs_lv_labels_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_lv_labels_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Both "pod" and "container" label names should work (routing-wise)
    for label in ["pod", "container", "level", "app"] {
        let uri = format!("/api/logs/vlogs/label/{}/values", label);
        let (status, _) = authenticated_get(create_router(state.clone()), &uri, &cookie).await;

        // VictoriaLogs not available → 503; auth/perm checks pass
        assert_ne!(
            status,
            StatusCode::UNAUTHORIZED,
            "Label '{}' endpoint must not return 401",
            label
        );
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "Label '{}' endpoint must not return 403",
            label
        );
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "Label '{}' endpoint without VictoriaLogs must return 503",
            label
        );
    }
}

// ============================================================================
// GET /api/logs/vlogs/query — VictoriaLogs query
// ============================================================================

#[tokio::test]
async fn test_vlogs_query_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/vlogs/query").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/vlogs/query without auth must return 401"
    );
}

#[tokio::test]
async fn test_vlogs_query_without_logs_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_query_no_perm",
        "vlogs_query_no_perm@example.com",
        "password123",
        "downloader",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_query_no_perm",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/vlogs/query", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User without logs.view must get 403 on GET /api/logs/vlogs/query"
    );
}

#[tokio::test]
async fn test_vlogs_query_with_logs_view_returns_503_when_victorialogs_unavailable() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_query_admin",
        "vlogs_query_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_query_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/vlogs/query", &cookie).await;

    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "GET /api/logs/vlogs/query without VictoriaLogs must return 503"
    );
}

#[tokio::test]
async fn test_vlogs_query_with_query_params_returns_503() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vlogs_query_params_admin",
        "vlogs_query_params_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vlogs_query_params_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Test with explicit query parameters
    let uri = "/api/logs/vlogs/query?query=namespace%3Amedia&limit=100";
    let (status, _) = authenticated_get(create_router(state), uri, &cookie).await;

    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "GET /api/logs/vlogs/query with params must return 503 when VictoriaLogs unavailable"
    );
}

// ============================================================================
// Loki compatibility routes (alias to VictoriaLogs endpoints)
// ============================================================================

#[tokio::test]
async fn test_loki_namespaces_route_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/loki/namespaces").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/loki/namespaces without auth must return 401"
    );
}

#[tokio::test]
async fn test_loki_labels_route_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/loki/labels").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/loki/labels without auth must return 401"
    );
}

#[tokio::test]
async fn test_loki_query_route_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (status, _) = unauthenticated_get(app, "/api/logs/loki/query").await;

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /api/logs/loki/query without auth must return 401"
    );
}

#[tokio::test]
async fn test_loki_namespaces_with_logs_view_returns_503() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "loki_ns_admin",
        "loki_ns_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "loki_ns_admin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/logs/loki/namespaces", &cookie).await;

    // Loki routes are aliases to VictoriaLogs endpoints
    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "GET /api/logs/loki/namespaces (alias) without VictoriaLogs must return 503"
    );
}

// ============================================================================
// Cross-cutting: viewer role has logs.view permission
// ============================================================================

#[tokio::test]
async fn test_viewer_has_logs_view_permission() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "logs_viewer_perm_test",
        "logs_viewer_perm_test@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "logs_viewer_perm_test",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Test all VictoriaLogs endpoints — all should return 503, not 401 or 403
    let endpoints = vec![
        "/api/logs/vlogs/namespaces",
        "/api/logs/vlogs/labels",
        "/api/logs/vlogs/query",
    ];

    for endpoint in endpoints {
        let (status, body) =
            authenticated_get(create_router(state.clone()), endpoint, &cookie).await;

        assert_ne!(
            status,
            StatusCode::UNAUTHORIZED,
            "Viewer with logs.view must not get 401 on {}. Body: {}",
            endpoint,
            body
        );
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "Viewer with logs.view must not get 403 on {}. Body: {}",
            endpoint,
            body
        );
    }
}

#[tokio::test]
async fn test_admin_has_logs_view_permission_for_all_endpoints() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "logs_all_endpoints_admin",
        "logs_all_endpoints_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "logs_all_endpoints_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // K8s-dependent endpoints (return 500 without K8s)
    let k8s_endpoints = vec![
        "/api/logs/some-pod-name",
        "/api/logs/app/jellyfin",
        "/api/logs/raw/some-pod-name",
    ];

    for endpoint in k8s_endpoints {
        let (status, body) =
            authenticated_get(create_router(state.clone()), endpoint, &cookie).await;

        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "Admin with logs.view must get 500 (not 401/403) on {} without K8s. Body: {}",
            endpoint,
            body
        );
    }

    // VictoriaLogs-dependent endpoints (return 503 without VictoriaLogs)
    let vlogs_endpoints = vec![
        "/api/logs/vlogs/namespaces",
        "/api/logs/vlogs/labels",
        "/api/logs/vlogs/label/pod/values",
        "/api/logs/vlogs/query",
    ];

    for endpoint in vlogs_endpoints {
        let (status, body) =
            authenticated_get(create_router(state.clone()), endpoint, &cookie).await;

        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "Admin with logs.view must get 503 (not 401/403) on {} without VictoriaLogs. Body: {}",
            endpoint,
            body
        );
    }
}
