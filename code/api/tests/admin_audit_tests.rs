//! End-to-end audit coverage for administrative mutations.
mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};
use http_body_util::BodyExt;
use kubarr::{
    endpoints::create_router,
    models::{audit_log, prelude::AuditLog},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};
use tower::ServiceExt;

static JWT_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn setup() -> (kubarr::state::AppState, DatabaseConnection, i64, String) {
    JWT_INIT
        .get_or_init(|| async {
            let db = create_test_db_with_seed().await;
            kubarr::services::init_jwt_keys(&db).await.unwrap();
        })
        .await;
    let db = create_test_db_with_seed().await;
    let actor = create_test_user_with_role(
        &db,
        "auditadmin",
        "admin@example.com",
        "password123",
        "admin",
    )
    .await;
    let state = build_test_app_state_with_db(db.clone()).await;
    let response = create_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"username":"auditadmin","password":"password123"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("kubarr_session=") && !v.contains("kubarr_session_"))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    (state, db, actor.id, cookie)
}

async fn send(
    state: &kubarr::state::AppState,
    cookie: &str,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let response = create_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .method(method)
                .header("Cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "{} {}: {}",
        method,
        path,
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

async fn event(
    db: &DatabaseConnection,
    action: &str,
    target: &str,
    actor: i64,
) -> audit_log::Model {
    let logs = AuditLog::find().all(db).await.unwrap();
    logs.into_iter()
        .find(|log| {
            log.action == action
                && log.resource_id.as_deref() == Some(target)
                && log.user_id == Some(actor)
                && log.username.as_deref() == Some("auditadmin")
                && log.success
        })
        .unwrap_or_else(|| panic!("missing {action} on {target}"))
}

#[tokio::test]
async fn role_binding_audits_only_actual_deltas() {
    let (state, db, actor, cookie) = setup().await;
    let roles = kubarr::models::prelude::Role::find()
        .all(&db)
        .await
        .unwrap();
    let id_of = |name: &str| roles.iter().find(|role| role.name == name).unwrap().id;
    let admin = id_of("admin");
    let viewer = id_of("viewer");
    let downloader = id_of("downloader");
    let user = send(&state, &cookie, "POST", "/api/users", serde_json::json!({
        "username":"role_target", "email":"role_target@example.com", "password":"ROLE_PASSWORD_SENTINEL",
        "role_ids":[admin, viewer]
    })).await;
    let user_id = user["id"].as_i64().unwrap();
    let bindings = || async {
        let logs = AuditLog::find()
            .order_by_asc(audit_log::Column::Id)
            .all(&db)
            .await
            .unwrap();
        logs.into_iter()
            .filter(|log| matches!(log.action.as_str(), "role_assigned" | "role_unassigned"))
            .collect::<Vec<_>>()
    };
    let initial = bindings().await;
    assert_eq!(initial.len(), 2);
    for (log, id) in initial.iter().zip([admin, viewer]) {
        assert_eq!(log.action, "role_assigned");
        assert_eq!(log.resource_type, "role");
        assert_eq!(log.resource_id.as_deref(), Some(id.to_string().as_str()));
        assert_eq!(log.user_id, Some(actor));
        assert_eq!(log.username.as_deref(), Some("auditadmin"));
        assert!(log.success);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(log.details.as_deref().unwrap()).unwrap(),
            serde_json::json!({"target_user_id": user_id})
        );
    }
    let path = format!("/api/users/{user_id}");
    send(
        &state,
        &cookie,
        "PATCH",
        &path,
        serde_json::json!({"role_ids":[viewer, downloader]}),
    )
    .await;
    let changed = bindings().await;
    assert_eq!(changed.len(), 4);
    assert_eq!(changed[2].action, "role_unassigned");
    assert_eq!(
        changed[2].resource_id.as_deref(),
        Some(admin.to_string().as_str())
    );
    assert_eq!(changed[3].action, "role_assigned");
    assert_eq!(
        changed[3].resource_id.as_deref(),
        Some(downloader.to_string().as_str())
    );
    for log in &changed[2..] {
        assert_eq!(log.user_id, Some(actor));
        assert_eq!(log.resource_type, "role");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(log.details.as_deref().unwrap()).unwrap(),
            serde_json::json!({"target_user_id": user_id})
        );
    }
    send(
        &state,
        &cookie,
        "PATCH",
        &path,
        serde_json::json!({"role_ids":[viewer, downloader]}),
    )
    .await;
    assert_eq!(
        bindings().await.len(),
        4,
        "identical replacement must not emit binding events"
    );
    assert!(
        !serde_json::to_string(&AuditLog::find().all(&db).await.unwrap())
            .unwrap()
            .contains("ROLE_PASSWORD_SENTINEL")
    );
}

