use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, RwLock};

use sea_orm::DatabaseConnection;

use crate::services::audit::AuditService;
use crate::services::cadvisor::NamespaceNetworkMetrics;
use crate::services::catalog::AppCatalog;
use crate::services::chart_sync::ChartSyncService;
use crate::services::k8s::K8sClient;
use crate::services::notification::NotificationService;
use crate::services::proxy::ProxyService;

/// Database connection type alias
pub type DbConn = DatabaseConnection;

/// Cached app endpoint with expiration
#[derive(Clone)]
pub struct CachedEndpoint {
    pub base_url: String,
    pub base_path: Option<String>,
    pub expires_at: Instant,
}

/// Cache for app service endpoints (avoids K8s API calls on every request)
#[derive(Clone, Default)]
pub struct EndpointCache {
    cache: Arc<RwLock<HashMap<String, CachedEndpoint>>>,
    ttl: Duration,
}

/// Number of samples to keep for sliding window average
const RATE_WINDOW_SIZE: usize = 5;

/// A single rate sample
#[derive(Clone, Default)]
pub struct RateSample {
    pub rx_bytes: f64,
    pub tx_bytes: f64,
    pub rx_packets: f64,
    pub tx_packets: f64,
    pub rx_errors: f64,
    pub tx_errors: f64,
    pub rx_dropped: f64,
    pub tx_dropped: f64,
}

/// Cached rates for a namespace (sliding window average)
#[derive(Clone, Default)]
pub struct CachedRates {
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
    pub rx_packets_per_sec: f64,
    pub tx_packets_per_sec: f64,
    pub rx_errors_per_sec: f64,
    pub tx_errors_per_sec: f64,
    pub rx_dropped_per_sec: f64,
    pub tx_dropped_per_sec: f64,
}

/// Sliding window of rate samples for smooth averaging
#[derive(Clone, Default)]
pub struct RateHistory {
    samples: Vec<RateSample>,
}

impl RateHistory {
    /// Add a new sample and return the sliding window average
    pub fn add_sample(&mut self, sample: RateSample) -> CachedRates {
        self.samples.push(sample);

        // Keep only the last N samples
        if self.samples.len() > RATE_WINDOW_SIZE {
            self.samples.remove(0);
        }

        self.average()
    }

    /// Compute average of all samples in the window
    pub fn average(&self) -> CachedRates {
        if self.samples.is_empty() {
            return CachedRates::default();
        }

        let count = self.samples.len() as f64;
        let mut avg = CachedRates::default();

        for s in &self.samples {
            avg.rx_bytes_per_sec += s.rx_bytes;
            avg.tx_bytes_per_sec += s.tx_bytes;
            avg.rx_packets_per_sec += s.rx_packets;
            avg.tx_packets_per_sec += s.tx_packets;
            avg.rx_errors_per_sec += s.rx_errors;
            avg.tx_errors_per_sec += s.tx_errors;
            avg.rx_dropped_per_sec += s.rx_dropped;
            avg.tx_dropped_per_sec += s.tx_dropped;
        }

        avg.rx_bytes_per_sec /= count;
        avg.tx_bytes_per_sec /= count;
        avg.rx_packets_per_sec /= count;
        avg.tx_packets_per_sec /= count;
        avg.rx_errors_per_sec /= count;
        avg.tx_errors_per_sec /= count;
        avg.rx_dropped_per_sec /= count;
        avg.tx_dropped_per_sec /= count;

        avg
    }
}

/// Cached network metrics entry with timestamp for rate calculation
#[derive(Clone)]
pub struct CachedNetworkMetrics {
    pub metrics: NamespaceNetworkMetrics,
    pub timestamp: Instant,
    /// Sliding window of rate samples for averaging
    pub rate_history: RateHistory,
    /// Last calculated averaged rates
    pub last_rates: CachedRates,
}

/// Cache for network metrics to calculate rates from cumulative counters
#[derive(Clone, Default)]
pub struct NetworkMetricsCache {
    cache: Arc<RwLock<HashMap<String, CachedNetworkMetrics>>>,
    /// Maximum age before cache entry is considered stale (5 minutes)
    max_age: Duration,
}

