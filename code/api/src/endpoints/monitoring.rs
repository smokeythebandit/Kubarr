use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::Result;
use crate::middleware::permissions::{Authorized, MonitoringView};
use crate::services::catalog::kubarr_system_component;
use crate::services::k8s::{PodMetrics, PodStatus, ServiceEndpoint};
use crate::state::AppState;

/// VictoriaMetrics URL (inside cluster)
const VICTORIAMETRICS_URL: &str = "http://victoriametrics.victoriametrics.svc.cluster.local:8428";

/// Create monitoring routes
pub fn monitoring_routes(state: AppState) -> Router {
    Router::new()
        .route("/vm/apps", get(get_app_metrics))
        .route("/vm/cluster", get(get_cluster_metrics))
        .route("/vm/gpus", get(get_gpu_metrics))
        .route("/vm/app/{app_name}", get(get_app_detail_metrics))
        .route(
            "/vm/cluster/network-history",
            get(get_cluster_network_history),
        )
        .route(
            "/vm/cluster/metrics-history",
            get(get_cluster_metrics_history),
        )
        .route("/vm/available", get(check_vm_available))
        .route("/pods", get(get_pods))
        .route("/metrics", get(get_metrics))
        .route("/health/{app_name}", get(get_app_health))
        .route("/endpoints/{app_name}", get(get_endpoints))
        .route("/metrics-available", get(check_metrics_available))
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppMetrics {
    pub app_name: String,
    pub namespace: String,
    pub cpu_usage_cores: f64,
    pub memory_usage_bytes: i64,
    pub memory_usage_mb: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_usage_percent: Option<f64>,
    pub network_receive_bytes_per_sec: f64,
    pub network_transmit_bytes_per_sec: f64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ClusterMetrics {
    pub total_cpu_cores: f64,
    pub total_memory_bytes: i64,
    pub used_cpu_cores: f64,
    pub used_memory_bytes: i64,
    pub cpu_usage_percent: f64,
    pub memory_usage_percent: f64,
    pub container_count: i32,
    pub pod_count: i32,
    pub network_receive_bytes_per_sec: f64,
    pub network_transmit_bytes_per_sec: f64,
    pub total_storage_bytes: i64,
    pub used_storage_bytes: i64,
    pub storage_usage_percent: f64,
}

/// Fresh, node-level exporter telemetry. Null means no recent valid sample.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GpuDeviceMetrics {
    pub node: String,
    pub vendor: String,
    pub device: String,
    pub utilization_percent: Option<f64>,
    pub memory_used_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GpuMetricsResponse {
    /// False when VictoriaMetrics could not answer all GPU queries.
    pub available: bool,
    pub devices: Vec<GpuDeviceMetrics>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TimeSeriesPoint {
    pub timestamp: f64,
    pub value: f64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppHistoricalMetrics {
    pub app_name: String,
    pub namespace: String,
    pub cpu_series: Vec<TimeSeriesPoint>,
    pub memory_series: Vec<TimeSeriesPoint>,
    pub network_rx_series: Vec<TimeSeriesPoint>,
    pub network_tx_series: Vec<TimeSeriesPoint>,
    pub cpu_usage_cores: f64,
    pub memory_usage_bytes: i64,
    pub memory_usage_mb: f64,
    pub network_receive_bytes_per_sec: f64,
    pub network_transmit_bytes_per_sec: f64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppDetailMetrics {
    pub app_name: String,
    pub namespace: String,
    pub historical: AppHistoricalMetrics,
    pub pods: Vec<PodStatus>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AppHealth {
    pub app_name: String,
    pub namespace: String,
    pub healthy: bool,
    pub pods: Vec<PodStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Vec<PodMetrics>>,
    pub endpoints: Vec<ServiceEndpoint>,
    pub message: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct PodQuery {
    pub namespace: Option<String>,
    pub app: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct AppDetailQuery {
    pub duration: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct NetworkHistoryQuery {
    pub duration: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ClusterNetworkHistory {
    pub combined_series: Vec<TimeSeriesPoint>,
    pub rx_series: Vec<TimeSeriesPoint>,
    pub tx_series: Vec<TimeSeriesPoint>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ClusterMetricsHistory {
    pub cpu_series: Vec<TimeSeriesPoint>,
    pub memory_series: Vec<TimeSeriesPoint>,
    pub storage_series: Vec<TimeSeriesPoint>,
    pub pod_series: Vec<TimeSeriesPoint>,
    pub container_series: Vec<TimeSeriesPoint>,
}

// ============================================================================
// VictoriaMetrics Query Helpers
// ============================================================================

async fn query_vm(query: &str) -> Vec<serde_json::Value> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/query", VICTORIAMETRICS_URL);

    match client
        .get(&url)
        .query(&[("query", query)])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if data.get("status") == Some(&serde_json::json!("success")) {
                    return data["data"]["result"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                }
            }
            Vec::new()
        }
        Err(_) => Vec::new(),
    }
}

async fn query_vm_range(query: &str, start: f64, end: f64, step: &str) -> Vec<serde_json::Value> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/query_range", VICTORIAMETRICS_URL);

    match client
        .get(&url)
        .query(&[
            ("query", query),
            ("start", &start.to_string()),
            ("end", &end.to_string()),
            ("step", step),
        ])
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if data.get("status") == Some(&serde_json::json!("success")) {
                    return data["data"]["result"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                }
            }
            Vec::new()
        }
        Err(_) => Vec::new(),
    }
}

// ============================================================================
// Endpoint Handlers
// ============================================================================

// Do not accept an arbitrary PromQL expression or URL from the request. The
// instant-query response timestamp is the evaluation time, NOT the scrape time.
const GPU_MAX_AGE_SECONDS: f64 = 120.0;
const GPU_MAX_DEVICES: usize = 256;
const GPU_QUERIES: [(&str, &str); 3] = [
    (
        "default_rollup(DCGM_FI_DEV_GPU_UTIL[120s])",
        "tlast_over_time(DCGM_FI_DEV_GPU_UTIL[120s])",
    ),
    (
        "default_rollup(DCGM_FI_DEV_FB_USED[120s])",
        "tlast_over_time(DCGM_FI_DEV_FB_USED[120s])",
    ),
    (
        "default_rollup(DCGM_FI_DEV_FB_FREE[120s])",
        "tlast_over_time(DCGM_FI_DEV_FB_FREE[120s])",
    ),
];

async fn query_gpu_series(
    query: &'static str,
    now: f64,
) -> std::result::Result<Vec<serde_json::Value>, ()> {
    let client = reqwest::Client::new();
    let mut response = client
        .get(format!("{VICTORIAMETRICS_URL}/api/v1/query"))
        .query(&[("query", query), ("time", &now.to_string())])
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    // Bound the response as well as the number of devices accepted below.
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if body.len().saturating_add(chunk.len()) > 1_048_576 {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    parse_gpu_response(&body)
}

fn parse_gpu_response(body: &[u8]) -> std::result::Result<Vec<serde_json::Value>, ()> {
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|_| ())?;
    if json["status"] != "success" || json["data"]["resultType"] != "vector" {
        return Err(());
    }
    let results = json["data"]["result"].as_array().ok_or(())?;
    if results.len() > GPU_MAX_DEVICES {
        return Err(());
    }
    Ok(results.clone())
}

fn gpu_sample(result: &serde_json::Value, now: f64) -> Option<(String, String, String, f64)> {
    let labels = result.get("metric")?;
    // Hostname and UUID are exporter labels, not app/pod allocation labels.
    let node = labels.get("Hostname")?.as_str()?.trim();
    let uuid = labels.get("UUID")?.as_str()?.trim();
    if node.is_empty() || uuid.is_empty() {
        return None;
    }
    let instance = labels
        .get("GPU_I_ID")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let profile = labels
        .get("GPU_I_PROFILE")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let values = result.get("value")?.as_array()?;
    let ts = values.first()?.as_f64()?;
    let value = values.get(1)?.as_str()?.parse::<f64>().ok()?;
    if !ts.is_finite() || !value.is_finite() || ts > now + 30.0 || now - ts > GPU_MAX_AGE_SECONDS {
        return None;
    }
    let device = if instance.is_empty() {
        uuid.to_string()
    } else {
        format!("{uuid} (MIG {profile} instance {instance})")
    };
    Some((node.to_string(), uuid.to_string(), device, value))
}

fn gpu_series_identity(result: &serde_json::Value) -> Option<String> {
    let mut labels = result.get("metric")?.as_object()?.clone();
    // Rollup functions can differ in whether they retain the metric name.
    labels.remove("__name__");
    serde_json::to_string(&labels).ok()
}

fn with_scrape_timestamps(
    values: &[serde_json::Value],
    timestamps: &[serde_json::Value],
    now: f64,
) -> Vec<serde_json::Value> {
    let source_times: BTreeMap<_, _> = timestamps
        .iter()
        .filter_map(|series| {
            let key = gpu_series_identity(series)?;
            let source = series["value"][1].as_str()?.parse::<f64>().ok()?;
            if !source.is_finite() || source > now + 30.0 || now - source > GPU_MAX_AGE_SECONDS {
                return None;
            }
            Some((key, source))
        })
        .collect();
    values
        .iter()
        .filter_map(|series| {
            let source = source_times.get(&gpu_series_identity(series)?)?;
            let mut series = series.clone();
            // gpu_sample must validate the *source* time, not the eval time.
            series["value"][0] = serde_json::json!(source);
            Some(series)
        })
        .collect()
}

fn assemble_gpu_metrics(
    utilization: &[serde_json::Value],
    used: &[serde_json::Value],
    free: &[serde_json::Value],
    now: f64,
) -> Vec<GpuDeviceMetrics> {
    // Key on the full device identity (including MIG instance); never merge
    // samples from different nodes or instances with the same GPU index.
    let mut devices: BTreeMap<(String, String), (GpuDeviceMetrics, Option<u64>)> = BTreeMap::new();
    for (series, kind) in [(utilization, 0), (used, 1), (free, 2)] {
        for result in series {
            let Some((node, _uuid, device, value)) = gpu_sample(result, now) else {
                continue;
            };
            let entry = devices
                .entry((node.clone(), device.clone()))
                .or_insert_with(|| {
                    (
                        GpuDeviceMetrics {
                            node,
                            vendor: "NVIDIA".to_string(),
                            device,
                            utilization_percent: None,
                            memory_used_bytes: None,
                            memory_total_bytes: None,
                        },
                        None,
                    )
                });
            match kind {
                0 if (0.0..=100.0).contains(&value) => entry.0.utilization_percent = Some(value),
                1 | 2 if value >= 0.0 && value <= (u64::MAX / 1_048_576) as f64 => {
                    let bytes = (value * 1_048_576.0) as u64;
                    if kind == 1 {
                        entry.0.memory_used_bytes = Some(bytes);
                    } else {
                        entry.1 = Some(bytes);
                    }
                }
                _ => {}
            }
        }
    }
    devices
        .into_values()
        .map(|(mut device, free)| {
            device.memory_total_bytes = device
                .memory_used_bytes
                .zip(free)
                .and_then(|(used, free)| used.checked_add(free));
            device
        })
        .collect()
}

/// Get fresh NVIDIA DCGM node/device telemetry from VictoriaMetrics.
#[utoipa::path(
    get,
    path = "/api/monitoring/vm/gpus",
    tag = "Monitoring",
    responses((status = 200, body = GpuMetricsResponse))
)]
async fn get_gpu_metrics(_auth: Authorized<MonitoringView>) -> Result<Json<GpuMetricsResponse>> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let (utilization, utilization_time, used, used_time, free, free_time) = tokio::join!(
        query_gpu_series(GPU_QUERIES[0].0, now),
        query_gpu_series(GPU_QUERIES[0].1, now),
        query_gpu_series(GPU_QUERIES[1].0, now),
        query_gpu_series(GPU_QUERIES[1].1, now),
        query_gpu_series(GPU_QUERIES[2].0, now),
        query_gpu_series(GPU_QUERIES[2].1, now),
    );
    let available = utilization.is_ok()
        && utilization_time.is_ok()
        && used.is_ok()
        && used_time.is_ok()
        && free.is_ok()
        && free_time.is_ok();
    let utilization = with_scrape_timestamps(
        &utilization.unwrap_or_default(),
        &utilization_time.unwrap_or_default(),
        now,
    );
    let used = with_scrape_timestamps(
        &used.unwrap_or_default(),
        &used_time.unwrap_or_default(),
        now,
    );
    let free = with_scrape_timestamps(
        &free.unwrap_or_default(),
        &free_time.unwrap_or_default(),
        now,
    );
    let devices = assemble_gpu_metrics(&utilization, &used, &free, now);
    Ok(Json(GpuMetricsResponse { available, devices }))
}

/// Get resource metrics for all installed apps from VictoriaMetrics
#[utoipa::path(
    get,
    path = "/api/monitoring/vm/apps",
    tag = "Monitoring",
    responses(
        (status = 200, body = Vec<AppMetrics>)
    )
)]
async fn get_app_metrics(
    State(state): State<AppState>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<Vec<AppMetrics>>> {
    // Get list of known app namespaces from catalog
    let catalog = state.catalog.read().await;
    let mut allowed_namespaces: std::collections::HashSet<String> = catalog
        .get_all_apps()
        .iter()
        .map(|app| app.name.clone())
        .collect();

    // Add monitoring/system namespaces
    allowed_namespaces.insert("kubarr-system".to_string());
    allowed_namespaces.insert("victoriametrics".to_string());
    allowed_namespaces.insert("victorialogs".to_string());
    allowed_namespaces.insert("fluent-bit".to_string());
    allowed_namespaces.insert("grafana".to_string());

    // Query CPU usage by namespace
    let cpu_query = r#"sum by (namespace) (rate(container_cpu_usage_seconds_total{container!="",container!="POD"}[5m]))"#;
    let cpu_results = query_vm(cpu_query).await;

    // Query memory usage by namespace
    let memory_query = r#"sum by (namespace) (container_memory_working_set_bytes{container!="",container!="POD"})"#;
    let memory_results = query_vm(memory_query).await;

    // Query network receive rate by namespace
    let network_rx_query =
        r#"sum by (namespace) (rate(container_network_receive_bytes_total{interface!="lo"}[5m]))"#;
    let network_rx_results = query_vm(network_rx_query).await;

    // Query network transmit rate by namespace
    let network_tx_query =
        r#"sum by (namespace) (rate(container_network_transmit_bytes_total{interface!="lo"}[5m]))"#;
    let network_tx_results = query_vm(network_tx_query).await;

    let mut metrics_map = std::collections::HashMap::new();

    // Process CPU results
    for result in &cpu_results {
        if let Some(namespace) = result["metric"]["namespace"].as_str() {
            // Only include namespaces in our allowed list
            if !allowed_namespaces.contains(namespace) {
                continue;
            }
            if let Some(value) = result["value"][1].as_str() {
                let cpu_val: f64 = value.parse().unwrap_or(0.0);
                metrics_map.insert(
                    namespace.to_string(),
                    AppMetrics {
                        app_name: namespace.to_string(),
                        namespace: namespace.to_string(),
                        cpu_usage_cores: (cpu_val * 10000.0).round() / 10000.0,
                        memory_usage_bytes: 0,
                        memory_usage_mb: 0.0,
                        cpu_usage_percent: None,
                        memory_usage_percent: None,
                        network_receive_bytes_per_sec: 0.0,
                        network_transmit_bytes_per_sec: 0.0,
                    },
                );
            }
        }
    }

    // Process memory results
    for result in &memory_results {
        if let Some(namespace) = result["metric"]["namespace"].as_str() {
            // Only include namespaces in our allowed list
            if !allowed_namespaces.contains(namespace) {
                continue;
            }
            if let Some(value) = result["value"][1].as_str() {
                let mem_val: i64 = value.parse::<f64>().unwrap_or(0.0) as i64;
                if let Some(metrics) = metrics_map.get_mut(namespace) {
                    metrics.memory_usage_bytes = mem_val;
                    metrics.memory_usage_mb =
                        (mem_val as f64 / (1024.0 * 1024.0) * 100.0).round() / 100.0;
                } else {
                    metrics_map.insert(
                        namespace.to_string(),
                        AppMetrics {
                            app_name: namespace.to_string(),
                            namespace: namespace.to_string(),
                            cpu_usage_cores: 0.0,
                            memory_usage_bytes: mem_val,
                            memory_usage_mb: (mem_val as f64 / (1024.0 * 1024.0) * 100.0).round()
                                / 100.0,
                            cpu_usage_percent: None,
                            memory_usage_percent: None,
                            network_receive_bytes_per_sec: 0.0,
                            network_transmit_bytes_per_sec: 0.0,
                        },
                    );
                }
            }
        }
    }

    // Process network receive results
    for result in &network_rx_results {
        if let Some(namespace) = result["metric"]["namespace"].as_str() {
            if !allowed_namespaces.contains(namespace) {
                continue;
            }
            if let Some(value) = result["value"][1].as_str() {
                let rx_val: f64 = value.parse().unwrap_or(0.0);
                if let Some(metrics) = metrics_map.get_mut(namespace) {
                    metrics.network_receive_bytes_per_sec = (rx_val * 100.0).round() / 100.0;
                }
            }
        }
    }

    // Process network transmit results
    for result in &network_tx_results {
        if let Some(namespace) = result["metric"]["namespace"].as_str() {
            if !allowed_namespaces.contains(namespace) {
                continue;
            }
            if let Some(value) = result["value"][1].as_str() {
                let tx_val: f64 = value.parse().unwrap_or(0.0);
                if let Some(metrics) = metrics_map.get_mut(namespace) {
                    metrics.network_transmit_bytes_per_sec = (tx_val * 100.0).round() / 100.0;
                }
            }
        }
    }

    Ok(Json(metrics_map.into_values().collect()))
}

/// Get overall cluster resource metrics from VictoriaMetrics
#[utoipa::path(
    get,
    path = "/api/monitoring/vm/cluster",
    tag = "Monitoring",
    responses(
        (status = 200, body = ClusterMetrics)
    )
)]
async fn get_cluster_metrics(_auth: Authorized<MonitoringView>) -> Result<Json<ClusterMetrics>> {
    // Total CPU cores
    let total_cpu = query_vm("sum(machine_cpu_cores)")
        .await
        .first()
        .and_then(|r| r["value"][1].as_str())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);

    // Total memory
    let total_memory = query_vm("sum(machine_memory_bytes)")
        .await
        .first()
        .and_then(|r| r["value"][1].as_str())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0) as i64;

    // Used CPU
    let used_cpu = query_vm(
        r#"sum(rate(container_cpu_usage_seconds_total{container!="",container!="POD"}[5m]))"#,
    )
    .await
    .first()
    .and_then(|r| r["value"][1].as_str())
    .and_then(|v| v.parse::<f64>().ok())
    .unwrap_or(0.0);

    // Used memory
    let used_memory =
        query_vm(r#"sum(container_memory_working_set_bytes{container!="",container!="POD"})"#)
            .await
            .first()
            .and_then(|r| r["value"][1].as_str())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0) as i64;

    // Container count
    let container_count = query_vm(r#"count(container_last_seen{container!="",container!="POD"})"#)
        .await
        .first()
        .and_then(|r| r["value"][1].as_str())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0) as i32;

    // Pod count
    let pod_count = query_vm(
        r#"count(count by (pod, namespace) (container_last_seen{container!="",container!="POD"}))"#,
    )
    .await
    .first()
    .and_then(|r| r["value"][1].as_str())
    .and_then(|v| v.parse::<f64>().ok())
    .unwrap_or(0.0) as i32;

    // Network receive rate
    let network_rx =
        query_vm(r#"sum(rate(container_network_receive_bytes_total{interface!="lo"}[5m]))"#)
            .await
            .first()
            .and_then(|r| r["value"][1].as_str())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);

    // Network transmit rate
    let network_tx =
        query_vm(r#"sum(rate(container_network_transmit_bytes_total{interface!="lo"}[5m]))"#)
            .await
            .first()
            .and_then(|r| r["value"][1].as_str())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);

    // Storage metrics
    let total_storage = query_vm(r#"max(container_fs_limit_bytes{id="/",device=~"/dev/.*"})"#)
        .await
        .first()
        .and_then(|r| r["value"][1].as_str())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0) as i64;

    let used_storage = query_vm(r#"max(container_fs_usage_bytes{id="/",device=~"/dev/.*"})"#)
        .await
        .first()
        .and_then(|r| r["value"][1].as_str())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0) as i64;

    Ok(Json(ClusterMetrics {
        total_cpu_cores: (total_cpu * 100.0).round() / 100.0,
        total_memory_bytes: total_memory,
        used_cpu_cores: (used_cpu * 10000.0).round() / 10000.0,
        used_memory_bytes: used_memory,
        cpu_usage_percent: if total_cpu > 0.0 {
            (used_cpu / total_cpu * 10000.0).round() / 100.0
        } else {
            0.0
        },
        memory_usage_percent: if total_memory > 0 {
            (used_memory as f64 / total_memory as f64 * 10000.0).round() / 100.0
        } else {
            0.0
        },
        container_count,
        pod_count,
        network_receive_bytes_per_sec: (network_rx * 100.0).round() / 100.0,
        network_transmit_bytes_per_sec: (network_tx * 100.0).round() / 100.0,
        total_storage_bytes: total_storage,
        used_storage_bytes: used_storage,
        storage_usage_percent: if total_storage > 0 {
            (used_storage as f64 / total_storage as f64 * 10000.0).round() / 100.0
        } else {
            0.0
        },
    }))
}

