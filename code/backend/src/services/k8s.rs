use std::collections::{BTreeMap, HashMap};

use k8s_openapi::api::core::v1::{
    NFSVolumeSource, Namespace, PersistentVolume, PersistentVolumeClaim, PersistentVolumeClaimSpec,
    PersistentVolumeSpec, Pod, Secret, Service, VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{Api, ListParams, PostParams},
    config::{Config, KubeConfigOptions, Kubeconfig},
    Client,
};
use serde::Deserialize;

use crate::config::CONFIG;
use crate::error::{AppError, Result};

/// Kubernetes client manager
pub struct K8sClient {
    client: Client,
}

impl K8sClient {
    /// Create a new Kubernetes client
    pub async fn new() -> Result<Self> {
        let client = if CONFIG.kubernetes.in_cluster {
            // In-cluster config
            let config = Config::incluster()?;
            Client::try_from(config)?
        } else if let Some(ref kubeconfig_path) = CONFIG.kubernetes.kubeconfig_path {
            // Explicit kubeconfig path
            let kubeconfig = Kubeconfig::read_from(kubeconfig_path)?;
            let config =
                Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default()).await?;
            Client::try_from(config)?
        } else {
            // Default kubeconfig
            Client::try_default().await?
        };

        Ok(Self { client })
    }

    /// Get the Kubernetes client
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Check if metrics-server is available
    #[allow(clippy::expect_used)]
    pub async fn check_metrics_server_available(&self) -> bool {
        // Try to list pod metrics
        let result = self
            .client
            .request::<PodMetricsList>(
                http::Request::get("/apis/metrics.k8s.io/v1beta1/pods?limit=1")
                    .body(vec![])
                    .expect("static URL is valid"),
            )
            .await;

        result.is_ok()
    }

    /// Get pod status for a namespace
    pub async fn get_pod_status(
        &self,
        namespace: &str,
        app_name: Option<&str>,
    ) -> Result<Vec<PodStatus>> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);

        let lp = if let Some(app) = app_name {
            ListParams::default().labels(&format!("app.kubernetes.io/name={}", app))
        } else {
            ListParams::default()
        };

        let pod_list = pods.list(&lp).await?;

        let mut statuses = Vec::new();
        for pod in pod_list {
            let metadata = pod.metadata;
            let spec = pod.spec.unwrap_or_default();
            let status = pod.status.unwrap_or_default();

            let name = metadata.name.unwrap_or_default();
            let labels = metadata.labels.unwrap_or_default();

            // Calculate age
            let age = if let Some(creation) = metadata.creation_timestamp {
                let now = jiff::Timestamp::now();
                let seconds = now.duration_since(creation.0).as_secs();
                format_age(seconds)
            } else {
                "unknown".to_string()
            };

            // Get restart count
            let restart_count = status
                .container_statuses
                .as_ref()
                .map(|cs| cs.iter().map(|c| c.restart_count).sum())
                .unwrap_or(0);

            // Check if ready
            let ready = status
                .conditions
                .as_ref()
                .and_then(|conditions| {
                    conditions
                        .iter()
                        .find(|c| c.type_ == "Ready")
                        .map(|c| c.status == "True")
                })
                .unwrap_or(false);

            // Get app label
            let app_label = labels
                .get("app.kubernetes.io/name")
                .or_else(|| labels.get("app"))
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());

            statuses.push(PodStatus {
                name,
                app: app_label,
                namespace: namespace.to_string(),
                status: status.phase.unwrap_or_else(|| "Unknown".to_string()),
                ready,
                restart_count,
                age,
                node: spec.node_name,
                ip: status.pod_ip,
                cpu_usage: None,
                memory_usage: None,
            });
        }

        Ok(statuses)
    }

    /// Get pod metrics for a namespace
    pub async fn get_pod_metrics(
        &self,
        namespace: &str,
        app_name: Option<&str>,
    ) -> Result<Vec<PodMetrics>> {
        // Get pod metrics from metrics-server
        let url = format!("/apis/metrics.k8s.io/v1beta1/namespaces/{}/pods", namespace);
        let request = http::Request::get(&url)
            .body(vec![])
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let metrics_list: PodMetricsList = match self.client.request(request).await {
            Ok(m) => m,
            Err(_) => return Ok(Vec::new()),
        };

        let mut metrics = Vec::new();
        for item in metrics_list.items {
            // Filter by app if specified
            if let Some(app) = app_name {
                let labels = item.metadata.labels.unwrap_or_default();
                let app_label = labels
                    .get("app.kubernetes.io/name")
                    .or_else(|| labels.get("app"));
                if app_label != Some(&app.to_string()) {
                    continue;
                }
            }

            let mut total_cpu = 0i64;
            let mut total_memory = 0i64;

            for container in &item.containers {
                total_cpu += parse_cpu(&container.usage.cpu);
                total_memory += parse_memory(&container.usage.memory);
            }

            metrics.push(PodMetrics {
                name: item.metadata.name.unwrap_or_default(),
                namespace: namespace.to_string(),
                cpu_usage: format_cpu(total_cpu),
                memory_usage: format_memory(total_memory),
            });
        }

        Ok(metrics)
    }

    /// Get service endpoints for an app
    pub async fn get_service_endpoints(
        &self,
        app_name: &str,
        namespace: &str,
    ) -> Result<Vec<ServiceEndpoint>> {
        let services: Api<Service> = Api::namespaced(self.client.clone(), namespace);

        let service = match services.get(app_name).await {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()),
        };

        // Read kubarr.io/base-path annotation from service metadata
        let base_path = service
            .metadata
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get("kubarr.io/base-path"))
            .cloned();

        let spec = service.spec.unwrap_or_default();
        let status = service.status.unwrap_or_default();

        let mut endpoints = Vec::new();
        for port in spec.ports.unwrap_or_default() {
            let port_num = port.port;
            let target_port = port.target_port.map(|tp| match tp {
                k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(i) => i.to_string(),
                k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(s) => s,
            });

            let port_forward_cmd = format!(
                "kubectl port-forward -n {} svc/{} {}:{}",
                namespace, app_name, port_num, port_num
            );

            // Check for external URL
            let external_url = if spec.type_.as_deref() == Some("LoadBalancer") {
                status
                    .load_balancer
                    .as_ref()
                    .and_then(|lb| lb.ingress.as_ref())
                    .and_then(|ingress| ingress.first())
                    .and_then(|ing| ing.ip.as_ref())
                    .map(|ip| format!("http://{}:{}", ip, port_num))
            } else {
                None
            };

            endpoints.push(ServiceEndpoint {
                name: app_name.to_string(),
                namespace: namespace.to_string(),
                port: port_num,
                target_port,
                port_forward_command: port_forward_cmd,
                url: external_url,
                service_type: spec
                    .type_
                    .clone()
                    .unwrap_or_else(|| "ClusterIP".to_string()),
                base_path: base_path.clone(),
            });
        }

        Ok(endpoints)
    }

    /// List pods in a namespace, optionally filtered by app name
    pub async fn list_pods(&self, namespace: &str, app_name: Option<&str>) -> Result<Vec<Pod>> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);

        let lp = if let Some(app) = app_name {
            ListParams::default().labels(&format!("app.kubernetes.io/name={}", app))
        } else {
            ListParams::default()
        };

        let pod_list = pods.list(&lp).await?;
        Ok(pod_list.items)
    }

    /// Get a specific pod by name
    pub async fn get_pod(&self, namespace: &str, pod_name: &str) -> Result<Pod> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let pod = pods.get(pod_name).await?;
        Ok(pod)
    }

    /// Get logs from a specific pod
    pub async fn get_pod_logs(
        &self,
        pod_name: &str,
        namespace: &str,
        container: Option<&str>,
        tail_lines: i32,
    ) -> Result<String> {
        use kube::api::LogParams;

        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);

        let mut log_params = LogParams {
            tail_lines: Some(tail_lines as i64),
            ..Default::default()
        };

        if let Some(c) = container {
            log_params.container = Some(c.to_string());
        }

        let logs = pods.logs(pod_name, &log_params).await?;
        Ok(logs)
    }

    /// Get a secret from a namespace
    pub async fn get_secret(&self, namespace: &str, secret_name: &str) -> Result<Secret> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let secret = secrets.get(secret_name).await?;
        Ok(secret)
    }

    /// Ensure an NFS-backed PV and PVC exist for a media app.
    ///
    /// Creates (idempotently):
    ///   - Namespace `{app_name}` if it doesn't exist
    ///   - PersistentVolume `hdd-media-{app_name}` pointing at the NFS server
    ///   - PersistentVolumeClaim `media-data` in namespace `{app_name}`
    pub async fn ensure_media_pvc(
        &self,
        app_name: &str,
        nfs_server: &str,
        nfs_path: &str,
    ) -> Result<()> {
        let pv_name = format!("hdd-media-{}", app_name);
        let pvc_name = "media-data";
        let capacity = {
            let mut m = BTreeMap::new();
            m.insert("storage".to_string(), Quantity("14Ti".to_string()));
            m
        };

        // 1. Create namespace if it doesn't exist
        let namespaces: Api<Namespace> = Api::all(self.client.clone());
        if namespaces.get(app_name).await.is_err() {
            let ns = Namespace {
                metadata: ObjectMeta {
                    name: Some(app_name.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            };
            namespaces
                .create(&PostParams::default(), &ns)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to create namespace: {}", e)))?;
        }

        // 2. Create PV if it doesn't exist
        let pvs: Api<PersistentVolume> = Api::all(self.client.clone());
        if pvs.get(&pv_name).await.is_err() {
            let pv = PersistentVolume {
                metadata: ObjectMeta {
                    name: Some(pv_name.clone()),
                    ..Default::default()
                },
                spec: Some(PersistentVolumeSpec {
                    capacity: Some(capacity.clone()),
                    access_modes: Some(vec!["ReadWriteMany".to_string()]),
                    persistent_volume_reclaim_policy: Some("Retain".to_string()),
                    nfs: Some(NFSVolumeSource {
                        server: nfs_server.to_string(),
                        path: nfs_path.to_string(),
                        read_only: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            pvs.create(&PostParams::default(), &pv)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to create PV: {}", e)))?;
        }

        // 3. Create PVC if it doesn't exist
        let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), app_name);
        if pvcs.get(pvc_name).await.is_err() {
            let pvc = PersistentVolumeClaim {
                metadata: ObjectMeta {
                    name: Some(pvc_name.to_string()),
                    namespace: Some(app_name.to_string()),
                    ..Default::default()
                },
                spec: Some(PersistentVolumeClaimSpec {
                    access_modes: Some(vec!["ReadWriteMany".to_string()]),
                    storage_class_name: Some("".to_string()),
                    volume_name: Some(pv_name),
                    resources: Some(VolumeResourceRequirements {
                        requests: Some(capacity),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            pvcs.create(&PostParams::default(), &pvc)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to create PVC: {}", e)))?;
        }

        Ok(())
    }

    /// Get database URL from PostgreSQL secret
    pub async fn get_database_url(&self, namespace: &str) -> Result<String> {
        let secret = self.get_secret(namespace, "kubarr-db-app").await?;

        let data = secret
            .data
            .ok_or_else(|| crate::error::AppError::NotFound("Secret data not found".to_string()))?;

        let uri_bytes = data.get("uri").ok_or_else(|| {
            crate::error::AppError::NotFound("uri key not found in secret".to_string())
        })?;

        let uri = String::from_utf8(uri_bytes.0.clone()).map_err(|e| {
            crate::error::AppError::Internal(format!("Failed to decode URI: {}", e))
        })?;

        Ok(uri)
    }
}

// ============================================================================
// Helper Types
// ============================================================================

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct PodStatus {
    pub name: String,
    pub app: String,
    pub namespace: String,
    pub status: String,
    pub ready: bool,
    #[serde(rename = "restarts")]
    pub restart_count: i32,
    pub age: String,
    pub node: Option<String>,
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_usage: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct PodMetrics {
    pub name: String,
    pub namespace: String,
    pub cpu_usage: String,
    pub memory_usage: String,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ServiceEndpoint {
    pub name: String,
    pub namespace: String,
    pub port: i32,
    pub target_port: Option<String>,
    pub port_forward_command: String,
    pub url: Option<String>,
    pub service_type: String,
    pub base_path: Option<String>,
}

// Metrics server response types
#[derive(Debug, Deserialize)]
struct PodMetricsList {
    items: Vec<PodMetricsItem>,
}

#[derive(Debug, Deserialize)]
struct PodMetricsItem {
    metadata: PodMetricsMetadata,
    containers: Vec<ContainerMetrics>,
}

#[derive(Debug, Deserialize)]
struct PodMetricsMetadata {
    name: Option<String>,
    labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct ContainerMetrics {
    usage: ContainerUsage,
}

#[derive(Debug, Deserialize)]
struct ContainerUsage {
    cpu: String,
    memory: String,
}

// ============================================================================
// Helper Functions
// ============================================================================

fn format_age(total_seconds: i64) -> String {
    if total_seconds < 60 {
        format!("{}s", total_seconds)
    } else if total_seconds < 3600 {
        format!("{}m", total_seconds / 60)
    } else if total_seconds < 86400 {
        format!("{}h", total_seconds / 3600)
    } else {
        format!("{}d", total_seconds / 86400)
    }
}

fn parse_cpu(cpu_str: &str) -> i64 {
    if let Some(s) = cpu_str.strip_suffix('n') {
        s.parse().unwrap_or(0)
    } else if let Some(s) = cpu_str.strip_suffix('u') {
        s.parse::<i64>().unwrap_or(0) * 1000
    } else if let Some(s) = cpu_str.strip_suffix('m') {
        s.parse::<i64>().unwrap_or(0) * 1_000_000
    } else {
        cpu_str.parse::<i64>().unwrap_or(0) * 1_000_000_000
    }
}

fn format_cpu(nanocores: i64) -> String {
    let millicores = nanocores / 1_000_000;
    if millicores < 1000 {
        format!("{}m", millicores)
    } else {
        format!("{:.2}", millicores as f64 / 1000.0)
    }
}

fn parse_memory(memory_str: &str) -> i64 {
    if let Some(s) = memory_str.strip_suffix("Ki") {
        s.parse::<i64>().unwrap_or(0) * 1024
    } else if let Some(s) = memory_str.strip_suffix("Mi") {
        s.parse::<i64>().unwrap_or(0) * 1024 * 1024
    } else if let Some(s) = memory_str.strip_suffix("Gi") {
        s.parse::<i64>().unwrap_or(0) * 1024 * 1024 * 1024
    } else if let Some(s) = memory_str.strip_suffix("Ti") {
        s.parse::<i64>().unwrap_or(0) * 1024 * 1024 * 1024 * 1024
    } else {
        memory_str.parse().unwrap_or(0)
    }
}

fn format_memory(bytes: i64) -> String {
    if bytes < 1024 * 1024 {
        format!("{}Ki", bytes / 1024)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{}Mi", bytes / (1024 * 1024))
    } else {
        format!("{:.2}Gi", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // format_age tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_format_age_zero_seconds() {
        assert_eq!(format_age(0), "0s");
    }

    #[test]
    fn test_format_age_thirty_seconds() {
        assert_eq!(format_age(30), "30s");
    }

    #[test]
    fn test_format_age_fifty_nine_seconds() {
        assert_eq!(format_age(59), "59s");
    }

    #[test]
    fn test_format_age_sixty_seconds_is_one_minute() {
        assert_eq!(format_age(60), "1m");
    }

    #[test]
    fn test_format_age_one_hundred_nineteen_seconds_is_one_minute() {
        // 119 / 60 == 1
        assert_eq!(format_age(119), "1m");
    }

    #[test]
    fn test_format_age_3599_seconds_is_59_minutes() {
        // 3599 / 60 == 59
        assert_eq!(format_age(3599), "59m");
    }

    #[test]
    fn test_format_age_3600_seconds_is_one_hour() {
        assert_eq!(format_age(3600), "1h");
    }

    #[test]
    fn test_format_age_86399_seconds_is_23_hours() {
        // 86399 / 3600 == 23
        assert_eq!(format_age(86399), "23h");
    }

    #[test]
    fn test_format_age_86400_seconds_is_one_day() {
        assert_eq!(format_age(86400), "1d");
    }

    #[test]
    fn test_format_age_172800_seconds_is_two_days() {
        assert_eq!(format_age(172800), "2d");
    }

    // -------------------------------------------------------------------------
    // parse_cpu tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_cpu_nanocores() {
        assert_eq!(parse_cpu("500n"), 500);
    }

    #[test]
    fn test_parse_cpu_nanocores_100() {
        assert_eq!(parse_cpu("100n"), 100);
    }

    #[test]
    fn test_parse_cpu_microcores() {
        // 1u == 1 * 1000 nanocores
        assert_eq!(parse_cpu("1u"), 1000);
    }

    #[test]
    fn test_parse_cpu_millicores() {
        // 500m == 500 * 1_000_000 nanocores
        assert_eq!(parse_cpu("500m"), 500_000_000);
    }

    #[test]
    fn test_parse_cpu_millicores_250() {
        assert_eq!(parse_cpu("250m"), 250_000_000);
    }

    #[test]
    fn test_parse_cpu_whole_cores_one() {
        // "1" == 1 * 1_000_000_000 nanocores
        assert_eq!(parse_cpu("1"), 1_000_000_000);
    }

    #[test]
    fn test_parse_cpu_whole_cores_two() {
        assert_eq!(parse_cpu("2"), 2_000_000_000);
    }

    #[test]
    fn test_parse_cpu_invalid_returns_zero() {
        assert_eq!(parse_cpu("invalid"), 0);
    }

    // -------------------------------------------------------------------------
    // format_cpu tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_format_cpu_zero() {
        // 0 nanocores → 0 millicores → "0m"
        assert_eq!(format_cpu(0), "0m");
    }

    #[test]
    fn test_format_cpu_500_millicores() {
        assert_eq!(format_cpu(500_000_000), "500m");
    }

    #[test]
    fn test_format_cpu_999_millicores() {
        assert_eq!(format_cpu(999_000_000), "999m");
    }

    #[test]
    fn test_format_cpu_one_core() {
        // 1_000_000_000 nanocores == 1000 millicores → "1.00" (cores)
        assert_eq!(format_cpu(1_000_000_000), "1.00");
    }

    #[test]
    fn test_format_cpu_two_and_half_cores() {
        // 2_500_000_000 nanocores == 2500 millicores → "2.50" cores
        assert_eq!(format_cpu(2_500_000_000), "2.50");
    }

    // -------------------------------------------------------------------------
    // parse_memory tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_memory_kibibytes() {
        // 512Ki == 512 * 1024 == 524288 bytes
        assert_eq!(parse_memory("512Ki"), 524_288);
    }

    #[test]
    fn test_parse_memory_mebibytes() {
        // 256Mi == 256 * 1024 * 1024 == 268435456 bytes
        assert_eq!(parse_memory("256Mi"), 268_435_456);
    }

    #[test]
    fn test_parse_memory_gibibytes() {
        // 1Gi == 1024^3 == 1073741824 bytes
        assert_eq!(parse_memory("1Gi"), 1_073_741_824);
    }

    #[test]
    fn test_parse_memory_raw_bytes() {
        // no suffix → parsed as plain integer
        assert_eq!(parse_memory("1024"), 1024);
    }

    #[test]
    fn test_parse_memory_tebibytes() {
        // 2Ti == 2 * 1024^4 bytes
        let expected: i64 = 2 * 1024 * 1024 * 1024 * 1024;
        assert_eq!(parse_memory("2Ti"), expected);
    }

    #[test]
    fn test_parse_memory_invalid_returns_zero() {
        assert_eq!(parse_memory("bad"), 0);
    }

    // -------------------------------------------------------------------------
    // format_memory tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_format_memory_zero() {
        // 0 bytes → 0 / 1024 == 0 → "0Ki"
        assert_eq!(format_memory(0), "0Ki");
    }

    #[test]
    fn test_format_memory_one_kibibyte() {
        assert_eq!(format_memory(1024), "1Ki");
    }

    #[test]
    fn test_format_memory_one_mebibyte() {
        assert_eq!(format_memory(1_048_576), "1Mi");
    }

    #[test]
    fn test_format_memory_one_gibibyte() {
        assert_eq!(format_memory(1_073_741_824), "1.00Gi");
    }

    #[test]
    fn test_format_memory_two_gibibytes() {
        assert_eq!(format_memory(2_147_483_648), "2.00Gi");
    }

    #[test]
    fn test_format_memory_below_mebibyte_shows_ki() {
        // 512 * 1024 == 524288 bytes — less than 1 MiB → shown as Ki
        assert_eq!(format_memory(524_288), "512Ki");
    }

    #[test]
    fn test_format_memory_below_gibibyte_shows_mi() {
        // 128 * 1024 * 1024 == 134217728 bytes — less than 1 GiB → shown as Mi
        assert_eq!(format_memory(134_217_728), "128Mi");
    }

    // -------------------------------------------------------------------------
    // PodStatus serialization
    // -------------------------------------------------------------------------

    #[test]
    fn pod_status_ser_minimal() {
        let s = PodStatus {
            name: "sonarr-abc123".to_string(),
            app: "sonarr".to_string(),
            namespace: "media".to_string(),
            status: "Running".to_string(),
            ready: true,
            restart_count: 0,
            age: "2d".to_string(),
            node: None,
            ip: None,
            cpu_usage: None,
            memory_usage: None,
        };
        let json = serde_json::to_string(&s).expect("ser");
        assert!(json.contains("\"name\":\"sonarr-abc123\""));
        assert!(json.contains("\"status\":\"Running\""));
        assert!(json.contains("\"restarts\":0")); // renamed field
        assert!(!json.contains("cpu_usage")); // skip_serializing_if None
        assert!(!json.contains("memory_usage")); // skip_serializing_if None
    }

    #[test]
    fn pod_status_ser_with_metrics() {
        let s = PodStatus {
            name: "radarr-xyz".to_string(),
            app: "radarr".to_string(),
            namespace: "media".to_string(),
            status: "Running".to_string(),
            ready: true,
            restart_count: 2,
            age: "1h".to_string(),
            node: Some("node1".to_string()),
            ip: Some("10.0.0.5".to_string()),
            cpu_usage: Some(0.5),
            memory_usage: Some(256 * 1024 * 1024),
        };
        let json = serde_json::to_string(&s).expect("ser");
        assert!(json.contains("\"cpu_usage\":0.5"));
        assert!(json.contains("\"memory_usage\""));
        assert!(json.contains("\"node\":\"node1\""));
    }

    // -------------------------------------------------------------------------
    // PodMetrics serialization
    // -------------------------------------------------------------------------

    #[test]
    fn pod_metrics_ser() {
        let m = PodMetrics {
            name: "sonarr-123".to_string(),
            namespace: "media".to_string(),
            cpu_usage: "100m".to_string(),
            memory_usage: "256Mi".to_string(),
        };
        let json = serde_json::to_string(&m).expect("ser");
        assert!(json.contains("\"name\":\"sonarr-123\""));
        assert!(json.contains("\"cpu_usage\":\"100m\""));
        assert!(json.contains("\"memory_usage\":\"256Mi\""));
    }

    // -------------------------------------------------------------------------
    // ServiceEndpoint serialization
    // -------------------------------------------------------------------------

    #[test]
    fn service_endpoint_ser() {
        let e = ServiceEndpoint {
            name: "sonarr".to_string(),
            namespace: "media".to_string(),
            port: 8989,
            target_port: Some("8989".to_string()),
            port_forward_command: "kubectl port-forward svc/sonarr 8989:8989".to_string(),
            url: Some("http://localhost:8989".to_string()),
            service_type: "ClusterIP".to_string(),
            base_path: None,
        };
        let json = serde_json::to_string(&e).expect("ser");
        assert!(json.contains("\"name\":\"sonarr\""));
        assert!(json.contains("\"port\":8989"));
        assert!(json.contains("\"service_type\":\"ClusterIP\""));
    }

    // -------------------------------------------------------------------------
    // Private metrics struct deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn container_usage_deser() {
        let json = r#"{"cpu":"500m","memory":"256Mi"}"#;
        let usage: ContainerUsage = serde_json::from_str(json).expect("deser");
        assert_eq!(usage.cpu, "500m");
        assert_eq!(usage.memory, "256Mi");
    }

    #[test]
    fn container_metrics_deser() {
        let json = r#"{"usage":{"cpu":"100m","memory":"128Mi"}}"#;
        let cm: ContainerMetrics = serde_json::from_str(json).expect("deser");
        assert_eq!(cm.usage.cpu, "100m");
        assert_eq!(cm.usage.memory, "128Mi");
    }

    #[test]
    fn pod_metrics_metadata_deser() {
        let json = r#"{"name":"my-pod","labels":{"app":"sonarr"}}"#;
        let meta: PodMetricsMetadata = serde_json::from_str(json).expect("deser");
        assert_eq!(meta.name.as_deref(), Some("my-pod"));
        let labels = meta.labels.unwrap();
        assert_eq!(labels.get("app").map(|s| s.as_str()), Some("sonarr"));
    }
}
