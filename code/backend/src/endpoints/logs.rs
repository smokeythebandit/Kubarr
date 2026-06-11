use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{AppError, Result};
use crate::middleware::permissions::{Authorized, LogsView};
use crate::state::AppState;

// VictoriaLogs service URL inside the cluster
const VICTORIALOGS_URL: &str = "http://victorialogs.victorialogs.svc.cluster.local:9428";

pub fn logs_routes(state: AppState) -> Router {
    Router::new()
        // VictoriaLogs endpoints (must be before /:pod_name to avoid conflicts)
        .route("/vlogs/namespaces", get(get_vlogs_namespaces))
        .route("/vlogs/labels", get(get_vlogs_labels))
        .route("/vlogs/label/{label}/values", get(get_vlogs_label_values))
        .route("/vlogs/query", get(query_vlogs))
        // Legacy Loki endpoints (redirect to VictoriaLogs)
        .route("/loki/namespaces", get(get_vlogs_namespaces))
        .route("/loki/labels", get(get_vlogs_labels))
        .route("/loki/label/{label}/values", get(get_vlogs_label_values))
        .route("/loki/query", get(query_vlogs))
        // Pod logs endpoints
        .route("/raw/{pod_name}", get(get_raw_pod_logs))
        .route("/app/{app_name}", get(get_app_logs))
        .route("/{pod_name}", get(get_pod_logs))
        .with_state(state)
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LogEntry {
    pub timestamp: String,
    pub line: String,
    pub pod_name: String,
    pub container: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PodLogsQuery {
    #[serde(default = "default_namespace")]
    pub namespace: String,
    pub container: Option<String>,
    #[serde(default = "default_tail")]
    pub tail: i32,
}

fn default_namespace() -> String {
    "media".to_string()
}

fn default_tail() -> i32 {
    100
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct VLogsQueryParams {
    #[serde(default = "default_vlogs_query")]
    pub query: String,
    pub start: Option<String>,
    pub end: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i32,
}

fn default_vlogs_query() -> String {
    "*".to_string()
}

fn default_limit() -> i32 {
    1000
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct VLogsQueryResponse {
    pub streams: Vec<VLogsStream>,
    pub total_entries: i32,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct VLogsStream {
    pub labels: HashMap<String, String>,
    pub entries: Vec<VLogsEntry>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct VLogsEntry {
    pub timestamp: String,
    pub line: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/logs/{pod_name}",
    tag = "Logs",
    params(
        ("pod_name" = String, Path, description = "Name of the pod")
    ),
    responses(
        (status = 200, description = "Pod log entries", body = Vec<LogEntry>)
    )
)]
/// Get logs from a specific pod
async fn get_pod_logs(
    State(state): State<AppState>,
    Path(pod_name): Path<String>,
    Query(params): Query<PodLogsQuery>,
    _auth: Authorized<LogsView>,
) -> Result<Json<Vec<LogEntry>>> {
    let k8s = state.k8s_client.read().await;
    let client = k8s
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;

    let logs = client
        .get_pod_logs(
            &pod_name,
            &params.namespace,
            params.container.as_deref(),
            params.tail,
        )
        .await?;

    let entries: Vec<LogEntry> = logs
        .lines()
        .map(|line| LogEntry {
            timestamp: Utc::now().to_rfc3339(),
            line: line.to_string(),
            pod_name: pod_name.clone(),
            container: params.container.clone(),
        })
        .collect();

    Ok(Json(entries))
}

#[utoipa::path(
    get,
    path = "/api/logs/app/{app_name}",
    tag = "Logs",
    params(
        ("app_name" = String, Path, description = "Name of the application")
    ),
    responses(
        (status = 200, description = "Application log entries from all pods", body = Vec<LogEntry>)
    )
)]
/// Get logs from all pods of an app
async fn get_app_logs(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    Query(params): Query<PodLogsQuery>,
    _auth: Authorized<LogsView>,
) -> Result<Json<Vec<LogEntry>>> {
    let k8s = state.k8s_client.read().await;
    let client = k8s
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;

    // Get all pods for the app
    let pods = client.list_pods(&params.namespace, Some(&app_name)).await?;

    let mut all_entries = Vec::new();

    for pod in pods {
        if let Some(pod_name) = pod.metadata.name {
            match client
                .get_pod_logs(
                    &pod_name,
                    &params.namespace,
                    params.container.as_deref(),
                    params.tail,
                )
                .await
            {
                Ok(logs) => {
                    for line in logs.lines() {
                        all_entries.push(LogEntry {
                            timestamp: Utc::now().to_rfc3339(),
                            line: line.to_string(),
                            pod_name: pod_name.clone(),
                            container: params.container.clone(),
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to get logs for pod {}: {}", pod_name, e);
                }
            }
        }
    }

    Ok(Json(all_entries))
}

#[utoipa::path(
    get,
    path = "/api/logs/raw/{pod_name}",
    tag = "Logs",
    params(
        ("pod_name" = String, Path, description = "Name of the pod")
    ),
    responses(
        (status = 200, description = "Raw pod logs as plain text", body = String)
    )
)]
/// Get raw logs from a pod as plain text
async fn get_raw_pod_logs(
    State(state): State<AppState>,
    Path(pod_name): Path<String>,
    Query(params): Query<PodLogsQuery>,
    _auth: Authorized<LogsView>,
) -> Result<String> {
    let k8s = state.k8s_client.read().await;
    let client = k8s
        .as_ref()
        .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;

    let logs = client
        .get_pod_logs(
            &pod_name,
            &params.namespace,
            params.container.as_deref(),
            params.tail,
        )
        .await?;

    Ok(logs)
}

// ============== VictoriaLogs Endpoints ==============

#[utoipa::path(
    get,
    path = "/api/logs/vlogs/namespaces",
    tag = "Logs",
    responses(
        (status = 200, description = "List of namespaces with logs", body = Vec<String>)
    )
)]
/// Get all namespaces that have logs in VictoriaLogs
async fn get_vlogs_namespaces(_auth: Authorized<LogsView>) -> Result<Json<Vec<String>>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to create HTTP client: {}", e)))?;

    // VictoriaLogs uses /select/logsql/field_values for getting field values
    // Requires a query parameter
    let response = client
        .get(format!("{}/select/logsql/field_values", VICTORIALOGS_URL))
        .query(&[("query", "*"), ("field", "namespace"), ("limit", "1000")])
        .send()
        .await
        .map_err(|e| {
            AppError::ServiceUnavailable(format!("Failed to connect to VictoriaLogs: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "VictoriaLogs returned error: {} - {}",
            status, body
        )));
    }

    // VictoriaLogs returns JSON with values array
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse VictoriaLogs response: {}", e)))?;

    let namespaces: Vec<String> = json
        .get("values")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("value").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(namespaces))
}

