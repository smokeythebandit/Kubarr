//! Network metrics broadcaster for real-time WebSocket updates.
//!
//! Spawns a background task that fetches cAdvisor metrics every second
//! and broadcasts to all connected WebSocket clients via tokio broadcast channel.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::Serialize;
use tokio::time::interval;
use tracing::{debug, warn};

use crate::services::cadvisor::{aggregate_by_namespace, fetch_cadvisor_metrics};
use crate::state::{AppState, NetworkMetricsCache, RateSample};

/// WebSocket message containing network metrics
#[derive(Debug, Clone, Serialize)]
pub struct NetworkMetricsMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub timestamp: i64,
    pub topology: NetworkTopologyData,
    pub stats: Vec<NetworkStatsData>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkTopologyData {
    pub nodes: Vec<NetworkNodeData>,
    pub edges: Vec<NetworkEdgeData>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkNodeData {
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

#[derive(Debug, Clone, Serialize)]
pub struct NetworkEdgeData {
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

#[derive(Debug, Clone, Serialize)]
pub struct NetworkStatsData {
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

/// Start the background network metrics broadcaster
pub fn start_network_broadcaster(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(1));

        loop {
            ticker.tick().await;

            let subscriber_count = state.network_metrics_tx.receiver_count();

            // Fetch and broadcast metrics (always fetch to keep cache fresh)
            match fetch_and_compute_metrics(&state).await {
                Ok(message) => {
                    // Only broadcast if there are subscribers
                    if subscriber_count > 0 {
                        debug!(
                            "Broadcasting metrics to {} subscribers: {} nodes, {} stats entries",
                            subscriber_count,
                            message.topology.nodes.len(),
                            message.stats.len()
                        );
                        match serde_json::to_string(&message) {
                            Ok(json) => {
                                let _ = state.network_metrics_tx.send(json);
                            }
                            Err(e) => {
                                warn!("Failed to serialize network metrics: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch network metrics: {}", e);
                }
            }
        }
    });
}

/// Fetch cAdvisor metrics and compute topology + stats
async fn fetch_and_compute_metrics(state: &AppState) -> Result<NetworkMetricsMessage, String> {
    let k8s_guard = state.k8s_client.read().await;
    let k8s = k8s_guard
        .as_ref()
        .ok_or_else(|| "Kubernetes client not available".to_string())?;

    // Fetch raw metrics from all nodes
    let raw_metrics = fetch_cadvisor_metrics(k8s.client()).await;
    let current_metrics = aggregate_by_namespace(&raw_metrics);

    // Build topology and stats
    let mut nodes: Vec<NetworkNodeData> = Vec::new();
    let mut stats: Vec<NetworkStatsData> = Vec::new();
    let mut namespace_set: HashSet<String> = HashSet::new();

    // Color palette for nodes
    let colors = [
        "#3b82f6", "#22c55e", "#f59e0b", "#ef4444", "#8b5cf6", "#06b6d4", "#ec4899", "#f97316",
    ];

    let mut color_index = 0;

    for (namespace, current) in &current_metrics {
        if is_excluded_namespace(namespace) {
            continue;
        }

        namespace_set.insert(namespace.clone());

        // Calculate instantaneous rates and create a sample
        let sample = if let Some(cached) = state.network_metrics_cache.get(namespace).await {
            let elapsed = cached.timestamp.elapsed().as_secs_f64();

            RateSample {
                rx_bytes: NetworkMetricsCache::rate_from_delta(
                    current.receive_bytes_total,
                    cached.metrics.receive_bytes_total,
                    elapsed,
                )
                .unwrap_or(0.0),
                tx_bytes: NetworkMetricsCache::rate_from_delta(
                    current.transmit_bytes_total,
                    cached.metrics.transmit_bytes_total,
                    elapsed,
                )
                .unwrap_or(0.0),
                rx_packets: NetworkMetricsCache::rate_from_delta(
                    current.receive_packets_total,
                    cached.metrics.receive_packets_total,
                    elapsed,
                )
                .unwrap_or(0.0),
                tx_packets: NetworkMetricsCache::rate_from_delta(
                    current.transmit_packets_total,
                    cached.metrics.transmit_packets_total,
                    elapsed,
                )
                .unwrap_or(0.0),
                rx_errors: NetworkMetricsCache::rate_from_delta(
                    current.receive_errors_total,
                    cached.metrics.receive_errors_total,
                    elapsed,
                )
                .unwrap_or(0.0),
                tx_errors: NetworkMetricsCache::rate_from_delta(
                    current.transmit_errors_total,
                    cached.metrics.transmit_errors_total,
                    elapsed,
                )
                .unwrap_or(0.0),
                rx_dropped: NetworkMetricsCache::rate_from_delta(
                    current.receive_packets_dropped_total,
                    cached.metrics.receive_packets_dropped_total,
                    elapsed,
                )
                .unwrap_or(0.0),
                tx_dropped: NetworkMetricsCache::rate_from_delta(
                    current.transmit_packets_dropped_total,
                    cached.metrics.transmit_packets_dropped_total,
                    elapsed,
                )
                .unwrap_or(0.0),
            }
        } else {
            // First sample for this namespace
            RateSample::default()
        };

        // Add sample to sliding window and get averaged rates
        let avg_rates = state
            .network_metrics_cache
            .add_sample(namespace, current.clone(), sample)
            .await;

        let rx_bytes = avg_rates.rx_bytes_per_sec;
        let tx_bytes = avg_rates.tx_bytes_per_sec;
        let rx_packets = avg_rates.rx_packets_per_sec;
        let tx_packets = avg_rates.tx_packets_per_sec;
        let rx_errors = avg_rates.rx_errors_per_sec;
        let tx_errors = avg_rates.tx_errors_per_sec;
        let rx_dropped = avg_rates.rx_dropped_per_sec;
        let tx_dropped = avg_rates.tx_dropped_per_sec;

        // Build node
        nodes.push(NetworkNodeData {
            id: namespace.clone(),
            name: capitalize_first(namespace),
            node_type: "app".to_string(),
            rx_bytes_per_sec: (rx_bytes * 100.0).round() / 100.0,
            tx_bytes_per_sec: (tx_bytes * 100.0).round() / 100.0,
            total_traffic: ((rx_bytes + tx_bytes) * 100.0).round() / 100.0,
            pod_count: current.pod_count as i32,
            color: colors[color_index % colors.len()].to_string(),
        });
        color_index += 1;

        // Build stats
        stats.push(NetworkStatsData {
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
    }

    // Add external/internet node if there's significant traffic
    let total_rx: f64 = nodes.iter().map(|n| n.rx_bytes_per_sec).sum();
    let total_tx: f64 = nodes.iter().map(|n| n.tx_bytes_per_sec).sum();
    if total_rx > 0.0 || total_tx > 0.0 {
        nodes.push(NetworkNodeData {
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
    let edges = discover_service_connections(k8s, &namespace_set).await;

    // Drop K8s guard before building response
    drop(k8s_guard);

    // Deduplicate edges
    let mut edges = edges;
    edges.sort_by(|a, b| (&a.source, &a.target).cmp(&(&b.source, &b.target)));
    edges.dedup_by(|a, b| a.source == b.source && a.target == b.target);

    let timestamp = chrono::Utc::now().timestamp_millis();

    Ok(NetworkMetricsMessage {
        msg_type: "network_metrics".to_string(),
        timestamp,
        topology: NetworkTopologyData { nodes, edges },
        stats,
    })
}

/// Check if a namespace should be excluded from display
fn is_excluded_namespace(namespace: &str) -> bool {
    namespace.starts_with("kube-")
        || namespace == "local-path-storage"
        || namespace == "default"
        || namespace == "linux"
        || namespace.is_empty()
}

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
        assert!(is_excluded_namespace("kube-"));
    }

    #[test]
    fn excluded_exact_matches() {
        assert!(is_excluded_namespace("local-path-storage"));
        assert!(is_excluded_namespace("default"));
        assert!(is_excluded_namespace("linux"));
        assert!(is_excluded_namespace(""));
    }

    #[test]
    fn not_excluded_regular_namespaces() {
        assert!(!is_excluded_namespace("media"));
        assert!(!is_excluded_namespace("kubarr"));
        assert!(!is_excluded_namespace("my-app"));
        assert!(!is_excluded_namespace("kube")); // no trailing dash
    }

    // -------------------------------------------------------------------------
    // capitalize_first
    // -------------------------------------------------------------------------

    #[test]
    fn capitalize_empty() {
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn capitalize_lowercase_word() {
        assert_eq!(capitalize_first("media"), "Media");
        assert_eq!(capitalize_first("kubarr"), "Kubarr");
    }

    #[test]
    fn capitalize_already_upper() {
        assert_eq!(capitalize_first("Media"), "Media");
    }

    #[test]
    fn capitalize_hyphenated() {
        assert_eq!(capitalize_first("my-app"), "My-app");
    }

    #[test]
    fn capitalize_single_char() {
        assert_eq!(capitalize_first("m"), "M");
    }

    // -------------------------------------------------------------------------
    // Struct construction (serde coverage)
    // -------------------------------------------------------------------------

    #[test]
    fn network_metrics_message_serde() {
        let msg = NetworkMetricsMessage {
            msg_type: "network_metrics".to_string(),
            timestamp: 1000,
            topology: NetworkTopologyData {
                nodes: vec![],
                edges: vec![],
            },
            stats: vec![],
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains("\"type\":\"network_metrics\""));
    }

    #[test]
    fn network_node_data_serde() {
        let node = NetworkNodeData {
            id: "media".to_string(),
            name: "Media".to_string(),
            node_type: "app".to_string(),
            rx_bytes_per_sec: 1.5,
            tx_bytes_per_sec: 2.5,
            total_traffic: 4.0,
            pod_count: 3,
            color: "#3b82f6".to_string(),
        };
        let json = serde_json::to_string(&node).expect("serialize");
        assert!(json.contains("\"type\":\"app\""));
        assert!(json.contains("\"pod_count\":3"));
    }

    #[test]
    fn network_edge_data_serde_with_optional_fields() {
        let edge = NetworkEdgeData {
            source: "a".to_string(),
            target: "b".to_string(),
            edge_type: "config".to_string(),
            port: Some(80),
            protocol: Some("HTTP".to_string()),
            label: String::new(),
        };
        let json = serde_json::to_string(&edge).expect("serialize");
        assert!(json.contains("\"port\":80"));
        assert!(json.contains("\"protocol\":\"HTTP\""));
    }

    #[test]
    fn network_edge_data_serde_without_optional_fields() {
        let edge = NetworkEdgeData {
            source: "a".to_string(),
            target: "b".to_string(),
            edge_type: "egress".to_string(),
            port: None,
            protocol: None,
            label: String::new(),
        };
        let json = serde_json::to_string(&edge).expect("serialize");
        // skip_serializing_if means None fields should be absent
        assert!(!json.contains("\"port\""));
        assert!(!json.contains("\"protocol\""));
    }

    #[test]
    fn network_stats_data_serde() {
        let stats = NetworkStatsData {
            namespace: "media".to_string(),
            app_name: "Media".to_string(),
            rx_bytes_per_sec: 1.0,
            tx_bytes_per_sec: 2.0,
            rx_packets_per_sec: 3.0,
            tx_packets_per_sec: 4.0,
            rx_errors_per_sec: 0.0,
            tx_errors_per_sec: 0.0,
            rx_dropped_per_sec: 0.0,
            tx_dropped_per_sec: 0.0,
            pod_count: 2,
        };
        let json = serde_json::to_string(&stats).expect("serialize");
        assert!(json.contains("\"namespace\":\"media\""));
    }

    #[test]
    fn network_topology_data_clone() {
        let data = NetworkTopologyData {
            nodes: vec![NetworkNodeData {
                id: "x".to_string(),
                name: "X".to_string(),
                node_type: "app".to_string(),
                rx_bytes_per_sec: 0.0,
                tx_bytes_per_sec: 0.0,
                total_traffic: 0.0,
                pod_count: 1,
                color: "#000".to_string(),
            }],
            edges: vec![],
        };
        let cloned = data.clone();
        assert_eq!(cloned.nodes.len(), 1);
    }
}

/// Discover service connections from Kubernetes
async fn discover_service_connections(
    k8s: &crate::services::K8sClient,
    namespaces: &HashSet<String>,
) -> Vec<NetworkEdgeData> {
    use k8s_openapi::api::core::v1::{ConfigMap, Endpoints, Service};
    use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
    use kube::api::{Api, ListParams};

    let mut edges: Vec<NetworkEdgeData> = Vec::new();
    let mut seen_edges: HashSet<(String, String)> = HashSet::new();
    let mut service_to_namespace: HashMap<String, String> = HashMap::new();

    // Build a map of service names to their namespaces
    for ns in namespaces {
        let services: Api<Service> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(svc_list) = services.list(&ListParams::default()).await {
            for svc in svc_list.items {
                if let Some(name) = &svc.metadata.name {
                    service_to_namespace.insert(name.clone(), ns.clone());
                    service_to_namespace.insert(format!("{}.{}", name, ns), ns.clone());
                    service_to_namespace
                        .insert(format!("{}.{}.svc.cluster.local", name, ns), ns.clone());
                }
            }
        }
    }

    // For each namespace, look for references to other services in ConfigMaps
    for ns in namespaces {
        let configmaps: Api<ConfigMap> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(cm_list) = configmaps.list(&ListParams::default()).await {
            for cm in cm_list.items {
                if let Some(data) = &cm.data {
                    for value in data.values() {
                        for (svc_name, target_ns) in &service_to_namespace {
                            if target_ns != ns && value.contains(svc_name) {
                                let edge_key = (ns.clone(), target_ns.clone());
                                if !seen_edges.contains(&edge_key) {
                                    seen_edges.insert(edge_key);
                                    edges.push(NetworkEdgeData {
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
                                                edges.push(NetworkEdgeData {
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

    // Look for common patterns in service annotations
    for ns in namespaces {
        let services: Api<Service> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(svc_list) = services.list(&ListParams::default()).await {
            for svc in svc_list.items {
                if let Some(annotations) = svc.metadata.annotations {
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
                                    edges.push(NetworkEdgeData {
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
                            edges.push(NetworkEdgeData {
                                source: "external".to_string(),
                                target: ns.clone(),
                                edge_type: "ingress".to_string(),
                                port: spec.ports.as_ref().and_then(|p| p.first()).map(|p| p.port),
                                protocol: Some("HTTP".to_string()),
                                label: String::new(),
                            });
                            break;
                        }
                    }
                }
            }
        }
    }

    // Check for Ingress resources
    for ns in namespaces {
        let ingresses: Api<Ingress> = Api::namespaced(k8s.client().clone(), ns);
        if let Ok(ing_list) = ingresses.list(&ListParams::default()).await {
            if let Some(ing) = ing_list.items.into_iter().next() {
                let ext_edge_key = ("external".to_string(), ns.clone());
                if !seen_edges.contains(&ext_edge_key) {
                    seen_edges.insert(ext_edge_key);
                    edges.push(NetworkEdgeData {
                        source: "external".to_string(),
                        target: ns.clone(),
                        edge_type: "ingress".to_string(),
                        port: Some(443),
                        protocol: Some("HTTPS".to_string()),
                        label: String::new(),
                    });
                }

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
                                                    edges.push(NetworkEdgeData {
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
    for ns in namespaces {
        let netpols: Api<NetworkPolicy> = Api::namespaced(k8s.client().clone(), ns);
        let mut egress_blocked = false;

        if let Ok(np_list) = netpols.list(&ListParams::default()).await {
            for np in np_list.items {
                if let Some(spec) = &np.spec {
                    if let Some(policy_types) = &spec.policy_types {
                        if policy_types.iter().any(|t| t == "Egress") {
                            if let Some(egress_rules) = &spec.egress {
                                if egress_rules.is_empty() {
                                    egress_blocked = true;
                                    break;
                                }
                                let allows_external = egress_rules.iter().any(|rule| {
                                    if rule.to.is_none() {
                                        return true;
                                    }
                                    if let Some(to_list) = &rule.to {
                                        to_list.iter().any(|peer| {
                                            if let Some(ip_block) = &peer.ip_block {
                                                ip_block.cidr == "0.0.0.0/0"
                                            } else {
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

        if !egress_blocked {
            let egress_edge_key = (ns.clone(), "external".to_string());
            if !seen_edges.contains(&egress_edge_key) {
                seen_edges.insert(egress_edge_key);
                edges.push(NetworkEdgeData {
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
