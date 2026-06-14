//! Notifications endpoint integration tests
//!
//! Covers all endpoints under `/api/notifications`:
//! - Channels (CRUD + test): `settings.view` / `settings.manage` required
//! - Events (list + update): `settings.view` / `settings.manage` required
//! - User preferences (list + upsert): any authenticated user
//! - User inbox (list, mark-read, mark-all-read, delete): any authenticated user
//! - Notification logs (list): `audit.view` required
//!
//! All operations are DB-only — no Kubernetes client is required.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sea_orm::{ActiveModelTrait, Set};
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

/// Create an admin user with all necessary permissions (including settings.* and audit.view)
/// and return the session cookie.
///
/// The migration-seeded admin role contains vpn.* but NOT settings.* or audit.*.
/// We add those missing permissions directly so the notification endpoints work.
async fn setup_admin_with_settings_perms(
    db: &sea_orm::DatabaseConnection,
    username: &str,
    email: &str,
    password: &str,
) {
    use kubarr::models::{role, role_permission};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    // create_test_user_with_role already handles the user + role assignment via
    // the "admin" role name. We only need to add the extra permissions that are
    // not present in the migration-seeded admin role.
    create_test_user_with_role(db, username, email, password, "admin").await;

    // Find the admin role id
    let admin_role = kubarr::models::prelude::Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(db)
        .await
        .unwrap()
        .expect("admin role must exist after seeding");

    let extra_perms = [
        "settings.view",
        "settings.manage",
        "notifications.view",
        "notifications.manage",
        "audit.view",
        "audit.manage",
    ];

    for perm in extra_perms {
        // Check if already present (to avoid unique constraint errors)
        let exists = kubarr::models::prelude::RolePermission::find()
            .filter(role_permission::Column::RoleId.eq(admin_role.id))
            .filter(role_permission::Column::Permission.eq(perm))
            .one(db)
            .await
            .unwrap()
            .is_some();

        if !exists {
            let p = role_permission::ActiveModel {
                role_id: Set(admin_role.id),
                permission: Set(perm.to_string()),
                ..Default::default()
            };
            p.insert(db).await.unwrap();
        }
    }
}

/// Seed a notification into the current user's inbox directly, returning its id.
async fn seed_inbox_notification(
    db: &sea_orm::DatabaseConnection,
    user_id: i64,
    title: &str,
    message: &str,
) -> i64 {
    use kubarr::models::user_notification;
    let now = chrono::Utc::now();
    let notif = user_notification::ActiveModel {
        user_id: Set(user_id),
        title: Set(title.to_string()),
        message: Set(message.to_string()),
        event_type: Set(None),
        severity: Set("info".to_string()),
        read: Set(false),
        created_at: Set(now),
        ..Default::default()
    };
    let inserted = notif.insert(db).await.unwrap();
    inserted.id
}

// ============================================================================
// GET /api/notifications/channels — list channels
// ============================================================================

#[tokio::test]
async fn test_list_channels_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/channels")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/notifications/channels without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_channels_as_admin_returns_all_channel_types() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "channeladmin",
        "channeladmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "channeladmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/notifications/channels", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/notifications/channels must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array(), "Channels response must be a JSON array");

    // The response enumerates all known channel types (email, telegram, etc.)
    // even when none are configured in the DB.
    let channels = json.as_array().unwrap();
    assert!(
        !channels.is_empty(),
        "Channel list must contain at least one entry (unconfigured defaults)"
    );

    // Every entry must have channel_type, enabled, and config fields
    for ch in channels {
        assert!(
            ch.get("channel_type").is_some(),
            "Each channel must have a channel_type field"
        );
        assert!(
            ch.get("enabled").is_some(),
            "Each channel must have an enabled field"
        );
        assert!(
            ch.get("config").is_some(),
            "Each channel must have a config field"
        );
    }
}

#[tokio::test]
async fn test_list_channels_viewer_lacks_settings_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "viewerchan",
        "viewerchan@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "viewerchan", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/notifications/channels", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without settings.view must get 403 on GET /api/notifications/channels"
    );
}

// ============================================================================
// GET /api/notifications/channels/{channel_type}
// ============================================================================

