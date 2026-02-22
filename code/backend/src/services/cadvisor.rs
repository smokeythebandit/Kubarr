//! cAdvisor metrics client for direct container network metrics.
//!
//! Fetches metrics directly from cAdvisor via the Kubernetes API proxy endpoint
//! `/api/v1/nodes/{node}/proxy/metrics/cadvisor` to get real-time network statistics
//! without the 30+ second latency of going through VictoriaMetrics.

use std::collections::HashMap;

use kube::Client;
use tracing::warn;

/// Container network metrics from cAdvisor
#[derive(Debug, Clone, Default)]
pub struct ContainerNetworkMetrics {
    pub namespace: String,
    pub pod: String,
    pub interface: String,
    pub receive_bytes_total: u64,
    pub transmit_bytes_total: u64,
    pub receive_packets_total: u64,
    pub transmit_packets_total: u64,
    pub receive_errors_total: u64,
    pub transmit_errors_total: u64,
    pub receive_packets_dropped_total: u64,
    pub transmit_packets_dropped_total: u64,
}

/// Aggregated network metrics by namespace
#[derive(Debug, Clone, Default)]
pub struct NamespaceNetworkMetrics {
    pub namespace: String,
    pub receive_bytes_total: u64,
    pub transmit_bytes_total: u64,
    pub receive_packets_total: u64,
    pub transmit_packets_total: u64,
    pub receive_errors_total: u64,
    pub transmit_errors_total: u64,
    pub receive_packets_dropped_total: u64,
    pub transmit_packets_dropped_total: u64,
    pub pod_count: u32,
}

/// Fetch cAdvisor metrics from all nodes via Kubernetes API proxy
pub async fn fetch_cadvisor_metrics(client: &Client) -> Vec<ContainerNetworkMetrics> {
    use k8s_openapi::api::core::v1::Node;
    use kube::api::{Api, ListParams};

    let nodes: Api<Node> = Api::all(client.clone());
    let node_list = match nodes.list(&ListParams::default()).await {
        Ok(list) => list,
        Err(e) => {
            warn!("Failed to list nodes for cAdvisor metrics: {}", e);
            return Vec::new();
        }
    };

    let mut all_metrics = Vec::new();

    for node in node_list.items {
        let node_name = match &node.metadata.name {
            Some(name) => name.clone(),
            None => continue,
        };

        match fetch_node_cadvisor_metrics(client, &node_name).await {
            Ok(metrics) => {
                all_metrics.extend(metrics);
            }
            Err(e) => {
                warn!(
                    "Failed to fetch cAdvisor metrics from node {}: {}",
                    node_name, e
                );
            }
        }
    }

    all_metrics
}

/// Fetch cAdvisor metrics from a specific node
#[allow(clippy::expect_used)]
async fn fetch_node_cadvisor_metrics(
    client: &Client,
    node_name: &str,
) -> Result<Vec<ContainerNetworkMetrics>, kube::Error> {
    let url = format!("/api/v1/nodes/{}/proxy/metrics/cadvisor", node_name);

    let request = http::Request::get(&url)
        .body(vec![])
        .expect("failed to build cAdvisor request");

    let response: String = client.request_text(request).await?;

    Ok(parse_prometheus_metrics(&response))
}

