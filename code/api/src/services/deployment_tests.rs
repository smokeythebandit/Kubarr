use super::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use http::{Request, Response};
use http_body_util::BodyExt;
use k8s_openapi::api::core::v1::Secret;
use kube::{client::Body, Client};
use sea_orm::{ActiveModelTrait, ConnectionTrait, Database, DbBackend, Schema, Set};
use serde_json::{json, Value};
use tower::service_fn;

use crate::models::{app_vpn_config, vpn_provider};
use crate::services::catalog::AppConfig;

const VERSION: &str = "1.29.2-rc.1+5.1";
const SUBNETS: &str = "10.0.0.0/8,172.16.0.0/12,192.168.0.0/16";
const PRIVATE_KEY: &str = "regression-private-key-never-in-helm";
const PRESHARED_KEY: &str = "regression-preshared-key-never-in-helm";

fn catalog(app_name: &str, is_system: bool) -> AppCatalog {
    let app: AppConfig = serde_json::from_value(json!({
        "name": app_name, "display_name": "Regression App", "description": "", "icon": "",
        "container_image": "test:1", "default_port": 8080,
        "resource_requirements": {"cpu_request": "100m", "cpu_limit": "1", "memory_request": "128Mi", "memory_limit": "1Gi"},
        "volumes": [], "environment_variables": {}, "category": "test",
        "is_system": is_system, "is_hidden": false, "is_browseable": true
    })).unwrap();
    AppCatalog::with_apps(HashMap::from([(app_name.to_owned(), app)]))
}

async fn database(app_name: &str) -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let schema = Schema::new(DbBackend::Sqlite);
    for table in [
        schema.create_table_from_entity(app_state::Entity),
        schema.create_table_from_entity(vpn_provider::Entity),
        schema.create_table_from_entity(app_vpn_config::Entity),
    ] {
        db.execute(DbBackend::Sqlite.build(&table)).await.unwrap();
    }
    app_state::ActiveModel {
        app_name: Set(app_name.to_owned()),
        namespace: Set(app_name.to_owned()),
        desired_state: Set("running".into()),
        observed_state: Set("missing".into()),
        healthy: Set(false),
        available_chart_version: Set(Some(VERSION.into())),
        update_available: Set(false),
        updated_at: Set(Utc::now()),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    db
}

// A strict transport script proves Secret creation/cleanup ordering around Helm.
fn kube_client(
    namespace: &str,
    app_name: &str,
    with_vpn: bool,
) -> (K8sClient, Arc<Mutex<Vec<Value>>>) {
    let namespace = namespace.to_owned();
    let app_name = app_name.to_owned();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let recorded = bodies.clone();
    let service = service_fn(move |request: Request<Body>| {
        let namespace = namespace.clone();
        let app_name = app_name.clone();
        let recorded = recorded.clone();
        async move {
            let ns_path = format!("/api/v1/namespaces/{namespace}");
            let secret_path = format!("{ns_path}/secrets");
            let named_secret_path = format!("{secret_path}/vpn-{app_name}");
            let steps = if with_vpn {
                vec![
                    ("GET", ns_path.as_str(), 404),
                    ("POST", "/api/v1/namespaces", 201),
                    ("GET", ns_path.as_str(), 404),
                    ("GET", ns_path.as_str(), 200),
                    ("DELETE", named_secret_path.as_str(), 404),
                    ("POST", secret_path.as_str(), 201),
                ]
            } else {
                vec![
                    ("GET", ns_path.as_str(), 200),
                    ("DELETE", named_secret_path.as_str(), 404),
                ]
            };
            let index = recorded.lock().unwrap().len();
            let (method, path, status) = steps.get(index).expect("unexpected kube request");
            assert_eq!(request.method(), *method);
            assert_eq!(request.uri().path(), *path);
            let bytes = request.into_body().collect().await.unwrap().to_bytes();
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            recorded.lock().unwrap().push(body.clone());
            let response = if *status >= 400 {
                json!({"apiVersion": "v1", "kind": "Status", "status": "Failure", "code": status,
                    "reason": "NotFound", "message": "not found"})
            } else if *method == "POST" {
                body
            } else {
                json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": namespace}})
            };
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(*status)
                    .body(Body::from(serde_json::to_vec(&response).unwrap()))
                    .unwrap(),
            )
        }
    });
    (
        K8sClient::from_client(Client::new(service, "default")),
        bodies,
    )
}

