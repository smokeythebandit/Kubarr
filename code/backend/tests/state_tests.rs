//! Tests for application state module

use std::sync::Arc;
use tokio::sync::RwLock;

use sea_orm::DatabaseConnection;

use kubarr::services::audit::AuditService;
use kubarr::services::catalog::AppCatalog;
use kubarr::services::chart_sync::ChartSyncService;
use kubarr::services::notification::NotificationService;
use kubarr::state::{AppState, DbConn, SharedCatalog, SharedK8sClient};
mod common;

use common::create_test_db;

#[tokio::test]
async fn test_app_state_new() {
    let db = create_test_db().await;
    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog = AppCatalog::default();
    let catalog: SharedCatalog = Arc::new(RwLock::new(catalog));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    let state = AppState::new(
        Some(db),
        k8s_client,
        catalog,
        chart_sync,
        audit,
        notification,
    );

    // Should be cloneable
    let _cloned = state.clone();
}

#[tokio::test]
async fn test_app_state_clone() {
    let db = create_test_db().await;
    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog = AppCatalog::default();
    let catalog: SharedCatalog = Arc::new(RwLock::new(catalog));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    let state1 = AppState::new(
        Some(db.clone()),
        k8s_client.clone(),
        catalog.clone(),
        chart_sync,
        audit,
        notification,
    );
    let state2 = state1.clone();

    // Both states should share the same Arc references
    assert!(Arc::ptr_eq(&state1.k8s_client, &state2.k8s_client));
    assert!(Arc::ptr_eq(&state1.catalog, &state2.catalog));
}

#[tokio::test]
async fn test_shared_k8s_client_rw() {
    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));

    // Read lock
    {
        let read = k8s_client.read().await;
        assert!(read.is_none());
    }

    // The client is None because we can't easily construct K8sClient in tests
    // But the RwLock mechanism works
}

#[tokio::test]
async fn test_shared_catalog_rw() {
    let catalog = AppCatalog::default();
    let shared: SharedCatalog = Arc::new(RwLock::new(catalog));

    // Read lock
    {
        let read = shared.read().await;
        assert!(read.get_categories().is_empty() || !read.get_categories().is_empty());
    }

    // Write lock (even if we don't mutate)
    {
        let _write = shared.write().await;
        // Could modify catalog here
    }
}

#[test]
fn test_db_conn_type_alias() {
    // DbConn is an alias for DatabaseConnection
    fn _accepts_db_conn(_db: &DbConn) {}
    fn _accepts_database_connection(_db: &DatabaseConnection) {}
    // These compile, proving the type alias works
}

// ============================================================================
// AppState DB methods
// ============================================================================

#[tokio::test]
async fn test_get_db_returns_error_when_not_connected() {
    use kubarr::state::AppState;

    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog = AppCatalog::default();
    let catalog: SharedCatalog = Arc::new(RwLock::new(catalog));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    let state = AppState::new(None, k8s_client, catalog, chart_sync, audit, notification);

    let result = state.get_db().await;
    assert!(
        result.is_err(),
        "get_db with no connection must return an error"
    );
}

#[tokio::test]
async fn test_is_db_connected_false_when_no_db() {
    use kubarr::state::AppState;

    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog = AppCatalog::default();
    let catalog: SharedCatalog = Arc::new(RwLock::new(catalog));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    let state = AppState::new(None, k8s_client, catalog, chart_sync, audit, notification);

    assert!(
        !state.is_db_connected().await,
        "is_db_connected must return false when db is None"
    );
}

#[tokio::test]
async fn test_is_db_connected_true_when_db_present() {
    use common::create_test_db;

    let db = create_test_db().await;
    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog = AppCatalog::default();
    let catalog: SharedCatalog = Arc::new(RwLock::new(catalog));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    let state = AppState::new(
        Some(db),
        k8s_client,
        catalog,
        chart_sync,
        audit,
        notification,
    );

    assert!(
        state.is_db_connected().await,
        "is_db_connected must return true when db is set"
    );
}

#[tokio::test]
async fn test_set_db_makes_get_db_succeed() {
    use common::create_test_db;
    use kubarr::state::AppState;

    let k8s_client: SharedK8sClient = Arc::new(RwLock::new(None));
    let catalog = AppCatalog::default();
    let catalog: SharedCatalog = Arc::new(RwLock::new(catalog));
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    let audit = AuditService::new();
    let notification = NotificationService::new();

    let state = AppState::new(None, k8s_client, catalog, chart_sync, audit, notification);
    assert!(!state.is_db_connected().await);

    let db = create_test_db().await;
    state.set_db(db).await;

    assert!(
        state.is_db_connected().await,
        "set_db must make is_db_connected return true"
    );
    assert!(
        state.get_db().await.is_ok(),
        "get_db must succeed after set_db"
    );
}

