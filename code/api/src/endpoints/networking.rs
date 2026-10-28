use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
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
        .route("/flow-classes", get(get_flow_classes))
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
    /// Measured flow-event rate, when Hubble reported traffic on this edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flows_per_sec: Option<f64>,
    /// True when traffic was actually seen. A Service-derived edge with no
    /// traffic stays in the graph but must not imply activity.
    pub observed: bool,
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

    // Fold in what Hubble actually saw. Pod-to-pod traffic is counted at
    // to-endpoint and traffic leaving the pod at to-stack, which is the last
    // point where the source pod is still identifiable.
    let observed = {
        use crate::endpoints::monitoring::query_vm_range;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs() as f64)
            .unwrap_or_default();
        let mut rows = query_vm_range(
            "sum by (source, destination) (rate(hubble_flows_processed_total{traffic_direction=\"egress\",subtype=\"to-endpoint\",verdict=\"FORWARDED\"}[1m]))",
            now,
            now,
            "1m",
        )
        .await;
        rows.extend(
            query_vm_range(
                "sum by (source, destination) (rate(hubble_flows_processed_total{traffic_direction=\"egress\",subtype=\"to-stack\",verdict=\"FORWARDED\"}[1m]))",
                now,
                now,
                "1m",
            )
            .await,
        );
        rows
    };
    let edges = merge_observed_edges(edges, &observed);

    // Give the class nodes somewhere to attach. The internet reuses the
    // existing external node; LAN and node traffic had nowhere to go before.
    let referenced: HashSet<&str> = edges.iter().map(|edge| edge.target.as_str()).collect();
    if referenced.contains(LAN_NODE) && !nodes.iter().any(|node| node.id == LAN_NODE) {
        nodes.push(NetworkNode {
            id: LAN_NODE.to_string(),
            name: "LAN".to_string(),
            // Its own type, not "external": the page identifies the internet
            // node by that type, and a second one would make the lookup depend
            // on array order.
            node_type: "lan".to_string(),
            rx_bytes_per_sec: 0.0,
            tx_bytes_per_sec: 0.0,
            total_traffic: 0.0,
            pod_count: 0,
            color: "#14b8a6".to_string(),
        });
    }
    if referenced.contains(NODE_NODE) && !nodes.iter().any(|node| node.id == NODE_NODE) {
        nodes.push(NetworkNode {
            id: NODE_NODE.to_string(),
            name: "Cluster nodes".to_string(),
            node_type: "infra".to_string(),
            rx_bytes_per_sec: 0.0,
            tx_bytes_per_sec: 0.0,
            total_traffic: 0.0,
            pod_count: 0,
            color: "#64748b".to_string(),
        });
    }
    // Traffic from the host itself: NFS mounts are performed by the kubelet.
    if edges.iter().any(|edge| edge.source == "host") && !nodes.iter().any(|node| node.id == "host")
    {
        nodes.push(NetworkNode {
            id: "host".to_string(),
            name: "Host".to_string(),
            node_type: "infra".to_string(),
            rx_bytes_per_sec: 0.0,
            tx_bytes_per_sec: 0.0,
            total_traffic: 0.0,
            pod_count: 0,
            color: "#64748b".to_string(),
        });
    }

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
                                        flows_per_sec: None,
                                        observed: false,
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
                                                    flows_per_sec: None,
                                                    observed: false,
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
                                        flows_per_sec: None,
                                        observed: false,
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
                                flows_per_sec: None,
                                observed: false,
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
                        flows_per_sec: None,
                        observed: false,
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
                                                        flows_per_sec: None,
                                                        observed: false,
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
                    flows_per_sec: None,
                    observed: false,
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

    // -------------------------------------------------------------------------
    // NetworkNode, NetworkEdge, NetworkTopology, NetworkStats serde
    // -------------------------------------------------------------------------

    #[test]
    fn network_node_ser() {
        let n = NetworkNode {
            id: "ns/media".to_string(),
            name: "media".to_string(),
            node_type: "namespace".to_string(),
            rx_bytes_per_sec: 1234.5,
            tx_bytes_per_sec: 5678.9,
            total_traffic: 6913.4,
            pod_count: 3,
            color: "#4CAF50".to_string(),
        };
        let json = serde_json::to_string(&n).expect("ser");
        assert!(json.contains("\"id\":\"ns/media\""));
        assert!(json.contains("\"type\":\"namespace\""));
        assert!(json.contains("\"pod_count\":3"));
    }

    #[test]
    fn network_edge_ser_with_optional() {
        let e = NetworkEdge {
            source: "ns/a".to_string(),
            target: "ns/b".to_string(),
            edge_type: "egress".to_string(),
            port: Some(80),
            protocol: Some("TCP".to_string()),
            label: "HTTP".to_string(),
            flows_per_sec: None,
            observed: false,
        };
        let json = serde_json::to_string(&e).expect("ser");
        assert!(json.contains("\"port\":80"));
        assert!(json.contains("\"protocol\":\"TCP\""));
    }

    #[test]
    fn network_edge_ser_without_optional() {
        let e = NetworkEdge {
            source: "ns/a".to_string(),
            target: "ns/b".to_string(),
            edge_type: "egress".to_string(),
            port: None,
            protocol: None,
            label: String::new(),
            flows_per_sec: None,
            observed: false,
        };
        let json = serde_json::to_string(&e).expect("ser");
        assert!(!json.contains("\"port\""));
        assert!(!json.contains("\"protocol\""));
    }

    #[test]
    fn network_topology_ser() {
        let t = NetworkTopology {
            nodes: vec![NetworkNode {
                id: "n1".to_string(),
                name: "media".to_string(),
                node_type: "namespace".to_string(),
                rx_bytes_per_sec: 0.0,
                tx_bytes_per_sec: 0.0,
                total_traffic: 0.0,
                pod_count: 1,
                color: "#fff".to_string(),
            }],
            edges: vec![],
        };
        let json = serde_json::to_string(&t).expect("ser");
        assert!(json.contains("\"nodes\""));
        assert!(json.contains("\"edges\":[]"));
    }

    #[test]
    fn network_stats_ser() {
        let s = NetworkStats {
            namespace: "media".to_string(),
            app_name: "sonarr".to_string(),
            rx_bytes_per_sec: 100.0,
            tx_bytes_per_sec: 200.0,
            rx_packets_per_sec: 10.0,
            tx_packets_per_sec: 20.0,
            rx_errors_per_sec: 0.0,
            tx_errors_per_sec: 0.0,
            rx_dropped_per_sec: 0.0,
            tx_dropped_per_sec: 0.0,
            pod_count: 1,
        };
        let json = serde_json::to_string(&s).expect("ser");
        assert!(json.contains("\"namespace\":\"media\""));
        assert!(json.contains("\"app_name\":\"sonarr\""));
    }
}

