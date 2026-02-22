use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tracing::debug;

use crate::error::Result;
use crate::middleware::permissions::{Authorized, NetworkingView};
use crate::services::cadvisor::{aggregate_by_namespace, fetch_cadvisor_metrics};
use crate::state::AppState;

/// Create networking routes
pub fn networking_routes(state: AppState) -> Router {
    Router::new()
        .route("/topology", get(get_network_topology))
        .route("/stats", get(get_network_stats))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct NetworkNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
    pub total_traffic: f64,
    pub pod_count: i32,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct NetworkEdge {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub edge_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    pub label: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NetworkTopology {
    pub nodes: Vec<NetworkNode>,
    pub edges: Vec<NetworkEdge>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NetworkStats {
    pub namespace: String,
    pub app_name: String,
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
    pub rx_packets_per_sec: f64,
    pub tx_packets_per_sec: f64,
    pub rx_errors_per_sec: f64,
    pub tx_errors_per_sec: f64,
    pub rx_dropped_per_sec: f64,
    pub tx_dropped_per_sec: f64,
    pub pod_count: i32,
}

// ============================================================================
// Namespace filtering
// ============================================================================

/// Check if a namespace should be excluded from display
fn is_excluded_namespace(namespace: &str) -> bool {
    namespace.starts_with("kube-")
        || namespace == "local-path-storage"
        || namespace == "default"
        || namespace == "linux"
        || namespace.is_empty()
}

// ============================================================================
// Endpoint Handlers
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/networking/topology",
    tag = "Networking",
    responses(
        (status = 200, description = "Network topology with nodes and edges", body = NetworkTopology)
    )
)]
/// Get network topology with nodes and edges
/// Uses direct cAdvisor metrics for real-time network data
async fn get_network_topology(
    State(state): State<AppState>,
    _auth: Authorized<NetworkingView>,
) -> Result<Json<NetworkTopology>> {
    // Fetch metrics directly from cAdvisor via K8s API
    let k8s_guard = state.k8s_client.read().await;
    let k8s = match k8s_guard.as_ref() {
        Some(k8s) => k8s,
        None => {
            // No K8s client available, return empty topology
            return Ok(Json(NetworkTopology {
                nodes: Vec::new(),
                edges: Vec::new(),
            }));
        }
    };

    // Fetch raw metrics from all nodes
    let raw_metrics = fetch_cadvisor_metrics(k8s.client()).await;
    let current_metrics = aggregate_by_namespace(&raw_metrics);

    // Calculate rates using cached values, with fallback to last known rates
    let mut metrics_map: HashMap<String, (f64, f64, i32)> = HashMap::new();

    for (namespace, current) in &current_metrics {
        if is_excluded_namespace(namespace) {
            continue;
        }

        // Read rates from cache (updated by background broadcaster)
        let (rx_rate, tx_rate) =
            if let Some(cached) = state.network_metrics_cache.get(namespace).await {
                (
                    cached.last_rates.rx_bytes_per_sec,
                    cached.last_rates.tx_bytes_per_sec,
                )
            } else {
                (0.0, 0.0)
            };

        metrics_map.insert(
            namespace.clone(),
            (rx_rate, tx_rate, current.pod_count as i32),
        );
    }

    // Color palette for nodes
    let colors = [
        "#3b82f6", // blue
        "#22c55e", // green
        "#f59e0b", // amber
        "#ef4444", // red
        "#8b5cf6", // violet
        "#06b6d4", // cyan
        "#ec4899", // pink
        "#f97316", // orange
    ];

    // Build nodes for all namespaces with traffic
    let mut nodes: Vec<NetworkNode> = Vec::new();
    for (i, (namespace, (rx, tx, pods))) in metrics_map.iter().enumerate() {
        nodes.push(NetworkNode {
            id: namespace.clone(),
            name: capitalize_first(namespace),
            node_type: "app".to_string(),
            rx_bytes_per_sec: (*rx * 100.0).round() / 100.0,
            tx_bytes_per_sec: (*tx * 100.0).round() / 100.0,
            total_traffic: ((*rx + *tx) * 100.0).round() / 100.0,
            pod_count: *pods,
            color: colors[i % colors.len()].to_string(),
        });
    }

    // Add external/internet node if there's significant traffic
    let total_rx: f64 = nodes.iter().map(|n| n.rx_bytes_per_sec).sum();
    let total_tx: f64 = nodes.iter().map(|n| n.tx_bytes_per_sec).sum();
    if total_rx > 0.0 || total_tx > 0.0 {
        nodes.push(NetworkNode {
            id: "external".to_string(),
            name: "Internet".to_string(),
            node_type: "external".to_string(),
            rx_bytes_per_sec: total_tx,
            tx_bytes_per_sec: total_rx,
            total_traffic: total_rx + total_tx,
            pod_count: 0,
            color: "#6b7280".to_string(),
        });
    }

    // Discover edges from Kubernetes Services
    let namespace_set: HashSet<String> = metrics_map.keys().cloned().collect();
    let edges = discover_service_connections(k8s, &namespace_set).await;

    // Drop the lock before returning
    drop(k8s_guard);

    // Deduplicate edges
    let mut edges = edges;
    edges.sort_by(|a, b| (&a.source, &a.target).cmp(&(&b.source, &b.target)));
    edges.dedup_by(|a, b| a.source == b.source && a.target == b.target);

    Ok(Json(NetworkTopology { nodes, edges }))
}

/// Discover service connections from Kubernetes
async fn discover_service_connections(
    k8s: &crate::services::K8sClient,
    namespaces: &HashSet<String>,
) -> Vec<NetworkEdge> {
    use k8s_openapi::api::core::v1::{ConfigMap, Endpoints, Service};
    use kube::api::{Api, ListParams};

    let mut edges: Vec<NetworkEdge> = Vec::new();
    let mut seen_edges: HashSet<(String, String)> = HashSet::new(); // Track source-target pairs
    let mut service_to_namespace: HashMap<String, String> = HashMap::new();

    // Build a map of service names to their namespaces
    for ns in namespaces {
        let services: Api<Service> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(svc_list) = services.list(&ListParams::default()).await {
            for svc in svc_list.items {
                if let Some(name) = &svc.metadata.name {
                    // Store both short name and FQDN
                    service_to_namespace.insert(name.clone(), ns.clone());
                    service_to_namespace.insert(format!("{}.{}", name, ns), ns.clone());
                    service_to_namespace
                        .insert(format!("{}.{}.svc.cluster.local", name, ns), ns.clone());
                }
            }
        }
    }

    // For each namespace, look for references to other services in ConfigMaps and environment
    for ns in namespaces {
        // Check ConfigMaps for service references
        let configmaps: Api<ConfigMap> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(cm_list) = configmaps.list(&ListParams::default()).await {
            for cm in cm_list.items {
                if let Some(data) = &cm.data {
                    for value in data.values() {
                        // Look for URLs or service references in config values
                        for (svc_name, target_ns) in &service_to_namespace {
                            if target_ns != ns && value.contains(svc_name) {
                                let edge_key = (ns.clone(), target_ns.clone());
                                if !seen_edges.contains(&edge_key) {
                                    seen_edges.insert(edge_key);
                                    edges.push(NetworkEdge {
                                        source: ns.clone(),
                                        target: target_ns.clone(),
                                        edge_type: "config".to_string(),
                                        port: None,
                                        protocol: Some("HTTP".to_string()),
                                        label: String::new(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check Endpoints to find cross-namespace connections
        let endpoints: Api<Endpoints> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(ep_list) = endpoints.list(&ListParams::default()).await {
            for ep in ep_list.items {
                if let Some(subsets) = &ep.subsets {
                    for subset in subsets {
                        if let Some(addresses) = &subset.addresses {
                            for addr in addresses {
                                // Check if this endpoint points to a pod in a different namespace
                                if let Some(target_ref) = &addr.target_ref {
                                    if let Some(target_ns) = &target_ref.namespace {
                                        if target_ns != ns && namespaces.contains(target_ns) {
                                            let edge_key = (ns.clone(), target_ns.clone());
                                            if !seen_edges.contains(&edge_key) {
                                                seen_edges.insert(edge_key);
                                                let port = subset
                                                    .ports
                                                    .as_ref()
                                                    .and_then(|p| p.first())
                                                    .map(|p| p.port);
                                                edges.push(NetworkEdge {
                                                    source: ns.clone(),
                                                    target: target_ns.clone(),
                                                    edge_type: "endpoint".to_string(),
                                                    port,
                                                    protocol: Some("TCP".to_string()),
                                                    label: String::new(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Look for common patterns in service names that indicate dependencies
    // e.g., proxy configs, ingress annotations, etc.
    for ns in namespaces {
        let services: Api<Service> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(svc_list) = services.list(&ListParams::default()).await {
            for svc in svc_list.items {
                if let Some(annotations) = svc.metadata.annotations {
                    // Check for upstream/backend annotations (common in ingress/proxy configs)
                    for value in annotations.values() {
                        for (svc_name, target_ns) in &service_to_namespace {
                            if target_ns != ns && value.contains(svc_name) {
                                let edge_key = (ns.clone(), target_ns.clone());
                                if !seen_edges.contains(&edge_key) {
                                    seen_edges.insert(edge_key);
                                    let port = svc
                                        .spec
                                        .as_ref()
                                        .and_then(|s| s.ports.as_ref())
                                        .and_then(|p| p.first())
                                        .map(|p| p.port);
                                    edges.push(NetworkEdge {
                                        source: ns.clone(),
                                        target: target_ns.clone(),
                                        edge_type: "upstream".to_string(),
                                        port,
                                        protocol: Some("HTTP".to_string()),
                                        label: String::new(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Add external edge to namespaces that have LoadBalancer or NodePort services
    for ns in namespaces {
        let edge_key = ("external".to_string(), ns.clone());
        if seen_edges.contains(&edge_key) {
            continue;
        }
        let services: Api<Service> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(svc_list) = services.list(&ListParams::default()).await {
            for svc in svc_list.items {
                if let Some(spec) = &svc.spec {
                    if let Some(svc_type) = &spec.type_ {
                        if svc_type == "LoadBalancer" || svc_type == "NodePort" {
                            seen_edges.insert(edge_key.clone());
                            edges.push(NetworkEdge {
                                source: "external".to_string(),
                                target: ns.clone(),
                                edge_type: "ingress".to_string(),
                                port: spec.ports.as_ref().and_then(|p| p.first()).map(|p| p.port),
                                protocol: Some("HTTP".to_string()),
                                label: String::new(),
                            });
                            break; // One external edge per namespace is enough
                        }
                    }
                }
            }
        }
    }

    // Check for Ingress resources that indicate external access
    use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
    for ns in namespaces {
        let ingresses: Api<Ingress> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(ing_list) = ingresses.list(&ListParams::default()).await {
            // Only process first ingress - one external edge per namespace is enough
            if let Some(ing) = ing_list.items.into_iter().next() {
                // Add external edge for namespaces with Ingress resources
                let ext_edge_key = ("external".to_string(), ns.clone());
                if !seen_edges.contains(&ext_edge_key) {
                    seen_edges.insert(ext_edge_key);
                    edges.push(NetworkEdge {
                        source: "external".to_string(),
                        target: ns.clone(),
                        edge_type: "ingress".to_string(),
                        port: Some(443),
                        protocol: Some("HTTPS".to_string()),
                        label: String::new(),
                    });
                }

                // Also check ingress backend services for connections
                if let Some(spec) = &ing.spec {
                    if let Some(rules) = &spec.rules {
                        for rule in rules {
                            if let Some(http) = &rule.http {
                                for path in &http.paths {
                                    if let Some(backend) = &path.backend.service {
                                        if let Some(target_ns) =
                                            service_to_namespace.get(&backend.name)
                                        {
                                            if target_ns != ns {
                                                let backend_edge_key =
                                                    (ns.clone(), target_ns.clone());
                                                if !seen_edges.contains(&backend_edge_key) {
                                                    seen_edges.insert(backend_edge_key);
                                                    edges.push(NetworkEdge {
                                                        source: ns.clone(),
                                                        target: target_ns.clone(),
                                                        edge_type: "ingress-backend".to_string(),
                                                        port: backend
                                                            .port
                                                            .as_ref()
                                                            .and_then(|p| p.number),
                                                        protocol: Some("HTTP".to_string()),
                                                        label: String::new(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Check NetworkPolicies to determine egress access to the internet
    // By default, Kubernetes allows all egress traffic. Only if there's a NetworkPolicy
    // that explicitly restricts egress would a namespace be blocked from the internet.
    for ns in namespaces {
        let netpols: Api<NetworkPolicy> = Api::namespaced(k8s.client().clone(), ns);
        let mut egress_blocked = false;

        if let Ok(np_list) = netpols.list(&ListParams::default()).await {
            for np in np_list.items {
                if let Some(spec) = &np.spec {
                    // Check if this policy has egress rules
                    if let Some(policy_types) = &spec.policy_types {
                        if policy_types.iter().any(|t| t == "Egress") {
                            // If there's an Egress policy type, check if it allows external
                            if let Some(egress_rules) = &spec.egress {
                                // If egress rules exist but are empty, all egress is blocked
                                if egress_rules.is_empty() {
                                    egress_blocked = true;
                                    break;
                                }
                                // If there are rules, check if they only allow cluster-internal
                                let allows_external = egress_rules.iter().any(|rule| {
                                    // Rule with no 'to' field allows all destinations
                                    if rule.to.is_none() {
                                        return true;
                                    }
                                    // Check if any 'to' allows external (0.0.0.0/0 or no ipBlock)
                                    if let Some(to_list) = &rule.to {
                                        to_list.iter().any(|peer| {
                                            if let Some(ip_block) = &peer.ip_block {
                                                // 0.0.0.0/0 allows all external
                                                ip_block.cidr == "0.0.0.0/0"
                                            } else {
                                                // No ipBlock means it could be namespace/pod selector
                                                // which doesn't restrict external access by itself
                                                peer.namespace_selector.is_none()
                                                    && peer.pod_selector.is_none()
                                            }
                                        })
                                    } else {
                                        true
                                    }
                                });
                                if !allows_external {
                                    egress_blocked = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        // If egress is not blocked, this namespace can reach the internet
        if !egress_blocked {
            let egress_edge_key = (ns.clone(), "external".to_string());
            if !seen_edges.contains(&egress_edge_key) {
                seen_edges.insert(egress_edge_key);
                edges.push(NetworkEdge {
                    source: ns.clone(),
                    target: "external".to_string(),
                    edge_type: "egress".to_string(),
                    port: None,
                    protocol: Some("TCP".to_string()),
                    label: String::new(),
                });
            }
        }
    }

    edges
}

#[utoipa::path(
    get,
    path = "/api/networking/stats",
    tag = "Networking",
    responses(
        (status = 200, description = "Network statistics per app", body = Vec<NetworkStats>)
    )
)]
/// Get detailed network statistics per app
/// Uses direct cAdvisor metrics for real-time network data
async fn get_network_stats(
    State(state): State<AppState>,
    _auth: Authorized<NetworkingView>,
) -> Result<Json<Vec<NetworkStats>>> {
    // Fetch metrics directly from cAdvisor via K8s API
    let k8s_guard = state.k8s_client.read().await;
    let k8s = match k8s_guard.as_ref() {
        Some(k8s) => k8s,
        None => {
            // No K8s client available, return empty stats
            return Ok(Json(Vec::new()));
        }
    };

    // Fetch raw metrics from all nodes
    let raw_metrics = fetch_cadvisor_metrics(k8s.client()).await;
    let current_metrics = aggregate_by_namespace(&raw_metrics);

    // Drop the lock before processing
    drop(k8s_guard);

    // Calculate rates using cached values, with fallback to last known rates
    let mut stats: Vec<NetworkStats> = Vec::new();

    for (namespace, current) in &current_metrics {
        if is_excluded_namespace(namespace) {
            continue;
        }

        // Read rates from cache (updated by background broadcaster)
        let (
            rx_bytes,
            tx_bytes,
            rx_packets,
            tx_packets,
            rx_errors,
            tx_errors,
            rx_dropped,
            tx_dropped,
        ) = if let Some(cached) = state.network_metrics_cache.get(namespace).await {
            let r = &cached.last_rates;
            (
                r.rx_bytes_per_sec,
                r.tx_bytes_per_sec,
                r.rx_packets_per_sec,
                r.tx_packets_per_sec,
                r.rx_errors_per_sec,
                r.tx_errors_per_sec,
                r.rx_dropped_per_sec,
                r.tx_dropped_per_sec,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        };

        stats.push(NetworkStats {
            namespace: namespace.clone(),
            app_name: capitalize_first(namespace),
            rx_bytes_per_sec: (rx_bytes * 100.0).round() / 100.0,
            tx_bytes_per_sec: (tx_bytes * 100.0).round() / 100.0,
            rx_packets_per_sec: (rx_packets * 100.0).round() / 100.0,
            tx_packets_per_sec: (tx_packets * 100.0).round() / 100.0,
            rx_errors_per_sec: (rx_errors * 100.0).round() / 100.0,
            tx_errors_per_sec: (tx_errors * 100.0).round() / 100.0,
            rx_dropped_per_sec: (rx_dropped * 100.0).round() / 100.0,
            tx_dropped_per_sec: (tx_dropped * 100.0).round() / 100.0,
            pod_count: current.pod_count as i32,
        });
        // Note: Cache is updated by get_network_topology, not here
        // to avoid race conditions when both endpoints are called simultaneously
    }

    Ok(Json(stats))
}

// ============================================================================
// WebSocket Handler
// ============================================================================

/// WebSocket upgrade handler for real-time network metrics
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    tracing::info!("WebSocket upgrade request received");
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle WebSocket connection
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to the broadcast channel
    let mut rx = state.network_metrics_tx.subscribe();

    tracing::info!(
        "New WebSocket client connected for network metrics, subscribers: {}",
        state.network_metrics_tx.receiver_count()
    );

    // Spawn task to forward broadcast messages to WebSocket
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages (ping/pong, close)
    let recv_task = tokio::spawn(async move {
        while let Some(result) = receiver.next().await {
            match result {
                Ok(Message::Ping(data)) => {
                    debug!("Received ping from WebSocket client");
                    // Pong is handled automatically by axum
                    let _ = data;
                }
                Ok(Message::Close(_)) => {
                    debug!("WebSocket client requested close");
                    break;
                }
                Err(e) => {
                    debug!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for either task to complete (client disconnect or error)
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    tracing::info!("WebSocket client disconnected");
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Capitalize first letter
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // is_excluded_namespace
    // -------------------------------------------------------------------------

    #[test]
    fn excluded_kube_prefix() {
        assert!(is_excluded_namespace("kube-system"));
        assert!(is_excluded_namespace("kube-public"));
        assert!(is_excluded_namespace("kube-node-lease"));
        assert!(is_excluded_namespace("kube-"));
    }

    #[test]
    fn excluded_exact_matches() {
        assert!(is_excluded_namespace("local-path-storage"));
        assert!(is_excluded_namespace("default"));
        assert!(is_excluded_namespace("linux"));
    }

    #[test]
    fn excluded_empty_string() {
        assert!(is_excluded_namespace(""));
    }

    #[test]
    fn not_excluded_regular_namespaces() {
        assert!(!is_excluded_namespace("media"));
        assert!(!is_excluded_namespace("kubarr"));
        assert!(!is_excluded_namespace("my-app"));
        assert!(!is_excluded_namespace("production"));
        assert!(!is_excluded_namespace("kube")); // "kube" alone, no dash
    }

    #[test]
    fn not_excluded_default_substring() {
        // Must be exact match, not just containing "default"
        assert!(!is_excluded_namespace("not-default"));
        assert!(!is_excluded_namespace("default-extra"));
    }

    // -------------------------------------------------------------------------
    // capitalize_first
    // -------------------------------------------------------------------------

    #[test]
    fn capitalize_empty_string() {
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn capitalize_single_lowercase() {
        assert_eq!(capitalize_first("m"), "M");
    }

    #[test]
    fn capitalize_already_uppercase() {
        assert_eq!(capitalize_first("Media"), "Media");
    }

    #[test]
    fn capitalize_lowercase_word() {
        assert_eq!(capitalize_first("media"), "Media");
    }

    #[test]
    fn capitalize_hyphenated_namespace() {
        // Only first letter is capitalized; rest unchanged
        assert_eq!(capitalize_first("my-app"), "My-app");
    }

    #[test]
    fn capitalize_all_lowercase_preserves_rest() {
        assert_eq!(capitalize_first("kubarr"), "Kubarr");
        assert_eq!(capitalize_first("jellyfin"), "Jellyfin");
    }

    #[test]
    fn capitalize_unicode_first_char() {
        assert_eq!(capitalize_first("über"), "Über");
    }
}
