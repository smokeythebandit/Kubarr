//! VPN endpoint integration tests
//!
//! Covers endpoints under `/api/vpn` that can be exercised without a live
//! Kubernetes cluster:
//!
//! - `GET  /api/vpn/providers`          — list providers (DB-only)
//! - `POST /api/vpn/providers`          — create provider (DB-only)
//! - `GET  /api/vpn/providers/{id}`     — get by id (DB-only)
//! - `PUT  /api/vpn/providers/{id}`     — update provider (DB-only)
//! - `DELETE /api/vpn/providers/{id}`   — attempts K8s; expect 500 when no K8s
//! - `GET  /api/vpn/supported-providers` — static list, no DB/K8s needed
//! - `GET  /api/vpn/apps`               — list app configs (DB-only, empty at start)
//! - Auth checks (401 without cookie)
//!
//! The test AppState is built with `k8s_client = None`, so any endpoint that
//! requires Kubernetes will fail with a 500 Internal Server Error. The tests
//! for those endpoints simply assert the endpoint is reachable (returns some
//! HTTP response) and does not panic.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, PaginatorTrait, QueryFilter, Statement,
};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;
use kubarr::models::vpn_provider::VpnType;
use kubarr::models::{app_operation, app_state, app_vpn_config, audit_log};
use kubarr::services::catalog::{AppCatalog, AppConfig, ResourceRequirements};
use kubarr::services::vpn::CreateVpnProviderRequest;

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
            if s.starts_with("kubarr_session=") && !s.contains("kubarr_session_") {
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

/// Make an authenticated POST request and return (status, body_string).
async fn authenticated_post(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("POST")
        .header("Cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(json_body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// Make an authenticated PUT request and return (status, body_string).
async fn authenticated_put(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    json_body: &str,
) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("PUT")
        .header("Cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(json_body.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// Make an authenticated DELETE request and return (status, body_string).
async fn authenticated_delete(app: axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(uri)
        .method("DELETE")
        .header("Cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// WireGuard provider create-request body used across tests.
fn wireguard_provider_body(name: &str) -> String {
    serde_json::json!({
        "name": name,
        "vpn_type": "wireguard",
        "service_provider": "custom",
        "credentials": {
            "private_key": "fake-key",
            "addresses": ["10.0.0.1/32"]
        }
    })
    .to_string()
}

/// OpenVPN provider create-request body.
fn openvpn_provider_body(name: &str) -> String {
    serde_json::json!({
        "name": name,
        "vpn_type": "openvpn",
        "service_provider": "nordvpn",
        "credentials": {
            "username": "vpnuser",
            "password": "vpnpassword"
        }
    })
    .to_string()
}

async fn set_test_catalog(state: &kubarr::state::AppState) {
    let app = |name: &str, is_system: bool| AppConfig {
        name: name.to_string(),
        display_name: name.to_string(),
        description: "test app".to_string(),
        icon: String::new(),
        container_image: "example/test:latest".to_string(),
        default_port: 8080,
        resource_requirements: ResourceRequirements {
            cpu_request: "10m".to_string(),
            cpu_limit: "100m".to_string(),
            memory_request: "32Mi".to_string(),
            memory_limit: "64Mi".to_string(),
        },
        volumes: Vec::new(),
        environment_variables: std::collections::HashMap::new(),
        category: "test".to_string(),
        is_system,
        is_hidden: false,
        is_browseable: true,
    };
    *state.catalog.write().await = AppCatalog::with_apps(std::collections::HashMap::from([
        ("sonarr".to_string(), app("sonarr", false)),
        ("kubarr-backend".to_string(), app("kubarr-backend", true)),
    ]));
}

// ============================================================================
// GET /api/vpn/providers — list providers
// ============================================================================

#[tokio::test]
async fn test_list_providers_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/providers")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/vpn/providers without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_providers_empty_on_fresh_db() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnlistadmin",
        "vpnlistadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpnlistadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/vpn/providers", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/vpn/providers must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("providers").is_some(),
        "Response must have providers array"
    );
    let providers = json["providers"].as_array().unwrap();
    assert!(providers.is_empty(), "Fresh DB must have 0 VPN providers");
}

#[tokio::test]
async fn test_list_providers_viewer_lacks_vpn_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnvieweruser",
        "vpnvieweruser@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpnvieweruser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(create_router(state), "/api/vpn/providers", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without vpn.view must get 403 on GET /api/vpn/providers"
    );
}

// ============================================================================
// POST /api/vpn/providers — create a WireGuard provider
// ============================================================================

#[tokio::test]
async fn test_create_provider_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/providers")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(wireguard_provider_body("TestVPN")))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/vpn/providers without auth must return 401"
    );
}