// ============================================================================
// Flow classes
//
// Hubble's flow metrics label each event with its destination, which is a bare
// namespace for cluster traffic and identity labels otherwise. That is what
// makes internet, LAN and in-cluster traffic separable; see
// docs/superpowers/specs/2026-10-28-network-flow-classification-design.md.
// ============================================================================

use crate::services::flow_classes::{classify_destination, FlowClass};

/// Flow-event rates for one destination class.
#[derive(Debug, Default, Clone, Serialize, utoipa::ToSchema)]
pub struct ClassRates {
    pub flows_per_sec: f64,
    pub drops_per_sec: f64,
}

#[derive(Debug, Default, Clone, Serialize, utoipa::ToSchema)]
pub struct ClassTotals {
    pub internet: ClassRates,
    pub lan: ClassRates,
    pub cluster: ClassRates,
}

#[derive(Debug, Default, Clone, Serialize, utoipa::ToSchema)]
pub struct AppClassRates {
    pub app: String,
    pub internet: ClassRates,
    pub lan: ClassRates,
    pub cluster: ClassRates,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct FlowSeriesPoint {
    pub t: i64,
    pub flows_per_sec: f64,
}

#[derive(Debug, Default, Clone, Serialize, utoipa::ToSchema)]
pub struct FlowSeries {
    pub internet: Vec<FlowSeriesPoint>,
    pub lan: Vec<FlowSeriesPoint>,
    pub cluster: Vec<FlowSeriesPoint>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct FlowClassResponse {
    pub observed_at: i64,
    /// True when Hubble reports no series at all. The UI must say the data is
    /// unavailable rather than drawing a convincing zero.
    pub stale: bool,
    pub totals: ClassTotals,
    pub by_app: Vec<AppClassRates>,
    pub series: FlowSeries,
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct FlowClassQuery {
    pub window: Option<String>,
}

/// Span in seconds and a step giving roughly sixty points.
fn window_span_and_step(window: &str) -> (i64, String) {
    match window {
        "1h" => (60 * 60, "1m".to_string()),
        "6h" => (6 * 60 * 60, "6m".to_string()),
        "24h" => (24 * 60 * 60, "24m".to_string()),
        _ => (15 * 60, "15s".to_string()),
    }
}

fn metric_label(entry: &serde_json::Value, name: &str) -> String {
    entry
        .get("metric")
        .and_then(|metric| metric.get(name))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn sample_value(entry: &serde_json::Value) -> Option<f64> {
    entry
        .get("value")
        .and_then(|value| value.as_array())
        .and_then(|pair| pair.get(1))
        .and_then(|raw| raw.as_str())
        .and_then(|text| text.parse::<f64>().ok())
}

fn rates_for(totals: &mut ClassTotals, class: FlowClass) -> &mut ClassRates {
    match class {
        FlowClass::Internet => &mut totals.internet,
        FlowClass::Lan => &mut totals.lan,
        FlowClass::Cluster => &mut totals.cluster,
    }
}

/// Add VictoriaMetrics rows into class totals.
///
/// Values accumulate rather than overwrite: every namespace is its own
/// destination label, so one class is the sum of many rows.
fn absorb_totals(totals: &mut ClassTotals, rows: &[serde_json::Value], drops: bool) {
    for row in rows {
        let Some(class) = classify_destination(&metric_label(row, "destination")) else {
            continue;
        };
        // A malformed sample is skipped, never read as zero.
        let Some(value) = sample_value(row) else {
            continue;
        };
        let rates = rates_for(totals, class);
        if drops {
            rates.drops_per_sec += value;
        } else {
            rates.flows_per_sec += value;
        }
    }
}

/// Group per-source rows by app and class.
///
/// An empty source is the host: LAN traffic is kubelet NFS activity rather than
/// anything an app did.
fn apps_from_vm(
    forwarded: &[serde_json::Value],
    dropped: &[serde_json::Value],
) -> Vec<AppClassRates> {
    use std::collections::BTreeMap;

    let mut by_app: BTreeMap<String, ClassTotals> = BTreeMap::new();
    for (rows, drops) in [(forwarded, false), (dropped, true)] {
        for row in rows {
            let Some(class) = classify_destination(&metric_label(row, "destination")) else {
                continue;
            };
            let Some(value) = sample_value(row) else {
                continue;
            };
            let source = metric_label(row, "source");
            let app = if source.is_empty() {
                "host".to_string()
            } else {
                source
            };
            let totals = by_app.entry(app).or_default();
            let rates = rates_for(totals, class);
            if drops {
                rates.drops_per_sec += value;
            } else {
                rates.flows_per_sec += value;
            }
        }
    }

    by_app
        .into_iter()
        .map(|(app, totals)| AppClassRates {
            app,
            internet: totals.internet,
            lan: totals.lan,
            cluster: totals.cluster,
        })
        .collect()
}

/// Hubble reporting no series at all means the pipeline is not working.
fn is_stale(rows: &[serde_json::Value]) -> bool {
    rows.is_empty()
}

/// Sum range-query rows per class and timestamp.
fn absorb_series(series: &mut FlowSeries, rows: &[serde_json::Value]) {
    use std::collections::BTreeMap;

    let mut per_class: BTreeMap<&'static str, BTreeMap<i64, f64>> = BTreeMap::new();
    for row in rows {
        let Some(class) = classify_destination(&metric_label(row, "destination")) else {
            continue;
        };
        let key = match class {
            FlowClass::Internet => "internet",
            FlowClass::Lan => "lan",
            FlowClass::Cluster => "cluster",
        };
        let Some(values) = row.get("values").and_then(|values| values.as_array()) else {
            continue;
        };
        let bucket = per_class.entry(key).or_default();
        for pair in values {
            let Some(pair) = pair.as_array() else {
                continue;
            };
            let Some(t) = pair.first().and_then(|t| t.as_f64()) else {
                continue;
            };
            let Some(value) = pair
                .get(1)
                .and_then(|raw| raw.as_str())
                .and_then(|text| text.parse::<f64>().ok())
            else {
                continue;
            };
            *bucket.entry(t as i64).or_insert(0.0) += value;
        }
    }

    for (key, points) in per_class {
        let points: Vec<FlowSeriesPoint> = points
            .into_iter()
            .map(|(t, flows_per_sec)| FlowSeriesPoint { t, flows_per_sec })
            .collect();
        match key {
            "internet" => series.internet = points,
            "lan" => series.lan = points,
            "cluster" => series.cluster = points,
            _ => {}
        }
    }
}

/// Flow-event rates by destination class
#[utoipa::path(
    get,
    path = "/api/networking/flow-classes",
    tag = "Networking",
    params(FlowClassQuery),
    responses((status = 200, body = FlowClassResponse))
)]
async fn get_flow_classes(
    Query(query): Query<FlowClassQuery>,
    _auth: Authorized<NetworkingView>,
) -> Result<impl IntoResponse> {
    use crate::endpoints::monitoring::query_vm_range;

    let window = query.window.unwrap_or_else(|| "15m".to_string());
    let (span, step) = window_span_and_step(&window);
    let end = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as f64)
        .unwrap_or_default();
    let start = end - span as f64;

