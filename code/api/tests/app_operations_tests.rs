//! Endpoint/permission-boundary tests, NOT session authentication tests.
//! Real apps routes and migrated SQLite; AuthenticatedUser is supplied through
//! request extensions exactly where session middleware normally places it.
//! No worker, Kubernetes client, Helm process, or external network is started.
mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use http_body_util::BodyExt;
use kubarr::{
    endpoints::apps::apps_routes,
    middleware::AuthenticatedUser,
    models::{app_operation, app_state},
    services::{catalog::AppCatalog, AppOperationResponse},
};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde_json::{json, Value};
use tower::ServiceExt;

const APP: &str = "endpoint-operation-fixture";

async fn setup() -> (Router, DatabaseConnection, AuthenticatedUser) {
    let db = common::create_test_db().await;
    let user =
        common::create_test_user(&db, "operator", "operator@example.test", "unused", true).await;
    let state = common::build_test_app_state_with_db(db.clone()).await;
    let apps = [(APP, false), ("system-operation-fixture", true)]
        .into_iter()
        .map(|(name, system)| {
            let app = serde_json::from_value(json!({
                "name": name, "display_name": name, "description": "Explicit test fixture",
                "icon": "", "container_image": "unused", "default_port": 8080,
                "resource_requirements": {"cpu_request": "100m", "cpu_limit": "1",
                    "memory_request": "128Mi", "memory_limit": "256Mi"},
                "volumes": [], "environment_variables": {}, "category": "test",
                "is_system": system, "is_hidden": false, "is_browseable": true
            }))
            .unwrap();
            (name.to_string(), app)
        })
        .collect();
    *state.catalog.write().await = AppCatalog::with_apps(apps);
    (
        apps_routes(state),
        db,
        AuthenticatedUser {
            user,
            permissions: vec![],
        },
    )
}

async fn request(
    app: &Router,
    auth: Option<&AuthenticatedUser>,
    method: &str,
    uri: &str,
    body: &str,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    if let Some(auth) = auth {
        request.extensions_mut().insert(auth.clone());
    }
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&body)
        .unwrap_or_else(|_| Value::String(String::from_utf8(body.to_vec()).unwrap()));
    (status, body)
}

async fn assert_no_rows(db: &DatabaseConnection) {
    assert!(app_operation::Entity::find()
        .all(db)
        .await
        .unwrap()
        .is_empty());
    assert!(app_state::Entity::find().all(db).await.unwrap().is_empty());
}

fn mutations() -> [(
    &'static str,
    String,
    &'static str,
    &'static str,
    &'static str,
); 4] {
    [
        (
            "POST",
            "/install".into(),
            "install",
            "apps.install",
            "installing",
        ),
        (
            "POST",
            format!("/{APP}/update"),
            "update",
            "apps.install",
            "installing",
        ),
        (
            "DELETE",
            format!("/{APP}"),
            "delete",
            "apps.delete",
            "deleting",
        ),
        (
            "POST",
            format!("/{APP}/restart"),
            "restart",
            "apps.restart",
            "installed",
        ),
    ]
}

#[tokio::test]
async fn permitted_endpoints_persist_exact_queued_operation_and_linked_state() {
    for (method, uri, kind, permission, observed) in mutations() {
        let (app, db, mut auth) = setup().await;
        auth.permissions = vec![permission.into()];
        let config = if kind == "install" {
            json!({"env.TZ": "Europe/London", "replicas": "2"})
        } else {
            json!({})
        };
        let body = json!({"app_name": APP, "custom_config": config}).to_string();
        let before = Utc::now();
        let (status, response) = request(&app, Some(&auth), method, &uri, &body).await;
        let after = Utc::now();
        assert_eq!(status, StatusCode::OK, "{response}");
        let response: AppOperationResponse = serde_json::from_value(response).unwrap();
        uuid::Uuid::parse_str(&response.id).unwrap();
        let operations = app_operation::Entity::find().all(&db).await.unwrap();
        assert_eq!(operations.len(), 1);
        let row = &operations[0];
        assert_eq!(
            serde_json::to_value(&response).unwrap(),
            serde_json::to_value(AppOperationResponse::from(row.clone())).unwrap()
        );
        assert_eq!(row.app_name, APP);
        assert_eq!(row.operation, kind);
        assert_eq!(row.status, "queued");
        assert_eq!(row.message, Some(format!("Queued {kind} for {APP}")));
        assert_eq!(row.error, None);
        assert_eq!(row.attempts, 0);
        assert_eq!(row.created_by, Some(auth.user.id));
        assert_eq!(row.started_at, None);
        assert_eq!(row.finished_at, None);
        assert_eq!(row.created_at, row.updated_at);
        assert!(row.created_at >= before && row.created_at <= after);
        assert_eq!(
            serde_json::from_str::<Value>(row.custom_config.as_deref().unwrap()).unwrap(),
            config
        );
        let states = app_state::Entity::find().all(&db).await.unwrap();
        assert_eq!(states.len(), 1);
        let state = &states[0];
        assert_eq!(state.app_name, APP);
        assert_eq!(state.namespace, APP);
        assert_eq!(
            state.desired_state,
            if kind == "delete" {
                "removed"
            } else {
                "installed"
            }
        );
        assert_eq!(state.observed_state, observed);
        assert_eq!(state.last_operation_id.as_deref(), Some(row.id.as_str()));
        assert_eq!(state.message, Some(format!("Queued {kind}")));
        assert!(!state.healthy);
        assert_eq!(state.installed_chart_version, None);
        assert_eq!(state.available_chart_version, None);
        assert!(!state.update_available);
        assert_eq!(state.last_checked_at, Some(state.updated_at));
        assert!(state.updated_at >= row.created_at && state.updated_at <= after);

        auth.permissions = vec!["apps.view".into()];
        let expected_operation = serde_json::to_value(response).unwrap();
        let expected_state = serde_json::to_value(state).unwrap();
        for (uri, expected) in [
            (
                format!("/operations/{}", row.id),
                expected_operation.clone(),
            ),
            ("/operations".into(), json!([expected_operation])),
            (format!("/{APP}/state"), expected_state.clone()),
            ("/states".into(), json!([expected_state])),
        ] {
            let (status, body) = request(&app, Some(&auth), "GET", &uri, "").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, expected);
        }
    }
}