/// Get cluster-wide network history for sparkline charts
#[utoipa::path(
    get,
    path = "/api/monitoring/vm/cluster/network-history",
    tag = "Monitoring",
    params(NetworkHistoryQuery),
    responses(
        (status = 200, body = ClusterNetworkHistory)
    )
)]
async fn get_cluster_network_history(
    Query(query): Query<NetworkHistoryQuery>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<ClusterNetworkHistory>> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = query.duration.unwrap_or_else(|| "15m".to_string());

    let duration_seconds: i64 = match duration.as_str() {
        "15m" => 15 * 60,
        "1h" => 60 * 60,
        "3h" => 3 * 60 * 60,
        _ => 15 * 60,
    };

    let end_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let start_time = end_time - duration_seconds as f64;

    let step = if duration_seconds <= 900 {
        "15s"
    } else if duration_seconds <= 3600 {
        "60s"
    } else {
        "120s"
    };

    let rx_query = r#"sum(rate(container_network_receive_bytes_total{interface!="lo"}[5m]))"#;
    let tx_query = r#"sum(rate(container_network_transmit_bytes_total{interface!="lo"}[5m]))"#;

    let rx_results = query_vm_range(rx_query, start_time, end_time, step).await;
    let tx_results = query_vm_range(tx_query, start_time, end_time, step).await;

    let parse_series = |results: Vec<serde_json::Value>| -> Vec<TimeSeriesPoint> {
        results
            .first()
            .and_then(|r| r["values"].as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| {
                        let ts = v[0].as_f64()?;
                        let val: f64 = v[1].as_str()?.parse().ok()?;
                        Some(TimeSeriesPoint {
                            timestamp: ts,
                            value: val,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let rx_series = parse_series(rx_results);
    let tx_series = parse_series(tx_results);

    // Combine RX+TX by matching timestamps
    let combined_series: Vec<TimeSeriesPoint> = rx_series
        .iter()
        .zip(tx_series.iter())
        .map(|(rx, tx)| TimeSeriesPoint {
            timestamp: rx.timestamp,
            value: ((rx.value + tx.value) * 100.0).round() / 100.0,
        })
        .collect();

    Ok(Json(ClusterNetworkHistory {
        combined_series,
        rx_series,
        tx_series,
    }))
}

/// Get cluster-wide metrics history for sparkline charts (CPU, Memory, Storage, Pods, Containers)
#[utoipa::path(
    get,
    path = "/api/monitoring/vm/cluster/metrics-history",
    tag = "Monitoring",
    params(NetworkHistoryQuery),
    responses(
        (status = 200, body = ClusterMetricsHistory)
    )
)]
async fn get_cluster_metrics_history(
    Query(query): Query<NetworkHistoryQuery>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<ClusterMetricsHistory>> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = query.duration.unwrap_or_else(|| "15m".to_string());

    let duration_seconds: i64 = match duration.as_str() {
        "15m" => 15 * 60,
        "1h" => 60 * 60,
        "3h" => 3 * 60 * 60,
        _ => 15 * 60,
    };

    let end_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let start_time = end_time - duration_seconds as f64;

    let step = if duration_seconds <= 900 {
        "15s"
    } else if duration_seconds <= 3600 {
        "60s"
    } else {
        "120s"
    };

    let cpu_query = r#"sum(rate(container_cpu_usage_seconds_total{container!="",container!="POD"}[5m])) / sum(machine_cpu_cores) * 100"#;
    let memory_query = r#"sum(container_memory_working_set_bytes{container!="",container!="POD"}) / sum(machine_memory_bytes) * 100"#;
    let storage_query = r#"max(container_fs_usage_bytes{id="/",device=~"/dev/.*"}) / max(container_fs_limit_bytes{id="/",device=~"/dev/.*"}) * 100"#;
    let pod_query =
        r#"count(count by (pod, namespace) (container_last_seen{container!="",container!="POD"}))"#;
    let container_query = r#"count(container_last_seen{container!="",container!="POD"})"#;

    let (cpu_results, memory_results, storage_results, pod_results, container_results) = tokio::join!(
        query_vm_range(cpu_query, start_time, end_time, step),
        query_vm_range(memory_query, start_time, end_time, step),
        query_vm_range(storage_query, start_time, end_time, step),
        query_vm_range(pod_query, start_time, end_time, step),
        query_vm_range(container_query, start_time, end_time, step),
    );

    let parse_series = |results: Vec<serde_json::Value>| -> Vec<TimeSeriesPoint> {
        results
            .first()
            .and_then(|r| r["values"].as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| {
                        let ts = v[0].as_f64()?;
                        let val: f64 = v[1].as_str()?.parse().ok()?;
                        Some(TimeSeriesPoint {
                            timestamp: ts,
                            value: (val * 100.0).round() / 100.0,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    Ok(Json(ClusterMetricsHistory {
        cpu_series: parse_series(cpu_results),
        memory_series: parse_series(memory_results),
        storage_series: parse_series(storage_results),
        pod_series: parse_series(pod_results),
        container_series: parse_series(container_results),
    }))
}

/// Get detailed metrics for a specific app
#[utoipa::path(
    get,
    path = "/api/monitoring/vm/app/{app_name}",
    tag = "Monitoring",
    params(
        ("app_name" = String, Path, description = "Application name"),
        AppDetailQuery,
    ),
    responses(
        (status = 200, body = AppDetailMetrics)
    )
)]
async fn get_app_detail_metrics(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    Query(query): Query<AppDetailQuery>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<AppDetailMetrics>> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = query.duration.unwrap_or_else(|| "1h".to_string());

    // Parse duration to seconds
    let duration_seconds: i64 = match duration.as_str() {
        "15m" => 15 * 60,
        "1h" => 60 * 60,
        "3h" => 3 * 60 * 60,
        "6h" => 6 * 60 * 60,
        "12h" => 12 * 60 * 60,
        "24h" => 24 * 60 * 60,
        _ => 3600,
    };

    let end_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let start_time = end_time - duration_seconds as f64;

    let step = if duration_seconds <= 3600 {
        "60s"
    } else if duration_seconds <= 21600 {
        "120s"
    } else {
        "300s"
    };

    // Build VictoriaMetrics queries by namespace
    let cpu_query = format!(
        r#"sum(rate(container_cpu_usage_seconds_total{{namespace="{}",container!="",container!="POD"}}[5m]))"#,
        app_name
    );
    let memory_query = format!(
        r#"sum(container_memory_working_set_bytes{{namespace="{}",container!="",container!="POD"}})"#,
        app_name
    );
    let network_rx_query = format!(
        r#"sum(rate(container_network_receive_bytes_total{{namespace="{}",interface!="lo"}}[5m]))"#,
        app_name
    );
    let network_tx_query = format!(
        r#"sum(rate(container_network_transmit_bytes_total{{namespace="{}",interface!="lo"}}[5m]))"#,
        app_name
    );

    // Query historical CPU
    let cpu_results = query_vm_range(&cpu_query, start_time, end_time, step).await;

    let cpu_series: Vec<TimeSeriesPoint> = cpu_results
        .first()
        .and_then(|r| r["values"].as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| {
                    let ts = v[0].as_f64()?;
                    let val: f64 = v[1].as_str()?.parse().ok()?;
                    Some(TimeSeriesPoint {
                        timestamp: ts,
                        value: val,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Query historical memory
    let memory_results = query_vm_range(&memory_query, start_time, end_time, step).await;

    let memory_series: Vec<TimeSeriesPoint> = memory_results
        .first()
        .and_then(|r| r["values"].as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| {
                    let ts = v[0].as_f64()?;
                    let val: f64 = v[1].as_str()?.parse().ok()?;
                    Some(TimeSeriesPoint {
                        timestamp: ts,
                        value: val,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Query historical network receive
    let network_rx_results = query_vm_range(&network_rx_query, start_time, end_time, step).await;

    let network_rx_series: Vec<TimeSeriesPoint> = network_rx_results
        .first()
        .and_then(|r| r["values"].as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| {
                    let ts = v[0].as_f64()?;
                    let val: f64 = v[1].as_str()?.parse().ok()?;
                    Some(TimeSeriesPoint {
                        timestamp: ts,
                        value: val,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Query historical network transmit
    let network_tx_results = query_vm_range(&network_tx_query, start_time, end_time, step).await;

    let network_tx_series: Vec<TimeSeriesPoint> = network_tx_results
        .first()
        .and_then(|r| r["values"].as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| {
                    let ts = v[0].as_f64()?;
                    let val: f64 = v[1].as_str()?.parse().ok()?;
                    Some(TimeSeriesPoint {
                        timestamp: ts,
                        value: val,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let current_cpu = cpu_series.last().map(|p| p.value).unwrap_or(0.0);
    let current_memory = memory_series.last().map(|p| p.value as i64).unwrap_or(0);
    let current_network_rx = network_rx_series.last().map(|p| p.value).unwrap_or(0.0);
    let current_network_tx = network_tx_series.last().map(|p| p.value).unwrap_or(0.0);

    // Get pod status
    let mut pods = if let Some(client) = state.k8s_client.read().await.as_ref() {
        client
            .get_pod_status(&app_name, None)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Query per-pod CPU and memory metrics from VictoriaMetrics
    let pod_cpu_query = format!(
        r#"sum(rate(container_cpu_usage_seconds_total{{namespace="{}",container!="",container!="POD"}}[5m])) by (pod)"#,
        app_name
    );
    let pod_memory_query = format!(
        r#"sum(container_memory_working_set_bytes{{namespace="{}",container!="",container!="POD"}}) by (pod)"#,
        app_name
    );

    let pod_cpu_results = query_vm(&pod_cpu_query).await;
    let pod_memory_results = query_vm(&pod_memory_query).await;

    // Build maps of pod name -> metric value
    let mut pod_cpu_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut pod_memory_map: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();

    for result in &pod_cpu_results {
        if let (Some(pod_name), Some(value_str)) = (
            result["metric"]["pod"].as_str(),
            result["value"]
                .as_array()
                .and_then(|v| v.get(1))
                .and_then(|v| v.as_str()),
        ) {
            if let Ok(value) = value_str.parse::<f64>() {
                pod_cpu_map.insert(pod_name.to_string(), value);
            }
        }
    }

    for result in &pod_memory_results {
        if let (Some(pod_name), Some(value_str)) = (
            result["metric"]["pod"].as_str(),
            result["value"]
                .as_array()
                .and_then(|v| v.get(1))
                .and_then(|v| v.as_str()),
        ) {
            if let Ok(value) = value_str.parse::<f64>() {
                pod_memory_map.insert(pod_name.to_string(), value as i64);
            }
        }
    }

    // Merge metrics into pod status
    for pod in &mut pods {
        pod.cpu_usage = pod_cpu_map.get(&pod.name).copied();
        pod.memory_usage = pod_memory_map.get(&pod.name).copied();
    }

    Ok(Json(AppDetailMetrics {
        app_name: app_name.clone(),
        namespace: app_name.clone(),
        historical: AppHistoricalMetrics {
            app_name: app_name.clone(),
            namespace: app_name.clone(),
            cpu_series,
            memory_series,
            network_rx_series,
            network_tx_series,
            cpu_usage_cores: (current_cpu * 10000.0).round() / 10000.0,
            memory_usage_bytes: current_memory,
            memory_usage_mb: (current_memory as f64 / (1024.0 * 1024.0) * 100.0).round() / 100.0,
            network_receive_bytes_per_sec: (current_network_rx * 100.0).round() / 100.0,
            network_transmit_bytes_per_sec: (current_network_tx * 100.0).round() / 100.0,
        },
        pods,
    }))
}

/// Check if VictoriaMetrics is available
#[utoipa::path(
    get,
    path = "/api/monitoring/vm/available",
    tag = "Monitoring",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn check_vm_available(_auth: Authorized<MonitoringView>) -> Result<Json<serde_json::Value>> {
    let client = reqwest::Client::new();
    // VictoriaMetrics uses /health endpoint for health checks
    let url = format!("{}/health", VICTORIAMETRICS_URL);

    let available = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    Ok(Json(serde_json::json!({
        "available": available,
        "message": if available { "VictoriaMetrics is available" } else { "Cannot connect to VictoriaMetrics" }
    })))
}

/// Get pod status
#[utoipa::path(
    get,
    path = "/api/monitoring/pods",
    tag = "Monitoring",
    params(PodQuery),
    responses(
        (status = 200, body = Vec<PodStatus>)
    )
)]
async fn get_pods(
    State(state): State<AppState>,
    Query(query): Query<PodQuery>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<Vec<PodStatus>>> {
    let namespace = query.namespace.unwrap_or_else(|| "media".to_string());

    let pods = if let Some(client) = state.k8s_client.read().await.as_ref() {
        client
            .get_pod_status(&namespace, query.app.as_deref())
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(Json(pods))
}

/// Get pod metrics
#[utoipa::path(
    get,
    path = "/api/monitoring/metrics",
    tag = "Monitoring",
    params(PodQuery),
    responses(
        (status = 200, body = Vec<PodMetrics>)
    )
)]
async fn get_metrics(
    State(state): State<AppState>,
    Query(query): Query<PodQuery>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<Vec<PodMetrics>>> {
    let namespace = query.namespace.unwrap_or_else(|| "media".to_string());

    let metrics = if let Some(client) = state.k8s_client.read().await.as_ref() {
        client
            .get_pod_metrics(&namespace, query.app.as_deref())
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(Json(metrics))
}

/// Get app health
#[utoipa::path(
    get,
    path = "/api/monitoring/health/{app_name}",
    tag = "Monitoring",
    params(
        ("app_name" = String, Path, description = "Application name"),
        PodQuery,
    ),
    responses(
        (status = 200, body = AppHealth)
    )
)]
async fn get_app_health(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    Query(query): Query<PodQuery>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<AppHealth>> {
    let component = kubarr_system_component(&app_name);
    let namespace = component
        .map(|component| component.namespace.to_string())
        .or(query.namespace)
        .unwrap_or_else(|| "media".to_string());
    let service_name = component
        .and_then(|component| component.service_name)
        .unwrap_or(&app_name);

    let (pods, metrics, endpoints) = if let Some(client) = state.k8s_client.read().await.as_ref() {
        let pods = client
            .get_pod_status(&namespace, Some(&app_name))
            .await
            .unwrap_or_default();
        let metrics = client
            .get_pod_metrics(&namespace, Some(&app_name))
            .await
            .ok();
        let endpoints = client
            .get_service_endpoints(service_name, &namespace)
            .await
            .unwrap_or_default();
        (pods, metrics, endpoints)
    } else {
        (Vec::new(), None, Vec::new())
    };

    // Determine health
    let (healthy, message) = if pods.is_empty() {
        (false, "No pods found".to_string())
    } else {
        let running_ready: Vec<_> = pods
            .iter()
            .filter(|p| p.status == "Running" && p.ready)
            .collect();

        if running_ready.len() != pods.len() {
            (
                false,
                format!("{}/{} pods ready", running_ready.len(), pods.len()),
            )
        } else {
            let high_restarts: Vec<_> = pods.iter().filter(|p| p.restart_count > 5).collect();
            if !high_restarts.is_empty() {
                (false, "Pods restarting frequently".to_string())
            } else {
                (true, "All pods running".to_string())
            }
        }
    };

    Ok(Json(AppHealth {
        app_name: app_name.clone(),
        namespace,
        healthy,
        pods,
        metrics,
        endpoints,
        message,
    }))
}

/// Get service endpoints
#[utoipa::path(
    get,
    path = "/api/monitoring/endpoints/{app_name}",
    tag = "Monitoring",
    params(
        ("app_name" = String, Path, description = "Application name"),
        PodQuery,
    ),
    responses(
        (status = 200, body = Vec<ServiceEndpoint>)
    )
)]
async fn get_endpoints(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    Query(query): Query<PodQuery>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<Vec<ServiceEndpoint>>> {
    let component = kubarr_system_component(&app_name);
    let namespace = component
        .map(|component| component.namespace.to_string())
        .or(query.namespace)
        .unwrap_or_else(|| "media".to_string());
    let service_name = component
        .and_then(|component| component.service_name)
        .unwrap_or(&app_name);

    let endpoints = if let Some(client) = state.k8s_client.read().await.as_ref() {
        client
            .get_service_endpoints(service_name, &namespace)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(Json(endpoints))
}

/// Check if metrics-server is available
#[utoipa::path(
    get,
    path = "/api/monitoring/metrics-available",
    tag = "Monitoring",
    responses(
        (status = 200, body = serde_json::Value)
    )
)]
async fn check_metrics_available(
    State(state): State<AppState>,
    _auth: Authorized<MonitoringView>,
) -> Result<Json<serde_json::Value>> {
    let available = if let Some(client) = state.k8s_client.read().await.as_ref() {
        client.check_metrics_server_available().await
    } else {
        false
    };

    Ok(Json(serde_json::json!({
        "available": available,
        "message": if available { "Metrics server is available" } else { "Metrics server not found" }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(metric_value: &str, ts: f64, node: &str, instance: &str) -> serde_json::Value {
        serde_json::json!({
            "metric": { "Hostname": node, "UUID": "GPU-123", "gpu": "0",
                "GPU_I_ID": instance, "GPU_I_PROFILE": "1g.5gb" },
            "value": [ts, metric_value]
        })
    }

    #[test]
    fn gpu_samples_join_by_node_and_mig_identity_and_keep_real_zeros() {
        let now = 1_800_000_000.0;
        let utilization = vec![
            sample("0", now - 60.0, "node-a", ""),
            sample("75", now - 20.0, "node-a", "2"),
            sample("20", now - 10.0, "node-b", ""),
        ];
        let used = vec![
            sample("0", now - 60.0, "node-a", ""),
            sample("128", now - 10.0, "node-a", "2"),
        ];
        let free = vec![
            sample("1024", now - 20.0, "node-a", ""),
            sample("384", now - 10.0, "node-a", "2"),
        ];
        let devices = assemble_gpu_metrics(&utilization, &used, &free, now);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].utilization_percent, Some(0.0));
        assert_eq!(devices[0].memory_used_bytes, Some(0));
        assert_eq!(devices[0].memory_total_bytes, Some(1024 * 1_048_576));
        assert!(devices[1].device.contains("MIG 1g.5gb instance 2"));
        assert_eq!(devices[1].memory_total_bytes, Some(512 * 1_048_576));
        assert_eq!(devices[2].memory_used_bytes, None);
        assert_eq!(
            serde_json::to_value(&devices[2]).unwrap()["memory_used_bytes"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn gpu_stale_invalid_and_missing_samples_are_not_zero() {
        let now = 1_800_000_000.0;
        let stale = sample("80", now - 121.0, "node-a", "");
        let future = sample("80", now + 31.0, "node-a", "");
        let nan = sample("NaN", now, "node-a", "");
        let missing = serde_json::json!({"metric": {"UUID": "GPU-123"}, "value": [now, "50"]});
        assert!(assemble_gpu_metrics(&[stale, future, nan, missing], &[], &[], now).is_empty());
        let devices = assemble_gpu_metrics(
            &[sample("101", now, "node-a", "")],
            &[sample("10", now, "node-a", "")],
            &[sample("-2", now, "node-a", "")],
            now,
        );
        assert_eq!(devices[0].utilization_percent, None);
        assert_eq!(devices[0].memory_total_bytes, None);
        assert_eq!(devices[0].memory_used_bytes, Some(10 * 1_048_576));
    }

    #[test]
    fn gpu_queries_pair_bounded_values_with_raw_sample_timestamps() {
        for (metric, (value_query, timestamp_query)) in [
            "DCGM_FI_DEV_GPU_UTIL",
            "DCGM_FI_DEV_FB_USED",
            "DCGM_FI_DEV_FB_FREE",
        ]
        .iter()
        .zip(GPU_QUERIES)
        {
            assert_eq!(value_query, format!("default_rollup({metric}[120s])"));
            assert_eq!(timestamp_query, format!("tlast_over_time({metric}[120s])"));
        }
    }

    #[test]
    fn gpu_fresh_evaluation_with_stale_source_sample_is_unavailable() {
        let now = 1_800_000_000.0;
        // VM's instant response timestamps are *evaluation* times. The actual
        // scrape may be older even when value[0] says "now".
        let values = vec![sample("87", now, "node-a", "")];
        let source_times = vec![sample(&(now - 180.0).to_string(), now, "node-a", "")];
        assert!(assemble_gpu_metrics(
            &with_scrape_timestamps(&values, &source_times, now),
            &[],
            &[],
            now
        )
        .is_empty());

        // A timestamp from another target (even on the same node/device)
        // cannot authenticate this sample; a missing timestamp cannot either.
        let mut wrong_target = sample(&(now - 30.0).to_string(), now, "node-a", "");
        wrong_target["metric"]["job"] = serde_json::json!("other");
        assert!(with_scrape_timestamps(&values, &[wrong_target], now).is_empty());
        assert!(with_scrape_timestamps(&values, &[], now).is_empty());

        let fresh_times = vec![sample(&(now - 30.0).to_string(), now, "node-a", "")];
        let verified = with_scrape_timestamps(&values, &fresh_times, now);
        assert_eq!(
            assemble_gpu_metrics(&verified, &[], &[], now)[0].utilization_percent,
            Some(87.0)
        );

        let used = vec![sample("100", now, "node-a", "")];
        let free = vec![sample("900", now, "node-a", "")];
        let stale_free = vec![sample(&(now - 180.0).to_string(), now, "node-a", "")];
        let device = assemble_gpu_metrics(
            &verified,
            &with_scrape_timestamps(&used, &fresh_times, now),
            &with_scrape_timestamps(&free, &stale_free, now),
            now,
        )
        .remove(0);
        assert_eq!(device.memory_used_bytes, Some(100 * 1_048_576));
        assert_eq!(device.memory_total_bytes, None);
    }

    #[test]
    fn gpu_vm_response_rejects_errors_malformed_payloads_and_excessive_series() {
        assert!(parse_gpu_response(
            br#"{"status":"error","data":{"resultType":"vector","result":[]}}"#
        )
        .is_err());
        assert!(parse_gpu_response(
            br#"{"status":"success","data":{"resultType":"matrix","result":[]}}"#
        )
        .is_err());
        assert!(parse_gpu_response(
            br#"{"status":"success","data":{"resultType":"vector","result":{}}}"#
        )
        .is_err());
        assert!(parse_gpu_response(b"garbage").is_err());
        assert!(parse_gpu_response(
            br#"{"status":"success","data":{"resultType":"vector","result":[]}}"#
        )
        .unwrap()
        .is_empty());
        let excessive = serde_json::json!({"status": "success", "data": {
            "resultType": "vector", "result": vec![serde_json::json!({}); GPU_MAX_DEVICES + 1]
        }});
        assert!(parse_gpu_response(&serde_json::to_vec(&excessive).unwrap()).is_err());
    }

    // -------------------------------------------------------------------------
    // TimeSeriesPoint serialization
    // -------------------------------------------------------------------------

    #[test]
    fn time_series_point_serializes() {
        let point = TimeSeriesPoint {
            timestamp: 1708560000.0,
            value: 42.5,
        };
        let json = serde_json::to_value(&point).unwrap();
        assert_eq!(json["timestamp"], 1708560000.0_f64);
        assert_eq!(json["value"], 42.5_f64);
    }

    #[test]
    fn time_series_point_zero_values() {
        let point = TimeSeriesPoint {
            timestamp: 0.0,
            value: 0.0,
        };
        let json = serde_json::to_value(&point).unwrap();
        assert_eq!(json["timestamp"], 0.0_f64);
        assert_eq!(json["value"], 0.0_f64);
    }

    // -------------------------------------------------------------------------
    // AppMetrics serialization (has skip_serializing_if on optional fields)
    // -------------------------------------------------------------------------

    #[test]
    fn app_metrics_serializes_with_optional_percentages() {
        let m = AppMetrics {
            app_name: "radarr".to_string(),
            namespace: "radarr".to_string(),
            cpu_usage_cores: 0.125,
            memory_usage_bytes: 134_217_728,
            memory_usage_mb: 128.0,
            cpu_usage_percent: Some(12.5),
            memory_usage_percent: Some(6.25),
            network_receive_bytes_per_sec: 1024.0,
            network_transmit_bytes_per_sec: 512.0,
        };
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["app_name"], "radarr");
        assert_eq!(json["namespace"], "radarr");
        assert_eq!(json["cpu_usage_cores"], 0.125_f64);
        assert_eq!(json["memory_usage_bytes"], 134_217_728_i64);
        assert_eq!(json["memory_usage_mb"], 128.0_f64);
        assert_eq!(json["cpu_usage_percent"], 12.5_f64);
        assert_eq!(json["memory_usage_percent"], 6.25_f64);
        assert_eq!(json["network_receive_bytes_per_sec"], 1024.0_f64);
        assert_eq!(json["network_transmit_bytes_per_sec"], 512.0_f64);
    }

    #[test]
    fn app_metrics_omits_none_optional_percentages() {
        let m = AppMetrics {
            app_name: "sonarr".to_string(),
            namespace: "sonarr".to_string(),
            cpu_usage_cores: 0.05,
            memory_usage_bytes: 67_108_864,
            memory_usage_mb: 64.0,
            cpu_usage_percent: None,
            memory_usage_percent: None,
            network_receive_bytes_per_sec: 0.0,
            network_transmit_bytes_per_sec: 0.0,
        };
        let json = serde_json::to_value(&m).unwrap();
        // skip_serializing_if = "Option::is_none" — keys must be absent
        assert!(json.get("cpu_usage_percent").is_none());
        assert!(json.get("memory_usage_percent").is_none());
        assert_eq!(json["app_name"], "sonarr");
    }

    // -------------------------------------------------------------------------
    // ClusterMetrics serialization
    // -------------------------------------------------------------------------

    #[test]
    fn cluster_metrics_serializes() {
        let cm = ClusterMetrics {
            total_cpu_cores: 4.0,
            total_memory_bytes: 8_589_934_592,
            used_cpu_cores: 1.5,
            used_memory_bytes: 2_147_483_648,
            cpu_usage_percent: 37.5,
            memory_usage_percent: 25.0,
            container_count: 10,
            pod_count: 5,
            network_receive_bytes_per_sec: 2048.0,
            network_transmit_bytes_per_sec: 1024.0,
            total_storage_bytes: 107_374_182_400,
            used_storage_bytes: 21_474_836_480,
            storage_usage_percent: 20.0,
        };
        let json = serde_json::to_value(&cm).unwrap();
        assert_eq!(json["total_cpu_cores"], 4.0_f64);
        assert_eq!(json["total_memory_bytes"], 8_589_934_592_i64);
        assert_eq!(json["used_cpu_cores"], 1.5_f64);
        assert_eq!(json["cpu_usage_percent"], 37.5_f64);
        assert_eq!(json["memory_usage_percent"], 25.0_f64);
        assert_eq!(json["container_count"], 10_i32);
        assert_eq!(json["pod_count"], 5_i32);
        assert_eq!(json["storage_usage_percent"], 20.0_f64);
    }

    // -------------------------------------------------------------------------
    // PodQuery deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn pod_query_deserializes_full() {
        let json = r#"{"namespace": "media", "app": "radarr"}"#;
        let q: PodQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.namespace.as_deref(), Some("media"));
        assert_eq!(q.app.as_deref(), Some("radarr"));
    }

    #[test]
    fn pod_query_deserializes_empty() {
        let json = r#"{}"#;
        let q: PodQuery = serde_json::from_str(json).unwrap();
        assert!(q.namespace.is_none());
        assert!(q.app.is_none());
    }

    // -------------------------------------------------------------------------
    // AppDetailQuery deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn app_detail_query_deserializes_with_duration() {
        let json = r#"{"duration": "3h"}"#;
        let q: AppDetailQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.duration.as_deref(), Some("3h"));
    }

    #[test]
    fn app_detail_query_deserializes_empty() {
        let json = r#"{}"#;
        let q: AppDetailQuery = serde_json::from_str(json).unwrap();
        assert!(q.duration.is_none());
    }

    // -------------------------------------------------------------------------
    // NetworkHistoryQuery deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn network_history_query_deserializes_with_duration() {
        let json = r#"{"duration": "1h"}"#;
        let q: NetworkHistoryQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.duration.as_deref(), Some("1h"));
    }

    #[test]
    fn network_history_query_deserializes_empty() {
        let json = r#"{}"#;
        let q: NetworkHistoryQuery = serde_json::from_str(json).unwrap();
        assert!(q.duration.is_none());
    }

    // -------------------------------------------------------------------------
    // ClusterNetworkHistory serialization
    // -------------------------------------------------------------------------

    #[test]
    fn cluster_network_history_serializes() {
        let history = ClusterNetworkHistory {
            combined_series: vec![TimeSeriesPoint {
                timestamp: 1708560000.0,
                value: 1536.0,
            }],
            rx_series: vec![TimeSeriesPoint {
                timestamp: 1708560000.0,
                value: 1024.0,
            }],
            tx_series: vec![TimeSeriesPoint {
                timestamp: 1708560000.0,
                value: 512.0,
            }],
        };
        let json = serde_json::to_value(&history).unwrap();
        assert!(json["combined_series"].is_array());
        assert!(json["rx_series"].is_array());
        assert!(json["tx_series"].is_array());
        assert_eq!(json["combined_series"][0]["value"], 1536.0_f64);
        assert_eq!(json["rx_series"][0]["value"], 1024.0_f64);
        assert_eq!(json["tx_series"][0]["value"], 512.0_f64);
    }

    #[test]
    fn cluster_network_history_empty_series_serializes() {
        let history = ClusterNetworkHistory {
            combined_series: vec![],
            rx_series: vec![],
            tx_series: vec![],
        };
        let json = serde_json::to_value(&history).unwrap();
        assert_eq!(json["combined_series"].as_array().unwrap().len(), 0);
        assert_eq!(json["rx_series"].as_array().unwrap().len(), 0);
        assert_eq!(json["tx_series"].as_array().unwrap().len(), 0);
    }

    // -------------------------------------------------------------------------
    // ClusterMetricsHistory serialization
    // -------------------------------------------------------------------------

    #[test]
    fn cluster_metrics_history_serializes() {
        let history = ClusterMetricsHistory {
            cpu_series: vec![TimeSeriesPoint {
                timestamp: 1708560000.0,
                value: 37.5,
            }],
            memory_series: vec![TimeSeriesPoint {
                timestamp: 1708560000.0,
                value: 25.0,
            }],
            storage_series: vec![],
            pod_series: vec![],
            container_series: vec![],
        };
        let json = serde_json::to_value(&history).unwrap();
        assert!(json["cpu_series"].is_array());
        assert!(json["memory_series"].is_array());
        assert!(json["storage_series"].is_array());
        assert_eq!(json["cpu_series"][0]["value"], 37.5_f64);
        assert_eq!(json["memory_series"][0]["value"], 25.0_f64);
    }

    // -------------------------------------------------------------------------
    // AppHistoricalMetrics serialization
    // -------------------------------------------------------------------------

    #[test]
    fn app_historical_metrics_serializes() {
        let h = AppHistoricalMetrics {
            app_name: "radarr".to_string(),
            namespace: "radarr".to_string(),
            cpu_series: vec![],
            memory_series: vec![],
            network_rx_series: vec![],
            network_tx_series: vec![],
            cpu_usage_cores: 0.125,
            memory_usage_bytes: 134_217_728,
            memory_usage_mb: 128.0,
            network_receive_bytes_per_sec: 1024.0,
            network_transmit_bytes_per_sec: 512.0,
        };
        let json = serde_json::to_value(&h).unwrap();
        assert_eq!(json["app_name"], "radarr");
        assert_eq!(json["namespace"], "radarr");
        assert_eq!(json["cpu_usage_cores"], 0.125_f64);
        assert_eq!(json["memory_usage_mb"], 128.0_f64);
        assert!(json["cpu_series"].is_array());
    }

    #[test]
    fn app_health_serializes() {
        use crate::services::k8s::{PodStatus, ServiceEndpoint};
        let h = AppHealth {
            app_name: "sonarr".to_string(),
            namespace: "media".to_string(),
            healthy: true,
            pods: vec![],
            metrics: None,
            endpoints: vec![ServiceEndpoint {
                name: "sonarr".to_string(),
                namespace: "media".to_string(),
                port: 8989,
                target_port: None,
                port_forward_command: "kubectl port-forward...".to_string(),
                url: None,
                service_type: "ClusterIP".to_string(),
                base_path: None,
                landing_path: None,
            }],
            message: "All pods running".to_string(),
        };
        let json = serde_json::to_value(&h).unwrap();
        assert_eq!(json["app_name"], "sonarr");
        assert_eq!(json["healthy"], true);
        assert!(json["pods"].is_array());
        assert!(json["endpoints"].is_array());
        // metrics should be omitted when None
        assert!(json.get("metrics").is_none() || json["metrics"].is_null());
    }
}