#[tokio::test]
async fn test_get_channel_by_type_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/channels/email")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/notifications/channels/email without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_unconfigured_channel_returns_default_dto() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "getchanadmin",
        "getchanadmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "getchanadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // "email" channel is not configured in a fresh DB — handler returns a default DTO
    let (status, body) = authenticated_get(
        create_router(state),
        "/api/notifications/channels/email",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/notifications/channels/email must return 200 even when unconfigured. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["channel_type"], "email",
        "channel_type must match the requested type"
    );
    assert_eq!(
        json["enabled"], false,
        "Unconfigured channel must default to enabled=false"
    );
}

// ============================================================================
// PUT /api/notifications/channels/{channel_type}
// ============================================================================

#[tokio::test]
async fn test_update_channel_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/channels/email")
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
        "PUT /api/notifications/channels/email without auth must return 401"
    );
}

#[tokio::test]
async fn test_update_email_channel_config_as_admin() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "updatechanadmin",
        "updatechanadmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "updatechanadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let request_body = serde_json::json!({
        "enabled": true,
        "config": {
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "from_address": "noreply@example.com"
        }
    })
    .to_string();

    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/channels/email",
        &cookie,
        &request_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/notifications/channels/email must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["channel_type"], "email",
        "Response must reflect the updated channel type"
    );
    assert_eq!(
        json["enabled"], true,
        "Response must reflect the updated enabled flag"
    );
    assert!(
        json.get("config").is_some(),
        "Response must include config field"
    );
}

#[tokio::test]
async fn test_update_channel_invalid_type_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "invalidchanadmin",
        "invalidchanadmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "invalidchanadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/channels/not_a_real_channel",
        &cookie,
        r#"{"enabled":true}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "PUT with an invalid channel type must return 400. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_update_channel_disable_existing() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "disablechanadmin",
        "disablechanadmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "disablechanadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First enable the channel
    let enable_body = serde_json::json!({"enabled": true}).to_string();
    let (enable_status, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/channels/telegram",
        &cookie,
        &enable_body,
    )
    .await;
    assert_eq!(
        enable_status,
        StatusCode::OK,
        "Enabling channel must return 200"
    );

    // Now disable it
    let disable_body = serde_json::json!({"enabled": false}).to_string();
    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/channels/telegram",
        &cookie,
        &disable_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Disabling a channel must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["enabled"], false,
        "Disabled channel must report enabled=false"
    );
}

// ============================================================================
// POST /api/notifications/channels/{channel_type}/test
// ============================================================================

#[tokio::test]
async fn test_test_channel_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/channels/email/test")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"destination":"test@example.com"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/notifications/channels/email/test without auth must return 401"
    );
}

#[tokio::test]
async fn test_test_channel_returns_200_with_error_when_unconfigured() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "testchanadmin",
        "testchanadmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "testchanadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let request_body = serde_json::json!({
        "destination": "test@example.com"
    })
    .to_string();

    // Testing an unconfigured channel will fail gracefully (success=false, error=Some(...))
    // but the HTTP status must still be 200 because test_channel swallows the error.
    let (status, body) = authenticated_post(
        create_router(state),
        "/api/notifications/channels/email/test",
        &cookie,
        &request_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/notifications/channels/email/test must return 200 even when delivery fails. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // The response always has success + optional error
    assert!(
        json.get("success").is_some(),
        "Test channel response must include success field"
    );
    // Since email is unconfigured the test will not succeed
    assert_eq!(
        json["success"], false,
        "Test on unconfigured channel should return success=false"
    );
}

// ============================================================================
// GET /api/notifications/events — list events
// ============================================================================

#[tokio::test]
async fn test_list_events_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/events")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/notifications/events without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_events_as_admin_returns_all_audit_event_types() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(&db, "eventsadmin", "eventsadmin@example.com", "password123")
        .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "eventsadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/notifications/events", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/notifications/events must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array(), "Events response must be a JSON array");

    let events = json.as_array().unwrap();
    assert!(
        !events.is_empty(),
        "Events list must not be empty (all audit action types are returned)"
    );

    // Every entry must have event_type, enabled, severity fields
    for ev in events {
        assert!(
            ev.get("event_type").is_some(),
            "Each event must have an event_type field"
        );
        assert!(
            ev.get("enabled").is_some(),
            "Each event must have an enabled field"
        );
        assert!(
            ev.get("severity").is_some(),
            "Each event must have a severity field"
        );
    }
}

