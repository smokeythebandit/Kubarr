//! `/api/notifications/routes`: per-event × per-channel routing.
//!
//! GET needs `settings.view`, PUT needs `settings.manage`. Missing routes are
//! off; changes are audited as `system_setting_changed`.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tower::util::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

use kubarr::endpoints::create_router;

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

async fn send(
    app: axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let mut request = Request::builder().uri(uri).method(method);
    if let Some(cookie) = cookie {
        request = request.header("Cookie", cookie);
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// Log in a fresh user with `role`; returns (app, cookie, db).
async fn login_as(role: &str, name: &str) -> (axum::Router, String, DatabaseConnection) {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    // Random per run: no credential literal in the source.
    let password = uuid::Uuid::new_v4().to_string();
    create_test_user_with_role(&db, name, &format!("{name}@example.com"), &password, role).await;
    let state = build_test_app_state_with_db(db.clone()).await;
    let body = serde_json::json!({ "username": name, "password": password });
    let response = create_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            (s.starts_with("kubarr_session=") && !s.contains("kubarr_session_"))
                .then(|| s.split(';').next().unwrap().to_string())
        })
        .expect("Login must set a session cookie");
    (create_router(state), cookie, db)
}

fn route_for<'a>(routes: &'a serde_json::Value, event_type: &str) -> &'a serde_json::Value {
    routes
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["event_type"] == event_type)
        .unwrap_or_else(|| panic!("{event_type} missing"))
}

#[tokio::test]
async fn routes_require_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let (status, _) = send(
        create_router(state.clone()),
        "GET",
        "/api/notifications/routes",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        create_router(state),
        "PUT",
        "/api/notifications/routes/login",
        None,
        Some(serde_json::json!({ "channels": { "email": true } })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn viewer_cannot_read_or_change_routes() {
    let (app, cookie, _) = login_as("viewer", "routeviewer").await;
    let (status, _) = send(
        app.clone(),
        "GET",
        "/api/notifications/routes",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(
        app,
        "PUT",
        "/api/notifications/routes/login",
        Some(&cookie),
        Some(serde_json::json!({ "channels": { "email": true } })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn list_routes_returns_every_event_with_seeded_defaults() {
    let (app, cookie, _) = login_as("admin", "routelister").await;
    let (status, body) = send(app, "GET", "/api/notifications/routes", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let routes: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(routes.as_array().unwrap().len() >= 20);

    let off = serde_json::json!({ "email": false, "telegram": false, "messagebird": false });
    // Severity seeding is covered by the service tests; `login` has no row: off.
    assert_eq!(route_for(&routes, "login")["channels"], off);
}

#[tokio::test]
async fn update_route_upserts_audits_and_keeps_unlisted_channels() {
    let (app, cookie, db) = login_as("admin", "routeeditor").await;
    let (status, body) = send(
        app.clone(),
        "PUT",
        "/api/notifications/routes/login",
        Some(&cookie),
        Some(serde_json::json!({ "channels": { "email": true, "telegram": true } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(
        app.clone(),
        "PUT",
        "/api/notifications/routes/login",
        Some(&cookie),
        Some(serde_json::json!({ "channels": { "telegram": false } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let updated: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        updated,
        serde_json::json!({
            "event_type": "login",
            "channels": { "email": true, "telegram": false, "messagebird": false }
        })
    );

    let (_, body) = send(app, "GET", "/api/notifications/routes", Some(&cookie), None).await;
    let routes: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(route_for(&routes, "login"), &updated);

    use kubarr::models::audit_log;
    let audits = audit_log::Entity::find()
        .filter(audit_log::Column::Action.eq("system_setting_changed"))
        .filter(audit_log::Column::ResourceId.eq("notification_routes"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(audits.len(), 2, "each change is audited");
}

#[tokio::test]
async fn update_route_rejects_unknown_channel_event_or_empty_body() {
    let (app, cookie, _) = login_as("admin", "routevalidator").await;
    for (uri, body) in [
        (
            "/api/notifications/routes/login",
            serde_json::json!({ "channels": { "signal": true } }),
        ),
        (
            "/api/notifications/routes/not_an_event",
            serde_json::json!({ "channels": { "email": true } }),
        ),
        (
            "/api/notifications/routes/login",
            serde_json::json!({ "channels": {} }),
        ),
    ] {
        let (status, response) = send(app.clone(), "PUT", uri, Some(&cookie), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {response}");
    }
    // A malformed body never reaches the handler.
    let (status, _) = send(
        app,
        "PUT",
        "/api/notifications/routes/login",
        Some(&cookie),
        Some(serde_json::json!({ "channels": { "email": "yes" } })),
    )
    .await;
    assert!(status.is_client_error());
}