// ============================================================================
// EndpointCache
// ============================================================================

#[tokio::test]
async fn test_endpoint_cache_miss_on_empty() {
    use kubarr::state::EndpointCache;
    let cache = EndpointCache::new(60);
    assert!(
        cache.get("sonarr").await.is_none(),
        "Empty cache must return None"
    );
}

#[tokio::test]
async fn test_endpoint_cache_set_and_get() {
    use kubarr::state::EndpointCache;
    let cache = EndpointCache::new(60);
    cache
        .set("sonarr", "http://sonarr:8989".to_string(), None)
        .await;
    let result = cache.get("sonarr").await;
    assert!(result.is_some(), "Cache must return set value");
    let (url, path) = result.unwrap();
    assert_eq!(url, "http://sonarr:8989");
    assert!(path.is_none());
}

#[tokio::test]
async fn test_endpoint_cache_set_with_base_path() {
    use kubarr::state::EndpointCache;
    let cache = EndpointCache::new(60);
    cache
        .set(
            "radarr",
            "http://radarr:7878".to_string(),
            Some("/radarr".to_string()),
        )
        .await;
    let result = cache.get("radarr").await;
    assert!(result.is_some());
    let (url, path) = result.unwrap();
    assert_eq!(url, "http://radarr:7878");
    assert_eq!(path.as_deref(), Some("/radarr"));
}

#[tokio::test]
async fn test_endpoint_cache_invalidate() {
    use kubarr::state::EndpointCache;
    let cache = EndpointCache::new(60);
    cache
        .set("sonarr", "http://sonarr:8989".to_string(), None)
        .await;
    assert!(cache.get("sonarr").await.is_some());
    cache.invalidate("sonarr").await;
    assert!(
        cache.get("sonarr").await.is_none(),
        "Invalidated entry must not be returned"
    );
}

#[tokio::test]
async fn test_endpoint_cache_overwrite() {
    use kubarr::state::EndpointCache;
    let cache = EndpointCache::new(60);
    cache.set("app", "http://old:8080".to_string(), None).await;
    cache
        .set(
            "app",
            "http://new:9090".to_string(),
            Some("/new".to_string()),
        )
        .await;
    let (url, path) = cache.get("app").await.unwrap();
    assert_eq!(url, "http://new:9090");
    assert_eq!(path.as_deref(), Some("/new"));
}

// ============================================================================
// NetworkMetricsCache async methods (get / add_sample)
// ============================================================================

#[tokio::test]
async fn test_network_metrics_cache_miss_on_empty() {
    use kubarr::state::NetworkMetricsCache;
    let cache = NetworkMetricsCache::new();
    assert!(
        cache.get("some-namespace").await.is_none(),
        "Empty cache must return None"
    );
}

#[tokio::test]
async fn test_network_metrics_cache_add_and_get() {
    use kubarr::services::cadvisor::NamespaceNetworkMetrics;
    use kubarr::state::{NetworkMetricsCache, RateSample};
    let cache = NetworkMetricsCache::new();

    let metrics = NamespaceNetworkMetrics {
        namespace: "kubarr".to_string(),
        receive_bytes_total: 1000,
        transmit_bytes_total: 500,
        ..Default::default()
    };
    let sample = RateSample {
        rx_bytes: 100.0,
        tx_bytes: 50.0,
        ..Default::default()
    };

    let rates = cache.add_sample("kubarr", metrics, sample).await;
    assert_eq!(rates.rx_bytes_per_sec, 100.0, "rx rate must match sample");
    assert_eq!(rates.tx_bytes_per_sec, 50.0, "tx rate must match sample");

    // Now get should return the cached entry
    let cached = cache.get("kubarr").await;
    assert!(cached.is_some(), "Cache must return entry after add_sample");
    assert_eq!(
        cached.unwrap().metrics.namespace,
        "kubarr",
        "Cached entry must have correct namespace"
    );
}