// ============================================================================
// PUT /api/notifications/events/{event_type}
// ============================================================================

#[tokio::test]
async fn test_update_event_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/events/login")
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
        "PUT /api/notifications/events/login without auth must return 401"
    );
}

#[tokio::test]
async fn test_update_event_enable_and_set_severity() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "eventupdateadmin",
        "eventupdateadmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "eventupdateadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let request_body = serde_json::json!({
        "enabled": true,
        "severity": "warning"
    })
    .to_string();

    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/events/login",
        &cookie,
        &request_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/notifications/events/login must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["event_type"], "login",
        "Response must reflect the updated event type"
    );
    assert_eq!(json["enabled"], true, "Response must reflect enabled=true");
    assert_eq!(
        json["severity"], "warning",
        "Response must reflect the updated severity"
    );
}

#[tokio::test]
async fn test_update_event_partial_only_enabled() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "eventpartialadmin",
        "eventpartialadmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "eventpartialadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Only set enabled, leave severity as default
    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/events/login_failed",
        &cookie,
        r#"{"enabled":true}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Partial update of event must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["enabled"], true);
    // Default severity is "info"
    assert_eq!(
        json["severity"], "info",
        "Default severity must be 'info' when not specified"
    );
}

#[tokio::test]
async fn test_update_event_idempotent_second_call_updates() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "eventidempotentadmin",
        "eventidempotentadmin@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "eventidempotentadmin",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First call: enable
    let (s1, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/events/logout",
        &cookie,
        r#"{"enabled":true,"severity":"info"}"#,
    )
    .await;
    assert_eq!(s1, StatusCode::OK);

    // Second call: change severity
    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/events/logout",
        &cookie,
        r#"{"severity":"error"}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Second update call must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["severity"], "error",
        "Second update must persist the new severity"
    );
}

// ============================================================================
// GET /api/notifications/preferences
// ============================================================================

#[tokio::test]
async fn test_get_preferences_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/preferences")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/notifications/preferences without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_preferences_returns_defaults_for_new_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "prefuser",
        "prefuser@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "prefuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/notifications/preferences",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/notifications/preferences must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array(), "Preferences response must be a JSON array");

    let prefs = json.as_array().unwrap();
    assert!(
        !prefs.is_empty(),
        "Preferences list must enumerate all channel types with defaults"
    );

    // All defaults should be disabled and unverified
    for pref in prefs {
        assert!(
            pref.get("channel_type").is_some(),
            "Each preference must have channel_type"
        );
        assert!(
            pref.get("enabled").is_some(),
            "Each preference must have enabled"
        );
        assert!(
            pref.get("verified").is_some(),
            "Each preference must have verified"
        );
        assert_eq!(
            pref["enabled"], false,
            "Default preference must be enabled=false"
        );
        assert_eq!(
            pref["verified"], false,
            "Default preference must be verified=false"
        );
    }
}

// ============================================================================
// PUT /api/notifications/preferences/{channel_type}
// ============================================================================

#[tokio::test]
async fn test_update_preference_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/preferences/email")
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
        "PUT /api/notifications/preferences/email without auth must return 401"
    );
}

#[tokio::test]
async fn test_update_preference_email_with_destination() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "updateprefuser",
        "updateprefuser@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "updateprefuser",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let request_body = serde_json::json!({
        "enabled": true,
        "destination": "alerts@example.com"
    })
    .to_string();

    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/preferences/email",
        &cookie,
        &request_body,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "PUT /api/notifications/preferences/email must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["channel_type"], "email",
        "Response must reflect the updated channel type"
    );
    assert_eq!(json["enabled"], true, "Preference must be enabled");
    // Destination is masked in the response for privacy
    assert!(
        json.get("destination").is_some(),
        "Response must include destination field"
    );
    // New destination is not auto-verified
    assert_eq!(
        json["verified"], false,
        "New destination must start as unverified"
    );
}

#[tokio::test]
async fn test_update_preference_invalid_channel_type_returns_400() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "invalidprefuser",
        "invalidprefuser@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "invalidprefuser",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/preferences/smoke_signal",
        &cookie,
        r#"{"enabled":true}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Invalid channel type in preferences must return 400. Body: {}",
        body
    );
}

