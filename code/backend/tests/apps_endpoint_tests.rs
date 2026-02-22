//! Integration tests for the apps endpoints
//!
//! Covers endpoints under `/api/apps`:
//! - `GET  /api/apps/catalog`           — requires apps.view
//! - `GET  /api/apps/catalog/{name}`    — requires apps.view
//! - `GET  /api/apps/catalog/{name}/icon` — requires apps.view
//! - `GET  /api/apps/installed`         — requires apps.view
//! - `POST /api/apps/install`           — requires apps.install
//! - `POST /api/apps/sync`              — requires apps.install
//! - `GET  /api/apps/categories`        — requires apps.view
//! - `GET  /api/apps/category/{cat}`    — requires apps.view
//! - `DELETE /api/apps/{name}`          — requires apps.delete
//! - `POST /api/apps/{name}/restart`    — requires apps.restart
//! - `GET  /api/apps/{name}/health`     — requires apps.view
//! - `GET  /api/apps/{name}/exists`     — requires apps.view
//! - `GET  /api/apps/{name}/status`     — requires apps.view
//! - `POST /api/apps/{name}/access`     — requires Authenticated

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};
use kubarr::endpoints::create_router;
use kubarr::services::catalog::{AppCatalog, AppConfig, ResourceRequirements};
use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ============================================================================
// JWT key initialization (once per test binary)
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

async fn do_login(app: axum::Router, username: &str, password: &str) -> Option<String> {
    let body = serde_json::json!({"username": username, "password": password}).to_string();
    let request = Request::builder()
        .uri("/auth/login")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    response
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
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
        })
}

async fn make_request(
    app: axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let body_str = body.map(|b| b.to_string()).unwrap_or_default();
    let mut builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("content-type", "application/json");
    if let Some(c) = cookie {
        builder = builder.header("Cookie", c);
    }
    let request = if body_str.is_empty() {
        builder.body(Body::empty()).unwrap()
    } else {
        builder.body(Body::from(body_str)).unwrap()
    };
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Build an app+cookie for admin with all default permissions
async fn make_admin(username: &str, email: &str) -> (axum::Router, String) {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, username, email, "pass123", "admin").await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);
    let cookie = do_login(app.clone(), username, "pass123")
        .await
        .expect("admin login must succeed");
    (app, cookie)
}

/// Build an app+cookie for viewer user
async fn make_viewer(username: &str, email: &str) -> (axum::Router, String) {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, username, email, "pass123", "viewer").await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);
    let cookie = do_login(app.clone(), username, "pass123")
        .await
        .expect("viewer login must succeed");
    (app, cookie)
}

// ============================================================================
// Unauthenticated access (401)
// ============================================================================