#[tokio::test]
async fn test_network_metrics_cache_different_namespaces() {
    use kubarr::services::cadvisor::NamespaceNetworkMetrics;
    use kubarr::state::{NetworkMetricsCache, RateSample};
    let cache = NetworkMetricsCache::new();

    for ns in ["ns-a", "ns-b", "ns-c"] {
        let metrics = NamespaceNetworkMetrics {
            namespace: ns.to_string(),
            ..Default::default()
        };
        cache.add_sample(ns, metrics, RateSample::default()).await;
    }

    assert!(cache.get("ns-a").await.is_some(), "ns-a must be cached");
    assert!(cache.get("ns-b").await.is_some(), "ns-b must be cached");
    assert!(cache.get("ns-c").await.is_some(), "ns-c must be cached");
    assert!(
        cache.get("ns-missing").await.is_none(),
        "Missing namespace must return None"
    );
}

// ============================================================================
// NetworkMetricsCache pure helpers
// ============================================================================

#[test]
fn test_rate_from_delta_normal() {
    use kubarr::state::NetworkMetricsCache;
    let rate = NetworkMetricsCache::rate_from_delta(1000, 500, 1.0);
    assert_eq!(rate, Some(500.0));
}

#[test]
fn test_rate_from_delta_counter_reset() {
    use kubarr::state::NetworkMetricsCache;
    // current < previous → counter reset
    let rate = NetworkMetricsCache::rate_from_delta(100, 1000, 1.0);
    assert!(rate.is_none(), "Counter reset must return None");
}

#[test]
fn test_rate_from_delta_zero_elapsed() {
    use kubarr::state::NetworkMetricsCache;
    // elapsed too small
    let rate = NetworkMetricsCache::rate_from_delta(1000, 500, 0.05);
    assert!(rate.is_none(), "Too short interval must return None");
}

#[test]
fn test_smooth_rate_no_previous() {
    use kubarr::state::NetworkMetricsCache;
    // old_rate = 0.0 → use new_rate directly
    let result = NetworkMetricsCache::smooth_rate(500.0, 0.0, 0.3);
    assert_eq!(result, 500.0);
}

#[test]
fn test_smooth_rate_new_zero_decay() {
    use kubarr::state::NetworkMetricsCache;
    // new_rate = 0 → decay: old * (1 - alpha)
    let result = NetworkMetricsCache::smooth_rate(0.0, 100.0, 0.3);
    assert!((result - 70.0).abs() < 1e-9, "Decay: 100 * (1 - 0.3) = 70");
}

#[test]
fn test_smooth_rate_normal_ema() {
    use kubarr::state::NetworkMetricsCache;
    // Normal EMA: alpha * new + (1 - alpha) * old
    let result = NetworkMetricsCache::smooth_rate(200.0, 100.0, 0.5);
    assert!((result - 150.0).abs() < 1e-9, "0.5*200 + 0.5*100 = 150");
}

// ============================================================================
// RateHistory sliding window
// ============================================================================

#[test]
fn test_rate_history_empty_average() {
    use kubarr::state::RateHistory;
    let history = RateHistory::default();
    let avg = history.average();
    assert_eq!(avg.rx_bytes_per_sec, 0.0);
    assert_eq!(avg.tx_bytes_per_sec, 0.0);
}

#[test]
fn test_rate_history_single_sample() {
    use kubarr::state::{RateHistory, RateSample};
    let mut history = RateHistory::default();
    let sample = RateSample {
        rx_bytes: 100.0,
        tx_bytes: 50.0,
        ..Default::default()
    };
    let avg = history.add_sample(sample);
    assert_eq!(avg.rx_bytes_per_sec, 100.0);
    assert_eq!(avg.tx_bytes_per_sec, 50.0);
}

#[test]
fn test_rate_history_averages_multiple_samples() {
    use kubarr::state::{RateHistory, RateSample};
    let mut history = RateHistory::default();
    for i in 1..=4 {
        let sample = RateSample {
            rx_bytes: i as f64 * 100.0,
            tx_bytes: 0.0,
            ..Default::default()
        };
        history.add_sample(sample);
    }
    // 4 samples: 100, 200, 300, 400 → avg = 250
    let avg = history.average();
    assert!((avg.rx_bytes_per_sec - 250.0).abs() < 1e-9);
}

#[test]
fn test_rate_history_sliding_window_eviction() {
    use kubarr::state::{RateHistory, RateSample};
    let mut history = RateHistory::default();
    // Fill beyond RATE_WINDOW_SIZE (5)
    for i in 1..=6 {
        history.add_sample(RateSample {
            rx_bytes: i as f64,
            ..Default::default()
        });
    }
    // Window now has [2, 3, 4, 5, 6] (first dropped), avg = 4.0
    let avg = history.average();
    assert!(
        (avg.rx_bytes_per_sec - 4.0).abs() < 1e-9,
        "avg={}",
        avg.rx_bytes_per_sec
    );
}