#[utoipa::path(
    get,
    path = "/api/logs/vlogs/labels",
    tag = "Logs",
    responses(
        (status = 200, description = "List of available log labels", body = Vec<String>)
    )
)]
/// Get all available labels (field names) from VictoriaLogs
async fn get_vlogs_labels(_auth: Authorized<LogsView>) -> Result<Json<Vec<String>>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to create HTTP client: {}", e)))?;

    // VictoriaLogs uses /select/logsql/field_names with query parameter
    let response = client
        .get(format!("{}/select/logsql/field_names", VICTORIALOGS_URL))
        .query(&[("query", "*"), ("limit", "1000")])
        .send()
        .await
        .map_err(|e| {
            AppError::ServiceUnavailable(format!("Failed to connect to VictoriaLogs: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "VictoriaLogs returned error: {} - {}",
            status, body
        )));
    }

    // VictoriaLogs returns JSON with values array
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse VictoriaLogs response: {}", e)))?;

    let labels: Vec<String> = json
        .get("values")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("value").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(labels))
}

#[utoipa::path(
    get,
    path = "/api/logs/vlogs/label/{label}/values",
    tag = "Logs",
    params(
        ("label" = String, Path, description = "Label name to get values for")
    ),
    responses(
        (status = 200, description = "List of values for the specified label", body = Vec<String>)
    )
)]
/// Get all values for a specific field from VictoriaLogs
async fn get_vlogs_label_values(
    Path(label): Path<String>,
    _auth: Authorized<LogsView>,
) -> Result<Json<Vec<String>>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to create HTTP client: {}", e)))?;

    let response = client
        .get(format!("{}/select/logsql/field_values", VICTORIALOGS_URL))
        .query(&[("query", "*"), ("field", label.as_str()), ("limit", "1000")])
        .send()
        .await
        .map_err(|e| {
            AppError::ServiceUnavailable(format!("Failed to connect to VictoriaLogs: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "VictoriaLogs returned error: {} - {}",
            status, body
        )));
    }

    // VictoriaLogs returns JSON with values array
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse VictoriaLogs response: {}", e)))?;

    let values: Vec<String> = json
        .get("values")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("value").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(values))
}

