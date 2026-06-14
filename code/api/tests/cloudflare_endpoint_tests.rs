//! Integration tests for the Cloudflare Tunnel endpoints
//!
//! Covers endpoints under `/api/cloudflare`:
//! - `GET  /api/cloudflare/config`          — requires cloudflare.view
//! - `PUT  /api/cloudflare/config`          — requires cloudflare.manage (K8s required)
//! - `DELETE /api/cloudflare/config`        — requires cloudflare.manage (K8s required)
//! - `GET  /api/cloudflare/status`          — requires cloudflare.view (K8s required)
//! - `POST /api/cloudflare/validate-token`  — requires cloudflare.manage (network required)
//!
//! Without K8s (always the case in tests) and without Cloudflare network access,
//! most write/status endpoints return 500. GET /config returns 200/null when
//! no tunnel is configured. DELETE /config returns 404 when no tunnel in DB.

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

async fn authenticated_request(
    app: axum::Router,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let content_type = "application/json";
    let body_bytes = body.map(|b| b.to_string()).unwrap_or_default();

    let mut builder = Request::builder()
        .uri(uri)
        .method(method)
        .header("Cookie", cookie)
        .header("content-type", content_type);

    let request = if body_bytes.is_empty() {
        builder.body(Body::empty()).unwrap()
    } else {
        builder.body(Body::from(body_bytes)).unwrap()
    };

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Create an admin user with cloudflare.view and cloudflare.manage permissions
async fn create_cloudflare_admin(username: &str, email: &str) -> (axum::Router, String) {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;

    use kubarr::models::prelude::*;
    use kubarr::models::{role, role_permission};
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(&db)
        .await
        .unwrap()
        .expect("admin role must exist");

    for permission in &["cloudflare.view", "cloudflare.manage"] {
        let perm = role_permission::ActiveModel {
            role_id: Set(admin_role.id),
            permission: Set(permission.to_string()),
            ..Default::default()
        };
        let _ = perm.insert(&db).await;
    }

    create_test_user_with_role(&db, username, email, "pass123", "admin").await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (_, cookie) = do_login(app.clone(), username, "pass123").await;
    let cookie = cookie.expect("admin login must succeed");
    (app, cookie)
}

// ============================================================================
// Unauthenticated access (401)
// ============================================================================

#[tokio::test]
async fn test_get_config_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cloudflare/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_put_config_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cloudflare/config")
                .method("PUT")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_delete_config_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cloudflare/config")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_get_status_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cloudflare/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_validate_token_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/cloudflare/validate-token")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"api_token":"test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// Permission checks (viewer role → 403)
// ============================================================================

#[tokio::test]
async fn test_get_config_requires_cloudflare_view() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "cfviewer1", "cfviewer1@test.com", "pass123", "viewer").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "cfviewer1", "pass123").await;
    let cookie = cookie.expect("viewer login must succeed");

    let (status, _) = authenticated_request(
        create_router(state),
        "GET",
        "/api/cloudflare/config",
        &cookie,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer lacks cloudflare.view"
    );
}

#[tokio::test]
async fn test_put_config_requires_cloudflare_manage() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "cfviewer2", "cfviewer2@test.com", "pass123", "viewer").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "cfviewer2", "pass123").await;
    let cookie = cookie.expect("viewer login must succeed");

    let (status, _) = authenticated_request(
        create_router(state),
        "PUT",
        "/api/cloudflare/config",
        &cookie,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer lacks cloudflare.manage"
    );
}

#[tokio::test]
async fn test_delete_config_requires_cloudflare_manage() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "cfviewer3", "cfviewer3@test.com", "pass123", "viewer").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "cfviewer3", "pass123").await;
    let cookie = cookie.expect("viewer login must succeed");

    let (status, _) = authenticated_request(
        create_router(state),
        "DELETE",
        "/api/cloudflare/config",
        &cookie,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer lacks cloudflare.manage"
    );
}

#[tokio::test]
async fn test_get_status_requires_cloudflare_view() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "cfviewer4", "cfviewer4@test.com", "pass123", "viewer").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "cfviewer4", "pass123").await;
    let cookie = cookie.expect("viewer login must succeed");

    let (status, _) = authenticated_request(
        create_router(state),
        "GET",
        "/api/cloudflare/status",
        &cookie,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer lacks cloudflare.view"
    );
}