#[tokio::test]
async fn test_create_wireguard_provider_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpncreateadmin",
        "vpncreateadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpncreateadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_post(
        create_router(state),
        "/api/vpn/providers",
        &cookie,
        &wireguard_provider_body("My WireGuard VPN"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/vpn/providers must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.get("id").is_some(), "Created provider must have an id");
    assert_eq!(
        json["name"], "My WireGuard VPN",
        "Created provider must have the requested name"
    );
    assert_eq!(
        json["vpn_type"], "wireguard",
        "Created provider must have vpn_type=wireguard"
    );
    assert_eq!(
        json["service_provider"], "custom",
        "Created provider must have service_provider=custom"
    );
    assert_eq!(
        json["enabled"], true,
        "Created provider must default to enabled=true"
    );
    assert_eq!(
        json["kill_switch"], true,
        "Created provider must default to kill_switch=true"
    );
    assert_eq!(json["firewall_outbound_subnets"], "");
    assert_eq!(
        json["app_count"], 0,
        "New provider must have 0 associated apps"
    );
    // credentials_json must NOT be returned in the response
    assert!(
        json.get("credentials_json").is_none(),
        "Credentials must not be exposed in the response"
    );
}

#[tokio::test]
async fn test_create_openvpn_provider_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnopenvpnadmin",
        "vpnopenvpnadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpnopenvpnadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_post(
        create_router(state),
        "/api/vpn/providers",
        &cookie,
        &openvpn_provider_body("My OpenVPN"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Creating an OpenVPN provider must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["vpn_type"], "openvpn");
    assert_eq!(json["name"], "My OpenVPN");
}

#[tokio::test]
async fn test_create_wireguard_provider_missing_private_key_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnbadcreadadmin",
        "vpnbadcreadadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpnbadcreadadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // WireGuard credentials without the required private_key field
    let bad_body = serde_json::json!({
        "name": "Bad WireGuard",
        "vpn_type": "wireguard",
        "service_provider": "custom",
        "credentials": {
            "addresses": ["10.0.0.1/32"]
        }
    })
    .to_string();

    let (status, body) = authenticated_post(
        create_router(state),
        "/api/vpn/providers",
        &cookie,
        &bad_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "WireGuard provider without private_key must return 400. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_create_openvpn_provider_missing_username_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnbadovpnadmin",
        "vpnbadovpnadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpnbadovpnadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let bad_body = serde_json::json!({
        "name": "Bad OpenVPN",
        "vpn_type": "openvpn",
        "service_provider": "nordvpn",
        "credentials": {
            "password": "secret"
        }
    })
    .to_string();

    let (status, body) = authenticated_post(
        create_router(state),
        "/api/vpn/providers",
        &cookie,
        &bad_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "OpenVPN provider without username must return 400. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/vpn/providers/{id} — get by id
// ============================================================================

#[tokio::test]
async fn test_get_provider_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/providers/1")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/vpn/providers/1 without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_provider_not_found_returns_404() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpngetadmin",
        "vpngetadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpngetadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/vpn/providers/99999", &cookie).await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "GET /api/vpn/providers/99999 must return 404 when provider does not exist"
    );
}

