//! App access audit route: denied requests never create an app access event.
mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use kubarr::{
    endpoints::apps::apps_routes,
    middleware::AuthenticatedUser,
    models::{audit_log, user},
    services::catalog::AppCatalog,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tower::ServiceExt;

#[tokio::test]
async fn access_denial_leaves_no_app_access_event() {
    let db = common::create_test_db().await;
    let now = Utc::now();
    let user = user::ActiveModel {
        username: Set("auditviewer".into()),
        email: Set("auditviewer@example.test".into()),
        hashed_password: Set("unused".into()),
        is_active: Set(true),
        is_approved: Set(true),
        totp_secret: Set(None),
        totp_enabled: Set(false),
        totp_verified_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let state = common::build_test_app_state_with_db(db.clone()).await;
    let fixture = serde_json::from_value(serde_json::json!({
        "name": "sonarr", "display_name": "Sonarr", "description": "Fixture",
        "icon": "", "container_image": "unused", "default_port": 8080,
        "resource_requirements": {"cpu_request":"100m","cpu_limit":"1",
            "memory_request":"128Mi","memory_limit":"256Mi"},
        "volumes":[],"environment_variables":{},"category":"test",
        "is_system":false,"is_hidden":false,"is_browseable":true
    }))
    .unwrap();
    *state.catalog.write().await = AppCatalog::with_apps(std::collections::HashMap::from([(
        "sonarr".into(),
        fixture,
    )]));
    let app = apps_routes(state);
    for (path, expected) in [
        ("/missing/access", StatusCode::NOT_FOUND),
        ("/sonarr/access", StatusCode::FORBIDDEN),
    ] {
        let mut request = Request::builder()
            .uri(path)
            .method("POST")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(AuthenticatedUser {
            user: user.clone(),
            permissions: vec![],
        });
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected);
    }
    assert!(audit_log::Entity::find()
        .filter(audit_log::Column::Action.eq("app_accessed"))
        .all(&db)
        .await
        .unwrap()
        .is_empty());
}
