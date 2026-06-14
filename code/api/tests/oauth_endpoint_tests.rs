//! OAuth endpoint integration tests
//!
//! Covers endpoints under `/api/oauth` that can be exercised with a DB only
//! (no external OAuth provider interaction needed):
//!
//! - `GET  /api/oauth/available`            — public, lists enabled providers with credentials
//! - `GET  /api/oauth/providers`            — list all providers (requires settings.view)
//! - `GET  /api/oauth/providers/{provider}` — get a specific provider (requires settings.view)
//! - `PUT  /api/oauth/providers/{provider}` — update provider settings (requires settings.manage)
//! - `GET  /api/oauth/accounts`             — list linked OAuth accounts for current user (Authenticated)
//! - `DELETE /api/oauth/accounts/{provider}` — unlink an account (Authenticated)
//! - `GET  /api/oauth/{provider}/login`     — redirect to OAuth provider (DB lookup + redirect)
//! - `GET  /api/oauth/link/{provider}`      — start link flow (Authenticated, redirects)
//!
//! Auth checks:
//! - Unauthenticated access to protected routes → 401
//! - Authenticated user with viewer role (no settings.view) → 403
//! - Authenticated admin (has settings.view/settings.manage) → success

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

// ============================================================================
// GET /api/oauth/available — public endpoint
// ============================================================================

#[tokio::test]
async fn test_list_available_providers_requires_auth() {
    // The /api/oauth/available endpoint is behind the auth middleware — it
    // must return 401 when no session cookie is supplied.
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/oauth/available")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/oauth/available must return 401 without authentication"
    );
}

#[tokio::test]
async fn test_list_available_providers_empty_on_fresh_db() {
    // On a fresh DB there are no OAuth providers configured, so the
    // available list must be empty. Requires authentication.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthavailadmin",
        "oauthavailadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthavailadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/oauth/available", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/oauth/available must return 200 when authenticated"
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_array(),
        "Available providers must be a JSON array. Got: {}",
        body
    );
    assert!(
        json.as_array().unwrap().is_empty(),
        "Available providers must be empty on fresh DB"
    );
}

#[tokio::test]
async fn test_list_available_providers_only_shows_enabled_with_credentials() {
    // Enable a provider with credentials — it should appear in /available.
    // An enabled provider without credentials must NOT appear.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthadmin1",
        "oauthadmin1@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "oauthadmin1", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Enable google provider with full credentials
    let update_body = serde_json::json!({
        "enabled": true,
        "client_id": "test_client_id_123",
        "client_secret": "test_secret_456"
    })
    .to_string();

    let (put_status, put_body) = authenticated_put(
        create_router(state.clone()),
        "/api/oauth/providers/google",
        &cookie,
        &update_body,
    )
    .await;
    assert_eq!(
        put_status,
        StatusCode::OK,
        "Updating google provider must succeed. Body: {}",
        put_body
    );

    // Now check available — google must appear
    let (status, body) = authenticated_get(
        create_router(state.clone()),
        "/api/oauth/available",
        &cookie,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let providers = json.as_array().unwrap();
    let ids: Vec<&str> = providers.iter().filter_map(|p| p["id"].as_str()).collect();
    assert!(
        ids.contains(&"google"),
        "Google must appear in available providers after being enabled with credentials. Got: {:?}",
        ids
    );

    // Enable microsoft provider but WITHOUT credentials (no client_id/secret)
    let no_creds_body = serde_json::json!({
        "enabled": true
    })
    .to_string();
    let _ = authenticated_put(
        create_router(state.clone()),
        "/api/oauth/providers/microsoft",
        &cookie,
        &no_creds_body,
    )
    .await;

    // Refresh available list — microsoft must NOT appear (no credentials)
    let (_, body2) = authenticated_get(create_router(state), "/api/oauth/available", &cookie).await;
    let json2: serde_json::Value = serde_json::from_str(&body2).unwrap();
    let ids2: Vec<&str> = json2
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(
        !ids2.contains(&"microsoft"),
        "Microsoft must NOT appear without credentials. Got: {:?}",
        ids2
    );
}

// ============================================================================
// GET /api/oauth/providers — requires settings.view
// ============================================================================

#[tokio::test]
async fn test_list_providers_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/oauth/providers")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/oauth/providers without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_providers_viewer_lacks_settings_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthviewer1",
        "oauthviewer1@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "oauthviewer1", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/oauth/providers", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without settings.view must get 403 on GET /api/oauth/providers"
    );
}

#[tokio::test]
async fn test_list_providers_as_admin_returns_200() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthadmin2",
        "oauthadmin2@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "oauthadmin2", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/oauth/providers", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Admin must be able to list OAuth providers. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_array(),
        "Providers response must be a JSON array. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/oauth/providers/{provider} — requires settings.view
// ============================================================================

#[tokio::test]
async fn test_get_provider_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/oauth/providers/google")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/oauth/providers/google without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_nonexistent_provider_returns_404() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthgetadmin",
        "oauthgetadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "oauthgetadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/oauth/providers/nonexistent_provider_xyz",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "GET /api/oauth/providers/{{unknown}} must return 404"
    );
}

