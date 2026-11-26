//! `/api/notifications/quiet-hours`: the signed-in user's own quiet hours.
//!
//! Any authenticated user; each user only reads and writes their own row.

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

fn quiet(enabled: bool, from: i32, to: i32, tz: &str) -> serde_json::Value {
    serde_json::json!({
        "enabled": enabled,
        "from_minute": from,
        "to_minute": to,
        "timezone": tz,
        "critical_bypass": true,
    })
}

#[tokio::test]
async fn quiet_hours_require_auth() {
    let state = build_test_app_state_with_db(create_test_db_with_seed().await).await;
    let (status, _) = send(
        create_router(state.clone()),
        "GET",
        "/api/notifications/quiet-hours",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        create_router(state),
        "PUT",
        "/api/notifications/quiet-hours",
        None,
        Some(quiet(true, 1380, 420, "UTC")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn defaults_before_anything_is_saved() {
    let (app, cookie, _) = login_as("viewer", "quietdefault").await;
    let (status, body) = send(
        app,
        "GET",
        "/api/notifications/quiet-hours",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let dto: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(dto["enabled"], false);
    assert_eq!(dto["configured"], false);
    assert_eq!(dto["critical_bypass"], true);
    assert_eq!(dto["from_minute"], 1380);
    assert_eq!(dto["to_minute"], 420);
}

#[tokio::test]
async fn viewer_saves_and_reads_back_own_quiet_hours() {
    let (app, cookie, db) = login_as("viewer", "quietviewer").await;
    let (status, body) = send(
        app.clone(),
        "PUT",
        "/api/notifications/quiet-hours",
        Some(&cookie),
        Some(quiet(true, 1380, 420, "Europe/Amsterdam")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(
        app,
        "GET",
        "/api/notifications/quiet-hours",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let dto: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(dto["enabled"], true);
    assert_eq!(dto["configured"], true);
    assert_eq!(dto["timezone"], "Europe/Amsterdam");
    assert_eq!(dto["from_minute"], 1380);
    assert_eq!(dto["to_minute"], 420);

    let user = kubarr::models::user::Entity::find()
        .filter(kubarr::models::user::Column::Username.eq("quietviewer"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let row = kubarr::models::user_quiet_hours::Entity::find_by_id(user.id)
        .one(&db)
        .await
        .unwrap()
        .expect("row stored for the signed-in user");
    assert!(row.enabled);
    assert_eq!(
        kubarr::models::user_quiet_hours::Entity::find()
            .all(&db)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn invalid_quiet_hours_are_rejected() {
    let (app, cookie, _) = login_as("viewer", "quietinvalid").await;
    for body in [
        quiet(true, 420, 420, "UTC"),
        quiet(false, 420, 420, "UTC"),
        quiet(true, -1, 420, "UTC"),
        quiet(true, 1380, 1440, "UTC"),
        quiet(true, 1380, 420, "Not/AZone"),
        quiet(true, 1380, 420, ""),
    ] {
        let (status, text) = send(
            app.clone(),
            "PUT",
            "/api/notifications/quiet-hours",
            Some(&cookie),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} → {text}");
    }
}

#[tokio::test]
async fn users_only_see_their_own_quiet_hours() {
    ensure_jwt_keys().await;
    let db = create_test_db_with_seed().await;
    let state = build_test_app_state_with_db(db.clone()).await;
    let mut cookies = Vec::new();
    for name in ["quietalice", "quietbob"] {
        let password = uuid::Uuid::new_v4().to_string();
        create_test_user_with_role(
            &db,
            name,
            &format!("{name}@example.com"),
            &password,
            "viewer",
        )
        .await;
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
            .unwrap();
        cookies.push(cookie);
    }
    let (status, _) = send(
        create_router(state.clone()),
        "PUT",
        "/api/notifications/quiet-hours",
        Some(&cookies[0]),
        Some(quiet(true, 60, 120, "Asia/Tokyo")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        create_router(state),
        "GET",
        "/api/notifications/quiet-hours",
        Some(&cookies[1]),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let dto: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(dto["configured"], false, "bob must not see alice's row");
    assert_eq!(dto["enabled"], false);
}