impl NetworkMetricsCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            max_age: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Get cached metrics for a namespace
    pub async fn get(&self, namespace: &str) -> Option<CachedNetworkMetrics> {
        let cache = self.cache.read().await;
        if let Some(entry) = cache.get(namespace) {
            // Check if not stale
            if entry.timestamp.elapsed() < self.max_age {
                return Some(entry.clone());
            }
        }
        None
    }

    /// Update cache with new metrics and a new rate sample
    /// Returns the sliding window average of rates
    pub async fn add_sample(
        &self,
        namespace: &str,
        metrics: NamespaceNetworkMetrics,
        sample: RateSample,
    ) -> CachedRates {
        let mut cache = self.cache.write().await;

        let entry = cache
            .entry(namespace.to_string())
            .or_insert_with(|| CachedNetworkMetrics {
                metrics: metrics.clone(),
                timestamp: Instant::now(),
                rate_history: RateHistory::default(),
                last_rates: CachedRates::default(),
            });

        // Update metrics and timestamp
        entry.metrics = metrics;
        entry.timestamp = Instant::now();

        // Add sample to history and get averaged rates
        let averaged_rates = entry.rate_history.add_sample(sample);
        entry.last_rates = averaged_rates.clone();

        averaged_rates
    }

    /// Calculate rate between two values over elapsed time
    /// Returns None if rate cannot be calculated (counter reset, etc.)
    pub fn rate_from_delta(current: u64, previous: u64, elapsed_secs: f64) -> Option<f64> {
        if elapsed_secs > 0.1 && current >= previous {
            Some((current - previous) as f64 / elapsed_secs)
        } else {
            // Counter reset, invalid, or too short interval
            None
        }
    }

    /// Apply exponential moving average smoothing to a rate
    /// This prevents abrupt jumps from actual values to 0 when traffic is bursty
    /// alpha controls responsiveness: higher = more responsive, lower = smoother
    pub fn smooth_rate(new_rate: f64, old_rate: f64, alpha: f64) -> f64 {
        if old_rate == 0.0 {
            // No previous value, use new rate directly
            new_rate
        } else if new_rate == 0.0 {
            // New measurement is 0, decay slowly toward 0
            old_rate * (1.0 - alpha)
        } else {
            // Normal EMA calculation
            alpha * new_rate + (1.0 - alpha) * old_rate
        }
    }
}