    // Trace points are not interchangeable. External classes are counted at
    // to-network, where internet and LAN are measured identically; cluster
    // traffic at to-endpoint; and per-app attribution at to-stack, the last
    // point before masquerading erases the pod identity.
    let external = |verdict: &str| {
        format!(
            "sum by (destination) (rate(hubble_flows_processed_total{{traffic_direction=\"egress\",subtype=\"to-network\",verdict=\"{verdict}\"}}[1m]))"
        )
    };
    let internal = |verdict: &str| {
        format!(
            "sum by (destination) (rate(hubble_flows_processed_total{{traffic_direction=\"egress\",subtype=\"to-endpoint\",verdict=\"{verdict}\"}}[1m]))"
        )
    };
    let per_app = |verdict: &str| {
        format!(
            "sum by (source, destination) (rate(hubble_flows_processed_total{{traffic_direction=\"egress\",subtype=\"to-stack\",verdict=\"{verdict}\"}}[1m]))"
        )
    };

    let external_forwarded = query_vm_range(&external("FORWARDED"), end, end, &step).await;
    let external_dropped = query_vm_range(&external("DROPPED"), end, end, &step).await;
    let internal_forwarded = query_vm_range(&internal("FORWARDED"), end, end, &step).await;
    let internal_dropped = query_vm_range(&internal("DROPPED"), end, end, &step).await;
    let app_forwarded = query_vm_range(&per_app("FORWARDED"), end, end, &step).await;
    let app_dropped = query_vm_range(&per_app("DROPPED"), end, end, &step).await;