#[tokio::test]
async fn test_get_provider_by_id_after_create() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpngetbyidadmin",
        "vpngetbyidadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpngetbyidadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a provider first
    let (create_status, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/vpn/providers",
        &cookie,
        &wireguard_provider_body("Get By ID VPN"),
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "Provider creation must succeed. Body: {}",
        create_body
    );

    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let provider_id = created["id"]
        .as_i64()
        .expect("Created provider must have numeric id");

    // Now fetch it by ID
    let uri = format!("/api/vpn/providers/{}", provider_id);
    let (status, body) = authenticated_get(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/vpn/providers/{{id}} must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["id"], provider_id, "Returned provider id must match");
    assert_eq!(
        json["name"], "Get By ID VPN",
        "Returned provider name must match"
    );
    assert_eq!(json["vpn_type"], "wireguard");
}

// ============================================================================
// PUT /api/vpn/providers/{id} — update provider
// ============================================================================

#[tokio::test]
async fn test_update_provider_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/providers/1")
                .method("PUT")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Renamed"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "PUT /api/vpn/providers/1 without auth must return 401"
    );
}

#[tokio::test]
async fn test_update_provider_name_and_kill_switch() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnupdateadmin",
        "vpnupdateadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpnupdateadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a provider to update
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/vpn/providers",
        &cookie,
        &wireguard_provider_body("Original Name"),
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let provider_id = created["id"].as_i64().unwrap();

    // Update name and kill_switch
    let update_body = serde_json::json!({
        "name": "Updated Name",
        "kill_switch": false
    })
    .to_string();

    let uri = format!("/api/vpn/providers/{}", provider_id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/vpn/providers/{{id}} must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["name"], "Updated Name", "Name must be updated");
    assert_eq!(
        json["kill_switch"], false,
        "kill_switch must be updated to false"
    );
    assert_eq!(json["id"], provider_id, "Provider id must not change");
}

#[tokio::test]
async fn test_update_provider_not_found_returns_404() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnupdatenotfound",
        "vpnupdatenotfound@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpnupdatenotfound",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_put(
        create_router(state),
        "/api/vpn/providers/99999",
        &cookie,
        r#"{"name":"Ghost"}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "PUT on non-existent provider must return 404"
    );
}

#[tokio::test]
async fn test_update_provider_disable_it() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpndisableadmin",
        "vpndisableadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpndisableadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/vpn/providers",
        &cookie,
        &wireguard_provider_body("DisableMe"),
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let id = created["id"].as_i64().unwrap();

    // Disable the provider
    let update_body = serde_json::json!({"enabled": false}).to_string();
    let uri = format!("/api/vpn/providers/{}", id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Disabling a provider must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["enabled"], false, "Provider must be disabled");
}

#[tokio::test]
async fn test_update_provider_firewall_subnets() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnfwadmin",
        "vpnfwadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpnfwadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/vpn/providers",
        &cookie,
        &wireguard_provider_body("FW Test VPN"),
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let id = created["id"].as_i64().unwrap();

    let custom_subnets = "192.168.1.0/24,10.10.0.0/16";
    let update_body = serde_json::json!({"firewall_outbound_subnets": custom_subnets}).to_string();

    let uri = format!("/api/vpn/providers/{}", id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Updating firewall subnets must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["firewall_outbound_subnets"], custom_subnets,
        "Firewall subnets must be updated"
    );
}

// ============================================================================
// DELETE /api/vpn/providers/{id}
// ============================================================================

#[tokio::test]
async fn test_delete_provider_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/providers/1")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "DELETE /api/vpn/providers/1 without auth must return 401"
    );
}

#[tokio::test]
async fn test_delete_unassigned_provider_does_not_require_k8s() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpndeladmin",
        "vpndeladmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpndeladmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First create a provider so we have a valid id
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/vpn/providers",
        &cookie,
        &wireguard_provider_body("DeleteMe"),
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let id = created["id"].as_i64().unwrap();

    let uri = format!("/api/vpn/providers/{}", id);
    let (status, _) = authenticated_delete(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Deleting an unassigned provider must not require Kubernetes"
    );
}

#[tokio::test]
async fn test_delete_provider_not_found_returns_error() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpndelnotfound",
        "vpndelnotfound@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpndelnotfound",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_delete(create_router(state), "/api/vpn/providers/99999", &cookie).await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "DELETE on a non-existent provider must return 404"
    );
}