#[tokio::test]
async fn test_update_preference_changing_destination_resets_verified() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "resetverfuser",
        "resetverfuser@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "resetverfuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First: set destination
    let body1 =
        serde_json::json!({"enabled": true, "destination": "first@example.com"}).to_string();
    let (s1, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/preferences/email",
        &cookie,
        &body1,
    )
    .await;
    assert_eq!(s1, StatusCode::OK);

    // Second: change destination — verified must reset to false
    let body2 = serde_json::json!({"destination": "second@example.com"}).to_string();
    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/preferences/email",
        &cookie,
        &body2,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "Body: {}", body);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["verified"], false,
        "Changing destination must reset verified=false"
    );
}

// ============================================================================
// GET /api/notifications/inbox
// ============================================================================

#[tokio::test]
async fn test_get_inbox_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/inbox")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/notifications/inbox without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_inbox_empty_for_new_user() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "inboxuser",
        "inboxuser@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "inboxuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/notifications/inbox", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/notifications/inbox must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("notifications").is_some(),
        "Inbox response must have notifications array"
    );
    assert!(
        json.get("total").is_some(),
        "Inbox response must have total count"
    );
    assert!(
        json.get("unread").is_some(),
        "Inbox response must have unread count"
    );
    assert_eq!(
        json["total"], 0,
        "Fresh user must have 0 total notifications"
    );
    assert_eq!(
        json["unread"], 0,
        "Fresh user must have 0 unread notifications"
    );
}

#[tokio::test]
async fn test_get_inbox_shows_seeded_notification() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let user = create_test_user_with_role(
        &db,
        "inboxseeded",
        "inboxseeded@example.com",
        "password123",
        "admin",
    )
    .await;

    // Seed a notification directly into the DB
    seed_inbox_notification(&db, user.id, "Test Alert", "Something happened").await;

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "inboxseeded", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/notifications/inbox", &cookie).await;

    assert_eq!(status, StatusCode::OK, "Body: {}", body);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["total"], 1, "Inbox must show the seeded notification");
    assert_eq!(json["unread"], 1, "Seeded notification must be unread");

    let notifications = json["notifications"].as_array().unwrap();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0]["title"], "Test Alert");
    assert_eq!(notifications[0]["message"], "Something happened");
    assert_eq!(notifications[0]["read"], false);
}

// ============================================================================
// POST /api/notifications/inbox/{id}/read — mark as read
// ============================================================================

#[tokio::test]
async fn test_mark_as_read_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/inbox/1/read")
                .method("POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/notifications/inbox/1/read without auth must return 401"
    );
}

#[tokio::test]
async fn test_mark_notification_as_read() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let user = create_test_user_with_role(
        &db,
        "markreaduser",
        "markreaduser@example.com",
        "password123",
        "admin",
    )
    .await;

    let notif_id = seed_inbox_notification(&db, user.id, "Readable", "Mark me read").await;

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "markreaduser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let uri = format!("/api/notifications/inbox/{}/read", notif_id);
    let (status, body) = authenticated_post(create_router(state.clone()), &uri, &cookie, "").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/notifications/inbox/{{id}}/read must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["success"], true,
        "Mark as read must return success=true"
    );

    // Verify the inbox now shows 0 unread
    let (inbox_status, inbox_body) =
        authenticated_get(create_router(state), "/api/notifications/inbox", &cookie).await;
    assert_eq!(inbox_status, StatusCode::OK);
    let inbox_json: serde_json::Value = serde_json::from_str(&inbox_body).unwrap();
    assert_eq!(
        inbox_json["unread"], 0,
        "After marking as read, unread count must be 0"
    );
}

// ============================================================================
// POST /api/notifications/inbox/read-all — mark all as read
// ============================================================================

#[tokio::test]
async fn test_mark_all_as_read_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/inbox/read-all")
                .method("POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "POST /api/notifications/inbox/read-all without auth must return 401"
    );
}

