//! Proxy endpoint integration tests
//!
//! Exercises `proxy_routes()` by mounting it inside a test-only router with
//! the `require_auth` middleware. This covers:
//!
//! - `proxy_app_root`     (`/{app_name}`)
//! - `proxy_app_with_path` (`/{app_name}/{*path}`)
//! - Unauthenticated access → 401
//! - Authenticated, no `app.*` permission → 403
//! - Authenticated, has `app.*`, K8s unavailable → 503

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware as axum_middleware, Router,
};
use http_body_util::BodyExt;
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::{create_router, proxy};
use kubarr::middleware::require_auth;
use kubarr::models::{role, role_permission, user, user_role};
use kubarr::state::AppState;

// ============================================================================
// JWT initialization (shared across all tests)
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
// Test router helpers
// ============================================================================

/// Build a router that mounts proxy_routes at /proxy/* with auth middleware.
fn build_proxy_test_app(state: AppState) -> Router {
    Router::new()
        .nest("/proxy", proxy::proxy_routes(state.clone()))
        .layer(axum_middleware::from_fn_with_state(state, require_auth))
}

/// POST /auth/login, return (status, session cookie value).
async fn do_login(
    full_app: axum::Router,
    username: &str,
    password: &str,
) -> (StatusCode, Option<String>) {
    let body = serde_json::json!({ "username": username, "password": password }).to_string();
    let req = Request::builder()
        .uri("/auth/login")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = full_app.oneshot(req).await.unwrap();
    let status = resp.status();
    let cookie = resp
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

// ============================================================================
// Unauthenticated access
// ============================================================================

#[tokio::test]
async fn proxy_app_root_without_auth_returns_401() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = build_proxy_test_app(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/proxy/myapp")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn proxy_app_with_path_without_auth_returns_401() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = build_proxy_test_app(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/proxy/myapp/some/deep/path")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// No app permission → 403
// ============================================================================

#[tokio::test]
async fn proxy_app_root_no_permission_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Viewer role has app.jellyfin and app.jellyseerr but NOT app.qbittorrent.
    // Requesting /proxy/qbittorrent should return 403.
    create_test_user_with_role(
        &db,
        "proxyviewer",
        "proxyviewer@test.com",
        "pass12345",
        "viewer",
    )
    .await;

    let state = build_test_app_state_with_db(db).await;
    let full_app = create_router(state.clone());
    let proxy_app = build_proxy_test_app(state);

    let (status, cookie) = do_login(full_app, "proxyviewer", "pass12345").await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("must get session cookie");

    let resp = proxy_app
        .oneshot(
            Request::builder()
                .uri("/proxy/qbittorrent")
                .method("GET")
                .header("Cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Viewer has no app.* and no app.qbittorrent permission → 403
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ============================================================================
// Has app.* permission, K8s unavailable → 503
// ============================================================================

#[tokio::test]
async fn proxy_app_root_app_wildcard_no_k8s_returns_503() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let now = chrono::Utc::now();

    // Create a user with app.* permission
    use kubarr::models::prelude::*;
    use sea_orm::{ActiveModelTrait, Set};

    let user_model = user::ActiveModel {
        username: Set("proxyuser".to_string()),
        email: Set("proxyuser@test.com".to_string()),
        hashed_password: Set(kubarr::services::security::hash_password("pass12345").unwrap()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    let u = user_model.insert(&db).await.unwrap();

    // Create a role with app.* permission
    let role_model = role::ActiveModel {
        name: Set("app_wildcard".to_string()),
        description: Set(None),
        is_system: Set(false),
        requires_2fa: Set(false),
        created_at: Set(now),
        ..Default::default()
    };
    let r = role_model.insert(&db).await.unwrap();

    let perm = role_permission::ActiveModel {
        role_id: Set(r.id),
        permission: Set("app.*".to_string()),
        ..Default::default()
    };
    perm.insert(&db).await.unwrap();

    let ur = user_role::ActiveModel {
        user_id: Set(u.id),
        role_id: Set(r.id),
    };
    ur.insert(&db).await.unwrap();

    let state = build_test_app_state_with_db(db).await;
    let full_app = create_router(state.clone());
    let proxy_app = build_proxy_test_app(state);

    let (login_status, cookie) = do_login(full_app, "proxyuser", "pass12345").await;
    assert_eq!(login_status, StatusCode::OK);
    let cookie = cookie.expect("must get session cookie");

    let resp = proxy_app
        .oneshot(
            Request::builder()
                .uri("/proxy/jellyfin")
                .method("GET")
                .header("Cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Permission check passes, but K8s is None → ServiceUnavailable (503)
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn proxy_app_with_path_app_wildcard_no_k8s_returns_503() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let now = chrono::Utc::now();

    use kubarr::models::prelude::*;
    use sea_orm::{ActiveModelTrait, Set};

    let user_model = user::ActiveModel {
        username: Set("proxypathuser".to_string()),
        email: Set("proxypathuser@test.com".to_string()),
        hashed_password: Set(kubarr::services::security::hash_password("pass12345").unwrap()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    let u = user_model.insert(&db).await.unwrap();

    let role_model = role::ActiveModel {
        name: Set("app_wildcard2".to_string()),
        description: Set(None),
        is_system: Set(false),
        requires_2fa: Set(false),
        created_at: Set(now),
        ..Default::default()
    };
    let r = role_model.insert(&db).await.unwrap();

    let perm = role_permission::ActiveModel {
        role_id: Set(r.id),
        permission: Set("app.*".to_string()),
        ..Default::default()
    };
    perm.insert(&db).await.unwrap();

    let ur = user_role::ActiveModel {
        user_id: Set(u.id),
        role_id: Set(r.id),
    };
    ur.insert(&db).await.unwrap();

    let state = build_test_app_state_with_db(db).await;
    let full_app = create_router(state.clone());
    let proxy_app = build_proxy_test_app(state);

    let (login_status, cookie) = do_login(full_app, "proxypathuser", "pass12345").await;
    assert_eq!(login_status, StatusCode::OK);
    let cookie = cookie.expect("must get session cookie");

    let resp = proxy_app
        .oneshot(
            Request::builder()
                .uri("/proxy/jellyfin/web/index.html")
                .method("GET")
                .header("Cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ============================================================================
// Cached endpoint: proxy returns 503 when proxy_http fails (no real target)
// ============================================================================

#[tokio::test]
async fn proxy_app_with_cached_endpoint_attempts_proxy_http() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let now = chrono::Utc::now();

    use kubarr::models::prelude::*;
    use sea_orm::{ActiveModelTrait, Set};

    let user_model = user::ActiveModel {
        username: Set("proxycacheuser".to_string()),
        email: Set("proxycacheuser@test.com".to_string()),
        hashed_password: Set(kubarr::services::security::hash_password("pass12345").unwrap()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    let u = user_model.insert(&db).await.unwrap();

    let role_model = role::ActiveModel {
        name: Set("app_wildcard3".to_string()),
        description: Set(None),
        is_system: Set(false),
        requires_2fa: Set(false),
        created_at: Set(now),
        ..Default::default()
    };
    let r = role_model.insert(&db).await.unwrap();

    let perm = role_permission::ActiveModel {
        role_id: Set(r.id),
        permission: Set("app.*".to_string()),
        ..Default::default()
    };
    perm.insert(&db).await.unwrap();

    let ur = user_role::ActiveModel {
        user_id: Set(u.id),
        role_id: Set(r.id),
    };
    ur.insert(&db).await.unwrap();

    let state = build_test_app_state_with_db(db).await;

    // Pre-seed endpoint cache with an unreachable URL so proxy_http is attempted
    state
        .endpoint_cache
        .set(
            "jellyfin",
            "http://127.0.0.1:19999".to_string(),
            Some(String::new()),
        )
        .await;

    let full_app = create_router(state.clone());
    let proxy_app = build_proxy_test_app(state);

    let (login_status, cookie) = do_login(full_app, "proxycacheuser", "pass12345").await;
    assert_eq!(login_status, StatusCode::OK);
    let cookie = cookie.expect("must get session cookie");

    let resp = proxy_app
        .oneshot(
            Request::builder()
                .uri("/proxy/jellyfin")
                .method("GET")
                .header("Cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // proxy_http will fail because 127.0.0.1:19999 is not listening
    // This should return some error status (5xx)
    assert!(
        resp.status().is_server_error(),
        "Expected 5xx when proxy target is unreachable, got {}",
        resp.status()
    );
}