#[tokio::test]
async fn test_vpn_assignment_and_removal_enqueue_atomic_updates_without_k8s() {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnlifecycleadmin",
        "vpnlifecycleadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let provider = kubarr::services::vpn::create_vpn_provider(
        &db,
        CreateVpnProviderRequest {
            name: "Lifecycle VPN".to_string(),
            vpn_type: VpnType::WireGuard,
            service_provider: Some("custom".to_string()),
            credentials: serde_json::json!({ "private_key": "key" }),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: "10.0.0.0/8".to_string(),
        },
    )
    .await
    .expect("create provider");
    let state = build_test_app_state_with_db(db.clone()).await;
    set_test_catalog(&state).await;
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpnlifecycleadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("login cookie");
    let request = serde_json::to_string(&serde_json::json!({
        "vpn_provider_id": provider.id,
        "port_forwarding": true
    }))
    .unwrap();

    for (app_name, expected) in [
        ("missing", StatusCode::NOT_FOUND),
        ("kubarr-backend", StatusCode::BAD_REQUEST),
    ] {
        let (status, _) = authenticated_put(
            create_router(state.clone()),
            &format!("/api/vpn/apps/{app_name}"),
            &cookie,
            &request,
        )
        .await;
        assert_eq!(status, expected);
    }
    assert_eq!(app_vpn_config::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(app_operation::Entity::find().count(&db).await.unwrap(), 0);

    let vpn_logs = || audit_log::Entity::find().filter(audit_log::Column::ResourceType.eq("vpn"));
    assert_eq!(vpn_logs().count(&db).await.unwrap(), 0);

    let (status, body) = authenticated_put(
        create_router(state.clone()),
        "/api/vpn/apps/sonarr",
        &cookie,
        &request,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "assignment failed: {body}");
    let assigned: serde_json::Value = serde_json::from_str(&body).unwrap();
    let assign_operation_id = assigned["operation_id"].as_str().expect("operation id");
    assert_eq!(assigned["vpn_provider_id"], provider.id);
    assert!(assigned["port_forwarding"].as_bool().unwrap());
    let operation = app_operation::Entity::find_by_id(assign_operation_id)
        .one(&db)
        .await
        .unwrap()
        .expect("queued assignment operation");
    assert_eq!(operation.operation, "update");
    assert_eq!(operation.status, "queued");
    let logs = vpn_logs().all(&db).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action, "vpn_assigned");
    assert_eq!(logs[0].username.as_deref(), Some("vpnlifecycleadmin"));
    assert_eq!(logs[0].resource_id.as_deref(), Some("sonarr"));
    let details: serde_json::Value =
        serde_json::from_str(logs[0].details.as_ref().unwrap()).unwrap();
    assert_eq!(
        details,
        serde_json::json!({"app_name":"sonarr", "provider_id":provider.id, "operation_id":assign_operation_id, "phase":"queued"})
    );
    let app_state = app_state::Entity::find_by_id("sonarr")
        .one(&db)
        .await
        .unwrap()
        .expect("app state");
    assert_eq!(
        app_state.last_operation_id.as_deref(),
        Some(assign_operation_id)
    );

    let (status, body) =
        authenticated_delete(create_router(state), "/api/vpn/apps/sonarr", &cookie).await;
    assert_eq!(status, StatusCode::OK, "removal failed: {body}");
    let removed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(removed["message"].as_str().is_some());
    let remove_operation_id = removed["operation_id"].as_str().expect("operation id");
    assert_ne!(remove_operation_id, assign_operation_id);
    assert!(app_vpn_config::Entity::find_by_id("sonarr")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    assert_eq!(app_operation::Entity::find().count(&db).await.unwrap(), 2);
    let logs = vpn_logs().all(&db).await.unwrap();
    assert_eq!(logs.len(), 2);
    let removal = logs.iter().find(|log| log.action == "vpn_removed").unwrap();
    assert_eq!(removal.resource_id.as_deref(), Some("sonarr"));
    let details: serde_json::Value =
        serde_json::from_str(removal.details.as_ref().unwrap()).unwrap();
    assert_eq!(
        details,
        serde_json::json!({"app_name":"sonarr", "provider_id":provider.id, "operation_id":remove_operation_id, "phase":"queued"})
    );
}