#[tokio::test]
async fn test_mark_all_as_read_clears_unread_count() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let user = create_test_user_with_role(
        &db,
        "markalluser",
        "markalluser@example.com",
        "password123",
        "admin",
    )
    .await;

    // Seed multiple unread notifications
    seed_inbox_notification(&db, user.id, "Alert 1", "First").await;
    seed_inbox_notification(&db, user.id, "Alert 2", "Second").await;
    seed_inbox_notification(&db, user.id, "Alert 3", "Third").await;

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "markalluser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Verify all are unread before
    let (_, before_body) = authenticated_get(
        create_router(state.clone()),
        "/api/notifications/inbox",
        &cookie,
    )
    .await;
    let before_json: serde_json::Value = serde_json::from_str(&before_body).unwrap();
    assert_eq!(before_json["unread"], 3, "Should start with 3 unread");

    // Mark all as read
    let (status, body) = authenticated_post(
        create_router(state.clone()),
        "/api/notifications/inbox/read-all",
        &cookie,
        "",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "POST /api/notifications/inbox/read-all must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json["success"], true,
        "mark_all_as_read must return success=true"
    );

    // Verify unread is now 0
    let (_, after_body) =
        authenticated_get(create_router(state), "/api/notifications/inbox", &cookie).await;
    let after_json: serde_json::Value = serde_json::from_str(&after_body).unwrap();
    assert_eq!(
        after_json["unread"], 0,
        "After mark-all-read, unread count must be 0"
    );
    assert_eq!(
        after_json["total"], 3,
        "Total count must remain 3 after marking read"
    );
}

// ============================================================================
// DELETE /api/notifications/inbox/{id}
// ============================================================================

#[tokio::test]
async fn test_delete_notification_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/inbox/1")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "DELETE /api/notifications/inbox/1 without auth must return 401"
    );
}

#[tokio::test]
async fn test_delete_notification_removes_it_from_inbox() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let user = create_test_user_with_role(
        &db,
        "deletenotifuser",
        "deletenotifuser@example.com",
        "password123",
        "admin",
    )
    .await;

    let notif_id = seed_inbox_notification(&db, user.id, "Deletable", "Delete me").await;

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "deletenotifuser",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let uri = format!("/api/notifications/inbox/{}", notif_id);
    let (status, body) = authenticated_delete(create_router(state.clone()), &uri, &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "DELETE /api/notifications/inbox/{{id}} must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["success"], true, "Delete must return success=true");

    // Verify inbox is now empty
    let (_, inbox_body) =
        authenticated_get(create_router(state), "/api/notifications/inbox", &cookie).await;
    let inbox_json: serde_json::Value = serde_json::from_str(&inbox_body).unwrap();
    assert_eq!(
        inbox_json["total"], 0,
        "After deletion, inbox must be empty"
    );
}

// ============================================================================
// GET /api/notifications/inbox/count — unread count
// ============================================================================

#[tokio::test]
async fn test_get_unread_count_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/inbox/count")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/notifications/inbox/count without auth must return 401"
    );
}

#[tokio::test]
async fn test_get_unread_count_returns_correct_count() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    let user = create_test_user_with_role(
        &db,
        "countuser",
        "countuser@example.com",
        "password123",
        "admin",
    )
    .await;

    seed_inbox_notification(&db, user.id, "N1", "msg1").await;
    seed_inbox_notification(&db, user.id, "N2", "msg2").await;

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "countuser", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) = authenticated_get(
        create_router(state),
        "/api/notifications/inbox/count",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/notifications/inbox/count must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("count").is_some(),
        "Response must include count field"
    );
    assert_eq!(json["count"], 2, "Unread count must be 2");
}

// ============================================================================
// GET /api/notifications/logs — requires audit.view
// ============================================================================

#[tokio::test]
async fn test_list_logs_requires_auth() {
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db).await;
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/logs")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "GET /api/notifications/logs without auth must return 401"
    );
}

#[tokio::test]
async fn test_list_logs_as_admin_returns_empty_list_on_fresh_db() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(&db, "logsadmin", "logsadmin@example.com", "password123").await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "logsadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/notifications/logs", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "GET /api/notifications/logs must return 200 for admin. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("logs").is_some(),
        "Logs response must have logs array"
    );
    assert!(
        json.get("total").is_some(),
        "Logs response must have total count"
    );
    assert_eq!(json["total"], 0, "Fresh DB must have 0 notification logs");
}

