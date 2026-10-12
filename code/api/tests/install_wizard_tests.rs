//! Isolated chart fixture: no registry, Kubernetes, or provider credentials are read.
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use kubarr::{
    endpoints::create_router,
    models::{
        app_operation, app_state, app_storage_config, app_vpn_config, role, role_permission,
        vpn_provider,
    },
    services::{
        app_storage,
        catalog::{AppCatalog, AppConfig, ResourceRequirements},
        install_options::InstallStorage,
        vpn::{self, AssignVpnRequest},
    },
};
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;
use tower::ServiceExt;

async fn call(
    app: axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    json: Option<serde_json::Value>,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let response = app
        .oneshot(
            builder
                .body(Body::from(json.map(|v| v.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
            .to_string(),
    )
}
fn app(name: &str) -> AppConfig {
    AppConfig {
        name: name.into(),
        display_name: name.into(),
        description: "fixture".into(),
        icon: "".into(),
        container_image: "fixture".into(),
        default_port: 1234,
        resource_requirements: ResourceRequirements {
            cpu_request: "100m".into(),
            cpu_limit: "1".into(),
            memory_request: "128Mi".into(),
            memory_limit: "256Mi".into(),
        },
        volumes: vec![],
        environment_variables: HashMap::new(),
        category: "media".into(),
        is_system: false,
        is_hidden: false,
        is_browseable: true,
    }
}
async fn login(app: axum::Router, name: &str) -> String {
    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"username":name,"password":"pass123"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|v| {
            let value = v.to_str().ok()?;
            value
                .starts_with("kubarr_session_0=")
                .then(|| value.split(';').next().unwrap().to_string())
        })
        .expect("login cookie")
}

#[tokio::test]
async fn options_permissions_install_validation_and_atomic_vpn_queue() {
    // A separate integration-test binary owns its process environment. Initialise
    // CONFIG only after pointing it at these disposable, deliberately fake charts.
    let charts = tempfile::tempdir().unwrap();
    for name in [
        "qbittorrent",
        "redmine",
        "plex",
        "jackett",
        "sonarr",
        "jellyfin",
    ] {
        let dir = charts.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("Chart.yaml"),
            format!("name: {name}\nversion: 1.2.3\n"),
        )
        .unwrap();
        std::fs::write(dir.join("values.yaml"), match name {
            "qbittorrent" => "storage:\n  mounts:\n    - {name: downloads, source: downloads/qbittorrent, target: /downloads, mode: read-write}\n  config:\n    create: true\n    existingClaim: ''\n    storageClassName: ''\n    size: 5Gi\nvpn:\n  enabled: false\n",
            "plex" => "storage:\n  mounts: []\nvpn:\n  enabled: false\n",
            "jackett" => "storage:\n  mounts: []\n",
            "sonarr" => "storage:\n  mounts: []\nvpn:\n  enabled: false\n",
            "jellyfin" => "storage:\n  mounts: []\n  config:\n    create: true\n    existingClaim: ''\n    storageClassName: ''\n    size: 5Gi\nvpn:\n  enabled: false\n",
            _ => "storage:\n  mounts:\n    - {name: attachments, source: redmine/files, target: /usr/src/redmine/files, mode: read-write}\n  workloads:\n    postgres:\n      mounts:\n        - {name: database, source: redmine/postgres, target: /var/lib/postgresql/data, mode: read-write}\n",
        }).unwrap();
    }
    std::env::set_var("KUBARR_CHARTS_DIR", charts.path());
    let db = common::create_test_db_with_seed().await;
    kubarr::services::init_jwt_keys(&db).await.unwrap();
    common::create_test_user_with_role(
        &db,
        "wizard_admin",
        "wizard_admin@test.com",
        "pass123",
        "admin",
    )
    .await;
    common::create_test_user_with_role(
        &db,
        "wizard_viewer",
        "wizard_viewer@test.com",
        "pass123",
        "viewer",
    )
    .await;
    let state = common::build_test_app_state_with_db(db.clone()).await;
    *state.catalog.write().await = AppCatalog::with_apps(HashMap::from([
        ("qbittorrent".into(), app("qbittorrent")),
        ("redmine".into(), app("redmine")),
        ("plex".into(), app("plex")),
        ("jackett".into(), app("jackett")),
        ("sonarr".into(), app("sonarr")),
        ("jellyfin".into(), app("jellyfin")),
    ]));
    let router = create_router(state);
    let admin = login(router.clone(), "wizard_admin").await;
    let viewer = login(router.clone(), "wizard_viewer").await;
    let path = "/api/apps/catalog/qbittorrent/install-options";
    assert_eq!(
        call(router.clone(), "GET", path, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = call(router.clone(), "GET", path, Some(&viewer), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let options: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(options["chart_version"], "1.2.3");
    assert_eq!(options["mounts"][0]["source"], "downloads/qbittorrent");
    assert_eq!(options["private_claim"], "qbittorrent-config");
    assert_eq!(options["vpn_supported"], true);
    let (status, redmine_options) = call(
        router.clone(),
        "GET",
        "/api/apps/catalog/redmine/install-options",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let redmine_options: serde_json::Value = serde_json::from_str(&redmine_options).unwrap();
    assert_eq!(redmine_options["vpn_supported"], false);
    assert_eq!(
        redmine_options["workloads"]["postgres"][0]["target"],
        "/var/lib/postgresql/data"
    );
    let install = |value: serde_json::Value, token: &str| {
        let router = router.clone();
        let token = token.to_string();
        async move {
            call(
                router,
                "POST",
                "/api/apps/install",
                Some(&token),
                Some(value),
            )
            .await
        }
    };
    let base = serde_json::json!({"app_name":"qbittorrent","chart_version":"1.2.3","vpn":null});
    assert_eq!(
        install(base.clone(), &viewer).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        install(
            serde_json::json!({"app_name":"qbittorrent","chart_version":"stale","vpn":null}),
            &admin
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(install(serde_json::json!({"app_name":"qbittorrent","chart_version":"1.2.3","vpn":null,"custom_config":{"storage.mounts[0].source":"private"}}), &admin).await.0, StatusCode::BAD_REQUEST);
    for storage in [
        serde_json::json!({"mounts":[]}),
        serde_json::json!({"mounts":[{"name":"downloads","source":"../private","target":"/downloads","mode":"read-write"}]}),
        serde_json::json!({"mounts":[],"migrationConfirmed":true}),
    ] {
        let mut body = base.clone();
        body["storage"] = storage;
        assert_ne!(install(body, &admin).await.0, StatusCode::OK);
    }
    let now = chrono::Utc::now();
    let provider = vpn_provider::ActiveModel {
        name: Set("Fixture VPN".into()),
        vpn_type: Set(vpn_provider::VpnType::WireGuard),
        service_provider: Set(None),
        credentials_json: Set("{}".into()),
        enabled: Set(false),
        kill_switch: Set(true),
        firewall_outbound_subnets: Set("".into()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let mut choice = base.clone();
    choice["vpn"] = serde_json::json!({"vpn_provider_id":provider.id});
    assert_ne!(install(choice.clone(), &admin).await.0, StatusCode::OK);
    assert!(app_vpn_config::Entity::find_by_id("qbittorrent")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    assert!(app_operation::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .is_empty());
    let provider_id = provider.id;
    let mut active: vpn_provider::ActiveModel = provider.into();
    active.enabled = Set(true);
    active.update(&db).await.unwrap();
    let viewer_role = role::Entity::find()
        .filter(role::Column::Name.eq("viewer"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    role_permission::ActiveModel {
        role_id: Set(viewer_role.id),
        permission: Set("apps.install".into()),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let installer = login(router.clone(), "wizard_viewer").await;
    let mut unauthorized_vpn = base.clone();
    unauthorized_vpn["vpn"] = serde_json::json!({"vpn_provider_id": provider_id});
    assert_eq!(
        install(unauthorized_vpn, &installer).await.0,
        StatusCode::FORBIDDEN
    );
    vpn::assign_vpn_to_app(
        &db,
        "redmine",
        AssignVpnRequest {
            vpn_provider_id: provider_id,
            kill_switch_override: None,
            port_forwarding: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        install(
            serde_json::json!({"app_name":"redmine","chart_version":"1.2.3","vpn":null}),
            &installer
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(app_operation::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .is_empty());
    // Assignment is written first within the transaction; GPU discovery then
    // fails without Kubernetes. Neither assignment nor queue may survive.
    let rollback = serde_json::json!({"app_name":"plex","chart_version":"1.2.3","vpn":{"vpn_provider_id": provider_id}, "gpu":{"vendor":"intel","node_name":"gpu-node","resource_name":"gpu.intel.com/i915"}});
    assert_ne!(install(rollback, &admin).await.0, StatusCode::OK);
    assert!(app_vpn_config::Entity::find_by_id("plex")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    assert!(app_storage_config::Entity::find_by_id("plex")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    assert!(app_operation::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .is_empty());
    let (status, response) = install(choice, &admin).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let operation: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(operation["status"], "queued");
    let queued = app_operation::Entity::find_by_id(operation["id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let payload: serde_json::Value =
        serde_json::from_str(queued.custom_config.as_deref().unwrap()).unwrap();
    assert!(payload["__kubarr_install_choices"].is_string()); // stored as an opaque JSON string
    let pinned: serde_json::Value =
        serde_json::from_str(payload["__kubarr_install_choices"].as_str().unwrap()).unwrap();
    assert_eq!(pinned["chart_version"], "1.2.3");
    assert_eq!(
        pinned["storage"]["mounts"][0]["source"],
        "downloads/qbittorrent"
    );
    assert_eq!(pinned["storage"]["config"]["size"], "5Gi");
    assert_eq!(
        pinned["vpn_provider_id"],
        app_vpn_config::Entity::find_by_id("qbittorrent")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .vpn_provider_id
    );
    let stored = app_storage_config::Entity::find_by_id("qbittorrent")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.schema_version, 1);
    assert_eq!(stored.chart_version, "1.2.3");
    let saved: serde_json::Value = serde_json::from_str(&stored.config_json).unwrap();
    assert_eq!(saved, pinned["storage"]);
    let (status, visible) = call(router.clone(), "GET", path, Some(&viewer), None).await;
    assert_eq!(status, StatusCode::OK, "{visible}");
    let visible: serde_json::Value = serde_json::from_str(&visible).unwrap();
    assert_eq!(visible["saved_storage"], saved);
    assert_eq!(visible["saved_chart_version"], "1.2.3");
    let (status, redmine_install) = install(
        serde_json::json!({"app_name":"redmine","chart_version":"1.2.3","vpn":null}),
        &admin,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{redmine_install}");
    assert!(app_vpn_config::Entity::find_by_id("redmine")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    let redmine_queued: serde_json::Value = serde_json::from_str(&redmine_install).unwrap();
    app_operation::Entity::update_many()
        .col_expr(app_operation::Column::Status, "succeeded".into())
        .filter(app_operation::Column::Id.eq(redmine_queued["id"].as_str().unwrap()))
        .exec(&db)
        .await
        .unwrap();
    let (status, legacy_reinstall) =
        install(serde_json::json!({"app_name":"redmine"}), &admin).await;
    assert_eq!(status, StatusCode::OK, "{legacy_reinstall}");
    let legacy: serde_json::Value = serde_json::from_str(&legacy_reinstall).unwrap();
    let legacy_operation = app_operation::Entity::find_by_id(legacy["id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let config: serde_json::Value =
        serde_json::from_str(legacy_operation.custom_config.as_deref().unwrap()).unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_str(config["__kubarr_storage_snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(
        snapshot["storage"]["workloads"]["postgres"]["mounts"][0]["source"],
        "redmine/postgres"
    );
    // Plex has optional NFS mounts: [] is an explicit empty list in the queued
    // payload, unlike omitted storage which leaves chart defaults intact.
    let (status, plex_install) = install(serde_json::json!({"app_name":"plex","chart_version":"1.2.3","storage":{"mounts":[]},"vpn":null}), &admin).await;
    assert_eq!(status, StatusCode::OK, "{plex_install}");
    let op: serde_json::Value = serde_json::from_str(&plex_install).unwrap();
    let model = app_operation::Entity::find_by_id(op["id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let queue: serde_json::Value =
        serde_json::from_str(model.custom_config.as_deref().unwrap()).unwrap();
    let choices: serde_json::Value =
        serde_json::from_str(queue["__kubarr_install_choices"].as_str().unwrap()).unwrap();
    assert_eq!(choices["storage"]["mounts"], serde_json::json!([]));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &app_storage_config::Entity::find_by_id("plex")
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .config_json
        )
        .unwrap()["mounts"],
        serde_json::json!([])
    );
    let mut failed: app_operation::ActiveModel = queued.clone().into();
    failed.status = Set("failed".into());
    failed.update(&db).await.unwrap();
    let (status, retried) = call(
        router.clone(),
        "POST",
        &format!("/api/apps/operations/{}/retry", queued.id),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retried}");
    let retried: serde_json::Value = serde_json::from_str(&retried).unwrap();
    let replacement = app_operation::Entity::find_by_id(retried["id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replacement.custom_config, queued.custom_config);
    app_operation::Entity::update_many()
        .col_expr(app_operation::Column::Status, "succeeded".into())
        .filter(app_operation::Column::Id.eq(&replacement.id))
        .exec(&db)
        .await
        .unwrap();
    let (status, changed_vpn) = call(
        router.clone(),
        "PUT",
        "/api/vpn/apps/qbittorrent",
        Some(&admin),
        Some(serde_json::json!({"vpn_provider_id":provider_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed_vpn}");
    let vpn_change: serde_json::Value = serde_json::from_str(&changed_vpn).unwrap();
    let vpn_op = app_operation::Entity::find_by_id(vpn_change["operation_id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let vpn_snapshot: serde_json::Value =
        serde_json::from_str(vpn_op.custom_config.as_deref().unwrap()).unwrap();
    let vpn_storage: serde_json::Value =
        serde_json::from_str(vpn_snapshot["__kubarr_storage_snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(vpn_storage["storage"], saved);
    assert_eq!(vpn_storage["chart_version"], "1.2.3");
    app_operation::Entity::update_many()
        .col_expr(app_operation::Column::Status, "succeeded".into())
        .filter(app_operation::Column::Id.eq(&vpn_op.id))
        .exec(&db)
        .await
        .unwrap();
    let (status, gpu_update) = call(
        router.clone(),
        "POST",
        "/api/apps/plex/update",
        Some(&admin),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{gpu_update}");
    let update: serde_json::Value = serde_json::from_str(&gpu_update).unwrap();
    let updated = app_operation::Entity::find_by_id(update["id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let gpu_snapshot: serde_json::Value =
        serde_json::from_str(updated.custom_config.as_deref().unwrap()).unwrap();
    let gpu_storage: serde_json::Value =
        serde_json::from_str(gpu_snapshot["__kubarr_storage_snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(gpu_storage["storage"]["mounts"], serde_json::json!([]));
    assert_eq!(gpu_storage["chart_version"], "1.2.3");
    let (status, delete_response) = call(
        router.clone(),
        "DELETE",
        "/api/apps/qbittorrent",
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{delete_response}");
    assert_eq!(
        app_storage_config::Entity::find_by_id("qbittorrent")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .config_json,
        stored.config_json
    );
    let (status, legacy_first) = install(serde_json::json!({"app_name":"jackett"}), &admin).await;
    assert_eq!(status, StatusCode::OK, "{legacy_first}");
    let legacy_first: serde_json::Value = serde_json::from_str(&legacy_first).unwrap();
    let operation = app_operation::Entity::find_by_id(legacy_first["id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(operation.custom_config.as_deref(), Some("{}"));
    assert!(app_storage_config::Entity::find_by_id("jackett")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    // Simulate a successful fresh Helm install whose later cleanup failed:
    // a retry must promote only the durable proof, not change the user's mounts.
    let (status, first) = install(
        serde_json::json!({"app_name":"jellyfin","chart_version":"1.2.3","vpn":null}),
        &admin,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first: serde_json::Value = serde_json::from_str(&first).unwrap();
    let first_id = first["id"].as_str().unwrap();
    let first_row = app_operation::Entity::find_by_id(first_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let first_config: serde_json::Value =
        serde_json::from_str(first_row.custom_config.as_deref().unwrap()).unwrap();
    let first_pin: serde_json::Value =
        serde_json::from_str(first_config["__kubarr_install_choices"].as_str().unwrap()).unwrap();
    assert_eq!(first_pin["fresh_private_config"], false);
    app_storage::mark_fresh_private_config(&db, "jellyfin", "1.2.3")
        .await
        .unwrap();
    app_operation::Entity::update_many()
        .col_expr(app_operation::Column::Status, "failed".into())
        .filter(app_operation::Column::Id.eq(first_id))
        .exec(&db)
        .await
        .unwrap();
    let (status, retry) = call(
        router.clone(),
        "POST",
        &format!("/api/apps/operations/{first_id}/retry"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    let retry: serde_json::Value = serde_json::from_str(&retry).unwrap();
    let replacement = app_operation::Entity::find_by_id(retry["id"].as_str().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let replacement_config: serde_json::Value =
        serde_json::from_str(replacement.custom_config.as_deref().unwrap()).unwrap();
    let replacement_pin: serde_json::Value = serde_json::from_str(
        replacement_config["__kubarr_install_choices"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(replacement_pin["fresh_private_config"], true);
    assert_eq!(replacement_pin["storage"], first_pin["storage"]);
    // Fail the final write after the VPN assignment and operation insert.
    let operations_before = app_operation::Entity::find().all(&db).await.unwrap().len();
    db.execute_unprepared("CREATE TRIGGER block_saved_storage BEFORE INSERT ON app_storage_configs BEGIN SELECT RAISE(ABORT, 'storage write failed'); END").await.unwrap();
    let (status, _) = install(serde_json::json!({"app_name":"sonarr","chart_version":"1.2.3","vpn":{"vpn_provider_id":provider_id}}), &admin).await;
    assert_ne!(status, StatusCode::OK);
    assert!(app_storage_config::Entity::find_by_id("sonarr")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    assert!(app_vpn_config::Entity::find_by_id("sonarr")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    assert!(app_state::Entity::find_by_id("sonarr")
        .one(&db)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        app_operation::Entity::find().all(&db).await.unwrap().len(),
        operations_before
    );
    app_operation::Entity::update_many()
        .col_expr(app_operation::Column::Status, "failed".into())
        .filter(app_operation::Column::Id.eq(&updated.id))
        .exec(&db)
        .await
        .unwrap();
    let revised: InstallStorage = serde_json::from_value(serde_json::json!({"mounts":[{"name":"other","source":"different","target":"/different","mode":"read"}]})).unwrap();
    app_storage::save(&db, "plex", "1.2.3", &revised)
        .await
        .unwrap();
    let (status, detail) = call(
        router.clone(),
        "POST",
        &format!("/api/apps/operations/{}/retry", updated.id),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{detail}");
}