    let mut totals = ClassTotals::default();
    absorb_totals(&mut totals, &external_forwarded, false);
    absorb_totals(&mut totals, &external_dropped, true);
    absorb_totals(&mut totals, &internal_forwarded, false);
    absorb_totals(&mut totals, &internal_dropped, true);

    let mut series = FlowSeries::default();
    absorb_series(
        &mut series,
        &query_vm_range(&external("FORWARDED"), start, end, &step).await,
    );
    absorb_series(
        &mut series,
        &query_vm_range(&internal("FORWARDED"), start, end, &step).await,
    );

    let stale = is_stale(&external_forwarded) && is_stale(&internal_forwarded);

    Ok((
        [crate::endpoints::short_cache(10)],
        Json(FlowClassResponse {
            observed_at: end as i64,
            stale,
            totals,
            by_app: apps_from_vm(&app_forwarded, &app_dropped),
            series,
        }),
    ))
}

#[cfg(test)]
mod flow_class_tests {
    use super::*;

    #[test]
    fn windows_map_to_a_span_and_about_sixty_points() {
        assert_eq!(window_span_and_step("15m"), (15 * 60, "15s".to_string()));
        assert_eq!(window_span_and_step("1h"), (60 * 60, "1m".to_string()));
        assert_eq!(window_span_and_step("6h"), (6 * 60 * 60, "6m".to_string()));
        assert_eq!(
            window_span_and_step("24h"),
            (24 * 60 * 60, "24m".to_string())
        );
        assert_eq!(
            window_span_and_step("garbage"),
            (15 * 60, "15s".to_string())
        );
    }