#[tokio::test]
async fn settings_and_role_mutations_record_actor_and_target() {
    let (state, db, actor, cookie) = setup().await;
    send(
        &state,
        &cookie,
        "PUT",
        "/api/settings/registration_enabled",
        serde_json::json!({"value":"false"}),
    )
    .await;
    let setting = event(&db, "system_setting_changed", "registration_enabled", actor).await;
    assert_eq!(setting.resource_type, "system");
    assert!(setting.details.is_none());

    let role = send(
        &state,
        &cookie,
        "POST",
        "/api/roles",
        serde_json::json!({"name":"audited_role"}),
    )
    .await;
    let id = role["id"].as_i64().unwrap().to_string();
    let created = event(&db, "role_created", &id, actor).await;
    assert_eq!(created.resource_type, "role");
    send(
        &state,
        &cookie,
        "PUT",
        &format!("/api/roles/{id}/permissions"),
        serde_json::json!({"permissions":["users.view"]}),
    )
    .await;
    let logs = AuditLog::find().all(&db).await.unwrap();
    assert!(logs
        .iter()
        .any(|l| l.action == "role_updated" && l.resource_id.as_deref() == Some(id.as_str())));
    send(
        &state,
        &cookie,
        "DELETE",
        &format!("/api/roles/{id}"),
        serde_json::json!({}),
    )
    .await;
    event(&db, "role_deleted", &id, actor).await;
}

#[tokio::test]
async fn user_password_and_invitation_audits_exclude_secrets() {
    let (state, db, actor, cookie) = setup().await;
    let user = send(&state, &cookie, "POST", "/api/users", serde_json::json!({
        "username":"targetuser", "email":"target@example.com", "password":"SECRET_PASSWORD_SENTINEL"
    })).await;
    let id = user["id"].as_i64().unwrap().to_string();
    event(&db, "user_created", &id, actor).await;
    send(
        &state,
        &cookie,
        "PATCH",
        &format!("/api/users/{id}/password"),
        serde_json::json!({"new_password":"SECRET_RESET_SENTINEL"}),
    )
    .await;
    event(&db, "password_changed", &id, actor).await;
    let invite = send(
        &state,
        &cookie,
        "POST",
        "/api/users/invites",
        serde_json::json!({}),
    )
    .await;
    let invite_id = invite["id"].as_i64().unwrap().to_string();
    event(&db, "invite_created", &invite_id, actor).await;
    send(
        &state,
        &cookie,
        "DELETE",
        &format!("/api/users/invites/{invite_id}"),
        serde_json::json!({}),
    )
    .await;
    event(&db, "invite_deleted", &invite_id, actor).await;
    let logs = AuditLog::find().all(&db).await.unwrap();
    let serialized = serde_json::to_string(&logs).unwrap();
    for secret in [
        "SECRET_PASSWORD_SENTINEL",
        "SECRET_RESET_SENTINEL",
        invite["code"].as_str().unwrap(),
    ] {
        assert!(!serialized.contains(secret), "audit row contained secret");
    }
}

#[tokio::test]
async fn approval_and_deletion_are_distinct_events() {
    let (state, db, actor, cookie) = setup().await;
    let user = send(&state, &cookie, "POST", "/api/users", serde_json::json!({
        "username":"pendinguser", "email":"pending@example.com", "password":"SECRET_PENDING_SENTINEL"
    })).await;
    let id = user["id"].as_i64().unwrap().to_string();
    send(
        &state,
        &cookie,
        "POST",
        &format!("/api/users/{id}/approve"),
        serde_json::json!({}),
    )
    .await;
    event(&db, "user_approved", &id, actor).await;
    send(
        &state,
        &cookie,
        "DELETE",
        &format!("/api/users/{id}"),
        serde_json::json!({}),
    )
    .await;
    event(&db, "user_deleted", &id, actor).await;
    let logs = AuditLog::find().all(&db).await.unwrap();
    assert!(!serde_json::to_string(&logs)
        .unwrap()
        .contains("SECRET_PENDING_SENTINEL"));
}

