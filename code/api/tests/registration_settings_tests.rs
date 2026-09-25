//! Safe in-memory registration/approval/invite authorization contracts.
mod common;
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};
use http_body_util::BodyExt;
use kubarr::endpoints::create_router;
use kubarr::models::{
    prelude::{Role, SystemSetting, User},
    role, system_setting,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tower::ServiceExt;

async fn send(
    app: axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value, Option<String>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let response = app
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .find(|s| s.starts_with("kubarr_session="))
        .map(|s| s.split(';').next().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        cookie,
    )
}

async fn assert_registered_access(app: &impl Fn() -> axum::Router, cookie: &str) {
    let (status, me, _) = send(
        app(),
        "GET",
        "/api/users/me",
        Some(cookie),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["roles"], serde_json::json!([]));
    assert_eq!(me["permissions"], serde_json::json!([]));
    assert_eq!(me["allowed_apps"], serde_json::json!([]));
    for path in [
        "/api/logs/vlogs/namespaces",
        "/api/storage/download?path=private.txt",
        "/api/storage/browse?path=.",
    ] {
        let (status, _, _) = send(app(), "GET", path, Some(cookie), serde_json::json!({})).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn registration_approval_invitation_and_permissions() {
    let db = create_test_db_with_seed().await;
    kubarr::services::init_jwt_keys(&db).await.unwrap();
    create_test_user_with_role(
        &db,
        "registration_admin",
        "regadmin@example.test",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db.clone()).await;
    let app = || create_router(state.clone());
    let (status, _, admin) = send(
        app(),
        "POST",
        "/auth/login",
        None,
        serde_json::json!({"username":"registration_admin", "password":"password123"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let admin = admin.unwrap();
    let (status, available, _) = send(
        app(),
        "GET",
        "/api/roles/permissions",
        Some(&admin),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for key in ["audit.view", "audit.manage", "users.reset_password"] {
        assert!(
            available
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["key"] == key),
            "Permission matrix must expose {key}"
        );
    }
    let register = |username: &str, invite: Option<&str>| {
        serde_json::json!({
            "username": username, "email": format!("{username}@example.test"),
            "password":"test-only-password", "invite_code": invite
        })
    };

    let mut setting: system_setting::ActiveModel =
        SystemSetting::find_by_id("registration_enabled")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
    setting.value = Set("false".to_string());
    setting.update(&db).await.unwrap();
    let (status, _, _) = send(
        app(),
        "POST",
        "/auth/register",
        None,
        register("blocked_user", None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(User::find()
        .filter(kubarr::models::user::Column::Username.eq("blocked_user"))
        .one(&db)
        .await
        .unwrap()
        .is_none());

    let (status, invite, _) = send(
        app(),
        "POST",
        "/api/users/invites",
        Some(&admin),
        serde_json::json!({"expires_in_days":7}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = invite["code"].as_str().unwrap();
    let (status, response, _) = send(
        app(),
        "POST",
        "/auth/register",
        None,
        register("invited_user", Some(code)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["status"], "approved");
    let (status, _, _) = send(
        app(),
        "POST",
        "/auth/register",
        None,
        register("reused_user", Some(code)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, invited_cookie) = send(
        app(),
        "POST",
        "/auth/login",
        None,
        serde_json::json!({"username":"invited_user", "password":"test-only-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_registered_access(&app, invited_cookie.as_deref().unwrap()).await;
    let (status, _, _) = send(
        app(),
        "GET",
        "/api/settings",
        invited_cookie.as_deref(),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        app(),
        "POST",
        "/api/users/invites",
        invited_cookie.as_deref(),
        serde_json::json!({"expires_in_days":7}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let mut setting: system_setting::ActiveModel =
        SystemSetting::find_by_id("registration_enabled")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
    setting.value = Set("true".to_string());
    setting.update(&db).await.unwrap();
    let (status, response, _) = send(
        app(),
        "POST",
        "/auth/register",
        None,
        register("pending_user", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["status"], "pending");
    let (status, _, _) = send(
        app(),
        "POST",
        "/auth/login",
        None,
        serde_json::json!({"username":"pending_user", "password":"test-only-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, pending, _) = send(
        app(),
        "GET",
        "/api/users/pending",
        Some(&admin),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = pending
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "pending_user")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (status, _, _) = send(
        app(),
        "POST",
        &format!("/api/users/{id}/approve"),
        Some(&admin),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(
        app(),
        "POST",
        &format!("/api/users/{id}/reject"),
        Some(&admin),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(User::find_by_id(id).one(&db).await.unwrap().is_some());
    let (status, _, approved_cookie) = send(
        app(),
        "POST",
        "/auth/login",
        None,
        serde_json::json!({"username":"pending_user", "password":"test-only-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_registered_access(&app, approved_cookie.as_deref().unwrap()).await;

    let (status, audit, _) = send(
        app(),
        "GET",
        "/api/audit?action=invite_used",
        Some(&admin),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(audit["logs"].as_array().unwrap().iter().any(|log| {
        log["username"] == "invited_user" && log["resource_id"] == invite["id"].to_string()
    }));

    let (status, response, _) = send(
        app(),
        "POST",
        "/auth/register",
        None,
        register("rejected_user", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["status"], "pending");
    let rejected_id = User::find()
        .filter(kubarr::models::user::Column::Username.eq("rejected_user"))
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .id;
    let (status, _, _) = send(
        app(),
        "POST",
        &format!("/api/users/{rejected_id}/reject"),
        Some(&admin),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(User::find_by_id(rejected_id)
        .one(&db)
        .await
        .unwrap()
        .is_none());

    let mut setting: system_setting::ActiveModel =
        SystemSetting::find_by_id("registration_require_approval")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
    setting.value = Set("false".to_string());
    setting.update(&db).await.unwrap();
    let (status, response, _) = send(
        app(),
        "POST",
        "/auth/register",
        None,
        register("auto_user", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["status"], "approved");
    let (status, _, auto_cookie) = send(
        app(),
        "POST",
        "/auth/login",
        None,
        serde_json::json!({"username":"auto_user", "password":"test-only-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_registered_access(&app, auto_cookie.as_deref().unwrap()).await;

    // An admin who explicitly assigns viewer still grants the seeded viewer permissions.
    let viewer = Role::find()
        .filter(role::Column::Name.eq("viewer"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let (status, _, _) = send(
        app(),
        "POST",
        "/api/users",
        Some(&admin),
        serde_json::json!({"username":"assigned_viewer", "email":"assigned_viewer@example.test", "password":"test-only-password", "role_ids":[viewer.id]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, viewer_cookie) = send(
        app(),
        "POST",
        "/auth/login",
        None,
        serde_json::json!({"username":"assigned_viewer", "password":"test-only-password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, me, _) = send(
        app(),
        "GET",
        "/api/users/me",
        viewer_cookie.as_deref(),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(me["permissions"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("logs.view")));
    assert!(me["permissions"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("storage.download")));
}
