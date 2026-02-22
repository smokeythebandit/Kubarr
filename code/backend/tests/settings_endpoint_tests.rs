//! Settings endpoint integration tests
//!
//! Covers:
//! - `GET /api/settings` — list settings (requires settings.view)
//! - `GET /api/settings/{key}` — get a specific setting (requires settings.view)
//! - `PUT /api/settings/{key}` — update a setting (requires settings.manage)
//! - Permission enforcement: viewer role cannot access settings

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

/// Make an authenticated GET request and return (status, body).
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

/// Make an authenticated PUT request and return (status, body).
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

// ============================================================================
// GET /api/settings — requires auth
// ============================================================================

#[tokio::test]
async fn test_get_settings_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/settings")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/settings without a session cookie must return 401"
    );
}

// ============================================================================
// GET /api/settings — admin can list settings
// ============================================================================

#[tokio::test]
async fn test_get_settings_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "settingsadmin",
        "settingsadmin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "settingsadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(create_router(state), "/api/settings", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/settings must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    // Response must be an object containing a "settings" map
    assert!(
        json.get("settings").is_some(),
        "Settings response must include a 'settings' field. Body: {}",
        body
    );

    let settings = &json["settings"];
    assert!(
        settings.is_object(),
        "Settings must be a JSON object. Body: {}",
        body
    );

    // The two well-known default settings must be present
    assert!(
        settings.get("registration_enabled").is_some(),
        "Settings must include 'registration_enabled'. Body: {}",
        body
    );
    assert!(
        settings.get("registration_require_approval").is_some(),
        "Settings must include 'registration_require_approval'. Body: {}",
        body
    );
}

// ============================================================================
// GET /api/settings/{key}
// ============================================================================

#[tokio::test]
async fn test_get_single_setting() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "getsettingadmin",
        "getsetting@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "getsettingadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/settings/registration_enabled",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/settings/registration_enabled must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["key"], "registration_enabled",
        "Setting key must match the requested key"
    );
    assert!(
        json.get("value").is_some(),
        "Setting response must include a value field"
    );
}

#[tokio::test]
async fn test_get_unknown_setting_returns_404() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "unknownsetting",
        "unknownsetting@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "unknownsetting",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _body) = authenticated_get(
        create_router(state),
        "/api/settings/nonexistent_key_12345",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "GET /api/settings/{{unknown_key}} must return 404"
    );
}

// ============================================================================
// PUT /api/settings/{key} — update a setting
// ============================================================================

#[tokio::test]
async fn test_update_setting() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "updatesettingadmin",
        "updatesetting@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "updatesettingadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let update_body = serde_json::json!({
        "value": "false"
    })
    .to_string();

    let (status, body) = authenticated_put(
        create_router(state.clone()),
        "/api/settings/registration_enabled",
        &cookie,
        &update_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/settings/registration_enabled must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["key"], "registration_enabled",
        "Response must confirm the updated key"
    );
    assert_eq!(
        json["value"], "false",
        "Response must reflect the new value"
    );

    // Verify the change persisted by reading back
    let (get_status, get_body) = authenticated_get(
        create_router(state),
        "/api/settings/registration_enabled",
        &cookie,
    )
    .await;
    assert_eq!(get_status, StatusCode::OK);
    let get_json: serde_json::Value = serde_json::from_str(&get_body).unwrap();
    assert_eq!(
        get_json["value"], "false",
        "Updated setting value must persist on subsequent GET"
    );
}

#[tokio::test]
async fn test_update_unknown_setting_returns_error() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "updateunknown",
        "updateunknown@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "updateunknown", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let update_body = serde_json::json!({
        "value": "some_value"
    })
    .to_string();

    let (status, _body) = authenticated_put(
        create_router(state),
        "/api/settings/completely_unknown_key",
        &cookie,
        &update_body,
    )
    .await;

    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
        "PUT /api/settings/{{unknown}} must return 400 or 404. Got: {}",
        status
    );
}

// ============================================================================
// Permission enforcement: viewer cannot access settings
// ============================================================================