#[tokio::test]
async fn unauthenticated_and_non_permitted_mutations_leave_no_rows() {
    let (app, db, mut auth) = setup().await;
    for (method, uri, _, required, _) in mutations() {
        let body = json!({"app_name": APP}).to_string();
        let (status, _) = request(&app, None, method, &uri, &body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_no_rows(&db).await;
        // Give every OTHER app permission to detect use of the wrong extractor.
        auth.permissions = ["apps.view", "apps.install", "apps.delete", "apps.restart"]
            .into_iter()
            .filter(|p| *p != required)
            .map(str::to_string)
            .collect();
        let (status, _) = request(&app, Some(&auth), method, &uri, &body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_no_rows(&db).await;
    }
}

#[tokio::test]
async fn malformed_install_and_unknown_catalog_requests_leave_no_rows() {
    let (app, db, mut auth) = setup().await;
    auth.permissions = vec!["apps.install".into(), "apps.restart".into()];
    for (body, expected) in [
        ("{", StatusCode::BAD_REQUEST),
        ("{}", StatusCode::UNPROCESSABLE_ENTITY),
        (r#"{"app_name": 123}"#, StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let (status, _) = request(&app, Some(&auth), "POST", "/install", body).await;
        assert_eq!(status, expected);
        assert_no_rows(&db).await;
    }
    let (status, _) = request(
        &app,
        Some(&auth),
        "POST",
        "/install",
        &json!({"app_name": APP, "custom_config": {"replicas": 2}}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_no_rows(&db).await;
    for uri in ["/install", "/missing/update", "/missing/restart"] {
        let (status, body) =
            request(&app, Some(&auth), "POST", uri, r#"{"app_name":"missing"}"#).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_no_rows(&db).await;
    }
}

#[tokio::test]
async fn system_delete_is_rejected_but_uncatalogued_delete_is_queued() {
    let (app, db, mut auth) = setup().await;
    auth.permissions = vec!["apps.delete".into()];
    let (status, _) = request(&app, Some(&auth), "DELETE", "/system-operation-fixture", "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_no_rows(&db).await;
    // Deletion intentionally permits apps that have disappeared from the catalog.
    let (status, body) = request(&app, Some(&auth), "DELETE", "/missing", "").await;
    assert_eq!(status, StatusCode::OK);
    let row = app_operation::Entity::find_by_id(body["id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.app_name, "missing");
    assert_eq!(row.operation, "delete");
    assert_eq!(row.status, "queued");
    let state = app_state::Entity::find_by_id("missing")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.desired_state, "removed");
    assert_eq!(state.last_operation_id, Some(row.id));
}

#[tokio::test]
async fn operation_and_state_reads_require_view_permission_and_unknown_ids_return_404() {
    let (app, db, mut auth) = setup().await;
    for uri in [
        "/operations",
        "/operations/missing",
        "/states",
        "/missing/state",
    ] {
        let (status, _) = request(&app, None, "GET", uri, "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = request(&app, Some(&auth), "GET", uri, "").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    auth.permissions = vec!["apps.view".into()];
    for uri in ["/operations/missing", "/missing/state"] {
        let (status, _) = request(&app, Some(&auth), "GET", uri, "").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    assert_no_rows(&db).await;
}