#[tokio::test]
async fn test_unauthorized_vpn_assignment_does_not_mutate_database() {
    let db = create_test_db_with_seed().await;
    let provider = kubarr::services::vpn::create_vpn_provider(
        &db,
        CreateVpnProviderRequest {
            name: "Protected VPN".to_string(),
            vpn_type: VpnType::WireGuard,
            service_provider: None,
            credentials: serde_json::json!({ "private_key": "key" }),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: "10.0.0.0/8".to_string(),
        },
    )
    .await
    .expect("create provider");
    let state = build_test_app_state_with_db(db.clone()).await;
    set_test_catalog(&state).await;
    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/vpn/apps/sonarr")
                .method("PUT")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "vpn_provider_id": provider.id }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(app_vpn_config::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(app_operation::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(
        audit_log::Entity::find()
            .filter(audit_log::Column::ResourceType.eq("vpn"))
            .count(&db)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn test_vpn_assignment_rolls_back_when_enqueue_fails() {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnrollbackadmin",
        "vpnrollbackadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let provider = kubarr::services::vpn::create_vpn_provider(
        &db,
        CreateVpnProviderRequest {
            name: "Rollback VPN".to_string(),
            vpn_type: VpnType::WireGuard,
            service_provider: None,
            credentials: serde_json::json!({ "private_key": "key" }),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: "10.0.0.0/8".to_string(),
        },
    )
    .await
    .expect("create provider");
    let state = build_test_app_state_with_db(db.clone()).await;
    set_test_catalog(&state).await;
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpnrollbackadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("login cookie");

    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "DROP TABLE app_states".to_string(),
    ))
    .await
    .expect("drop app_states to force queue failure");

    let (status, _) = authenticated_put(
        create_router(state),
        "/api/vpn/apps/sonarr",
        &cookie,
        &serde_json::json!({ "vpn_provider_id": provider.id }).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(app_vpn_config::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(app_operation::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(
        audit_log::Entity::find()
            .filter(audit_log::Column::ResourceType.eq("vpn"))
            .count(&db)
            .await
            .unwrap(),
        0
    );
}

// ============================================================================
// GET /api/vpn/supported-providers — static list
// ============================================================================

#[tokio::test]
async fn test_list_supported_providers_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/supported-providers")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/vpn/supported-providers without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_supported_providers_returns_static_list() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "supportedadmin",
        "supportedadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "supportedadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/vpn/supported-providers",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/vpn/supported-providers must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("providers").is_some(),
        "Response must have providers array"
    );

    let providers = json["providers"].as_array().unwrap();
    assert!(
        !providers.is_empty(),
        "Supported providers list must not be empty"
    );

    // Verify structure of each entry
    for p in providers {
        assert!(p.get("id").is_some(), "Each provider must have id");
        assert!(p.get("name").is_some(), "Each provider must have name");
        assert!(
            p.get("vpn_types").is_some(),
            "Each provider must have vpn_types"
        );
        assert!(
            p.get("description").is_some(),
            "Each provider must have description"
        );
        assert!(
            p.get("supports_port_forwarding").is_some(),
            "Each provider must have supports_port_forwarding"
        );
    }

    // Verify known providers are present
    let ids: Vec<&str> = providers.iter().filter_map(|p| p["id"].as_str()).collect();

    assert!(
        ids.contains(&"custom"),
        "Supported providers must include 'custom'"
    );
    assert!(
        ids.contains(&"mullvad"),
        "Supported providers must include 'mullvad'"
    );
    assert!(
        ids.contains(&"nordvpn"),
        "Supported providers must include 'nordvpn'"
    );
}

#[tokio::test]
async fn test_list_supported_providers_includes_wireguard_types() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "supportedwgadmin",
        "supportedwgadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "supportedwgadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (_, body) = authenticated_get(
        create_router(state),
        "/api/vpn/supported-providers",
        &cookie,
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let providers = json["providers"].as_array().unwrap();

    // "custom" must support both wireguard and openvpn
    let custom = providers
        .iter()
        .find(|p| p["id"] == "custom")
        .expect("custom provider must be present");

    let vpn_types = custom["vpn_types"].as_array().unwrap();
    let type_strings: Vec<&str> = vpn_types.iter().filter_map(|t| t.as_str()).collect();
    assert!(
        type_strings.contains(&"wireguard"),
        "custom provider must support wireguard"
    );
    assert!(
        type_strings.contains(&"openvpn"),
        "custom provider must support openvpn"
    );
}

// ============================================================================
// GET /api/vpn/apps — list app VPN configs
// ============================================================================

#[tokio::test]
async fn test_list_app_configs_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/apps")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/vpn/apps without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_app_configs_empty_on_fresh_db() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnappsadmin",
        "vpnappsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpnappsadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(create_router(state), "/api/vpn/apps", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/vpn/apps must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("configs").is_some(),
        "Response must have configs array"
    );
    let configs = json["configs"].as_array().unwrap();
    assert!(configs.is_empty(), "Fresh DB must have 0 app VPN configs");
}