fn ready_postgresql_client() -> K8sClient {
    let service = service_fn(|request: Request<Body>| async move {
        let response = match request.uri().path() {
            "/api/v1/namespaces/kubarr-database" => {
                json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "kubarr-database"}})
            }
            "/apis/apps/v1/namespaces/kubarr-database/statefulsets/kubarr-db" => json!({
                "apiVersion": "apps/v1",
                "kind": "StatefulSet",
                "metadata": {"name": "kubarr-db", "namespace": "kubarr-database"},
                "spec": {"replicas": 1, "selector": {"matchLabels": {"app": "kubarr-db"}}, "serviceName": "kubarr-db"},
                "status": {"readyReplicas": 1}
            }),
            path => panic!("unexpected kube request: {path}"),
        };
        Ok::<_, std::convert::Infallible>(
            Response::builder()
                .status(200)
                .body(Body::from(serde_json::to_vec(&response).unwrap()))
                .unwrap(),
        )
    });
    K8sClient::from_client(Client::new(service, "default"))
}

#[tokio::test]
async fn release_metadata_command_is_single_typed_lookup_and_preserves_prerelease() {
    let (k8s, _) = kube_client("regression-metadata", "regression-metadata", false);
    let catalog = catalog("regression-metadata", false);
    let manager = DeploymentManager::new(&k8s, &catalog);
    let lifecycle = lifecycle_for_app_name("regression-metadata");
    let mut calls = 0;
    let metadata = manager
        .release_metadata_with_command(&lifecycle, |args| {
            calls += 1;
            assert_eq!(
                args,
                [
                    "get",
                    "metadata",
                    "regression-metadata",
                    "-n",
                    "regression-metadata",
                    "-o",
                    "json"
                ]
            );
            Ok(Some(
                serde_json::to_vec(&json!({"version": VERSION, "status": "deployed"})).unwrap(),
            ))
        })
        .unwrap()
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(metadata.version, VERSION);
    assert_eq!(metadata.status, "deployed");
}

#[tokio::test]
async fn release_metadata_rejects_missing_status() {
    let (k8s, _) = kube_client("regression-metadata", "regression-metadata", false);
    let catalog = catalog("regression-metadata", false);
    let manager = DeploymentManager::new(&k8s, &catalog);
    let lifecycle = lifecycle_for_app_name("regression-metadata");
    let error = manager
        .release_metadata_with_command(&lifecycle, |_| {
            Ok(Some(
                serde_json::to_vec(&json!({"version": VERSION})).unwrap(),
            ))
        })
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing field `status`"), "{error}");
}

#[tokio::test]
async fn ready_workload_is_unhealthy_until_helm_release_is_deployed() {
    for helm_status in [
        Some("pending-upgrade"),
        Some("pending-install"),
        Some("failed"),
        Some("deployed"),
        None,
    ] {
        let k8s = ready_postgresql_client();
        let catalog = catalog("postgresql", true);
        let manager = DeploymentManager::new(&k8s, &catalog);
        let health = manager
            .app_health_with_metadata_command("postgresql", |_| {
                Ok(helm_status.map(|status| ReleaseMetadata {
                    version: VERSION.into(),
                    status: status.into(),
                }))
            })
            .await
            .unwrap();

        assert_eq!(health["deployments"][0]["healthy"], true);
        if helm_status.is_none() || helm_status == Some("deployed") {
            assert_eq!(health["healthy"], true);
            assert_eq!(health["status"], "healthy");
            assert!(health.get("helm_status").is_none());
        } else {
            assert_eq!(health["healthy"], false);
            assert_eq!(health["status"], "unhealthy");
            assert_eq!(health["helm_status"], helm_status.unwrap());
            assert!(health["message"]
                .as_str()
                .unwrap()
                .contains(helm_status.unwrap()));
        }
    }
}