#[utoipa::path(
    get,
    path = "/api/logs/vlogs/query",
    tag = "Logs",
    responses(
        (status = 200, description = "VictoriaLogs query results", body = VLogsQueryResponse)
    )
)]
/// Query logs from VictoriaLogs using LogsQL
async fn query_vlogs(
    Query(params): Query<VLogsQueryParams>,
    _auth: Authorized<LogsView>,
) -> Result<Json<VLogsQueryResponse>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to create HTTP client: {}", e)))?;

    // Default to last hour if no time range specified
    let now = Utc::now();
    let end = params.end.unwrap_or_else(|| now.to_rfc3339());
    let start = params.start.unwrap_or_else(|| {
        let start_time = now - Duration::hours(1);
        start_time.to_rfc3339()
    });

    // Convert Loki-style query to LogsQL if needed
    let query = convert_loki_to_logsql(&params.query);

    let response = client
        .get(format!("{}/select/logsql/query", VICTORIALOGS_URL))
        .query(&[
            ("query", query.as_str()),
            ("start", start.as_str()),
            ("end", end.as_str()),
            ("limit", &params.limit.to_string()),
        ])
        .send()
        .await
        .map_err(|e| {
            AppError::ServiceUnavailable(format!("Failed to connect to VictoriaLogs: {}", e))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "VictoriaLogs returned error: {} - {}",
            status, body
        )));
    }

    // VictoriaLogs returns JSON Lines format
    let text = response
        .text()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read VictoriaLogs response: {}", e)))?;

    // Parse JSON Lines response
    let mut streams_map: HashMap<String, VLogsStream> = HashMap::new();
    let mut total_entries = 0;

    for line in text.lines() {
        if line.is_empty() {
            continue;
        }

        if let Ok(log_entry) = serde_json::from_str::<serde_json::Value>(line) {
            let namespace = log_entry
                .get("namespace")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let pod = log_entry
                .get("pod")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let container = log_entry
                .get("container")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let stream_key = format!("{}/{}/{}", namespace, pod, container);

            let timestamp = log_entry
                .get("_time")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let raw_msg = log_entry.get("_msg").and_then(|v| v.as_str()).unwrap_or("");

            // Parse the log message - handle different formats:
            // 1. key_value format: log="actual message"
            // 2. JSON format: {"log": "actual message"}
            // 3. Plain text
            let log_line = if let Some(content) = raw_msg.strip_prefix("log=") {
                // key_value format: log="message" or log=message
                if content.starts_with('"') && content.ends_with('"') && content.len() > 1 {
                    content[1..content.len() - 1].to_string()
                } else {
                    content.to_string()
                }
            } else if raw_msg.starts_with('{') {
                // JSON format: try to extract "log" field
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(raw_msg) {
                    json.get("log")
                        .and_then(|v| v.as_str())
                        .unwrap_or(raw_msg)
                        .to_string()
                } else {
                    raw_msg.to_string()
                }
            } else {
                raw_msg.to_string()
            };

            // Extract level from the log entry
            let level = log_entry
                .get("level")
                .and_then(|v| v.as_str())
                .map(|s| s.to_uppercase());

            let stream = streams_map.entry(stream_key).or_insert_with(|| {
                let mut labels = HashMap::new();
                labels.insert("namespace".to_string(), namespace);
                labels.insert("pod".to_string(), pod);
                labels.insert("container".to_string(), container);
                VLogsStream {
                    labels,
                    entries: Vec::new(),
                }
            });

            stream.entries.push(VLogsEntry {
                timestamp,
                line: log_line,
                level,
            });
            total_entries += 1;
        }
    }

    let streams: Vec<VLogsStream> = streams_map.into_values().collect();

    Ok(Json(VLogsQueryResponse {
        streams,
        total_entries,
    }))
}

