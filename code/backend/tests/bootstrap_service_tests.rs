//! Unit tests for the bootstrap service
//!
//! Covers `src/services/bootstrap.rs` (BootstrapService) and
//! `src/services/network_broadcaster.rs` struct types.
//!
//! Tests focus on:
//! - BootstrapService construction and in-memory state
//! - get_status() / is_complete() / has_started() behaviour
//! - broadcast() event serialization
//! - save_server_config() / get_server_config() DB helpers
//! - ComponentStatus and BootstrapEvent struct construction
//! - BOOTSTRAP_COMPONENTS constant values
//! - NetworkMetricsMessage / NetworkTopologyData / NetworkStatsData structs

mod common;
use common::create_test_db_with_seed;

use kubarr::services::bootstrap::{
    BootstrapEvent, BootstrapService, ComponentStatus, BOOTSTRAP_COMPONENTS,
};
use kubarr::services::network_broadcaster::{
    NetworkEdgeData, NetworkMetricsMessage, NetworkNodeData, NetworkStatsData, NetworkTopologyData,
};

use kubarr::services::catalog::AppCatalog;
use kubarr::services::k8s::K8sClient;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

// ============================================================================
// Helper: create a BootstrapService backed by a real DB
// ============================================================================

async fn make_service() -> (BootstrapService, broadcast::Receiver<String>) {
    let db = create_test_db_with_seed().await;
    let shared_db = Arc::new(RwLock::new(Some(db)));
    let k8s: Arc<RwLock<Option<K8sClient>>> = Arc::new(RwLock::new(None));
    let catalog = Arc::new(RwLock::new(AppCatalog::default()));
    let (tx, rx) = broadcast::channel(100);
    let svc = BootstrapService::new(shared_db, k8s, catalog, tx);
    (svc, rx)
}

// ============================================================================
// BOOTSTRAP_COMPONENTS constant
// ============================================================================

#[test]
fn bootstrap_components_has_four_entries() {
    assert_eq!(
        BOOTSTRAP_COMPONENTS.len(),
        4,
        "expected 4 bootstrap components"
    );
}

#[test]
fn bootstrap_components_includes_postgresql() {
    assert!(
        BOOTSTRAP_COMPONENTS.iter().any(|(c, _)| *c == "postgresql"),
        "bootstrap components must include 'postgresql'"
    );
}

#[test]
fn bootstrap_components_includes_victoriametrics() {
    assert!(
        BOOTSTRAP_COMPONENTS
            .iter()
            .any(|(c, _)| *c == "victoriametrics"),
        "bootstrap components must include 'victoriametrics'"
    );
}

#[test]
fn bootstrap_components_includes_victorialogs() {
    assert!(
        BOOTSTRAP_COMPONENTS
            .iter()
            .any(|(c, _)| *c == "victorialogs"),
        "bootstrap components must include 'victorialogs'"
    );
}

#[test]
fn bootstrap_components_includes_fluent_bit() {
    assert!(
        BOOTSTRAP_COMPONENTS.iter().any(|(c, _)| *c == "fluent-bit"),
        "bootstrap components must include 'fluent-bit'"
    );
}

// ============================================================================
// ComponentStatus struct
// ============================================================================

#[test]
fn component_status_construction() {
    let cs = ComponentStatus {
        component: "postgresql".to_string(),
        display_name: "PostgreSQL".to_string(),
        status: "pending".to_string(),
        message: Some("Waiting to install".to_string()),
        error: None,
    };
    assert_eq!(cs.component, "postgresql");
    assert_eq!(cs.display_name, "PostgreSQL");
    assert_eq!(cs.status, "pending");
    assert!(cs.message.is_some());
    assert!(cs.error.is_none());
}

#[test]
fn component_status_serialization() {
    let cs = ComponentStatus {
        component: "victoriametrics".to_string(),
        display_name: "VictoriaMetrics".to_string(),
        status: "healthy".to_string(),
        message: None,
        error: None,
    };
    let json = serde_json::to_value(&cs).expect("serialize ComponentStatus");
    assert_eq!(json["component"], "victoriametrics");
    assert_eq!(json["status"], "healthy");
    assert!(json["message"].is_null());
}

// ============================================================================
// BootstrapEvent enum
// ============================================================================

#[test]
fn bootstrap_event_component_started_serializes_correctly() {
    let evt = BootstrapEvent::ComponentStarted {
        component: "postgresql".to_string(),
        message: "Deploying...".to_string(),
    };
    let json = serde_json::to_value(&evt).expect("serialize ComponentStarted");
    assert_eq!(json["type"], "component_started");
    assert_eq!(json["component"], "postgresql");
    assert_eq!(json["message"], "Deploying...");
}