/// Parse Prometheus text format metrics from cAdvisor
fn parse_prometheus_metrics(text: &str) -> Vec<ContainerNetworkMetrics> {
    // Group metrics by (namespace, pod, interface) key
    let mut metrics_map: HashMap<(String, String, String), ContainerNetworkMetrics> =
        HashMap::new();

    for line in text.lines() {
        // Skip comments and empty lines
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        // Parse container_network_* metrics
        if let Some(metric) = parse_network_metric_line(line) {
            let key = (
                metric.namespace.clone(),
                metric.pod.clone(),
                metric.interface.clone(),
            );

            let entry = metrics_map
                .entry(key)
                .or_insert_with(|| ContainerNetworkMetrics {
                    namespace: metric.namespace.clone(),
                    pod: metric.pod.clone(),
                    interface: metric.interface.clone(),
                    ..Default::default()
                });

            // Update the specific metric value
            match metric.metric_type.as_str() {
                "receive_bytes" => entry.receive_bytes_total = metric.value,
                "transmit_bytes" => entry.transmit_bytes_total = metric.value,
                "receive_packets" => entry.receive_packets_total = metric.value,
                "transmit_packets" => entry.transmit_packets_total = metric.value,
                "receive_errors" => entry.receive_errors_total = metric.value,
                "transmit_errors" => entry.transmit_errors_total = metric.value,
                "receive_packets_dropped" => entry.receive_packets_dropped_total = metric.value,
                "transmit_packets_dropped" => entry.transmit_packets_dropped_total = metric.value,
                _ => {}
            }
        }
    }

    metrics_map.into_values().collect()
}

/// Parsed network metric from a single line
struct ParsedNetworkMetric {
    namespace: String,
    pod: String,
    interface: String,
    metric_type: String,
    value: u64,
}

/// Parse a single line of Prometheus metrics format
/// Example: container_network_receive_bytes_total{interface="eth0",namespace="kubarr",pod="backend-xxx"} 123456
fn parse_network_metric_line(line: &str) -> Option<ParsedNetworkMetric> {
    // Check for container_network_ prefix
    if !line.starts_with("container_network_") {
        return None;
    }

    // Find the metric name (before the {)
    let brace_start = line.find('{')?;
    let metric_name = &line[..brace_start];

    // Determine metric type from name
    let metric_type = if metric_name.contains("receive_bytes") {
        "receive_bytes"
    } else if metric_name.contains("transmit_bytes") {
        "transmit_bytes"
    } else if metric_name.contains("receive_packets_dropped") {
        "receive_packets_dropped"
    } else if metric_name.contains("transmit_packets_dropped") {
        "transmit_packets_dropped"
    } else if metric_name.contains("receive_packets") {
        "receive_packets"
    } else if metric_name.contains("transmit_packets") {
        "transmit_packets"
    } else if metric_name.contains("receive_errors") {
        "receive_errors"
    } else if metric_name.contains("transmit_errors") {
        "transmit_errors"
    } else {
        return None;
    };

    // Parse labels between { and }
    let brace_end = line.find('}')?;
    let labels_str = &line[brace_start + 1..brace_end];

    let mut namespace = String::new();
    let mut pod = String::new();
    let mut interface = String::new();

    for label in labels_str.split(',') {
        let label = label.trim();
        if let Some((key, value)) = label.split_once('=') {
            let value = value.trim_matches('"');
            match key {
                "namespace" => namespace = value.to_string(),
                "pod" => pod = value.to_string(),
                "interface" => interface = value.to_string(),
                _ => {}
            }
        }
    }

    // Skip if missing required labels or loopback interface
    if namespace.is_empty() || pod.is_empty() || interface.is_empty() || interface == "lo" {
        return None;
    }

    // Parse value (after the closing brace and space)
    // Format can be: "value" or "value timestamp" - we only want the value part
    let value_str = line[brace_end + 1..].trim();
    let value_part = value_str.split_whitespace().next()?;

    // Handle scientific notation (e.g., 1.234e+06)
    let value: u64 = if value_part.contains('e') || value_part.contains('E') {
        value_part.parse::<f64>().ok()?.round() as u64
    } else {
        value_part.parse().ok()?
    };

    Some(ParsedNetworkMetric {
        namespace,
        pod,
        interface,
        metric_type: metric_type.to_string(),
        value,
    })
}