#[tokio::test]
async fn deploy_app_composes_helm4_flags_and_release_workload_namespaces() {
    for (app_name, is_system, namespace, release_namespace) in [
        (
            "regression-media",
            false,
            "regression-media",
            "regression-media",
        ),
        ("postgresql", true, "kubarr-database", "default"),
    ] {
        // A DB-pinned version avoids depending on any installed charts or global env changes.
        let db = database(app_name).await;
        let catalog = catalog(app_name, is_system);
        for wait in [false, true] {
            for reuse_values in [false, true] {
                let (k8s, bodies) = kube_client(namespace, app_name, false);
                let manager = DeploymentManager::with_db(&k8s, &catalog, &db);
                let request = DeploymentRequest {
                    app_name: app_name.into(),
                    custom_config: HashMap::from([("image.tag".into(), "regression".into())]),
                    reuse_values,
                    gpu: None,
                    wait,
                };
                let mut calls = 0;
                let result = manager
                    .deploy_app_with_command(&request, None, |args| {
                        calls += 1;
                        assert_eq!(
                            bodies.lock().unwrap().len(),
                            1,
                            "namespace must be checked first"
                        );
                        assert_eq!(
                            &args[..4],
                            &["upgrade", "--install", "--server-side=false", app_name]
                        );
                        assert_eq!(args[4], format!("{}/{app_name}", CONFIG.charts.registry));
                        assert_eq!(
                            &args[5..8],
                            &["-n", release_namespace, "--create-namespace"]
                        );
                        assert!(args.windows(2).any(|pair| pair == ["--version", VERSION]));
                        assert_eq!(args.contains(&"--reuse-values"), reuse_values);
                        assert_eq!(args.contains(&"--wait=legacy"), wait);
                        assert_eq!(args.contains(&"--rollback-on-failure"), wait);
                        assert_eq!(args.contains(&"--timeout"), wait);
                        if wait {
                            assert!(args.windows(2).any(|pair| pair == ["--timeout", "10m"]));
                        }
                        for obsolete in ["--atomic", "--wait", "--force"] {
                            assert!(!args.contains(&obsolete));
                        }
                        let values: Vec<_> = args
                            .windows(2)
                            .filter(|pair| pair[0] == "--set")
                            .map(|pair| pair[1])
                            .collect();
                        assert!(values.contains(&format!("namespace.name={namespace}").as_str()));
                        assert!(values.contains(&"namespace.create=false"));
                        assert!(values.contains(&"image.tag=regression"));
                        assert_eq!(
                            values.contains(&"securityContext.allowPrivilegeEscalation=true"),
                            !is_system
                        );
                        assert!(!args.contains(&"--values"));
                        assert!(values.contains(&"vpn.enabled=false"));
                        assert!(values.contains(&"vpn.secretName="));
                        Ok("deployed".into())
                    })
                    .await
                    .unwrap();
                assert_eq!(calls, 1);
                assert_eq!(result.app_name, app_name);
                assert_eq!(result.namespace, namespace);
                assert_eq!(result.status, "installing");
                assert_eq!(
                    bodies.lock().unwrap().len(),
                    2,
                    "managed VPN Secret cleanup must follow successful Helm"
                );
            }
        }
    }
}

async fn assert_vpn_values_lifetime(helm_fails: bool) {
    let app_name = "regression-vpn";
    let db = database(app_name).await;
    let provider = vpn::create_vpn_provider(&db, vpn::CreateVpnProviderRequest {
        name: "Regression VPN".into(),
        vpn_type: vpn_provider::VpnType::WireGuard,
        service_provider: Some("custom".into()),
        credentials: json!({"private_key": PRIVATE_KEY, "preshared_key": PRESHARED_KEY, "addresses": ["10.2.0.2/32"]}),
        enabled: true,
        kill_switch: true,
        firewall_outbound_subnets: SUBNETS.into(),
    }).await.unwrap();
    vpn::assign_vpn_to_app(
        &db,
        app_name,
        vpn::AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: Some(false),
            port_forwarding: Some(true),
        },
    )
    .await
    .unwrap();
    let (k8s, bodies) = kube_client(app_name, app_name, true);
    let catalog = catalog(app_name, false);
    let manager = DeploymentManager::with_db(&k8s, &catalog, &db);
    let request = DeploymentRequest {
        app_name: app_name.into(),
        custom_config: HashMap::new(),
        reuse_values: false,
        gpu: None,
        wait: true,
    };
    let mut values_path = None;
    let result = manager
        .deploy_app_with_command(&request, None, |args| {
            let bodies = bodies.lock().unwrap();
            assert_eq!(
                bodies.len(),
                6,
                "namespace must be visible and Secret created before Helm"
            );
            assert_eq!(bodies[1]["metadata"]["name"], app_name);
            let secret: Secret = serde_json::from_value(bodies[5].clone()).unwrap();
            assert_eq!(secret.metadata.namespace.as_deref(), Some(app_name));
            assert_eq!(secret.metadata.name.as_deref(), Some("vpn-regression-vpn"));
            let data = secret.data.unwrap();
            assert_eq!(data["WIREGUARD_PRIVATE_KEY"].0, PRIVATE_KEY.as_bytes());
            assert_eq!(data["WIREGUARD_PRESHARED_KEY"].0, PRESHARED_KEY.as_bytes());

            let paths: Vec<_> = args
                .windows(2)
                .filter(|pair| pair[0] == "--values")
                .map(|pair| pair[1])
                .collect();
            assert_eq!(paths.len(), 1);
            let path = PathBuf::from(paths[0]);
            let yaml =
                fs::read_to_string(&path).expect("values file must exist during Helm execution");
            let values: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(
                values["vpn"]["firewallOutboundSubnets"].as_str(),
                Some(SUBNETS)
            );
            assert_eq!(
                values,
                serde_yaml::to_value(json!({"vpn": {"firewallOutboundSubnets": SUBNETS}})).unwrap()
            );
            assert!(!args
                .iter()
                .any(|arg| arg.contains("firewallOutboundSubnets") || arg.contains(SUBNETS)));
            for credential in [PRIVATE_KEY, PRESHARED_KEY] {
                assert!(!args.iter().any(|arg| arg.contains(credential)));
                assert!(!yaml.contains(credential));
            }
            for value in [
                "vpn.enabled=true",
                "vpn.secretName=vpn-regression-vpn",
                "vpn.killSwitch=false",
                "vpn.portForwarding.enabled=true",
            ] {
                assert!(args.windows(2).any(|pair| pair == ["--set", value]));
            }
            values_path = Some(path);
            if helm_fails {
                Err(AppError::Internal("injected Helm failure".into()))
            } else {
                Ok("deployed".into())
            }
        })
        .await;
    let path = values_path.expect("Helm must execute");
    assert_eq!(
        fs::metadata(path).unwrap_err().kind(),
        std::io::ErrorKind::NotFound,
        "values file must be removed on both success and failure"
    );
    if helm_fails {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("injected Helm failure"));
    } else {
        assert_eq!(result.unwrap().namespace, app_name);
    }
}

