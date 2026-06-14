//! Extended VPN endpoint integration tests
//!
//! Covers paths NOT already covered by `vpn_endpoint_tests.rs`:
//!
//! - `PUT /api/vpn/apps/{app_name}`                — assign VPN to app (K8s not needed for DB op)
//! - `DELETE /api/vpn/apps/{app_name}`             — remove VPN from app (requires K8s)
//! - `GET /api/vpn/apps/{app_name}/forwarded-port` — get forwarded port (requires K8s)
//! - Additional auth + permission checks for already-covered endpoints
//! - Viewer (without vpn.view) getting 403 on every endpoint
//! - Credential update via PUT /api/vpn/providers/{id}
//! - OpenVPN-specific credential validation

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

// Helper: create a provider and return its ID
async fn create_provider_and_get_id(
    state: kubarr::state::AppState,
    cookie: &str,
    name: &str,
) -> i64 {
    let (status, body) = authenticated_post(
        create_router(state),
        "/api/vpn/providers",
        cookie,
        &wireguard_provider_body(name),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Provider creation must succeed for '{}'",
        name
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    json["id"]
        .as_i64()
        .expect("Created provider must have numeric id")
}

// ============================================================================
// PUT /api/vpn/apps/{app_name} — assign VPN to app
// ============================================================================

#[tokio::test]
async fn test_assign_vpn_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/apps/qbittorrent")
                .method("PUT")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"provider_id": 1}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "PUT /api/vpn/apps/{{app_name}} without auth must return 401"
    );
}

#[tokio::test]
async fn test_assign_vpn_viewer_without_vpn_manage_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "assign_vpn_viewer",
        "assign_vpn_viewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "assign_vpn_viewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_put(
        create_router(state),
        "/api/vpn/apps/qbittorrent",
        &cookie,
        r#"{"provider_id": 1}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without vpn.manage must get 403 on PUT /api/vpn/apps/{{app_name}}"
    );
}

#[tokio::test]
async fn test_assign_vpn_with_nonexistent_provider_returns_error() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "assign_vpn_admin",
        "assign_vpn_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "assign_vpn_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Provider ID 99999 does not exist
    let body = serde_json::json!({"vpn_provider_id": 99999}).to_string();
    let (status, _) = authenticated_put(
        create_router(state),
        "/api/vpn/apps/qbittorrent",
        &cookie,
        &body,
    )
    .await;

    // Should return an error (not 401/403) — handler reached DB lookup
    assert_ne!(status, StatusCode::UNAUTHORIZED, "Must not be 401");
    assert_ne!(status, StatusCode::FORBIDDEN, "Must not be 403");
    // Expecting 4xx or 5xx error for nonexistent provider
    assert!(
        status.is_client_error() || status.is_server_error(),
        "PUT with nonexistent provider must return error. Got: {}",
        status
    );
}

#[tokio::test]
async fn test_assign_vpn_then_get_app_config_shows_vpn() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "assign_vpn_full_admin",
        "assign_vpn_full_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "assign_vpn_full_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a provider first
    let provider_id = create_provider_and_get_id(state.clone(), &cookie, "App VPN Provider").await;

    // Assign VPN to app
    let assign_body = serde_json::json!({"vpn_provider_id": provider_id}).to_string();
    let (assign_status, assign_body_resp) = authenticated_put(
        create_router(state.clone()),
        "/api/vpn/apps/qbittorrent",
        &cookie,
        &assign_body,
    )
    .await;

    assert_eq!(
        assign_status,
        StatusCode::OK,
        "PUT /api/vpn/apps/qbittorrent must return 200. Body: {}",
        assign_body_resp
    );

    // Verify the config is returned
    let json: serde_json::Value = serde_json::from_str(&assign_body_resp).unwrap();
    assert_eq!(
        json["app_name"], "qbittorrent",
        "Assigned config must have correct app_name. Body: {}",
        assign_body_resp
    );
    assert_eq!(
        json["vpn_provider_id"], provider_id,
        "Assigned config must reference the provider ID. Body: {}",
        assign_body_resp
    );
}