    #[test]
    fn folds_victoriametrics_rows_into_class_totals() {
        let rows = serde_json::json!([
            { "metric": { "destination": "reserved:world" }, "value": [1.0, "4.133"] },
            { "metric": { "destination": "cidr:192.168.1.0/24,reserved:world" }, "value": [1.0, "5.083"] }
        ]);
        let mut totals = ClassTotals::default();
        absorb_totals(&mut totals, rows.as_array().expect("array"), false);

        assert_eq!(totals.internet.flows_per_sec, 4.133);
        assert_eq!(totals.lan.flows_per_sec, 5.083);
        assert_eq!(totals.cluster.flows_per_sec, 0.0);
    }

    #[test]
    fn several_cluster_destinations_add_up_into_one_class() {
        // Every namespace is its own destination label, so cluster totals are a
        // sum rather than a single row.
        let rows = serde_json::json!([
            { "metric": { "destination": "victoriametrics" }, "value": [1.0, "13.625"] },
            { "metric": { "destination": "kubarr-backend" }, "value": [1.0, "10.9"] }
        ]);
        let mut totals = ClassTotals::default();
        absorb_totals(&mut totals, rows.as_array().expect("array"), false);

        assert_eq!(totals.cluster.flows_per_sec, 24.525);
    }

    #[test]
    fn drops_land_in_their_own_field() {
        let rows = serde_json::json!([
            { "metric": { "destination": "reserved:world" }, "value": [1.0, "0.2"] }
        ]);
        let mut totals = ClassTotals::default();
        absorb_totals(&mut totals, rows.as_array().expect("array"), true);

        assert_eq!(totals.internet.drops_per_sec, 0.2);
        assert_eq!(totals.internet.flows_per_sec, 0.0);
    }

    #[test]
    fn per_app_rows_group_by_source_and_class() {
        let rows = serde_json::json!([
            { "metric": { "source": "qbittorrent", "destination": "reserved:world" }, "value": [1.0, "10.99"] },
            { "metric": { "source": "", "destination": "cidr:192.168.1.0/24,reserved:world" }, "value": [1.0, "4.692"] }
        ]);

        let apps = apps_from_vm(rows.as_array().expect("array"), &[]);

        let qb = apps
            .iter()
            .find(|a| a.app == "qbittorrent")
            .expect("qbittorrent");
        assert_eq!(qb.internet.flows_per_sec, 10.99);
        // An empty source is the host: LAN traffic is kubelet NFS, not an app.
        let host = apps.iter().find(|a| a.app == "host").expect("host");
        assert_eq!(host.lan.flows_per_sec, 4.692);
    }