#[tokio::test]
async fn test_list_logs_viewer_without_audit_view_returns_403() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "viewerlogsuser",
        "viewerlogsuser@example.com",
        "password123",
        "viewer",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "viewerlogsuser",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, _) =
        authenticated_get(create_router(state), "/api/notifications/logs", &cookie).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Viewer without audit.view must get 403 on GET /api/notifications/logs"
    );
}

// ============================================================================
// Additional coverage tests for previously-uncovered branches
// ============================================================================

// list_channels: `if let Some(channel) = existing` branch (lines 247-257)
#[tokio::test]
async fn test_list_channels_returns_existing_channel_branch() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "listchannelsexist",
        "listchannelsexist@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "listchannelsexist",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Insert an email channel
    let (s, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/channels/email",
        &cookie,
        r#"{"enabled":true,"config":{"host":"smtp.test.com"}}"#,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "update_channel must return 200");

    // Now list channels — the email channel now exists, hitting the Some(channel) branch
    let (status, body) =
        authenticated_get(create_router(state), "/api/notifications/channels", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "list_channels must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let channels = json.as_array().expect("response must be an array");
    let email_ch = channels
        .iter()
        .find(|c| c["channel_type"] == "email")
        .expect("email channel must appear in list");
    assert_eq!(
        email_ch["enabled"], true,
        "email channel must show enabled=true"
    );
}

// get_channel: `Some(ch)` match arm (lines 296-307)
#[tokio::test]
async fn test_get_channel_returns_existing_config() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "getchannelexist",
        "getchannelexist@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "getchannelexist",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Insert a telegram channel
    let (s, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/channels/telegram",
        &cookie,
        r#"{"enabled":false,"config":{"chat_id":"12345"}}"#,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Get the telegram channel — hits the Some(ch) branch
    let (status, body) = authenticated_get(
        create_router(state),
        "/api/notifications/channels/telegram",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "get_channel must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["channel_type"], "telegram");
    // Config contains chat_id but it's not sensitive so it won't be masked
    assert!(
        json["config"].get("chat_id").is_some(),
        "config must include chat_id"
    );
}

// update_channel: `active.config = Set(...)` update branch (line 366)
#[tokio::test]
async fn test_update_channel_config_on_existing_channel() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "updatechconfig",
        "updatechconfig@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "updatechconfig",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First call: insert messagebird channel
    let (s1, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/channels/messagebird",
        &cookie,
        r#"{"enabled":false}"#,
    )
    .await;
    assert_eq!(s1, StatusCode::OK);

    // Second call: update existing channel WITH config — hits line 366
    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/channels/messagebird",
        &cookie,
        r#"{"config":{"originator":"KubarrTest"}}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "second update_channel must return 200. Body: {}",
        body
    );
}

// list_events: `if let Some(event) = existing` branch (lines 480-483)
#[tokio::test]
async fn test_list_events_after_event_inserted() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "listeventsexist",
        "listeventsexist@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "listeventsexist",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Insert an event setting
    let (s, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/events/login",
        &cookie,
        r#"{"enabled":true,"severity":"info"}"#,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Now list events — the login event now exists, hitting the Some(event) branch
    let (status, body) =
        authenticated_get(create_router(state), "/api/notifications/events", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "list_events must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let events = json.as_array().expect("response must be an array");
    let login_ev = events
        .iter()
        .find(|e| e["event_type"] == "login")
        .expect("login event must appear in list");
    assert_eq!(login_ev["enabled"], true);
}

// update_event: `active.enabled = Set(enabled)` when existing (line 531)
#[tokio::test]
async fn test_update_event_existing_sets_enabled() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "updateeventenabled",
        "updateeventenabled@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "updateeventenabled",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First call: insert the event
    let (s1, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/events/user_created",
        &cookie,
        r#"{"severity":"warning"}"#,
    )
    .await;
    assert_eq!(s1, StatusCode::OK);

    // Second call: update existing event WITH enabled — hits line 531
    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/events/user_created",
        &cookie,
        r#"{"enabled":true}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "second update_event with enabled must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["enabled"], true, "enabled must be persisted");
}

