//! Audit contract for VPN provider endpoints, without a Kubernetes cluster.
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use kubarr::{
    endpoints::create_router,
    models::{audit_log, user, vpn_provider},
    services::vpn::{self, AssignVpnRequest, CreateVpnProviderRequest, UpdateVpnProviderRequest},
    state::AppState,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, PaginatorTrait, QueryFilter, Statement,
};
use tower::ServiceExt;

mod common;
use common::{build_test_app_state_with_db, create_test_db_with_seed, create_test_user_with_role};

const SECRET: &str = "vpn-secret-sentinel-9f31";

async fn call(
    state: &AppState,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: &str,
) -> (StatusCode, String, Option<String>) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie);
    }
    let response = create_router(state.clone())
        .oneshot(req.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|s| s.starts_with("kubarr_session="))
        .map(|s| s.split(';').next().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned(), cookie)
}

#[tokio::test]
async fn provider_audits_only_successes_and_never_credentials() {
    let db = create_test_db_with_seed().await;
    kubarr::services::init_jwt_keys(&db).await.unwrap();
    create_test_user_with_role(
        &db,
        "vpn_auditor",
        "vpn_auditor@example.com",
        "password123",
        "admin",
    )
    .await;
    create_test_user_with_role(
        &db,
        "vpn_viewer",
        "vpn_viewer@example.com",
        "password123",
        "viewer",
    )
    .await;
    let actor = user::Entity::find()
        .filter(user::Column::Username.eq("vpn_auditor"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let state = build_test_app_state_with_db(db.clone()).await;
    let (status, _, cookie) = call(
        &state,
        "POST",
        "/auth/login",
        None,
        r#"{"username":"vpn_auditor","password":"password123"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.unwrap();
    let (_, _, viewer_cookie) = call(
        &state,
        "POST",
        "/auth/login",
        None,
        r#"{"username":"vpn_viewer","password":"password123"}"#,
    )
    .await;
    let viewer_cookie = viewer_cookie.unwrap();
    let create = serde_json::json!({"name":"Audit VPN", "vpn_type":"wireguard", "service_provider":"custom", "credentials":{"private_key":SECRET,"addresses":["10.0.0.1/32"]}}).to_string();
    assert_eq!(
        call(&state, "POST", "/api/vpn/providers", None, &create)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &state,
            "POST",
            "/api/vpn/providers",
            Some(&viewer_cookie),
            &create
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(vpn_provider::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(
        audit_log::Entity::find()
            .filter(audit_log::Column::ResourceType.eq("vpn"))
            .count(&db)
            .await
            .unwrap(),
        0
    );
    let invalid =
        serde_json::json!({"name":"Invalid", "vpn_type":"wireguard", "credentials":{}}).to_string();
    assert_eq!(
        call(
            &state,
            "POST",
            "/api/vpn/providers",
            Some(&cookie),
            &invalid
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(vpn_provider::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(
        audit_log::Entity::find()
            .filter(audit_log::Column::ResourceType.eq("vpn"))
            .count(&db)
            .await
            .unwrap(),
        0
    );
    let (status, body, _) =
        call(&state, "POST", "/api/vpn/providers", Some(&cookie), &create).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_i64()
        .unwrap();
    let uri = format!("/api/vpn/providers/{id}");
    let update = serde_json::json!({"credentials":{"private_key":SECRET}, "firewall_outbound_subnets": "172.16.0.0/12", "enabled":false}).to_string();
    let invalid_update =
        serde_json::json!({"name":"Should not persist", "credentials":{}}).to_string();
    assert_eq!(
        call(&state, "PUT", &uri, Some(&cookie), &invalid_update)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        vpn_provider::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Audit VPN"
    );
    assert_eq!(
        audit_log::Entity::find()
            .filter(audit_log::Column::ResourceType.eq("vpn"))
            .count(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        call(&state, "PUT", &uri, Some(&cookie), &update).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &state,
            "PUT",
            "/api/vpn/providers/999999",
            Some(&cookie),
            &update
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &state,
            "DELETE",
            "/api/vpn/providers/999999",
            Some(&cookie),
            ""
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    vpn::update_vpn_provider(
        &db,
        id,
        UpdateVpnProviderRequest {
            name: None,
            service_provider: None,
            credentials: None,
            enabled: Some(true),
            kill_switch: None,
            firewall_outbound_subnets: None,
        },
    )
    .await
    .unwrap();
    vpn::assign_vpn_to_app(
        &db,
        "sonarr",
        AssignVpnRequest {
            vpn_provider_id: id,
            kill_switch_override: None,
            port_forwarding: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        call(&state, "DELETE", &uri, Some(&cookie), "").await.0,
        StatusCode::BAD_REQUEST
    );
    assert!(vpn_provider::Entity::find_by_id(id)
        .one(&db)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        audit_log::Entity::find()
            .filter(audit_log::Column::ResourceType.eq("vpn"))
            .count(&db)
            .await
            .unwrap(),
        2
    );
    vpn::remove_vpn_from_app(&db, "sonarr").await.unwrap();
    assert_eq!(
        call(&state, "DELETE", &uri, Some(&cookie), "").await.0,
        StatusCode::OK
    );

    let logs = audit_log::Entity::find()
        .filter(audit_log::Column::ResourceType.eq("vpn"))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(logs.len(), 3);
    for (log, action) in logs.iter().zip([
        "vpn_provider_created",
        "vpn_provider_updated",
        "vpn_provider_deleted",
    ]) {
        assert_eq!(log.action, action);
        assert_eq!(log.resource_id.as_deref(), Some(id.to_string().as_str()));
        assert_eq!(log.user_id, Some(actor.id));
        assert_eq!(log.username.as_deref(), Some("vpn_auditor"));
        assert!(log.success);
        assert_eq!(log.ip_address, None);
        assert!(!serde_json::to_string(log).unwrap().contains(SECRET));
        assert!(!serde_json::to_string(log)
            .unwrap()
            .contains("172.16.0.0/12"));
    }
    let details: serde_json::Value =
        serde_json::from_str(logs[1].details.as_ref().unwrap()).unwrap();
    assert_eq!(
        details["fields"],
        serde_json::json!(["credentials", "enabled", "firewall_outbound_subnets"])
    );
}

#[tokio::test]
async fn provider_mutations_roll_back_if_audit_insert_fails() {
    for method in ["POST", "PUT", "DELETE"] {
        let db = create_test_db_with_seed().await;
        kubarr::services::init_jwt_keys(&db).await.unwrap();
        create_test_user_with_role(
            &db,
            "vpn_rollback",
            "vpn_rollback@example.com",
            "password123",
            "admin",
        )
        .await;
        let state = build_test_app_state_with_db(db.clone()).await;
        let (status, _, cookie) = call(
            &state,
            "POST",
            "/auth/login",
            None,
            r#"{"username":"vpn_rollback","password":"password123"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let cookie = cookie.unwrap();

        // Seed outside the endpoint before removing the audit table so update/delete
        // exercise a real existing provider without requiring an audit insert.
        let existing = if method == "POST" {
            None
        } else {
            Some(
                vpn::create_vpn_provider(
                    &db,
                    CreateVpnProviderRequest {
                        name: "Original".into(),
                        vpn_type: vpn_provider::VpnType::WireGuard,
                        service_provider: Some("custom".into()),
                        credentials: serde_json::json!({"private_key": SECRET}),
                        enabled: true,
                        kill_switch: true,
                        firewall_outbound_subnets: "10.0.0.0/8".into(),
                    },
                )
                .await
                .unwrap(),
            )
        };
        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TABLE audit_logs".to_string(),
        ))
        .await
        .unwrap();

        let uri = existing
            .as_ref()
            .map(|p| format!("/api/vpn/providers/{}", p.id))
            .unwrap_or_else(|| "/api/vpn/providers".into());
        let body = match method {
            "POST" => serde_json::json!({"name":"Rolled back", "vpn_type":"wireguard", "credentials":{"private_key":SECRET}}).to_string(),
            "PUT" => serde_json::json!({"name":"Changed", "enabled":false, "credentials":{"private_key":"replacement-secret"}}).to_string(),
            _ => String::new(),
        };
        let (status, _, _) = call(&state, method, &uri, Some(&cookie), &body).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{method}");
        let providers = vpn_provider::Entity::find().all(&db).await.unwrap();
        match existing {
            None => assert!(providers.is_empty(), "create must roll back"),
            Some(original) => {
                assert_eq!(providers.len(), 1, "{method} must preserve the provider");
                let persisted = &providers[0];
                assert_eq!(persisted.id, original.id);
                assert_eq!(persisted.name, "Original");
                assert!(persisted.enabled);
                assert!(persisted.credentials_json.contains(SECRET));
                assert!(!persisted.credentials_json.contains("replacement-secret"));
            }
        }
    }
}