    #[test]
    fn no_series_means_stale_rather_than_zero() {
        assert!(is_stale(&[]));
        let present = serde_json::json!([{ "metric": {}, "value": [1.0, "1"] }]);
        assert!(!is_stale(present.as_array().expect("array")));
    }

    #[test]
    fn malformed_samples_are_skipped_not_read_as_zero() {
        let rows = serde_json::json!([
            { "metric": { "destination": "reserved:world" }, "value": [1.0, "NaN-ish"] },
            { "metric": { "destination": "cidr:10.0.0.0/8,reserved:world" }, "value": [1.0, "2.5"] }
        ]);
        let mut totals = ClassTotals::default();
        absorb_totals(&mut totals, rows.as_array().expect("array"), false);

        assert_eq!(totals.internet.flows_per_sec, 0.0);
        assert_eq!(totals.lan.flows_per_sec, 2.5);
    }

    #[test]
    fn series_rows_sum_per_class_and_timestamp() {
        let rows = serde_json::json!([
            { "metric": { "destination": "victoriametrics" },
              "values": [[100.0, "2.0"], [160.0, "3.0"]] },
            { "metric": { "destination": "kubarr-backend" },
              "values": [[100.0, "1.0"], [160.0, "1.5"]] }
        ]);
        let mut series = FlowSeries::default();
        absorb_series(&mut series, rows.as_array().expect("array"));

        assert_eq!(series.cluster.len(), 2);
        assert_eq!(series.cluster[0].t, 100);
        assert_eq!(series.cluster[0].flows_per_sec, 3.0);
        assert_eq!(series.cluster[1].flows_per_sec, 4.5);
    }
}

// ============================================================================
// Observed topology edges
//
// `discover_service_connections` reports what Services make possible. Hubble
// reports what actually happened. The merge keeps Service metadata — names and
// ports, which Hubble has no label for — and takes existence and weight from
// the measurements.
// ============================================================================

/// Node id used for traffic leaving the cluster to the internet.
///
/// Reuses the existing `external` node rather than introducing a second
/// internet node, so the current page keeps rendering while sub-project B
/// reworks it.
const INTERNET_NODE: &str = "external";
const LAN_NODE: &str = "lan";
const NODE_NODE: &str = "node";

/// Resolve a Hubble destination label to a topology node id.
fn edge_target(destination: &str) -> Option<String> {
    match classify_destination(destination)? {
        FlowClass::Internet => Some(INTERNET_NODE.to_string()),
        FlowClass::Lan => Some(LAN_NODE.to_string()),
        FlowClass::Cluster => {
            if destination.contains(':') {
                // A reserved identity: the nodes themselves, not a namespace.
                Some(NODE_NODE.to_string())
            } else {
                Some(destination.to_string())
            }
        }
    }
}

/// Merge measured traffic into Service-derived edges.
pub(crate) fn merge_observed_edges(
    service_edges: Vec<NetworkEdge>,
    observed: &[serde_json::Value],
) -> Vec<NetworkEdge> {
    use std::collections::BTreeMap;

    let mut merged: BTreeMap<(String, String), NetworkEdge> = service_edges
        .into_iter()
        .map(|edge| ((edge.source.clone(), edge.target.clone()), edge))
        .collect();

    for row in observed {
        let Some(target) = edge_target(&metric_label(row, "destination")) else {
            continue;
        };
        let Some(rate) = sample_value(row) else {
            continue;
        };
        let source = metric_label(row, "source");
        // An empty source is the host: LAN traffic is kubelet NFS activity
        // rather than anything an app did.
        let source = if source.is_empty() {
            "host".to_string()
        } else {
            source
        };
        if source == target {
            continue;
        }

        let key = (source.clone(), target.clone());
        let entry = merged.entry(key).or_insert_with(|| NetworkEdge {
            source,
            target,
            edge_type: "observed".to_string(),
            port: None,
            protocol: None,
            label: String::new(),
            flows_per_sec: None,
            observed: false,
        });
        // Several destination rows can resolve to one node, so rates add up.
        entry.flows_per_sec = Some(entry.flows_per_sec.unwrap_or(0.0) + rate);
        entry.observed = true;
        if entry.label.is_empty() {
            entry.label = format!("{:.2} flows/s", entry.flows_per_sec.unwrap_or(0.0));
        }
    }

    merged.into_values().collect()
}

#[cfg(test)]
mod topology_merge_tests {
    use super::*;