// get_preferences: `if let Some(pref) = existing` branch (lines 593-597)
#[tokio::test]
async fn test_get_preferences_returns_existing_pref() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "getprefexist",
        "getprefexist@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "getprefexist", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Insert a preference
    let (s, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/preferences/email",
        &cookie,
        r#"{"enabled":true,"destination":"user@example.com"}"#,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Get preferences — the email pref now exists, hitting the Some(pref) branch (lines 593-597)
    let (status, body) = authenticated_get(
        create_router(state),
        "/api/notifications/preferences",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "get_preferences must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let prefs = json.as_array().expect("response must be an array");
    let email_pref = prefs
        .iter()
        .find(|p| p["channel_type"] == "email")
        .expect("email pref must appear");
    assert_eq!(email_pref["enabled"], true);
    // destination is masked but should be present
    assert!(email_pref["destination"].is_string());
}

// update_preference: `active.enabled = Set(enabled)` when existing (line 656)
#[tokio::test]
async fn test_update_preference_existing_sets_enabled() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "updateprefenabled",
        "updateprefenabled@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "updateprefenabled",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // First call: insert preference (no enabled field, destination only)
    let (s1, _) = authenticated_put(
        create_router(state.clone()),
        "/api/notifications/preferences/telegram",
        &cookie,
        r#"{"destination":"12345"}"#,
    )
    .await;
    assert_eq!(s1, StatusCode::OK);

    // Second call: update existing pref WITH enabled — hits line 656
    let (status, body) = authenticated_put(
        create_router(state),
        "/api/notifications/preferences/telegram",
        &cookie,
        r#"{"enabled":true}"#,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "second update_preference with enabled must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["enabled"], true, "enabled must be persisted");
}

// list_logs: channel_type and status filter branches (lines 746, 749)
#[tokio::test]
async fn test_list_logs_with_channel_and_status_filters() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    setup_admin_with_settings_perms(
        &db,
        "listlogsfilter",
        "listlogsfilter@example.com",
        "password123",
    )
    .await;
    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(
        create_router(state.clone()),
        "listlogsfilter",
        "password123",
    )
    .await;
    let cookie = cookie.expect("Login must set a session cookie");

    // Call list_logs with channel_type AND status filters — covers lines 746 and 749
    let (status, body) = authenticated_get(
        create_router(state),
        "/api/notifications/logs?channel_type=email&status=sent",
        &cookie,
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "list_logs with filters must return 200. Body: {}",
        body
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("logs").is_some(),
        "response must include logs array"
    );
    assert!(json.get("total").is_some(), "response must include total");
}

// list_logs with actual log entries — exercises LogDto mapping (notifications.rs lines 764-771)
#[tokio::test]
async fn test_list_logs_with_inserted_log_covers_dto_mapping() {
    ensure_jwt_keys().await;

    let db = create_test_db_with_seed().await;
    create_test_user_with_role(
        &db,
        "logdtoadmin",
        "logdtoadmin@example.com",
        "password123",
        "admin",
    )
    .await;

    // Insert a notification log directly
    {
        use kubarr::models::notification_log;
        let now = chrono::Utc::now();
        let log_entry = notification_log::ActiveModel {
            user_id: Set(None),
            channel_type: Set("email".to_string()),
            event_type: Set("test.event".to_string()),
            recipient: Set(Some("recipient@example.com".to_string())),
            status: Set("sent".to_string()),
            error_message: Set(None),
            created_at: Set(now),
            ..Default::default()
        };
        log_entry.insert(&db).await.unwrap();
    }

    let state = build_test_app_state_with_db(db).await;

    let (_, cookie) = do_login(create_router(state.clone()), "logdtoadmin", "password123").await;
    let cookie = cookie.expect("Login must set a session cookie");

    let (status, body) =
        authenticated_get(create_router(state), "/api/notifications/logs", &cookie).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "list_logs must return 200. Body: {}",
        body
    );

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let logs = json["logs"].as_array().expect("logs must be an array");
    assert!(
        !logs.is_empty(),
        "logs must contain the inserted entry. Body: {}",
        body
    );

    // Verify the DTO fields are populated (covers LogDto mapping lines 764-771)
    let log = &logs[0];
    assert!(log.get("id").is_some(), "log must have id");
    assert!(
        log.get("channel_type").is_some(),
        "log must have channel_type"
    );
    assert!(log.get("event_type").is_some(), "log must have event_type");
    assert!(log.get("status").is_some(), "log must have status");
    assert!(log.get("created_at").is_some(), "log must have created_at");
}