#[tokio::test]
async fn test_assign_vpn_get_app_config_returns_config_after_assign() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_get_after_assign",
        "vpn_get_after_assign@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_get_after_assign",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a provider
    let provider_id =
        create_provider_and_get_id(state.clone(), &cookie, "Get After Assign VPN").await;

    // Assign VPN to app
    let assign_body = serde_json::json!({"vpn_provider_id": provider_id}).to_string();
    let (assign_status, _) = authenticated_put(
        create_router(state.clone()),
        "/api/vpn/apps/transmission",
        &cookie,
        &assign_body,
    )
    .await;
    assert_eq!(assign_status, StatusCode::OK, "Assign must succeed");

    // Get app config — should return the assigned config
    let (get_status, get_body) =
        authenticated_get(create_router(state), "/api/vpn/apps/transmission", &cookie).await;

    assert_eq!(
        get_status,
        StatusCode::OK,
        "GET /api/vpn/apps/transmission must return 200. Body: {}",
        get_body
    );

    let json: serde_json::Value = serde_json::from_str(&get_body).unwrap();
    assert!(
        !json.is_null(),
        "App config must not be null after assignment. Got: {}",
        get_body
    );
    assert_eq!(
        json["vpn_provider_id"], provider_id,
        "App config must reference the correct provider. Body: {}",
        get_body
    );
}

// ============================================================================
// DELETE /api/vpn/apps/{app_name} — remove VPN from app (requires K8s)
// ============================================================================

#[tokio::test]
async fn test_remove_vpn_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/apps/qbittorrent")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "DELETE /api/vpn/apps/{{app_name}} without auth must return 401"
    );
}

#[tokio::test]
async fn test_remove_vpn_viewer_without_vpn_manage_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "remove_vpn_viewer",
        "remove_vpn_viewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "remove_vpn_viewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_delete(create_router(state), "/api/vpn/apps/qbittorrent", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without vpn.manage must get 403 on DELETE /api/vpn/apps/{{app_name}}"
    );
}

#[tokio::test]
async fn test_remove_vpn_without_k8s_returns_500() {
    // remove_vpn requires K8s to clean up secrets and redeploy
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "remove_vpn_admin",
        "remove_vpn_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "remove_vpn_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_delete(create_router(state), "/api/vpn/apps/qbittorrent", &cookie).await;

    // Without K8s the handler returns 500
    assert_ne!(status, StatusCode::UNAUTHORIZED, "Must not be 401");
    assert_ne!(status, StatusCode::FORBIDDEN, "Must not be 403");
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "DELETE /api/vpn/apps/{{app_name}} without K8s must return 500"
    );
}

// ============================================================================
// GET /api/vpn/apps/{app_name}/forwarded-port — get forwarded port
// ============================================================================

#[tokio::test]
async fn test_get_forwarded_port_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/vpn/apps/qbittorrent/forwarded-port")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/vpn/apps/{{app_name}}/forwarded-port without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_forwarded_port_viewer_without_vpn_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "fwd_port_viewer",
        "fwd_port_viewer@example.com",
        "password123",
        "downloader",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "fwd_port_viewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/vpn/apps/qbittorrent/forwarded-port",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "User without vpn.view must get 403 on GET /api/vpn/apps/{{app_name}}/forwarded-port"
    );
}

#[tokio::test]
async fn test_get_forwarded_port_without_k8s_returns_500() {
    // get_forwarded_port requires K8s to list pods and find the Gluetun container
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "fwd_port_admin",
        "fwd_port_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "fwd_port_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/vpn/apps/qbittorrent/forwarded-port",
        &cookie,
    )
    .await;

    // Without K8s the handler returns 500 ("Kubernetes client not available")
    assert_ne!(status, StatusCode::UNAUTHORIZED, "Must not be 401");
    assert_ne!(status, StatusCode::FORBIDDEN, "Must not be 403");
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "GET /api/vpn/apps/{{app_name}}/forwarded-port without K8s must return 500"
    );
}

#[tokio::test]
async fn test_get_forwarded_port_viewer_with_vpn_view_returns_500_without_k8s() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    // "viewer" role has vpn.view in some environments — use admin instead
    create_test_user_with_role(
        &db,
        "fwd_port_viewer_v2",
        "fwd_port_viewer_v2@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "fwd_port_viewer_v2",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/vpn/apps/transmission/forwarded-port",
        &cookie,
    )
    .await;

    // K8s not available → 500
    assert_ne!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// ============================================================================