impl EndpointCache {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            ttl: Duration::from_secs(ttl_seconds),
        }
    }

    /// Get cached endpoint for an app (base_url, base_path)
    pub async fn get(&self, app_name: &str) -> Option<(String, Option<String>)> {
        let cache = self.cache.read().await;
        if let Some(entry) = cache.get(app_name) {
            if entry.expires_at > Instant::now() {
                return Some((entry.base_url.clone(), entry.base_path.clone()));
            }
        }
        None
    }

    /// Cache an endpoint for an app
    pub async fn set(&self, app_name: &str, base_url: String, base_path: Option<String>) {
        let mut cache = self.cache.write().await;
        cache.insert(
            app_name.to_string(),
            CachedEndpoint {
                base_url,
                base_path,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }

    /// Invalidate cache for an app (e.g., when app is restarted)
    pub async fn invalidate(&self, app_name: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(app_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // RateHistory tests
    // -------------------------------------------------------------------------

    fn make_sample(rx: f64, tx: f64) -> RateSample {
        RateSample {
            rx_bytes: rx,
            tx_bytes: tx,
            rx_packets: 1.0,
            tx_packets: 1.0,
            rx_errors: 0.0,
            tx_errors: 0.0,
            rx_dropped: 0.0,
            tx_dropped: 0.0,
        }
    }

    #[test]
    fn rate_history_empty_average_returns_default() {
        let history = RateHistory::default();
        let avg = history.average();
        assert_eq!(avg.rx_bytes_per_sec, 0.0);
        assert_eq!(avg.tx_bytes_per_sec, 0.0);
    }

    #[test]
    fn rate_history_add_single_sample() {
        let mut history = RateHistory::default();
        let rates = history.add_sample(make_sample(100.0, 200.0));
        assert_eq!(rates.rx_bytes_per_sec, 100.0);
        assert_eq!(rates.tx_bytes_per_sec, 200.0);
    }

    #[test]
    fn rate_history_average_of_multiple_samples() {
        let mut history = RateHistory::default();
        history.add_sample(make_sample(100.0, 200.0));
        let rates = history.add_sample(make_sample(200.0, 400.0));
        // average of 100+200=300/2=150, 200+400=600/2=300
        assert_eq!(rates.rx_bytes_per_sec, 150.0);
        assert_eq!(rates.tx_bytes_per_sec, 300.0);
    }

    #[test]
    fn rate_history_window_capped_at_rate_window_size() {
        let mut history = RateHistory::default();
        // Add more than RATE_WINDOW_SIZE samples
        for i in 0..(RATE_WINDOW_SIZE + 3) {
            history.add_sample(make_sample(i as f64 * 10.0, 0.0));
        }
        // Only last RATE_WINDOW_SIZE samples should remain
        assert_eq!(history.samples.len(), RATE_WINDOW_SIZE);
    }

    #[test]
    fn rate_history_add_sample_returns_sliding_average() {
        let mut history = RateHistory::default();
        // Add RATE_WINDOW_SIZE identical samples
        for _ in 0..RATE_WINDOW_SIZE {
            history.add_sample(make_sample(50.0, 100.0));
        }
        let rates = history.average();
        assert_eq!(rates.rx_bytes_per_sec, 50.0);
        assert_eq!(rates.tx_bytes_per_sec, 100.0);
    }

    // -------------------------------------------------------------------------
    // NetworkMetricsCache static methods
    // -------------------------------------------------------------------------

    #[test]
    fn rate_from_delta_normal_case() {
        let result = NetworkMetricsCache::rate_from_delta(200, 100, 2.0);
        assert_eq!(result, Some(50.0));
    }

    #[test]
    fn rate_from_delta_counter_reset() {
        // current < previous → counter reset
        let result = NetworkMetricsCache::rate_from_delta(50, 200, 2.0);
        assert_eq!(result, None);
    }

    #[test]
    fn rate_from_delta_too_short_interval() {
        // elapsed < 0.1 → invalid
        let result = NetworkMetricsCache::rate_from_delta(200, 100, 0.05);
        assert_eq!(result, None);
    }

    #[test]
    fn rate_from_delta_exactly_zero_elapsed() {
        let result = NetworkMetricsCache::rate_from_delta(200, 100, 0.0);
        assert_eq!(result, None);
    }

    #[test]
    fn smooth_rate_no_previous_value() {
        // old_rate == 0.0 → returns new_rate directly
        let result = NetworkMetricsCache::smooth_rate(100.0, 0.0, 0.3);
        assert_eq!(result, 100.0);
    }

    #[test]
    fn smooth_rate_new_is_zero() {
        // new_rate == 0.0 → decay old_rate
        let result = NetworkMetricsCache::smooth_rate(0.0, 100.0, 0.3);
        assert!((result - 70.0).abs() < 1e-9);
    }

    #[test]
    fn smooth_rate_normal_ema() {
        // alpha * new + (1-alpha) * old
        let result = NetworkMetricsCache::smooth_rate(200.0, 100.0, 0.5);
        assert_eq!(result, 150.0);
    }

    // -------------------------------------------------------------------------
    // NetworkMetricsCache async methods
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn network_metrics_cache_get_returns_none_when_empty() {
        let cache = NetworkMetricsCache::new();
        let result = cache.get("test-ns").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn network_metrics_cache_add_sample_and_get() {
        let cache = NetworkMetricsCache::new();
        let metrics = NamespaceNetworkMetrics {
            namespace: "test-ns".to_string(),
            receive_bytes_total: 1000,
            transmit_bytes_total: 2000,
            receive_packets_total: 10,
            transmit_packets_total: 20,
            receive_errors_total: 0,
            transmit_errors_total: 0,
            receive_packets_dropped_total: 0,
            transmit_packets_dropped_total: 0,
            pod_count: 1,
        };
        let sample = RateSample {
            rx_bytes: 50.0,
            tx_bytes: 100.0,
            rx_packets: 1.0,
            tx_packets: 2.0,
            rx_errors: 0.0,
            tx_errors: 0.0,
            rx_dropped: 0.0,
            tx_dropped: 0.0,
        };
        let rates = cache.add_sample("test-ns", metrics, sample).await;
        assert_eq!(rates.rx_bytes_per_sec, 50.0);
        assert_eq!(rates.tx_bytes_per_sec, 100.0);

        let cached = cache.get("test-ns").await;
        assert!(cached.is_some());
    }

    // -------------------------------------------------------------------------
    // EndpointCache tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn endpoint_cache_get_returns_none_when_empty() {
        let cache = EndpointCache::new(60);
        let result = cache.get("myapp").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn endpoint_cache_set_and_get() {
        let cache = EndpointCache::new(60);
        cache
            .set(
                "myapp",
                "http://myapp.default.svc".to_string(),
                Some("/web".to_string()),
            )
            .await;
        let result = cache.get("myapp").await;
        assert!(result.is_some());
        let (url, path) = result.unwrap();
        assert_eq!(url, "http://myapp.default.svc");
        assert_eq!(path, Some("/web".to_string()));
    }

    #[tokio::test]
    async fn endpoint_cache_set_without_path() {
        let cache = EndpointCache::new(60);
        cache
            .set("otherapp", "http://otherapp.ns.svc".to_string(), None)
            .await;
        let result = cache.get("otherapp").await;
        assert!(result.is_some());
        let (url, path) = result.unwrap();
        assert_eq!(url, "http://otherapp.ns.svc");
        assert_eq!(path, None);
    }

    #[tokio::test]
    async fn endpoint_cache_invalidate() {
        let cache = EndpointCache::new(60);
        cache
            .set("myapp", "http://myapp.default.svc".to_string(), None)
            .await;
        assert!(cache.get("myapp").await.is_some());

        cache.invalidate("myapp").await;
        assert!(cache.get("myapp").await.is_none());
    }

    #[tokio::test]
    async fn endpoint_cache_invalidate_nonexistent_is_ok() {
        let cache = EndpointCache::new(60);
        // Should not panic
        cache.invalidate("nonexistent").await;
    }

    // -------------------------------------------------------------------------
    // AppState tests
    // -------------------------------------------------------------------------

    async fn make_app_state(db: Option<DbConn>) -> AppState {
        use crate::services::audit::AuditService;
        use crate::services::catalog::AppCatalog;
        use crate::services::chart_sync::ChartSyncService;
        use crate::services::notification::NotificationService;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let catalog = Arc::new(RwLock::new(AppCatalog::new()));
        let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
        let k8s_client = Arc::new(RwLock::new(None));

        AppState::new(
            db,
            k8s_client,
            catalog,
            chart_sync,
            AuditService::new(),
            NotificationService::new(),
        )
    }

    #[tokio::test]
    async fn appstate_get_db_returns_error_when_no_db() {
        let state = make_app_state(None).await;
        let result = state.get_db().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Database not connected") || err.to_string().contains("setup")
        );
    }

    #[tokio::test]
    async fn appstate_is_db_connected_returns_false_when_no_db() {
        let state = make_app_state(None).await;
        assert!(!state.is_db_connected().await);
    }

    #[tokio::test]
    async fn appstate_set_db_and_get_db() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory db");
        let state = make_app_state(None).await;
        state.set_db(db).await;
        assert!(state.is_db_connected().await);
        assert!(state.get_db().await.is_ok());
    }

    #[tokio::test]
    async fn appstate_with_db_at_construction() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory db");
        let state = make_app_state(Some(db)).await;
        assert!(state.is_db_connected().await);
        assert!(state.get_db().await.is_ok());
    }
}

/// Shared K8s client state
pub type SharedK8sClient = Arc<RwLock<Option<K8sClient>>>;

/// Shared app catalog state
pub type SharedCatalog = Arc<RwLock<AppCatalog>>;

/// Broadcast channel for real-time network metrics to WebSocket clients
pub type NetworkMetricsBroadcast = broadcast::Sender<String>;

/// Shared database connection (optional until PostgreSQL is installed)
pub type SharedDbConn = Arc<RwLock<Option<DbConn>>>;

/// Application state containing all shared resources
#[derive(Clone)]
pub struct AppState {
    pub db: SharedDbConn,
    pub k8s_client: SharedK8sClient,
    pub catalog: SharedCatalog,
    pub chart_sync: Arc<ChartSyncService>,
    pub audit: AuditService,
    pub notification: NotificationService,
    pub proxy: ProxyService,
    pub endpoint_cache: EndpointCache,
    pub network_metrics_cache: NetworkMetricsCache,
    pub network_metrics_tx: NetworkMetricsBroadcast,
}

impl AppState {
    pub fn new(
        db: Option<DbConn>,
        k8s_client: SharedK8sClient,
        catalog: SharedCatalog,
        chart_sync: Arc<ChartSyncService>,
        audit: AuditService,
        notification: NotificationService,
    ) -> Self {
        // Create broadcast channel for network metrics (capacity of 16 messages)
        let (network_metrics_tx, _) = broadcast::channel(16);

        Self {
            db: Arc::new(RwLock::new(db)),
            k8s_client,
            catalog,
            chart_sync,
            audit,
            notification,
            proxy: ProxyService::new(),
            endpoint_cache: EndpointCache::new(60), // Cache endpoints for 60 seconds
            network_metrics_cache: NetworkMetricsCache::new(),
            network_metrics_tx,
        }
    }

    /// Set the database connection after PostgreSQL is installed
    pub async fn set_db(&self, db: DbConn) {
        {
            let mut db_guard = self.db.write().await;
            *db_guard = Some(db.clone());
        }
        self.notification.set_db(db.clone()).await;
        self.audit.set_db(db).await;
    }

    /// Get the database connection (returns error if not connected)
    pub async fn get_db(&self) -> crate::error::Result<DbConn> {
        let db_guard = self.db.read().await;
        db_guard.clone().ok_or_else(|| {
            crate::error::AppError::ServiceUnavailable(
                "Database not connected. Please complete setup.".to_string(),
            )
        })
    }

    /// Check if database is connected
    pub async fn is_db_connected(&self) -> bool {
        let db_guard = self.db.read().await;
        db_guard.is_some()
    }
}