#[test]
fn bootstrap_event_component_progress_serializes_correctly() {
    let evt = BootstrapEvent::ComponentProgress {
        component: "victoriametrics".to_string(),
        message: "50% done".to_string(),
        progress: 50,
    };
    let json = serde_json::to_value(&evt).expect("serialize ComponentProgress");
    assert_eq!(json["type"], "component_progress");
    assert_eq!(json["progress"], 50);
}

#[test]
fn bootstrap_event_component_completed_serializes_correctly() {
    let evt = BootstrapEvent::ComponentCompleted {
        component: "fluent-bit".to_string(),
        message: "Done".to_string(),
    };
    let json = serde_json::to_value(&evt).expect("serialize ComponentCompleted");
    assert_eq!(json["type"], "component_completed");
    assert_eq!(json["component"], "fluent-bit");
}

#[test]
fn bootstrap_event_component_failed_serializes_correctly() {
    let evt = BootstrapEvent::ComponentFailed {
        component: "postgresql".to_string(),
        message: "Failed".to_string(),
        error: "helm error: exit code 1".to_string(),
    };
    let json = serde_json::to_value(&evt).expect("serialize ComponentFailed");
    assert_eq!(json["type"], "component_failed");
    assert_eq!(json["error"], "helm error: exit code 1");
}

#[test]
fn bootstrap_event_database_connected_serializes_correctly() {
    let evt = BootstrapEvent::DatabaseConnected {
        message: "Connection established".to_string(),
    };
    let json = serde_json::to_value(&evt).expect("serialize DatabaseConnected");
    assert_eq!(json["type"], "database_connected");
}

#[test]
fn bootstrap_event_bootstrap_complete_serializes_correctly() {
    let evt = BootstrapEvent::BootstrapComplete {
        message: "All done".to_string(),
    };
    let json = serde_json::to_value(&evt).expect("serialize BootstrapComplete");
    assert_eq!(json["type"], "bootstrap_complete");
    assert_eq!(json["message"], "All done");
}

// ============================================================================
// BootstrapService::new() and initial state
// ============================================================================

#[tokio::test]
async fn bootstrap_service_new_creates_pending_components() {
    let (svc, _rx) = make_service().await;
    // All 4 components should start in "pending" state (from in-memory fallback)
    // The DB is seeded but has no bootstrap_status rows yet
    let statuses = svc.get_status().await.expect("get_status must succeed");
    // DB is empty → returns in-memory statuses (4 pending components)
    assert_eq!(statuses.len(), 4, "must have 4 components");
    for s in &statuses {
        assert_eq!(
            s.status, "pending",
            "component {} must start as pending",
            s.component
        );
    }
}

#[tokio::test]
async fn bootstrap_service_is_not_complete_initially() {
    let (svc, _rx) = make_service().await;
    assert!(!svc.is_complete().await, "must not be complete initially");
}

#[tokio::test]
async fn bootstrap_service_has_not_started_initially() {
    let (svc, _rx) = make_service().await;
    assert!(!svc.has_started().await, "must not have started initially");
}

#[tokio::test]
async fn bootstrap_start_requires_validated_storage() {
    let (svc, _rx) = make_service().await;
    let result = svc.start_bootstrap().await;

    assert!(
        result.is_err(),
        "bootstrap must be blocked until storage is configured and validated"
    );
}

#[tokio::test]
async fn bootstrap_service_broadcast_sends_to_channel() {
    let db = create_test_db_with_seed().await;
    let shared_db = Arc::new(RwLock::new(Some(db)));
    let k8s: Arc<RwLock<Option<K8sClient>>> = Arc::new(RwLock::new(None));
    let catalog = Arc::new(RwLock::new(AppCatalog::default()));
    let (tx, mut rx) = broadcast::channel(100);
    let svc = BootstrapService::new(shared_db, k8s, catalog, tx);

    // Trigger a broadcast via start_bootstrap would be too heavy;
    // instead, use the public broadcast_tx directly
    let evt = BootstrapEvent::BootstrapComplete {
        message: "test broadcast".to_string(),
    };
    let json = serde_json::to_string(&evt).expect("serialize");
    svc.broadcast_tx.send(json.clone()).expect("must send");

    let received = rx.try_recv().expect("must receive immediately");
    assert_eq!(received, json);
}

// ============================================================================
// save_server_config / get_server_config
// ============================================================================

#[tokio::test]
async fn save_and_get_server_config_roundtrip() {
    use kubarr::services::bootstrap::{get_server_config, save_server_config};

    let db = create_test_db_with_seed().await;

    // No config initially
    let initial = get_server_config(&db)
        .await
        .expect("get_server_config must not fail");
    assert!(initial.is_none(), "no config initially");

    // Save a config
    let saved = save_server_config(&db, "my-server", "/data/storage", None)
        .await
        .expect("save_server_config must not fail");
    assert_eq!(saved.name, "my-server");
    assert_eq!(saved.storage_path, "/data/storage");

    // Retrieve it
    let fetched = get_server_config(&db)
        .await
        .expect("get_server_config after save")
        .expect("must have config after save");
    assert_eq!(fetched.name, "my-server");
    assert_eq!(fetched.storage_path, "/data/storage");
}