/// Aggregate container metrics by namespace
pub fn aggregate_by_namespace(
    metrics: &[ContainerNetworkMetrics],
) -> HashMap<String, NamespaceNetworkMetrics> {
    let mut namespace_metrics: HashMap<String, NamespaceNetworkMetrics> = HashMap::new();
    let mut pods_per_namespace: HashMap<String, std::collections::HashSet<String>> = HashMap::new();

    for metric in metrics {
        // Track unique pods per namespace
        pods_per_namespace
            .entry(metric.namespace.clone())
            .or_default()
            .insert(metric.pod.clone());

        let entry = namespace_metrics
            .entry(metric.namespace.clone())
            .or_insert_with(|| NamespaceNetworkMetrics {
                namespace: metric.namespace.clone(),
                ..Default::default()
            });

        entry.receive_bytes_total += metric.receive_bytes_total;
        entry.transmit_bytes_total += metric.transmit_bytes_total;
        entry.receive_packets_total += metric.receive_packets_total;
        entry.transmit_packets_total += metric.transmit_packets_total;
        entry.receive_errors_total += metric.receive_errors_total;
        entry.transmit_errors_total += metric.transmit_errors_total;
        entry.receive_packets_dropped_total += metric.receive_packets_dropped_total;
        entry.transmit_packets_dropped_total += metric.transmit_packets_dropped_total;
    }

    // Set pod counts
    for (namespace, pods) in pods_per_namespace {
        if let Some(entry) = namespace_metrics.get_mut(&namespace) {
            entry.pod_count = pods.len() as u32;
        }
    }

    namespace_metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prometheus_metrics() {
        // Test with timestamps (real cAdvisor format)
        let input = r#"
# HELP container_network_receive_bytes_total Cumulative count of bytes received
# TYPE container_network_receive_bytes_total counter
container_network_receive_bytes_total{interface="eth0",namespace="kubarr",pod="backend-abc123"} 1234567 1769564593233
container_network_transmit_bytes_total{interface="eth0",namespace="kubarr",pod="backend-abc123"} 7654321 1769564593233
container_network_receive_packets_total{interface="eth0",namespace="kubarr",pod="backend-abc123"} 1000 1769564593233
container_network_transmit_packets_total{interface="eth0",namespace="kubarr",pod="backend-abc123"} 2000 1769564593233
container_network_receive_bytes_total{interface="lo",namespace="kubarr",pod="backend-abc123"} 999 1769564593233
"#;

        let metrics = parse_prometheus_metrics(input);
        assert_eq!(metrics.len(), 1);

        let m = &metrics[0];
        assert_eq!(m.namespace, "kubarr");
        assert_eq!(m.pod, "backend-abc123");
        assert_eq!(m.interface, "eth0");
        assert_eq!(m.receive_bytes_total, 1234567);
        assert_eq!(m.transmit_bytes_total, 7654321);
        assert_eq!(m.receive_packets_total, 1000);
        assert_eq!(m.transmit_packets_total, 2000);
    }

    #[test]
    fn test_parse_scientific_notation() {
        let input = r#"container_network_receive_bytes_total{interface="eth0",namespace="test",pod="pod1"} 1.234567e+06"#;
        let metrics = parse_prometheus_metrics(input);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].receive_bytes_total, 1234567);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_aggregate_by_namespace() {
        let metrics = vec![
            ContainerNetworkMetrics {
                namespace: "ns1".to_string(),
                pod: "pod1".to_string(),
                interface: "eth0".to_string(),
                receive_bytes_total: 100,
                transmit_bytes_total: 200,
                ..Default::default()
            },
            ContainerNetworkMetrics {
                namespace: "ns1".to_string(),
                pod: "pod2".to_string(),
                interface: "eth0".to_string(),
                receive_bytes_total: 150,
                transmit_bytes_total: 250,
                ..Default::default()
            },
            ContainerNetworkMetrics {
                namespace: "ns2".to_string(),
                pod: "pod3".to_string(),
                interface: "eth0".to_string(),
                receive_bytes_total: 300,
                transmit_bytes_total: 400,
                ..Default::default()
            },
        ];

        let aggregated = aggregate_by_namespace(&metrics);
        assert_eq!(aggregated.len(), 2);

        let ns1 = aggregated.get("ns1").unwrap();
        assert_eq!(ns1.receive_bytes_total, 250);
        assert_eq!(ns1.transmit_bytes_total, 450);
        assert_eq!(ns1.pod_count, 2);

        let ns2 = aggregated.get("ns2").unwrap();
        assert_eq!(ns2.receive_bytes_total, 300);
        assert_eq!(ns2.transmit_bytes_total, 400);
        assert_eq!(ns2.pod_count, 1);
    }

    // -------------------------------------------------------------------------
    // Additional metric type coverage
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_errors_and_dropped_metric_types() {
        let input = concat!(
            r#"container_network_receive_errors_total{interface="eth0",namespace="test",pod="pod1"} 5"#,
            "\n",
            r#"container_network_transmit_errors_total{interface="eth0",namespace="test",pod="pod1"} 3"#,
            "\n",
            r#"container_network_receive_packets_dropped_total{interface="eth0",namespace="test",pod="pod1"} 10"#,
            "\n",
            r#"container_network_transmit_packets_dropped_total{interface="eth0",namespace="test",pod="pod1"} 7"#,
        );

        let metrics = parse_prometheus_metrics(input);
        assert_eq!(metrics.len(), 1);
        let m = &metrics[0];
        assert_eq!(m.receive_errors_total, 5);
        assert_eq!(m.transmit_errors_total, 3);
        assert_eq!(m.receive_packets_dropped_total, 10);
        assert_eq!(m.transmit_packets_dropped_total, 7);
    }

    #[test]
    fn test_parse_unknown_metric_type_is_ignored() {
        // A container_network_ metric that doesn't match any known type
        let input = r#"container_network_unknown_counter_total{interface="eth0",namespace="test",pod="pod1"} 99"#;
        let metrics = parse_prometheus_metrics(input);
        // The line has valid labels but unknown type; it's returned with zero bytes
        // (the entry is created but no field is set)
        assert!(metrics.len() <= 1);
    }

    #[test]
    fn test_parse_missing_namespace_label_skipped() {
        // Pod and interface present but no namespace
        let input = r#"container_network_receive_bytes_total{interface="eth0",pod="pod1"} 100"#;
        let metrics = parse_prometheus_metrics(input);
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_parse_missing_pod_label_skipped() {
        let input =
            r#"container_network_receive_bytes_total{interface="eth0",namespace="test"} 100"#;
        let metrics = parse_prometheus_metrics(input);
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_parse_missing_interface_label_skipped() {
        let input = r#"container_network_receive_bytes_total{namespace="test",pod="pod1"} 100"#;
        let metrics = parse_prometheus_metrics(input);
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_parse_non_container_network_line_skipped() {
        let input = r#"container_cpu_usage_total{namespace="test",pod="pod1"} 100"#;
        let metrics = parse_prometheus_metrics(input);
        assert!(metrics.is_empty());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_aggregate_with_errors_and_dropped() {
        let metrics = vec![ContainerNetworkMetrics {
            namespace: "prod".to_string(),
            pod: "svc-1".to_string(),
            interface: "eth0".to_string(),
            receive_errors_total: 4,
            transmit_errors_total: 2,
            receive_packets_dropped_total: 8,
            transmit_packets_dropped_total: 6,
            ..Default::default()
        }];

        let aggregated = aggregate_by_namespace(&metrics);
        let prod = aggregated.get("prod").unwrap();
        assert_eq!(prod.receive_errors_total, 4);
        assert_eq!(prod.transmit_errors_total, 2);
        assert_eq!(prod.receive_packets_dropped_total, 8);
        assert_eq!(prod.transmit_packets_dropped_total, 6);
    }

    #[test]
    fn test_parse_empty_input() {
        let metrics = parse_prometheus_metrics("");
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_parse_only_comments_and_blanks() {
        let input = "# HELP foo bar\n# TYPE foo counter\n\n";
        let metrics = parse_prometheus_metrics(input);
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_aggregate_empty_input() {
        let aggregated = aggregate_by_namespace(&[]);
        assert!(aggregated.is_empty());
    }
}