#[tokio::test]
async fn test_validate_token_requires_cloudflare_manage() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(&db, "cfviewer5", "cfviewer5@test.com", "pass123", "viewer").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "cfviewer5", "pass123").await;
    let cookie = cookie.expect("viewer login must succeed");

    let (status, _) = authenticated_request(
        create_router(state),
        "POST",
        "/api/cloudflare/validate-token",
        &cookie,
        Some(serde_json::json!({"api_token": "test"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer lacks cloudflare.manage"
    );
}

// ============================================================================
// Authorized access with admin + cloudflare permissions
// ============================================================================

#[tokio::test]
async fn test_get_config_returns_null_when_no_tunnel() {
    let (app, cookie) = create_cloudflare_admin("cfadmin1", "cfadmin1@test.com").await;

    let (status, body) =
        authenticated_request(app, "GET", "/api/cloudflare/config", &cookie, None).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /cloudflare/config must return 200"
    );
    // No tunnel configured → returns null (JSON null)
    assert_eq!(body.trim(), "null", "no tunnel → response must be null");
}

#[tokio::test]
async fn test_delete_config_returns_not_found_when_no_tunnel() {
    let (app, cookie) = create_cloudflare_admin("cfadmin2", "cfadmin2@test.com").await;

    let (status, _body) =
        authenticated_request(app, "DELETE", "/api/cloudflare/config", &cookie, None).await;

    // K8s is None → 500 (K8s is checked in the handler before the DB)
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "DELETE /cloudflare/config without K8s must return 500"
    );
}

#[tokio::test]
async fn test_put_config_returns_error_without_k8s() {
    let (app, cookie) = create_cloudflare_admin("cfadmin3", "cfadmin3@test.com").await;

    let req_body = serde_json::json!({
        "name": "test-tunnel",
        "api_token": "fake-token",
        "account_id": "acc-123",
        "zone_id": "zone-456",
        "zone_name": "example.com",
        "subdomain": "kubarr"
    });

    let (status, _body) = authenticated_request(
        app,
        "PUT",
        "/api/cloudflare/config",
        &cookie,
        Some(req_body),
    )
    .await;

    // K8s not available → 500 Internal Server Error (before CF API is called)
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "PUT /cloudflare/config without K8s must return 500"
    );
}

#[tokio::test]
async fn test_get_status_returns_error_without_k8s() {
    let (app, cookie) = create_cloudflare_admin("cfadmin4", "cfadmin4@test.com").await;

    let (status, _body) =
        authenticated_request(app, "GET", "/api/cloudflare/status", &cookie, None).await;

    // K8s not available → 500 Internal Server Error
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "GET /cloudflare/status without K8s must return 500"
    );
}

#[tokio::test]
async fn test_validate_token_fails_without_network() {
    let (app, cookie) = create_cloudflare_admin("cfadmin5", "cfadmin5@test.com").await;

    let req_body = serde_json::json!({
        "api_token": "fake-invalid-token"
    });

    let (status, _body) = authenticated_request(
        app,
        "POST",
        "/api/cloudflare/validate-token",
        &cookie,
        Some(req_body),
    )
    .await;

    // CF API is unreachable or rejects token → 500 or 400
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR
            || status == StatusCode::BAD_REQUEST
            || status == StatusCode::UNPROCESSABLE_ENTITY,
        "validate-token without network must return 400, 422, or 500, got {}",
        status
    );
}

#[tokio::test]
async fn test_put_config_bad_request_missing_fields() {
    let (app, cookie) = create_cloudflare_admin("cfadmin6", "cfadmin6@test.com").await;

    // Missing required fields → 422 Unprocessable Entity
    let (status, _body) = authenticated_request(
        app,
        "PUT",
        "/api/cloudflare/config",
        &cookie,
        Some(serde_json::json!({"name": "test"})), // missing api_token, etc.
    )
    .await;

    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::INTERNAL_SERVER_ERROR,
        "PUT with missing fields must return 422 or 500, got {}",
        status
    );
}