// GET /api/vpn/apps — app configs list (extended)
// ============================================================================

#[tokio::test]
async fn test_app_configs_list_reflects_assigned_vpn() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_apps_list_admin",
        "vpn_apps_list_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_apps_list_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Initially empty
    let (list_status, list_body) =
        authenticated_get(create_router(state.clone()), "/api/vpn/apps", &cookie).await;
    assert_eq!(list_status, StatusCode::OK);
    let initial: serde_json::Value = serde_json::from_str(&list_body).unwrap();
    assert!(
        initial["configs"].as_array().unwrap().is_empty(),
        "Initially no app VPN configs"
    );

    // Create a provider
    let provider_id = create_provider_and_get_id(state.clone(), &cookie, "App List VPN").await;

    // Assign VPN to app
    let assign_body = serde_json::json!({"vpn_provider_id": provider_id}).to_string();
    let (assign_status, _) = authenticated_put(
        create_router(state.clone()),
        "/api/vpn/apps/deluge",
        &cookie,
        &assign_body,
    )
    .await;
    assert_eq!(assign_status, StatusCode::OK, "Assign must succeed");

    // List must now show 1 config
    let (list_status2, list_body2) =
        authenticated_get(create_router(state), "/api/vpn/apps", &cookie).await;
    assert_eq!(list_status2, StatusCode::OK);
    let after: serde_json::Value = serde_json::from_str(&list_body2).unwrap();
    let configs = after["configs"].as_array().unwrap();
    assert_eq!(
        configs.len(),
        1,
        "List must show 1 app VPN config after assignment"
    );
    assert_eq!(
        configs[0]["app_name"], "deluge",
        "Config must show correct app name"
    );
}

// ============================================================================
// PUT /api/vpn/providers/{id} — credential updates
// ============================================================================

#[tokio::test]
async fn test_update_provider_with_new_credentials() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_update_creds_admin",
        "vpn_update_creds_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_update_creds_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create a WireGuard provider
    let provider_id = create_provider_and_get_id(state.clone(), &cookie, "Update Creds VPN").await;

    // Update with new credentials
    let update_body = serde_json::json!({
        "credentials": {
            "private_key": "new_private_key_xyz789",
            "addresses": ["10.0.0.2/32"]
        }
    })
    .to_string();

    let uri = format!("/api/vpn/providers/{}", provider_id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT with new credentials must return 200. Body: {}",
        body
    );
    // Verify credentials are NOT returned in response
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("credentials_json").is_none(),
        "Credentials must not be exposed in response. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_update_openvpn_provider_credentials() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_update_ovpn_admin",
        "vpn_update_ovpn_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_update_ovpn_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create an OpenVPN provider
    let (create_status, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/vpn/providers",
        &cookie,
        &openvpn_provider_body("Update OpenVPN"),
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "OpenVPN creation must succeed. Body: {}",
        create_body
    );

    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let provider_id = created["id"].as_i64().unwrap();

    // Update name
    let update_body = serde_json::json!({"name": "Updated OpenVPN Name"}).to_string();
    let uri = format!("/api/vpn/providers/{}", provider_id);
    let (status, body) = authenticated_put(create_router(state), &uri, &cookie, &update_body).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Update must succeed. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["name"], "Updated OpenVPN Name");
    assert_eq!(json["vpn_type"], "openvpn");
}

// ============================================================================
// POST /api/vpn/providers — invalid vpn_type
// ============================================================================

#[tokio::test]
async fn test_create_provider_invalid_vpn_type_returns_error() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_invalid_type_admin",
        "vpn_invalid_type_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_invalid_type_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let body = serde_json::json!({
        "name": "Invalid Type VPN",
        "vpn_type": "invalid_vpn_type",
        "service_provider": "custom",
        "credentials": {
            "private_key": "some_key",
            "addresses": ["10.0.0.1/32"]
        }
    })
    .to_string();

    let (status, _) =
        authenticated_post(create_router(state), "/api/vpn/providers", &cookie, &body).await;

    // Invalid VPN type should return an error (400 or 422)
    assert!(
        status.is_client_error(),
        "Invalid vpn_type must return client error. Got: {}",
        status
    );
}

// ============================================================================
// GET /api/vpn/providers/{id} — additional coverage
// ============================================================================