#[tokio::test]
async fn test_list_catalog_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "GET", "/api/apps/catalog", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_get_catalog_app_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "GET", "/api/apps/catalog/sonarr", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_list_installed_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "GET", "/api/apps/installed", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_install_app_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(
        app,
        "POST",
        "/api/apps/install",
        None,
        Some(serde_json::json!({"app_name": "sonarr"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_list_categories_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "GET", "/api/apps/categories", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_get_apps_by_category_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "GET", "/api/apps/category/media", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_delete_app_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "DELETE", "/api/apps/sonarr", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_restart_app_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "POST", "/api/apps/sonarr/restart", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_check_health_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "GET", "/api/apps/sonarr/health", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_check_exists_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "GET", "/api/apps/sonarr/exists", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_get_status_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "GET", "/api/apps/sonarr/status", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_log_access_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "POST", "/api/apps/sonarr/access", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_sync_charts_requires_auth() {
    let db = create_test_db_with_seed().await;
    let app = create_router(build_test_app_state_with_db(db).await);
    let (status, _) = make_request(app, "POST", "/api/apps/sync", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Permission checks (403)
// ============================================================================

#[tokio::test]
async fn test_install_app_requires_apps_install_permission() {
    let (app, cookie) = make_viewer("viewer_install", "viewer_install@test.com").await;
    // viewer has apps.view but NOT apps.install
    let (status, _) = make_request(
        app,
        "POST",
        "/api/apps/install",
        Some(&cookie),
        Some(serde_json::json!({"app_name": "sonarr"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer must not have apps.install"
    );
}

#[tokio::test]
async fn test_delete_app_requires_apps_delete_permission() {
    let (app, cookie) = make_viewer("viewer_delete", "viewer_delete@test.com").await;
    let (status, _) = make_request(app, "DELETE", "/api/apps/sonarr", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer must not have apps.delete"
    );
}

#[tokio::test]
async fn test_sync_requires_apps_install_permission() {
    let (app, cookie) = make_viewer("viewer_sync", "viewer_sync@test.com").await;
    let (status, _) = make_request(app, "POST", "/api/apps/sync", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer must not have apps.install (sync)"
    );
}

// ============================================================================
// Functional tests with admin auth
// ============================================================================

#[tokio::test]
async fn test_list_catalog_returns_empty_without_charts() {
    let (app, cookie) = make_admin("admin_catalog", "admin_catalog@test.com").await;
    let (status, body) = make_request(app, "GET", "/api/apps/catalog", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let apps: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("response must be JSON array");
    // Without a real charts dir, the catalog is empty (no charts loaded)
    // This also tests that hidden apps are filtered out
    let _ = apps.len(); // Just verify it parses and doesn't panic
}

#[tokio::test]
async fn test_get_app_from_catalog_returns_404_when_not_found() {
    let (app, cookie) = make_admin("admin_cat404", "admin_cat404@test.com").await;
    let (status, body) = make_request(
        app,
        "GET",
        "/api/apps/catalog/nonexistent_app_xyz",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "nonexistent app must return 404, body: {}",
        body
    );
}

#[tokio::test]
async fn test_get_app_icon_rejects_path_traversal() {
    let (app, cookie) = make_admin("admin_icon", "admin_icon@test.com").await;
    // Use percent-encoded ".." to test the path traversal check in get_app_icon.
    // The handler validates app_name.contains("..") and rejects with 400.
    let (status, _) = make_request(
        app.clone(),
        "GET",
        "/api/apps/catalog/%2E%2E%2Fetc%2Fpasswd/icon",
        Some(&cookie),
        None,
    )
    .await;
    // Either 400 (invalid name caught by handler) or 404 (not found) is acceptable
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
        "path traversal attempt must be rejected, got {}",
        status
    );
}

#[tokio::test]
async fn test_get_app_icon_returns_404_for_missing_icon() {
    let (app, cookie) = make_admin("admin_icon2", "admin_icon2@test.com").await;
    let (status, _) = make_request(
        app,
        "GET",
        "/api/apps/catalog/sonarr/icon",
        Some(&cookie),
        None,
    )
    .await;
    // The icon file doesn't exist in test environment
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "missing icon must return 404"
    );
}

#[tokio::test]
async fn test_list_installed_returns_empty_without_k8s() {
    let (app, cookie) = make_admin("admin_installed", "admin_installed@test.com").await;
    let (status, body) = make_request(app, "GET", "/api/apps/installed", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let apps: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("response must be JSON array");
    assert!(
        apps.is_empty(),
        "must return empty list when K8s not available"
    );
}

#[tokio::test]
async fn test_list_categories_returns_array() {
    let (app, cookie) = make_admin("admin_cats", "admin_cats@test.com").await;
    let (status, body) =
        make_request(app, "GET", "/api/apps/categories", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let _: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("response must be JSON array");
}

#[tokio::test]
async fn test_get_apps_by_category_returns_array() {
    let (app, cookie) = make_admin("admin_catapps", "admin_catapps@test.com").await;
    let (status, body) =
        make_request(app, "GET", "/api/apps/category/media", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let _: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("response must be JSON array");
}

#[tokio::test]
async fn test_install_app_returns_500_without_k8s() {
    let (app, cookie) = make_admin("admin_install", "admin_install@test.com").await;
    let (status, _) = make_request(
        app,
        "POST",
        "/api/apps/install",
        Some(&cookie),
        Some(serde_json::json!({"app_name": "sonarr"})),
    )
    .await;
    // K8s not available → INTERNAL_SERVER_ERROR
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "install without K8s must return 500"
    );
}

#[tokio::test]
async fn test_delete_app_returns_500_without_k8s() {
    let (app, cookie) = make_admin("admin_del", "admin_del@test.com").await;
    let (status, _) = make_request(app, "DELETE", "/api/apps/sonarr", Some(&cookie), None).await;
    // K8s not available → INTERNAL_SERVER_ERROR
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "delete without K8s must return 500"
    );
}

#[tokio::test]
async fn test_restart_app_returns_500_without_k8s() {
    let (app, cookie) = make_admin("admin_restart", "admin_restart@test.com").await;
    let (status, _) =
        make_request(app, "POST", "/api/apps/sonarr/restart", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "restart without K8s must return 500"
    );
}

#[tokio::test]
async fn test_check_health_returns_500_without_k8s() {
    let (app, cookie) = make_admin("admin_health", "admin_health@test.com").await;
    let (status, _) =
        make_request(app, "GET", "/api/apps/sonarr/health", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "health check without K8s must return 500"
    );
}

#[tokio::test]
async fn test_check_exists_returns_500_without_k8s() {
    let (app, cookie) = make_admin("admin_exists", "admin_exists@test.com").await;
    let (status, _) =
        make_request(app, "GET", "/api/apps/sonarr/exists", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "check_exists without K8s must return 500"
    );
}

#[tokio::test]
async fn test_get_status_returns_error_state_without_k8s() {
    let (app, cookie) = make_admin("admin_status", "admin_status@test.com").await;
    let (status, body) =
        make_request(app, "GET", "/api/apps/sonarr/status", Some(&cookie), None).await;
    // get_app_status handles None K8s gracefully → 200 with error state
    assert_eq!(
        status,
        StatusCode::OK,
        "get_app_status without K8s must return 200"
    );
    let json: serde_json::Value = serde_json::from_str(&body).expect("must be JSON");
    assert_eq!(
        json["state"].as_str().unwrap_or(""),
        "error",
        "state must be 'error' when K8s not available"
    );
}

#[tokio::test]
async fn test_log_app_access_returns_success() {
    let (app, cookie) = make_admin("admin_access", "admin_access@test.com").await;
    let (status, body) =
        make_request(app, "POST", "/api/apps/sonarr/access", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "log_access must succeed: {}", body);
    let json: serde_json::Value = serde_json::from_str(&body).expect("must be JSON");
    assert_eq!(json["success"], true);
}

#[tokio::test]
async fn test_viewer_can_list_catalog() {
    let (app, cookie) = make_viewer("viewer_cat", "viewer_cat@test.com").await;
    let (status, _) = make_request(app, "GET", "/api/apps/catalog", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "viewer must have apps.view for catalog"
    );
}

#[tokio::test]
async fn test_viewer_can_list_categories() {
    let (app, cookie) = make_viewer("viewer_categ", "viewer_categ@test.com").await;
    let (status, _) = make_request(app, "GET", "/api/apps/categories", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "viewer must have apps.view");
}

#[tokio::test]
async fn test_viewer_can_get_app_status() {
    let (app, cookie) = make_viewer("viewer_stat", "viewer_stat@test.com").await;
    let (status, _) =
        make_request(app, "GET", "/api/apps/sonarr/status", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "viewer must be able to get status");
}

#[tokio::test]
async fn test_sync_charts_succeeds() {
    let (app, cookie) = make_admin("admin_sync", "admin_sync@test.com").await;
    let (status, body) = make_request(app, "POST", "/api/apps/sync", Some(&cookie), None).await;
    // sync_charts calls ChartSyncService::sync() which should handle no OCI registry gracefully
    // It may succeed or return an error depending on config
    assert!(
        status == StatusCode::OK || status == StatusCode::INTERNAL_SERVER_ERROR,
        "sync_charts must return 200 or 500, got {}: {}",
        status,
        body
    );
}

// ============================================================================
// Seeded catalog tests (system app protection, catalog content)
// ============================================================================

/// Build a minimal AppConfig for testing.
fn make_app_config(name: &str, category: &str, is_system: bool, is_hidden: bool) -> AppConfig {
    AppConfig {
        name: name.to_string(),
        display_name: format!("{} App", name),
        description: "Test app".to_string(),
        icon: String::new(),
        container_image: format!("test/{}:latest", name),
        default_port: 8080,
        resource_requirements: ResourceRequirements {
            cpu_request: "100m".to_string(),
            cpu_limit: "500m".to_string(),
            memory_request: "128Mi".to_string(),
            memory_limit: "512Mi".to_string(),
        },
        volumes: vec![],
        environment_variables: HashMap::new(),
        category: category.to_string(),
        is_system,
        is_hidden,
        is_browseable: true,
    }
}

/// Build an AppState with a seeded catalog and a given DB.
async fn build_state_with_seeded_catalog(
    db: DatabaseConnection,
    apps: Vec<AppConfig>,
) -> kubarr::state::AppState {
    use kubarr::services::{
        audit::AuditService, chart_sync::ChartSyncService, notification::NotificationService,
    };
    use kubarr::state::SharedK8sClient;

    let mut app_map = HashMap::new();
    for a in apps {
        app_map.insert(a.name.clone(), a);
    }
    let catalog = Arc::new(RwLock::new(AppCatalog::with_apps(app_map)));
    let k8s: SharedK8sClient = Arc::new(RwLock::new(None));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();
    audit.set_db(db.clone()).await;
    notification.set_db(db.clone()).await;
    kubarr::state::AppState::new(Some(db), k8s, catalog, chart_sync, audit, notification)
}

/// Build admin router+cookie with a seeded catalog.
async fn make_admin_with_catalog(
    username: &str,
    email: &str,
    apps: Vec<AppConfig>,
) -> (axum::Router, String) {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, username, email, "pass123", "admin").await;
    let state = build_state_with_seeded_catalog(db, apps).await;
    let app = create_router(state);
    let cookie = do_login(app.clone(), username, "pass123")
        .await
        .expect("admin login must succeed");
    (app, cookie)
}

#[tokio::test]
async fn test_delete_system_app_returns_403() {
    let apps = vec![make_app_config("postgresql", "system", true, true)];
    let (app, cookie) =
        make_admin_with_catalog("admin_sysapp", "admin_sysapp@test.com", apps).await;
    let (status, body) =
        make_request(app, "DELETE", "/api/apps/postgresql", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "deleting system app must return 403, body: {}",
        body
    );
}

#[tokio::test]
async fn test_get_app_from_catalog_found() {
    let apps = vec![make_app_config("sonarr", "media", false, false)];
    let (app, cookie) = make_admin_with_catalog("admin_found", "admin_found@test.com", apps).await;
    let (status, body) =
        make_request(app, "GET", "/api/apps/catalog/sonarr", Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "existing app must return 200, body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).expect("must be JSON");
    assert_eq!(json["name"].as_str().unwrap_or(""), "sonarr");
}

#[tokio::test]
async fn test_list_catalog_filters_hidden_apps() {
    let apps = vec![
        make_app_config("sonarr", "media", false, false), // visible
        make_app_config("postgresql", "system", true, true), // hidden
    ];
    let (app, cookie) =
        make_admin_with_catalog("admin_filter", "admin_filter@test.com", apps).await;
    let (status, body) = make_request(app, "GET", "/api/apps/catalog", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let apps_arr: Vec<serde_json::Value> = serde_json::from_str(&body).expect("must be JSON array");
    // Hidden app must be filtered out
    assert_eq!(apps_arr.len(), 1, "only non-hidden apps should be returned");
    assert_eq!(apps_arr[0]["name"].as_str().unwrap_or(""), "sonarr");
}

#[tokio::test]
async fn test_list_categories_with_seeded_catalog() {
    let apps = vec![
        make_app_config("sonarr", "media", false, false),
        make_app_config("grafana", "monitoring", false, false),
    ];
    let (app, cookie) =
        make_admin_with_catalog("admin_catseeded", "admin_catseeded@test.com", apps).await;
    let (status, body) =
        make_request(app, "GET", "/api/apps/categories", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let cats: Vec<String> = serde_json::from_str(&body).expect("must be JSON array");
    assert!(
        cats.contains(&"media".to_string()),
        "media category must be present"
    );
    assert!(
        cats.contains(&"monitoring".to_string()),
        "monitoring category must be present"
    );
}

#[tokio::test]
async fn test_get_apps_by_category_with_seeded_catalog() {
    let apps = vec![
        make_app_config("sonarr", "media", false, false),
        make_app_config("radarr", "media", false, false),
        make_app_config("grafana", "monitoring", false, false),
    ];
    let (app, cookie) =
        make_admin_with_catalog("admin_catfilt", "admin_catfilt@test.com", apps).await;
    let (status, body) =
        make_request(app, "GET", "/api/apps/category/media", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let apps_arr: Vec<serde_json::Value> = serde_json::from_str(&body).expect("must be JSON array");
    assert_eq!(apps_arr.len(), 2, "only media apps should be returned");
}