/// Convert Loki LogQL query to VictoriaLogs LogsQL
pub fn convert_loki_to_logsql(query: &str) -> String {
    let query = query.trim();

    // Handle empty or wildcard queries
    if query.is_empty() || query == "*" {
        return "*".to_string();
    }

    // Handle Loki-style label matchers: {label="value"}
    if query.starts_with('{') && query.ends_with('}') {
        let inner = &query[1..query.len() - 1];

        // Parse label matchers
        let mut logsql_parts = Vec::new();
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            // Handle different operators
            if let Some(pos) = part.find("=~") {
                let label = part[..pos].trim();
                let value = part[pos + 2..].trim().trim_matches('"');
                logsql_parts.push(format!("{}:~{}", label, value));
            } else if let Some(pos) = part.find("!=") {
                let label = part[..pos].trim();
                let value = part[pos + 2..].trim().trim_matches('"');
                logsql_parts.push(format!("NOT {}:{}", label, value));
            } else if let Some(pos) = part.find('=') {
                let label = part[..pos].trim();
                let value = part[pos + 1..].trim().trim_matches('"');
                logsql_parts.push(format!("{}:{}", label, value));
            }
        }

        if logsql_parts.is_empty() {
            return "*".to_string();
        }

        return logsql_parts.join(" AND ");
    }

    // Return as-is for other queries (might already be LogsQL)
    query.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // -----------------------------------------------------------------------
    // convert_loki_to_logsql tests
    // -----------------------------------------------------------------------

    #[test]
    fn empty_query_returns_wildcard() {
        assert_eq!(convert_loki_to_logsql(""), "*");
    }

    #[test]
    fn whitespace_only_query_returns_wildcard() {
        assert_eq!(convert_loki_to_logsql("   "), "*");
    }

    #[test]
    fn wildcard_query_returns_wildcard() {
        assert_eq!(convert_loki_to_logsql("*"), "*");
    }

    #[test]
    fn single_equals_label_matcher() {
        let result = convert_loki_to_logsql(r#"{namespace="media"}"#);
        assert_eq!(result, "namespace:media");
    }

    #[test]
    fn multiple_label_matchers_joined_with_and() {
        let result = convert_loki_to_logsql(r#"{namespace="media",pod="sonarr"}"#);
        assert!(result.contains("namespace:media"));
        assert!(result.contains("pod:sonarr"));
        assert!(result.contains(" AND "));
    }

    #[test]
    fn not_equals_label_matcher() {
        let result = convert_loki_to_logsql(r#"{namespace!="default"}"#);
        assert_eq!(result, "NOT namespace:default");
    }

    #[test]
    fn regex_label_matcher() {
        let result = convert_loki_to_logsql(r#"{pod=~"sonarr.*"}"#);
        assert_eq!(result, "pod:~sonarr.*");
    }

    #[test]
    fn empty_braces_returns_wildcard() {
        let result = convert_loki_to_logsql("{}");
        assert_eq!(result, "*");
    }

    #[test]
    fn non_loki_query_passes_through() {
        let query = "some:logsql AND other:value";
        let result = convert_loki_to_logsql(query);
        assert_eq!(result, query);
    }

    #[test]
    fn label_value_with_quotes_stripped() {
        let result = convert_loki_to_logsql(r#"{app="myapp"}"#);
        assert_eq!(result, "app:myapp");
    }

    // -----------------------------------------------------------------------
    // Serde struct tests
    // -----------------------------------------------------------------------

    #[test]
    fn log_entry_ser() {
        let e = LogEntry {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            line: "some log line".to_string(),
            pod_name: "sonarr-abc123".to_string(),
            container: Some("sonarr".to_string()),
        };
        let json = serde_json::to_string(&e).expect("ser");
        assert!(json.contains("\"pod_name\":\"sonarr-abc123\""));
        assert!(json.contains("\"line\":\"some log line\""));
    }

    #[test]
    fn log_entry_ser_no_container() {
        let e = LogEntry {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            line: "line".to_string(),
            pod_name: "pod".to_string(),
            container: None,
        };
        let json = serde_json::to_string(&e).expect("ser");
        assert!(json.contains("\"container\":null"));
    }

    #[test]
    fn pod_logs_query_defaults() {
        let q: PodLogsQuery = serde_json::from_str("{}").expect("deser");
        assert_eq!(q.namespace, "media");
        assert_eq!(q.tail, 100);
        assert!(q.container.is_none());
    }

    #[test]
    fn pod_logs_query_custom() {
        let json = r#"{"namespace":"kubarr","container":"main","tail":50}"#;
        let q: PodLogsQuery = serde_json::from_str(json).expect("deser");
        assert_eq!(q.namespace, "kubarr");
        assert_eq!(q.tail, 50);
        assert_eq!(q.container.as_deref(), Some("main"));
    }

    #[test]
    fn vlogs_query_params_defaults() {
        let p: VLogsQueryParams = serde_json::from_str("{}").expect("deser");
        assert_eq!(p.query, "*");
        assert_eq!(p.limit, 1000);
        assert!(p.start.is_none());
    }

    #[test]
    fn vlogs_query_params_custom() {
        let json = r#"{"query":"{ns=\"media\"}","limit":500,"start":"2024-01-01T00:00:00Z"}"#;
        let p: VLogsQueryParams = serde_json::from_str(json).expect("deser");
        assert_eq!(p.limit, 500);
        assert!(p.start.is_some());
    }

    #[test]
    fn vlogs_entry_ser_with_level() {
        let e = VLogsEntry {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            line: "error occurred".to_string(),
            level: Some("error".to_string()),
        };
        let json = serde_json::to_string(&e).expect("ser");
        assert!(json.contains("\"level\":\"error\""));
    }

    #[test]
    fn vlogs_entry_ser_without_level_skipped() {
        let e = VLogsEntry {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            line: "info".to_string(),
            level: None,
        };
        let json = serde_json::to_string(&e).expect("ser");
        // level should be skipped due to skip_serializing_if
        assert!(!json.contains("level"));
    }

    #[test]
    fn vlogs_stream_ser() {
        let mut labels = HashMap::new();
        labels.insert("namespace".to_string(), "media".to_string());
        let s = VLogsStream {
            labels,
            entries: vec![VLogsEntry {
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                line: "test".to_string(),
                level: None,
            }],
        };
        let json = serde_json::to_string(&s).expect("ser");
        assert!(json.contains("\"labels\""));
        assert!(json.contains("media"));
    }

    #[test]
    fn vlogs_query_response_ser() {
        let r = VLogsQueryResponse {
            streams: vec![],
            total_entries: 0,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert!(json.contains("\"streams\":[]"));
        assert!(json.contains("\"total_entries\":0"));
    }
}