#[tokio::test]
async fn test_get_provider_after_update_returns_correct_data() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthgetafter",
        "oauthgetafter@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "oauthgetafter", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First create/update the google provider
    let update_body = serde_json::json!({
        "enabled": true,
        "client_id": "my_client_id",
        "client_secret": "my_secret"
    })
    .to_string();

    let (put_status, _) = authenticated_put(
        create_router(state.clone()),
        "/api/oauth/providers/google",
        &cookie,
        &update_body,
    )
    .await;
    assert_eq!(put_status, StatusCode::OK, "Provider update must succeed");

    // Now get the provider
    let (status, body) =
        authenticated_get(create_router(state), "/api/oauth/providers/google", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/oauth/providers/google must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["id"], "google", "Provider id must be 'google'");
    assert_eq!(json["enabled"], true, "Provider must be enabled");
    assert_eq!(
        json["client_id"], "my_client_id",
        "client_id must match what was set"
    );
    assert_eq!(
        json["has_secret"], true,
        "has_secret must be true when secret is set"
    );
    // The actual secret must NEVER be returned
    assert!(
        json.get("client_secret").is_none(),
        "client_secret must never be exposed in the response"
    );
}

// ============================================================================
// PUT /api/oauth/providers/{provider} — requires settings.manage
// ============================================================================

#[tokio::test]
async fn test_update_provider_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/oauth/providers/google")
                .method("PUT")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "PUT /api/oauth/providers/google without auth must return 401"
    );
}

#[tokio::test]
async fn test_update_provider_viewer_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthputviewer",
        "oauthputviewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthputviewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_put(
        create_router(state),
        "/api/oauth/providers/google",
        &cookie,
        r#"{"enabled":true}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer must get 403 on PUT /api/oauth/providers/{{provider}}"
    );
}

#[tokio::test]
async fn test_update_provider_enable_with_credentials() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthputadmin",
        "oauthputadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "oauthputadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let update_body = serde_json::json!({
        "enabled": true,
        "client_id": "google_client_id",
        "client_secret": "google_client_secret"
    })
    .to_string();

    let (status, body) = authenticated_put(
        create_router(state),
        "/api/oauth/providers/google",
        &cookie,
        &update_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/oauth/providers/google must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["id"], "google");
    assert_eq!(json["name"], "Google");
    assert_eq!(json["enabled"], true);
    assert_eq!(json["client_id"], "google_client_id");
    assert_eq!(json["has_secret"], true);
}

#[tokio::test]
async fn test_update_provider_disable() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthdisableadmin",
        "oauthdisableadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthdisableadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First enable
    let _ = authenticated_put(
        create_router(state.clone()),
        "/api/oauth/providers/google",
        &cookie,
        r#"{"enabled":true,"client_id":"id","client_secret":"secret"}"#,
    )
    .await;

    // Then disable
    let (status, body) = authenticated_put(
        create_router(state),
        "/api/oauth/providers/google",
        &cookie,
        r#"{"enabled":false}"#,
    )
    .await;

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
async fn test_update_microsoft_provider_creates_it() {
    // The PUT endpoint uses find-or-create logic; creating a "microsoft"
    // provider that didn't previously exist must succeed.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthmicrosoftadmin",
        "oauthmicrosoftadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthmicrosoftadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let update_body = serde_json::json!({
        "enabled": true,
        "client_id": "ms_client_id_abc",
        "client_secret": "ms_secret_xyz"
    })
    .to_string();

    let (status, body) = authenticated_put(
        create_router(state),
        "/api/oauth/providers/microsoft",
        &cookie,
        &update_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Creating microsoft provider via PUT must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["id"], "microsoft");
    assert_eq!(json["name"], "Microsoft");
    assert_eq!(json["enabled"], true);
    assert_eq!(json["client_id"], "ms_client_id_abc");
    assert_eq!(json["has_secret"], true);
}

// ============================================================================
// GET /api/oauth/accounts — requires Authenticated
// ============================================================================

#[tokio::test]
async fn test_list_linked_accounts_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/oauth/accounts")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/oauth/accounts without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_linked_accounts_empty_for_new_user() {
    // A freshly created admin user has no OAuth accounts linked.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthaccountsadmin",
        "oauthaccountsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthaccountsadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/oauth/accounts", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/oauth/accounts must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.is_array(),
        "Accounts response must be a JSON array. Body: {}",
        body
    );
    assert!(
        json.as_array().unwrap().is_empty(),
        "New user must have no linked OAuth accounts"
    );
}

#[tokio::test]
async fn test_list_linked_accounts_viewer_can_access_own_accounts() {
    // list_linked_accounts uses Authenticated (not Authorized), so viewers can
    // access their own linked accounts.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthvieweraccs",
        "oauthvieweraccs@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthvieweraccs",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/oauth/accounts", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Any authenticated user must be able to list their own OAuth accounts. Body: {}",
        body
    );
}

// ============================================================================
// DELETE /api/oauth/accounts/{provider} — requires Authenticated
// ============================================================================

#[tokio::test]
async fn test_unlink_account_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/oauth/accounts/google")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "DELETE /api/oauth/accounts/google without auth must return 401"
    );
}