#[tokio::test]
async fn save_server_config_upserts_on_second_call() {
    use kubarr::services::bootstrap::{get_server_config, save_server_config};

    let db = create_test_db_with_seed().await;

    save_server_config(&db, "original-name", "/original/path", None)
        .await
        .expect("first save");
    save_server_config(&db, "updated-name", "/updated/path", None)
        .await
        .expect("second save (upsert)");

    let fetched = get_server_config(&db)
        .await
        .expect("get after upsert")
        .expect("must exist");
    assert_eq!(fetched.name, "updated-name");
    assert_eq!(fetched.storage_path, "/updated/path");
}

// ============================================================================
// NetworkMetricsMessage and related structs
// ============================================================================

#[test]
fn network_node_data_construction() {
    let node = NetworkNodeData {
        id: "ns-kubarr".to_string(),
        name: "kubarr".to_string(),
        node_type: "namespace".to_string(),
        rx_bytes_per_sec: 1024.0,
        tx_bytes_per_sec: 512.0,
        total_traffic: 1536.0,
        pod_count: 3,
        color: "#4CAF50".to_string(),
    };
    assert_eq!(node.id, "ns-kubarr");
    assert_eq!(node.pod_count, 3);
    assert_eq!(node.total_traffic, 1536.0);
}

#[test]
fn network_node_data_serialization() {
    let node = NetworkNodeData {
        id: "ns-test".to_string(),
        name: "test".to_string(),
        node_type: "namespace".to_string(),
        rx_bytes_per_sec: 100.0,
        tx_bytes_per_sec: 200.0,
        total_traffic: 300.0,
        pod_count: 1,
        color: "#blue".to_string(),
    };
    let json = serde_json::to_value(&node).expect("serialize");
    assert_eq!(json["id"], "ns-test");
    assert_eq!(json["type"], "namespace");
    assert_eq!(json["rx_bytes_per_sec"], 100.0);
}

#[test]
fn network_edge_data_construction() {
    let edge = NetworkEdgeData {
        source: "ns-a".to_string(),
        target: "ns-b".to_string(),
        edge_type: "namespace-link".to_string(),
        port: Some(8080),
        protocol: Some("TCP".to_string()),
        label: "8080/TCP".to_string(),
    };
    assert_eq!(edge.source, "ns-a");
    assert_eq!(edge.port, Some(8080));
}

#[test]
fn network_edge_data_optional_fields_skip_when_none() {
    let edge = NetworkEdgeData {
        source: "ns-x".to_string(),
        target: "ns-y".to_string(),
        edge_type: "link".to_string(),
        port: None,
        protocol: None,
        label: "link".to_string(),
    };
    let json = serde_json::to_value(&edge).expect("serialize");
    assert!(
        !json.as_object().unwrap().contains_key("port"),
        "port must be omitted"
    );
    assert!(
        !json.as_object().unwrap().contains_key("protocol"),
        "protocol must be omitted"
    );
}

#[test]
fn network_stats_data_construction() {
    let stats = NetworkStatsData {
        namespace: "kubarr".to_string(),
        app_name: "sonarr".to_string(),
        rx_bytes_per_sec: 1000.0,
        tx_bytes_per_sec: 500.0,
        rx_packets_per_sec: 10.0,
        tx_packets_per_sec: 5.0,
        rx_errors_per_sec: 0.0,
        tx_errors_per_sec: 0.0,
        rx_dropped_per_sec: 0.0,
        tx_dropped_per_sec: 0.0,
        pod_count: 2,
    };
    assert_eq!(stats.namespace, "kubarr");
    assert_eq!(stats.pod_count, 2);
}

#[test]
fn network_topology_data_empty() {
    let topo = NetworkTopologyData {
        nodes: vec![],
        edges: vec![],
    };
    let json = serde_json::to_value(&topo).expect("serialize");
    assert!(json["nodes"].as_array().unwrap().is_empty());
    assert!(json["edges"].as_array().unwrap().is_empty());
}

#[test]
fn network_metrics_message_construction() {
    let msg = NetworkMetricsMessage {
        msg_type: "network_metrics".to_string(),
        timestamp: 1700000000,
        topology: NetworkTopologyData {
            nodes: vec![],
            edges: vec![],
        },
        stats: vec![],
    };
    let json = serde_json::to_value(&msg).expect("serialize");
    assert_eq!(json["type"], "network_metrics");
    assert_eq!(json["timestamp"], 1700000000i64);
}