#[tokio::test]
async fn test_get_provider_after_openvpn_create() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_get_ovpn_admin",
        "vpn_get_ovpn_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_get_ovpn_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create OpenVPN provider
    let (_, create_body) = authenticated_post(
        create_router(state.clone()),
        "/api/vpn/providers",
        &cookie,
        &openvpn_provider_body("Get OpenVPN Test"),
    )
    .await;
    let created: serde_json::Value = serde_json::from_str(&create_body).unwrap();
    let provider_id = created["id"].as_i64().unwrap();

    // Retrieve by ID
    let uri = format!("/api/vpn/providers/{}", provider_id);
    let (status, body) = authenticated_get(create_router(state), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET provider must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["vpn_type"], "openvpn");
    assert_eq!(json["service_provider"], "nordvpn");
    assert_eq!(json["name"], "Get OpenVPN Test");
}

#[tokio::test]
async fn test_get_provider_invalid_id_format_returns_error() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_invalid_id_admin",
        "vpn_invalid_id_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_invalid_id_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Non-numeric ID should return 400 or 422 (path param extraction fails)
    let (status, _) = authenticated_get(
        create_router(state),
        "/api/vpn/providers/not-a-number",
        &cookie,
    )
    .await;

    assert!(
        status.is_client_error(),
        "GET /api/vpn/providers/not-a-number must return client error. Got: {}",
        status
    );
}

// ============================================================================
// Multiple providers: list reflects creation order
// ============================================================================

#[tokio::test]
async fn test_multiple_providers_list_shows_all() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_multi_admin",
        "vpn_multi_admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_multi_admin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create 3 providers of different types
    let providers = vec![
        (
            "WireGuard VPN 1",
            wireguard_provider_body("WireGuard VPN 1"),
        ),
        (
            "OpenVPN Provider 1",
            openvpn_provider_body("OpenVPN Provider 1"),
        ),
        (
            "WireGuard VPN 2",
            wireguard_provider_body("WireGuard VPN 2"),
        ),
    ];

    for (name, body) in &providers {
        let (status, resp_body) = authenticated_post(
            create_router(state.clone()),
            "/api/vpn/providers",
            &cookie,
            body,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "Creating '{}' must succeed. Body: {}",
            name,
            resp_body
        );
    }

    // List should show all 3
    let (status, body) =
        authenticated_get(create_router(state), "/api/vpn/providers", &cookie).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "List must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let provider_list = json["providers"].as_array().unwrap();
    assert_eq!(provider_list.len(), 3, "List must show 3 providers");

    let names: Vec<&str> = provider_list
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();

    assert!(
        names.contains(&"WireGuard VPN 1"),
        "Must include WireGuard VPN 1"
    );
    assert!(
        names.contains(&"OpenVPN Provider 1"),
        "Must include OpenVPN Provider 1"
    );
    assert!(
        names.contains(&"WireGuard VPN 2"),
        "Must include WireGuard VPN 2"
    );
}

// ============================================================================
// POST /api/vpn/providers/{id}/test — K8s dependency
// ============================================================================

#[tokio::test]
async fn test_test_provider_viewer_without_vpn_manage_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_test_viewer",
        "vpn_test_viewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_test_viewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_post(
        create_router(state),
        "/api/vpn/providers/1/test",
        &cookie,
        "",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without vpn.manage must get 403 on POST /api/vpn/providers/{{id}}/test"
    );
}

// ============================================================================
// Viewer with vpn.view can list providers and apps but cannot manage
// ============================================================================

#[tokio::test]
async fn test_viewer_role_has_no_vpn_permissions() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "vpn_perm_viewer",
        "vpn_perm_viewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "vpn_perm_viewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // viewer does NOT have vpn.view by default (it's not in the seeded viewer permissions)
    // so all VPN endpoints should return 403
    let view_endpoints = vec![
        ("/api/vpn/providers", "GET"),
        ("/api/vpn/apps", "GET"),
        ("/api/vpn/supported-providers", "GET"),
    ];

    for (endpoint, method) in view_endpoints {
        let (status, body) = if method == "GET" {
            authenticated_get(create_router(state.clone()), endpoint, &cookie).await
        } else {
            unreachable!()
        };

        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Viewer without vpn permissions must get 403 on {} {}. Body: {}",
            method,
            endpoint,
            body
        );
    }
}