    fn service_edge(source: &str, target: &str, port: i32) -> NetworkEdge {
        NetworkEdge {
            source: source.to_string(),
            target: target.to_string(),
            edge_type: "service".to_string(),
            port: Some(port),
            protocol: Some("TCP".to_string()),
            label: format!("{port}/TCP"),
            flows_per_sec: None,
            observed: false,
        }
    }

    fn observed_row(source: &str, destination: &str, rate: &str) -> serde_json::Value {
        serde_json::json!({
            "metric": { "source": source, "destination": destination },
            "value": [1.0, rate]
        })
    }

    #[test]
    fn a_service_edge_with_traffic_keeps_its_port_and_gains_a_weight() {
        let merged = merge_observed_edges(
            vec![service_edge("kubarr-backend", "victoriametrics", 8428)],
            &[observed_row("kubarr-backend", "victoriametrics", "13.625")],
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].port, Some(8428));
        assert_eq!(merged[0].flows_per_sec, Some(13.625));
        assert!(merged[0].observed);
        // The Service label is not overwritten by the rate.
        assert_eq!(merged[0].label, "8428/TCP");
    }

    #[test]
    fn traffic_with_no_service_still_becomes_an_edge() {
        // qbittorrent to the internet has no Service and can never appear on a
        // Service-derived graph.
        let merged = merge_observed_edges(
            Vec::new(),
            &[observed_row("qbittorrent", "reserved:world", "10.99")],
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, "qbittorrent");
        assert_eq!(merged[0].target, INTERNET_NODE);
        assert!(merged[0].observed);
        assert_eq!(merged[0].port, None);
        assert_eq!(merged[0].label, "10.99 flows/s");
    }

    #[test]
    fn a_service_nobody_uses_is_kept_but_marked_unobserved() {
        // Retained so the graph still shows the wiring, but the UI can draw it
        // differently rather than implying traffic that is not happening.
        let merged = merge_observed_edges(vec![service_edge("sonarr", "jackett", 9117)], &[]);

        assert_eq!(merged.len(), 1);
        assert!(!merged[0].observed);
        assert_eq!(merged[0].flows_per_sec, None);
    }

    #[test]
    fn external_destinations_collapse_into_class_nodes() {
        let merged = merge_observed_edges(
            Vec::new(),
            &[
                observed_row("", "cidr:192.168.1.0/24,reserved:world", "4.692"),
                observed_row(
                    "kubarr-worker",
                    "cidr:10.0.0.0/8,cidr:192.168.1.20/32,reserved:host",
                    "7.4",
                ),
            ],
        );

        let targets: Vec<&str> = merged.iter().map(|e| e.target.as_str()).collect();
        assert!(targets.contains(&LAN_NODE));
        // Node traffic is cluster infrastructure, not LAN.
        assert!(targets.contains(&NODE_NODE));
        // An empty source is the host.
        assert!(merged.iter().any(|e| e.source == "host"));
    }

    #[test]
    fn rows_resolving_to_the_same_node_add_up() {
        // Several reserved identities all map to the node, and world-ipv4 and
        // world-ipv6 both map to the internet.
        let merged = merge_observed_edges(
            Vec::new(),
            &[
                observed_row("sonarr", "reserved:world-ipv4", "1.5"),
                observed_row("sonarr", "reserved:world-ipv6", "0.5"),
            ],
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].flows_per_sec, Some(2.0));
    }

    #[test]
    fn a_namespace_talking_to_itself_is_not_an_edge() {
        let merged = merge_observed_edges(Vec::new(), &[observed_row("sonarr", "sonarr", "4.0")]);

        assert!(merged.is_empty());
    }
}