#[tokio::test]
async fn test_viewer_cannot_access_settings() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "settingsviewer",
        "settingsviewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "settingsviewer",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(create_router(state), "/api/settings", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer must not be able to access /api/settings (lacks settings.view). Body: {}",
        body
    );
}

// ============================================================================
// Permission enforcement: viewer cannot update settings
// ============================================================================

#[tokio::test]
async fn test_viewer_cannot_update_settings() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "settingsviewerput",
        "settingsviewerput@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "settingsviewerput",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let update_body = serde_json::json!({
        "value": "false"
    })
    .to_string();

    let (status, body) = authenticated_put(
        create_router(state),
        "/api/settings/registration_enabled",
        &cookie,
        &update_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer must not be able to update settings (lacks settings.manage). Body: {}",
        body
    );
}

// ============================================================================
// Default fallback paths: when DB rows are missing, use DEFAULT_SETTINGS
// ============================================================================

#[tokio::test]
async fn test_get_settings_uses_default_when_all_rows_deleted() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Delete all seeded settings rows to exercise the ELSE branch
    {
        use kubarr::models::prelude::SystemSetting;
        use sea_orm::EntityTrait;
        SystemSetting::delete_many().exec(&db).await.unwrap();
    }

    create_test_user_with_role(
        &db,
        "settingsdefault1",
        "settingsdefault1@example.com",
        "password123",
        "admin",
    )
    .await;

    let state = build_test_app_state_with_db(db).await;
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "settingsdefault1",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(create_router(state), "/api/settings", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/settings must return 200 with defaults. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let settings = &json["settings"];
    assert!(
        settings.get("registration_enabled").is_some(),
        "Should include registration_enabled from defaults. Body: {}",
        body
    );
    assert_eq!(
        settings["registration_enabled"]["value"], "true",
        "Default value should be 'true'. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_get_single_setting_uses_default_when_row_deleted() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Delete just registration_enabled to exercise the DEFAULT_SETTINGS fallback
    {
        use kubarr::models::prelude::SystemSetting;
        use sea_orm::EntityTrait;
        SystemSetting::delete_by_id("registration_enabled")
            .exec(&db)
            .await
            .unwrap();
    }

    create_test_user_with_role(
        &db,
        "settingsdefault2",
        "settingsdefault2@example.com",
        "password123",
        "admin",
    )
    .await;

    let state = build_test_app_state_with_db(db).await;
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "settingsdefault2",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/settings/registration_enabled",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/settings/registration_enabled must return 200 using default. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["key"], "registration_enabled", "Key must match");
    assert_eq!(json["value"], "true", "Default value must be 'true'");
}

#[tokio::test]
async fn test_update_setting_inserts_new_row_when_row_deleted() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;

    // Delete the seeded row to exercise the INSERT branch
    {
        use kubarr::models::prelude::SystemSetting;
        use sea_orm::EntityTrait;
        SystemSetting::delete_by_id("registration_enabled")
            .exec(&db)
            .await
            .unwrap();
    }

    create_test_user_with_role(
        &db,
        "settingsinsert1",
        "settingsinsert1@example.com",
        "password123",
        "admin",
    )
    .await;

    let state = build_test_app_state_with_db(db).await;
    let (_, cookie) = do_login(
        create_router(state.clone()),
        "settingsinsert1",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let update_body = serde_json::json!({ "value": "false" }).to_string();
    let (status, body) = authenticated_put(
        create_router(state.clone()),
        "/api/settings/registration_enabled",
        &cookie,
        &update_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/settings must return 200 for INSERT path. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["key"], "registration_enabled");
    assert_eq!(json["value"], "false");

    // Verify it persisted
    let (get_status, get_body) = authenticated_get(
        create_router(state),
        "/api/settings/registration_enabled",
        &cookie,
    )
    .await;
    assert_eq!(get_status, StatusCode::OK);
    let get_json: serde_json::Value = serde_json::from_str(&get_body).unwrap();
    assert_eq!(
        get_json["value"], "false",
        "Inserted value must persist on subsequent GET"
    );
}