#[tokio::test]
async fn test_unlink_nonexistent_account_returns_404() {
    // Unlinking a provider that isn't linked to the current user must return 404.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthunlinkadmin",
        "oauthunlinkadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthunlinkadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_delete(create_router(state), "/api/oauth/accounts/google", &cookie).await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Unlinking a provider that is not linked must return 404"
    );
}

// ============================================================================
// GET /api/oauth/{provider}/login — OAuth flow initiation
// ============================================================================

#[tokio::test]
async fn test_oauth_login_nonexistent_provider_returns_404() {
    // The login endpoint is behind auth middleware. Requires a valid session.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthloginnoexist",
        "oauthloginnoexist@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthloginnoexist",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) = authenticated_get(
        create_router(state),
        "/api/oauth/nonexistent_provider/login",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "oauth_login for a nonexistent provider must return 404"
    );
}

#[tokio::test]
async fn test_oauth_login_disabled_provider_returns_400() {
    // Create a provider that exists but is disabled — login must return 400.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthlogindisabled",
        "oauthlogindisabled@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthlogindisabled",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create the google provider but leave it disabled (default)
    let _ = authenticated_put(
        create_router(state.clone()),
        "/api/oauth/providers/google",
        &cookie,
        r#"{"enabled":false,"client_id":"id","client_secret":"secret"}"#,
    )
    .await;

    // Try to initiate login — must fail with 400 (provider disabled)
    let (status, _) =
        authenticated_get(create_router(state), "/api/oauth/google/login", &cookie).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "oauth_login for a disabled provider must return 400"
    );
}

#[tokio::test]
async fn test_oauth_login_enabled_provider_redirects() {
    // An enabled provider with client_id set must redirect (302) to the
    // provider's authorization URL.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthloginenabled",
        "oauthloginenabled@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthloginenabled",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Enable google with credentials
    let (put_status, _) = authenticated_put(
        create_router(state.clone()),
        "/api/oauth/providers/google",
        &cookie,
        r#"{"enabled":true,"client_id":"real_client_id","client_secret":"real_secret"}"#,
    )
    .await;
    assert_eq!(put_status, StatusCode::OK, "Provider setup must succeed");

    // Now call /login with auth cookie — should redirect to Google
    let request = Request::builder()
        .uri("/api/oauth/google/login")
        .method("GET")
        .header("Cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let response = create_router(state).oneshot(request).await.unwrap();

    // Should be a redirect (302/303) to Google
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "oauth_login for an enabled provider must return 303"
    );

    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    assert!(
        location.contains("accounts.google.com"),
        "Redirect location must point to Google. Got: {}",
        location
    );
    assert!(
        location.contains("client_id=real_client_id"),
        "Redirect must include client_id. Got: {}",
        location
    );
}

// ============================================================================
// GET /api/oauth/link/{provider} — account linking start
// ============================================================================

#[tokio::test]
async fn test_link_account_start_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/oauth/link/google")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/oauth/link/google without auth must return 401"
    );
}

#[tokio::test]
async fn test_link_account_start_redirects_to_login() {
    // An authenticated user with no existing google link must be redirected
    // to the login endpoint to initiate the OAuth linking flow.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthlinkadmin",
        "oauthlinkadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthlinkadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/oauth/link/google", &cookie).await;

    // The handler redirects to /api/oauth/{provider}/login which is a 302.
    // tower/axum's oneshot follows one layer of redirects only if configured —
    // by default it returns the redirect response directly.
    assert_eq!(
        status,
        StatusCode::SEE_OTHER,
        "link_account_start for an un-linked provider must return 302"
    );
}

// ============================================================================
// Provider response structure validation
// ============================================================================

#[tokio::test]
async fn test_providers_list_response_structure() {
    // After creating two providers, the list must contain well-formed entries.
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "oauthstructureadmin",
        "oauthstructureadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "oauthstructureadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Create google and microsoft providers
    for provider in ["google", "microsoft"] {
        let body = serde_json::json!({
            "enabled": false,
            "client_id": format!("{}_client", provider),
            "client_secret": format!("{}_secret", provider)
        })
        .to_string();
        let _ = authenticated_put(
            create_router(state.clone()),
            &format!("/api/oauth/providers/{}", provider),
            &cookie,
            &body,
        )
        .await;
    }

    let (status, body) =
        authenticated_get(create_router(state), "/api/oauth/providers", &cookie).await;

    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let providers = json.as_array().unwrap();

    assert!(providers.len() >= 2, "Must have at least 2 providers");

    for p in providers {
        assert!(p.get("id").is_some(), "Provider must have 'id' field");
        assert!(p.get("name").is_some(), "Provider must have 'name' field");
        assert!(
            p.get("enabled").is_some(),
            "Provider must have 'enabled' field"
        );
        assert!(
            p.get("has_secret").is_some(),
            "Provider must have 'has_secret' field"
        );
        // client_secret must NEVER appear in a list response
        assert!(
            p.get("client_secret").is_none(),
            "client_secret must never be returned in provider list"
        );
    }
}