#[tokio::test]
async fn test_list_app_configs_viewer_lacks_vpn_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpnappsviewer",
        "vpnappsviewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpnappsviewer", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(create_router(state), "/api/vpn/apps", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without vpn.view must get 403 on GET /api/vpn/apps"
    );
}

// ============================================================================
// GET /api/vpn/apps/{app_name} — get app config
// ============================================================================

#[tokio::test]
async fn test_get_app_config_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/apps/qbittorrent")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/vpn/apps/qbittorrent without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_app_config_returns_null_for_unconfigured_app() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpngetappadmin",
        "vpngetappadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpngetappadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/vpn/apps/qbittorrent", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/vpn/apps/qbittorrent must return 200. Body: {}",
        body
    );

    // App has no VPN config — handler returns Json(None) which serializes to "null"
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_null(),
        "Unconfigured app must return null. Got: {}",
        body
    );
}

// ============================================================================
// POST /api/vpn/providers — provider count reflects list after create
// ============================================================================

#[tokio::test]
async fn test_provider_list_reflects_created_providers() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpncountadmin",
        "vpncountadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpncountadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create two providers
    for name in ["Provider Alpha", "Provider Beta"] {
        let (s, b) = authenticated_post(
            create_router(state.clone()),
            "/api/vpn/providers",
            &cookie,
            &wireguard_provider_body(name),
        )
        .await;
        assert_eq!(
            s,
            StatusCode::OK,
            "Creating '{}' must succeed. Body: {}",
            name,
            b
        );
    }

    // List providers
    let (status, body) =
        authenticated_get(create_router(state), "/api/vpn/providers", &cookie).await;
    assert_eq!(status, StatusCode::OK, "List must succeed. Body: {}", body);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let providers = json["providers"].as_array().unwrap();
    assert_eq!(
        providers.len(),
        2,
        "List must show 2 providers after 2 creations"
    );

    let names: Vec<&str> = providers
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    assert!(
        names.contains(&"Provider Alpha"),
        "Provider Alpha must be in the list"
    );
    assert!(
        names.contains(&"Provider Beta"),
        "Provider Beta must be in the list"
    );
}

// ============================================================================
// POST /api/vpn/providers/{id}/test — test provider connection
// ============================================================================

#[tokio::test]
async fn test_test_provider_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/providers/1/test")
                .method("POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/vpn/providers/1/test without auth must return 401"
    );
}

#[tokio::test]
async fn test_test_provider_returns_error_without_k8s() {
    // test_vpn_connection requires a Kubernetes client to create a test pod.
    // Without K8s the handler returns 500. We verify no panic occurs.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpntestadmin",
        "vpntestadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "vpntestadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a provider to test
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/vpn/providers",
        &cookie,
        &wireguard_provider_body("TestableVPN"),
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let id = created["id"].as_i64().unwrap();

    let uri = format!("/api/vpn/providers/{}/test", id);
    let (status, _) = authenticated_post(create_router(state), &uri, &cookie, "").await;

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "POST /api/vpn/providers/{{id}}/test without K8s must return 500"
    );
}