#[tokio::test]
async fn deploy_app_vpn_keeps_cidrs_and_credentials_out_of_args_and_cleans_values_on_success() {
    assert_vpn_values_lifetime(false).await;
}

#[tokio::test]
async fn deploy_app_vpn_cleans_values_and_propagates_helm_failure() {
    assert_vpn_values_lifetime(true).await;
}

#[tokio::test]
async fn deploy_app_assigned_vpn_explicitly_resets_disabled_port_forwarding() {
    let app_name = "regression-vpn-port-forwarding-off";
    let db = database(app_name).await;
    let provider = vpn::create_vpn_provider(
        &db,
        vpn::CreateVpnProviderRequest {
            name: "No forwarding VPN".into(),
            vpn_type: vpn_provider::VpnType::WireGuard,
            service_provider: Some("custom".into()),
            credentials: json!({"private_key": PRIVATE_KEY, "addresses": ["10.2.0.2/32"]}),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: SUBNETS.into(),
        },
    )
    .await
    .unwrap();
    vpn::assign_vpn_to_app(
        &db,
        app_name,
        vpn::AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: Some(false),
        },
    )
    .await
    .unwrap();
    let (k8s, _) = kube_client(app_name, app_name, true);
    let catalog = catalog(app_name, false);
    let request = DeploymentRequest {
        app_name: app_name.into(),
        custom_config: HashMap::new(),
        reuse_values: true,
        gpu: None,
        wait: false,
    };

    DeploymentManager::with_db(&k8s, &catalog, &db)
        .deploy_app_with_command(&request, None, |args| {
            assert!(args.contains(&"--reuse-values"));
            assert!(args
                .windows(2)
                .any(|pair| pair == ["--set", "vpn.portForwarding.enabled=false"]));
            assert!(!args
                .windows(2)
                .any(|pair| pair == ["--set", "vpn.portForwarding.enabled=true"]));
            Ok("deployed".into())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn deploy_app_namespace_failure_prevents_helm_execution() {
    let service = service_fn(|request: Request<Body>| async move {
        assert_eq!(request.method(), "GET");
        assert_eq!(request.uri().path(), "/api/v1/namespaces/regression-denied");
        Ok::<_, std::convert::Infallible>(Response::builder().status(403).body(Body::from(
            serde_json::to_vec(&json!({"apiVersion": "v1", "kind": "Status", "status": "Failure", "code": 403, "reason": "Forbidden", "message": "injected denial"})).unwrap()
        )).unwrap())
    });
    let k8s = K8sClient::from_client(Client::new(service, "default"));
    let db = database("regression-denied").await;
    let catalog = catalog("regression-denied", false);
    let request: DeploymentRequest =
        serde_json::from_value(json!({"app_name": "regression-denied"})).unwrap();
    let error = DeploymentManager::with_db(&k8s, &catalog, &db)
        .deploy_app_with_command(&request, None, |_| {
            panic!("Helm must not execute after namespace failure")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Failed to get namespace"), "{error}");
    assert!(error.contains("injected denial"), "{error}");
}

#[tokio::test]
async fn deploy_app_successfully_deletes_unassigned_managed_vpn_secret_after_helm() {
    let app_name = "regression-vpn-cleanup";
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let service = service_fn(move |request: Request<Body>| {
        let recorded = recorded.clone();
        async move {
            let method = request.method().clone();
            let path = request.uri().path().to_owned();
            recorded
                .lock()
                .unwrap()
                .push((method.clone(), path.clone()));
            let response = match (method.as_str(), path.as_str()) {
                ("GET", "/api/v1/namespaces/regression-vpn-cleanup") => json!({
                    "apiVersion": "v1", "kind": "Namespace",
                    "metadata": {"name": "regression-vpn-cleanup"}
                }),
                (
                    "DELETE",
                    "/api/v1/namespaces/regression-vpn-cleanup/secrets/vpn-regression-vpn-cleanup",
                ) => json!({
                    "apiVersion": "v1", "kind": "Status", "status": "Success", "code": 200
                }),
                _ => panic!("unexpected kube request: {method} {path}"),
            };
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(200)
                    .body(Body::from(serde_json::to_vec(&response).unwrap()))
                    .unwrap(),
            )
        }
    });
    let k8s = K8sClient::from_client(Client::new(service, "default"));
    let db = database(app_name).await;
    let catalog = catalog(app_name, false);
    let request: DeploymentRequest = serde_json::from_value(json!({"app_name": app_name})).unwrap();
    let helm_called = Arc::new(Mutex::new(false));
    let called = helm_called.clone();

    DeploymentManager::with_db(&k8s, &catalog, &db)
        .deploy_app_with_command(&request, None, |_| {
            *called.lock().unwrap() = true;
            assert_eq!(requests.lock().unwrap().len(), 1);
            Ok("deployed".into())
        })
        .await
        .unwrap();

    assert!(*helm_called.lock().unwrap());
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn deploy_app_helm_failure_retains_unassigned_managed_vpn_secret() {
    let app_name = "regression-vpn-rollback";
    let (k8s, requests) = kube_client(app_name, app_name, false);
    let db = database(app_name).await;
    let catalog = catalog(app_name, false);
    let request: DeploymentRequest = serde_json::from_value(json!({"app_name": app_name})).unwrap();

    let error = DeploymentManager::with_db(&k8s, &catalog, &db)
        .deploy_app_with_command(&request, None, |_| {
            Err(AppError::Internal("injected Helm rollback".into()))
        })
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("injected Helm rollback"), "{error}");
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "failed Helm must not delete credentials needed by a rollback pod"
    );
}

#[tokio::test]
async fn deploy_app_vpn_secret_cleanup_forbidden_fails_after_successful_helm() {
    let app_name = "regression-vpn-cleanup-denied";
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let service = service_fn(move |request: Request<Body>| {
        let recorded = recorded.clone();
        async move {
            let method = request.method().clone();
            let path = request.uri().path().to_owned();
            recorded
                .lock()
                .unwrap()
                .push((method.clone(), path.clone()));
            let (status, response) = match (method.as_str(), path.as_str()) {
                ("GET", "/api/v1/namespaces/regression-vpn-cleanup-denied") => (
                    200,
                    json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": app_name}}),
                ),
                (
                    "DELETE",
                    "/api/v1/namespaces/regression-vpn-cleanup-denied/secrets/vpn-regression-vpn-cleanup-denied",
                ) => (
                    403,
                    json!({"apiVersion": "v1", "kind": "Status", "status": "Failure", "code": 403, "reason": "Forbidden", "message": "injected cleanup denial"}),
                ),
                _ => panic!("unexpected kube request: {method} {path}"),
            };
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(status)
                    .body(Body::from(serde_json::to_vec(&response).unwrap()))
                    .unwrap(),
            )
        }
    });
    let k8s = K8sClient::from_client(Client::new(service, "default"));
    let db = database(app_name).await;
    let catalog = catalog(app_name, false);
    let request: DeploymentRequest = serde_json::from_value(json!({"app_name": app_name})).unwrap();
    let mut helm_calls = 0;

    let error = DeploymentManager::with_db(&k8s, &catalog, &db)
        .deploy_app_with_command(&request, None, |_| {
            helm_calls += 1;
            Ok("deployed".into())
        })
        .await
        .unwrap_err()
        .to_string();

    assert_eq!(helm_calls, 1);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(error.contains("VPN sidecar was removed"), "{error}");
    assert!(error.contains("failed to delete managed Secret"), "{error}");
    assert!(error.contains("injected cleanup denial"), "{error}");
}

#[tokio::test]
async fn deploy_app_vpn_secret_create_failure_prevents_helm_execution() {
    let app_name = "regression-secret-denied";
    let db = database(app_name).await;
    let provider = vpn::create_vpn_provider(
        &db,
        vpn::CreateVpnProviderRequest {
            name: "Denied VPN".into(),
            vpn_type: vpn_provider::VpnType::WireGuard,
            service_provider: Some("custom".into()),
            credentials: json!({"private_key": PRIVATE_KEY, "addresses": ["10.2.0.2/32"]}),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: SUBNETS.into(),
        },
    )
    .await
    .unwrap();
    vpn::assign_vpn_to_app(
        &db,
        app_name,
        vpn::AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: Some(false),
        },
    )
    .await
    .unwrap();

    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let service = service_fn(move |request: Request<Body>| {
        let recorded = recorded.clone();
        async move {
            let method = request.method().clone();
            let path = request.uri().path().to_owned();
            recorded
                .lock()
                .unwrap()
                .push((method.clone(), path.clone()));
            let (status, response) = match (method.as_str(), path.as_str()) {
                ("GET", "/api/v1/namespaces/regression-secret-denied") => (
                    200,
                    json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": app_name}}),
                ),
                (
                    "DELETE",
                    "/api/v1/namespaces/regression-secret-denied/secrets/vpn-regression-secret-denied",
                ) => (
                    404,
                    json!({"apiVersion": "v1", "kind": "Status", "status": "Failure", "code": 404, "reason": "NotFound"}),
                ),
                ("POST", "/api/v1/namespaces/regression-secret-denied/secrets") => (
                    403,
                    json!({"apiVersion": "v1", "kind": "Status", "status": "Failure", "code": 403, "reason": "Forbidden", "message": "injected secret denial"}),
                ),
                _ => panic!("unexpected kube request: {method} {path}"),
            };
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(status)
                    .body(Body::from(serde_json::to_vec(&response).unwrap()))
                    .unwrap(),
            )
        }
    });
    let k8s = K8sClient::from_client(Client::new(service, "default"));
    let catalog = catalog(app_name, false);
    let request: DeploymentRequest = serde_json::from_value(json!({"app_name": app_name})).unwrap();
    let error = DeploymentManager::with_db(&k8s, &catalog, &db)
        .deploy_app_with_command(&request, None, |_| {
            panic!("Helm must not execute after Secret creation failure")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("injected secret denial"), "{error}");
    assert_eq!(requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn deploy_app_vpn_lookup_failure_prevents_kube_and_helm_calls() {
    let app_name = "regression-vpn-lookup-error";
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let catalog = catalog(app_name, false);
    let service = service_fn(|request: Request<Body>| async move {
        panic!("Kubernetes must not be called after VPN lookup failure: {request:?}");
        #[allow(unreachable_code)]
        Ok::<_, std::convert::Infallible>(Response::new(Body::empty()))
    });
    let k8s = K8sClient::from_client(Client::new(service, "default"));
    let request: DeploymentRequest = serde_json::from_value(json!({"app_name": app_name})).unwrap();
    let error = DeploymentManager::with_db(&k8s, &catalog, &db)
        .deploy_app_with_command(&request, None, |_| {
            panic!("Helm must not execute after VPN lookup failure")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("app_vpn_configs"), "{error}");
}

#[tokio::test]
async fn deploy_app_disabled_assigned_vpn_prevents_kube_and_helm_calls() {
    let app_name = "regression-disabled-vpn";
    let db = database(app_name).await;
    let provider = vpn::create_vpn_provider(
        &db,
        vpn::CreateVpnProviderRequest {
            name: "Disabled VPN".into(),
            vpn_type: vpn_provider::VpnType::WireGuard,
            service_provider: Some("custom".into()),
            credentials: json!({"private_key": PRIVATE_KEY, "addresses": ["10.2.0.2/32"]}),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: SUBNETS.into(),
        },
    )
    .await
    .unwrap();
    vpn::assign_vpn_to_app(
        &db,
        app_name,
        vpn::AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: Some(false),
        },
    )
    .await
    .unwrap();
    vpn::update_vpn_provider(
        &db,
        provider.id,
        vpn::UpdateVpnProviderRequest {
            name: None,
            service_provider: None,
            credentials: None,
            enabled: Some(false),
            kill_switch: None,
            firewall_outbound_subnets: None,
        },
    )
    .await
    .unwrap();

    let catalog = catalog(app_name, false);
    let service = service_fn(|request: Request<Body>| async move {
        panic!("Kubernetes must not be called for a disabled assigned VPN: {request:?}");
        #[allow(unreachable_code)]
        Ok::<_, std::convert::Infallible>(Response::new(Body::empty()))
    });
    let k8s = K8sClient::from_client(Client::new(service, "default"));
    let request: DeploymentRequest = serde_json::from_value(json!({"app_name": app_name})).unwrap();
    let error = DeploymentManager::with_db(&k8s, &catalog, &db)
        .deploy_app_with_command(&request, None, |_| {
            panic!("Helm must not execute for a disabled assigned VPN")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.to_ascii_lowercase().contains("disabled"), "{error}");
}

#[tokio::test]
async fn deploy_app_without_database_explicitly_disables_reused_vpn_values() {
    let app_name = "regression-no-database";
    let catalog = catalog(app_name, false);
    let (k8s, requests) = kube_client(app_name, app_name, false);
    let request: DeploymentRequest = serde_json::from_value(json!({
        "app_name": app_name,
        "reuse_values": true
    }))
    .unwrap();
    DeploymentManager::new(&k8s, &catalog)
        .deploy_app_with_command(&request, None, |args| {
            assert!(args.contains(&"--reuse-values"));
            assert!(args
                .windows(2)
                .any(|pair| pair == ["--set", "vpn.enabled=false"]));
            assert!(args
                .windows(2)
                .any(|pair| pair == ["--set", "vpn.secretName="]));
            Ok("deployed".into())
        })
        .await
        .unwrap();
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "a manager without database assignment state must not delete Secrets"
    );
}

#[tokio::test]
async fn gpu_disable_clears_reused_values_and_omission_preserves_them() {
    let app_name = "plex";
    let catalog = catalog(app_name, false);
    for (gpu, expected) in [(None, false), (Some(None), true)] {
        let (k8s, _) = kube_client(app_name, app_name, false);
        let request = DeploymentRequest {
            app_name: app_name.into(),
            custom_config: HashMap::new(),
            gpu,
            reuse_values: true,
            wait: false,
        };
        DeploymentManager::new(&k8s, &catalog)
            .deploy_app_with_command(&request, None, |args| {
                assert!(args.contains(&"--reuse-values"));
                for value in [
                    "gpu.enabled=false",
                    "gpu.provider=intel",
                    "gpu.resourceName=",
                    "gpu.runtimeClassName=",
                    "nodeSelector.kubernetes\\.io/hostname=null",
                    "plex.resources.requests.nvidia\\.com/gpu=null",
                    "plex.resources.limits.gpu\\.intel\\.com/i915=null",
                ] {
                    assert_eq!(
                        args.windows(2).any(|pair| pair == ["--set", value]),
                        expected
                    );
                }
                Ok("deployed".into())
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn gpu_install_validates_node_then_sets_chart_provider_and_hostname_label() {
    let service = service_fn(|request: Request<Body>| async move {
        let body = match request.uri().path() {
            "/api/v1/nodes/gpu-node" => json!({
                "apiVersion":"v1", "kind":"Node",
                "metadata":{"name":"gpu-node","labels":{"kubernetes.io/hostname":"host-a"}},
                "status":{"conditions":[{"type":"Ready","status":"True","lastHeartbeatTime":"2026-01-01T00:00:00Z","lastTransitionTime":"2026-01-01T00:00:00Z"}],
                    "allocatable":{"nvidia.com/gpu.shared":"1"}}
            }),
            "/api/v1/namespaces/plex" => {
                json!({"apiVersion":"v1","kind":"Namespace","metadata":{"name":"plex"}})
            }
            path => panic!("unexpected kube request: {path}"),
        };
        Ok::<_, std::convert::Infallible>(Response::new(Body::from(
            serde_json::to_vec(&body).unwrap(),
        )))
    });
    let k8s = K8sClient::from_client(Client::new(service, "default"));
    let catalog = catalog("plex", false);
    let request: DeploymentRequest = serde_json::from_value(json!({
        "app_name":"plex", "gpu":{"vendor":"nvidia", "node_name":"gpu-node", "resource_name":"nvidia.com/gpu.shared"}
    }))
    .unwrap();
    DeploymentManager::new(&k8s, &catalog)
        .deploy_app_with_command(&request, None, |args| {
            for value in [
                "gpu.enabled=true",
                "gpu.provider=nvidia",
                "gpu.resourceName=nvidia.com/gpu.shared",
                "plex.resources.requests.nvidia\\.com/gpu=null",
                "plex.resources.limits.nvidia\\.com/gpu\\.shared=null",
                "nodeSelector.kubernetes\\.io/hostname=host-a",
            ] {
                assert!(
                    args.windows(2).any(|pair| pair == ["--set", value]),
                    "missing {value}"
                );
            }
            Ok("deployed".into())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn gpu_deploy_rejects_unadvertised_resource_before_helm_or_namespace_changes() {
    let service = service_fn(|request: Request<Body>| async move {
        assert_eq!(request.uri().path(), "/api/v1/nodes/gpu-node");
        let node = json!({
            "apiVersion":"v1", "kind":"Node",
            "metadata":{"name":"gpu-node","labels":{"kubernetes.io/hostname":"host-a"}},
            "status":{"conditions":[{"type":"Ready","status":"True","lastHeartbeatTime":"2026-01-01T00:00:00Z","lastTransitionTime":"2026-01-01T00:00:00Z"}],
                "allocatable":{"nvidia.com/gpu":"1"}}
        });
        Ok::<_, std::convert::Infallible>(Response::new(Body::from(
            serde_json::to_vec(&node).unwrap(),
        )))
    });
    let k8s = K8sClient::from_client(Client::new(service, "default"));
    let catalog = catalog("plex", false);
    let request: DeploymentRequest = serde_json::from_value(json!({
        "app_name":"plex", "gpu":{"vendor":"nvidia", "node_name":"gpu-node", "resource_name":"nvidia.com/gpu.shared"},
        "reuse_values":true
    })).unwrap();
    let error = DeploymentManager::new(&k8s, &catalog)
        .deploy_app_with_command(&request, None, |_| panic!("Helm must not run"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("nvidia.com/gpu.shared"));
}

#[tokio::test]
async fn deploy_app_rejects_managed_vpn_custom_config_before_api_calls() {
    let app_name = "regression-vpn-overrides";
    let db = database(app_name).await;
    let provider = vpn::create_vpn_provider(
        &db,
        vpn::CreateVpnProviderRequest {
            name: "Override VPN".into(),
            vpn_type: vpn_provider::VpnType::WireGuard,
            service_provider: Some("custom".into()),
            credentials: json!({"private_key": PRIVATE_KEY, "addresses": ["10.2.0.2/32"]}),
            enabled: true,
            kill_switch: true,
            firewall_outbound_subnets: SUBNETS.into(),
        },
    )
    .await
    .unwrap();
    vpn::assign_vpn_to_app(
        &db,
        app_name,
        vpn::AssignVpnRequest {
            vpn_provider_id: provider.id,
            kill_switch_override: None,
            port_forwarding: Some(false),
        },
    )
    .await
    .unwrap();
    let catalog = catalog(app_name, false);
    for key in [
        "vpn",
        "vpn.enabled",
        "vpn[0].enabled",
        "vpn.[0].enabled",
        "image.tag,vpn.killSwitch",
    ] {
        let service = service_fn(|request: Request<Body>| async move {
            panic!("Kubernetes must not be called for reserved custom config: {request:?}");
            #[allow(unreachable_code)]
            Ok::<_, std::convert::Infallible>(Response::new(Body::empty()))
        });
        let k8s = K8sClient::from_client(Client::new(service, "default"));
        let request = DeploymentRequest {
            app_name: app_name.into(),
            custom_config: HashMap::from([(key.into(), "false".into())]),
            reuse_values: true,
            gpu: None,
            wait: false,
        };
        let error = DeploymentManager::with_db(&k8s, &catalog, &db)
            .deploy_app_with_command(&request, None, |_| {
                panic!("Helm must not execute for reserved custom config")
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(key), "{error}");
        assert!(error.contains("use VPN settings"), "{error}");
    }

    for value in [
        "tag,vpn=false",
        "tag,vpn.enabled=false",
        "tag,vpn[0].enabled=false",
        "tag,vpn={enabled:false}",
        "tag,{ vpn.killSwitch=false,other}",
        r"tag\,vpn.secretName=attacker-secret",
    ] {
        let service = service_fn(|request: Request<Body>| async move {
            panic!("Kubernetes must not be called for an injected VPN override: {request:?}");
            #[allow(unreachable_code)]
            Ok::<_, std::convert::Infallible>(Response::new(Body::empty()))
        });
        let k8s = K8sClient::from_client(Client::new(service, "default"));
        let request = DeploymentRequest {
            app_name: app_name.into(),
            custom_config: HashMap::from([("image.tag".into(), value.into())]),
            reuse_values: true,
            gpu: None,
            wait: false,
        };
        let error = DeploymentManager::with_db(&k8s, &catalog, &db)
            .deploy_app_with_command(&request, None, |_| {
                panic!("Helm must not execute for an injected VPN override")
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("custom_config key 'image.tag'"), "{error}");
        assert!(error.contains("use VPN settings"), "{error}");
        assert!(
            !error.contains(value),
            "error must not disclose value: {error}"
        );
    }
}

#[test]
fn custom_config_allows_values_without_injected_vpn_assignments() {
    let custom_config = HashMap::from([
        ("image.tag".into(), "stable,replicaCount=2".into()),
        ("podAnnotations.example".into(), "{alpha,beta}".into()),
        ("environment.NOTE".into(), "vpn.enabled=false".into()),
    ]);
    validate_custom_config(&custom_config).unwrap();
}