#[tokio::test]
async fn setting_write_rolls_back_when_audit_insert_fails() {
    let (state, db, _actor, cookie) = setup().await;
    db.execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/settings/registration_enabled")
                .method("PUT")
                .header("Cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"value":"false"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_server_error());
    let setting = kubarr::models::prelude::SystemSetting::find_by_id("registration_enabled")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(setting.value, "true");
}

async fn failed_audit_request(
    state: kubarr::state::AppState,
    cookie: &str,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) {
    let response = create_router(state)
        .oneshot(
            Request::builder()
                .uri(uri)
                .method(method)
                .header("Cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_server_error(),
        "{} {}: {}",
        method,
        uri,
        response.status()
    );
}

#[tokio::test]
async fn role_permissions_remain_when_audit_insert_fails() {
    let (state, db, _, cookie) = setup().await;
    let role = send(
        &state,
        &cookie,
        "POST",
        "/api/roles",
        serde_json::json!({"name":"rollback_role"}),
    )
    .await;
    let id = role["id"].as_i64().unwrap();
    send(
        &state,
        &cookie,
        "PUT",
        &format!("/api/roles/{id}/permissions"),
        serde_json::json!({"permissions":["users.view","app.radarr"]}),
    )
    .await;
    db.execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    failed_audit_request(
        state,
        &cookie,
        "PUT",
        &format!("/api/roles/{id}/permissions"),
        serde_json::json!({"permissions":["apps.delete"]}),
    )
    .await;
    use kubarr::models::{role_app_permission, role_permission};
    let perms = kubarr::models::prelude::RolePermission::find()
        .filter(role_permission::Column::RoleId.eq(id))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(perms.len(), 1);
    assert_eq!(perms[0].permission, "users.view");
    let apps = kubarr::models::prelude::RoleAppPermission::find()
        .filter(role_app_permission::Column::RoleId.eq(id))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].app_name, "radarr");
}

#[tokio::test]
async fn user_role_bindings_remain_when_audit_insert_fails() {
    let (state, db, _, cookie) = setup().await;
    let user = send(
        &state,
        &cookie,
        "POST",
        "/api/users",
        serde_json::json!({
            "username":"binding_user", "email":"binding@example.com", "password":"safe-password"
        }),
    )
    .await;
    let id = user["id"].as_i64().unwrap();
    let admin_role = kubarr::models::prelude::Role::find()
        .filter(kubarr::models::role::Column::Name.eq("admin"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    send(
        &state,
        &cookie,
        "PATCH",
        &format!("/api/users/{id}"),
        serde_json::json!({"role_ids":[admin_role.id]}),
    )
    .await;
    db.execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    failed_audit_request(
        state,
        &cookie,
        "PATCH",
        &format!("/api/users/{id}"),
        serde_json::json!({"role_ids":[],"is_active":false}),
    )
    .await;
    let bindings = kubarr::models::prelude::UserRole::find()
        .filter(kubarr::models::user_role::Column::UserId.eq(id))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].role_id, admin_role.id);
    assert!(
        kubarr::models::prelude::User::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .is_active
    );
}

#[tokio::test]
async fn invite_creation_rolls_back_when_audit_insert_fails() {
    let (state, db, _, cookie) = setup().await;
    db.execute_unprepared("DROP TABLE audit_logs")
        .await
        .unwrap();
    failed_audit_request(
        state,
        &cookie,
        "POST",
        "/api/users/invites",
        serde_json::json!({}),
    )
    .await;
    assert!(kubarr::models::prelude::Invite::find()
        .all(&db)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn wrong_current_password_is_logged_without_credentials() {
    let (state, db, actor, cookie) = setup().await;
    let response = create_router(state).oneshot(Request::builder()
        .uri("/api/users/me/password").method("PATCH")
        .header("Cookie", cookie).header("content-type", "application/json")
        .body(Body::from(serde_json::json!({
            "current_password":"WRONG_PASSWORD_SENTINEL", "new_password":"NEW_PASSWORD_SENTINEL"
        }).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let logs = AuditLog::find().all(&db).await.unwrap();
    let failure = logs
        .iter()
        .find(|l| l.action == "password_changed" && !l.success)
        .unwrap();
    assert_eq!(
        failure.resource_id.as_deref(),
        Some(actor.to_string().as_str())
    );
    assert_eq!(
        failure.error_message.as_deref(),
        Some("verification_failed")
    );
    let serialized = serde_json::to_string(&logs).unwrap();
    assert!(!serialized.contains("WRONG_PASSWORD_SENTINEL"));
    assert!(!serialized.contains("NEW_PASSWORD_SENTINEL"));
}