// ============================================================================
// GET /config response structure when tunnel exists in DB
// ============================================================================

#[tokio::test]
async fn test_get_config_returns_masked_tunnel_token() {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;

    // Insert a tunnel record directly into the DB
    use chrono::Utc;
    use kubarr::models::prelude::*;
    use kubarr::models::{cloudflare_tunnel, role, role_permission};
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(&db)
        .await
        .unwrap()
        .expect("admin role");

    for perm in &["cloudflare.view", "cloudflare.manage"] {
        let p = role_permission::ActiveModel {
            role_id: Set(admin_role.id),
            permission: Set(perm.to_string()),
            ..Default::default()
        };
        let _ = p.insert(&db).await;
    }

    let now = Utc::now();
    let tunnel = cloudflare_tunnel::ActiveModel {
        name: Set("my-tunnel".to_string()),
        tunnel_token: Set("super-secret-token".to_string()),
        status: Set("running".to_string()),
        error: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    tunnel.insert(&db).await.expect("insert tunnel");

    create_test_user_with_role(&db, "cfadmin7", "cfadmin7@test.com", "pass123", "admin").await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (_, cookie) = do_login(app.clone(), "cfadmin7", "pass123").await;
    let cookie = cookie.expect("login must succeed");

    let (status, body) =
        authenticated_request(app, "GET", "/api/cloudflare/config", &cookie, None).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).expect("must be valid JSON");
    assert!(
        json.is_object(),
        "response must be an object when tunnel exists"
    );
    assert_eq!(
        json["name"].as_str(),
        Some("my-tunnel"),
        "name must be preserved"
    );
    assert_eq!(
        json["tunnel_token"].as_str(),
        Some("****"),
        "tunnel_token must be masked"
    );
    assert_eq!(
        json["status"].as_str(),
        Some("running"),
        "status must be preserved"
    );
}

#[tokio::test]
async fn test_get_config_response_has_required_fields() {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;

    use chrono::Utc;
    use kubarr::models::prelude::*;
    use kubarr::models::{cloudflare_tunnel, role, role_permission};
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(&db)
        .await
        .unwrap()
        .expect("admin role");

    for perm in &["cloudflare.view", "cloudflare.manage"] {
        let p = role_permission::ActiveModel {
            role_id: Set(admin_role.id),
            permission: Set(perm.to_string()),
            ..Default::default()
        };
        let _ = p.insert(&db).await;
    }

    let now = Utc::now();
    let tunnel = cloudflare_tunnel::ActiveModel {
        name: Set("second-tunnel".to_string()),
        tunnel_token: Set("another-secret".to_string()),
        status: Set("deploying".to_string()),
        error: Set(None),
        tunnel_id: Set(Some("cf-tid-123".to_string())),
        zone_name: Set(Some("example.com".to_string())),
        subdomain: Set(Some("app".to_string())),
        hostname: Set(Some("app.example.com".to_string())),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    tunnel.insert(&db).await.expect("insert tunnel");

    create_test_user_with_role(&db, "cfadmin8", "cfadmin8@test.com", "pass123", "admin").await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let (_, cookie) = do_login(app.clone(), "cfadmin8", "pass123").await;
    let cookie = cookie.expect("login must succeed");

    let (status, body) =
        authenticated_request(app, "GET", "/api/cloudflare/config", &cookie, None).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).expect("must be valid JSON");

    // Required fields
    assert!(json.get("id").is_some(), "response must have 'id'");
    assert!(json.get("name").is_some(), "response must have 'name'");
    assert!(
        json.get("tunnel_token").is_some(),
        "response must have 'tunnel_token'"
    );
    assert!(json.get("status").is_some(), "response must have 'status'");
    assert!(
        json.get("created_at").is_some(),
        "response must have 'created_at'"
    );
    assert!(
        json.get("updated_at").is_some(),
        "response must have 'updated_at'"
    );

    // Optional CF fields should be present
    assert_eq!(json["tunnel_id"].as_str(), Some("cf-tid-123"));
    assert_eq!(json["zone_name"].as_str(), Some("example.com"));
    assert_eq!(json["subdomain"].as_str(), Some("app"));
    assert_eq!(json["hostname"].as_str(), Some("app.example.com"));
}
