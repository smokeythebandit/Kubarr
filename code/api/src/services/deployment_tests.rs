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

// A strict transport script also proves that namespace visibility precedes Secret creation.
fn kube_client(namespace: &str, with_vpn: bool) -> (K8sClient, Arc<Mutex<Vec<Value>>>) {
    let namespace = namespace.to_owned();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let recorded = bodies.clone();
    let service = service_fn(move |request: Request<Body>| {
        let namespace = namespace.clone();
        let recorded = recorded.clone();
        async move {
            let ns_path = format!("/api/v1/namespaces/{namespace}");
            let secret_path = format!("{ns_path}/secrets");
            let named_secret_path = format!("{secret_path}/vpn-{namespace}");
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
                vec![("GET", ns_path.as_str(), 200)]
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
                let (k8s, bodies) = kube_client(namespace, false);
                let manager = DeploymentManager::with_db(&k8s, &catalog, &db);
                let request = DeploymentRequest {
                    app_name: app_name.into(),
                    custom_config: HashMap::from([("image.tag".into(), "regression".into())]),
                    reuse_values,
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
                        assert!(!args.iter().any(|arg| arg.starts_with("vpn.")));
                        Ok("deployed".into())
                    })
                    .await
                    .unwrap();
                assert_eq!(calls, 1);
                assert_eq!(result.app_name, app_name);
                assert_eq!(result.namespace, namespace);
                assert_eq!(result.status, "installing");
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
    let (k8s, bodies) = kube_client(app_name, true);
    let catalog = catalog(app_name, false);
    let manager = DeploymentManager::with_db(&k8s, &catalog, &db);
    let request = DeploymentRequest {
        app_name: app_name.into(),
        custom_config: HashMap::new(),
        reuse_values: false,
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
