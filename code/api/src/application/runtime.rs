//! Application runtime
//!
//! Handles initialization for the Kubarr backend.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use tokio::sync::RwLock;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::CONFIG;
use crate::db;
use crate::endpoints;
use crate::services::{
    init_jwt_keys, scheduler, start_network_broadcaster, AppCatalog, AppManager, AuditService,
    ChartSyncService, DomainReconciler, K8sClient, NotificationService,
};
use crate::state::AppState;

/// Initialize and run the application.
pub async fn run() -> anyhow::Result<()> {
    init_tracing();

    tracing::info!("Starting Kubarr backend v{}", env!("CARGO_PKG_VERSION"));

    let state = init_services().await?;

    // Start background network metrics broadcaster
    start_network_broadcaster(state.clone());
    tracing::info!("Network metrics broadcaster started");

    let app = create_app(state);

    serve(app).await
}

/// Initialize and run the app lifecycle worker process.
pub async fn run_worker() -> anyhow::Result<()> {
    init_tracing();

    tracing::info!("Starting Kubarr worker v{}", env!("CARGO_PKG_VERSION"));

    let k8s_client = init_kubernetes().await;
    let catalog = init_catalog();
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    if let Err(e) = chart_sync.sync().await {
        tracing::warn!("Initial worker chart sync failed: {}", e);
    }
    tokio::spawn({
        let chart_sync = chart_sync.clone();
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(
                    CONFIG.charts.sync_interval,
                ))
                .await;
                if let Err(e) = chart_sync.sync().await {
                    tracing::warn!("Periodic worker chart sync failed: {}", e);
                }
            }
        }
    });
    let conn = init_database(&k8s_client).await?;
    let domain_reconciler = Arc::new(DomainReconciler::new(conn.clone(), k8s_client.clone()));
    let manager = Arc::new(AppManager::new(conn, k8s_client, catalog));

    let poll_interval = env_duration("KUBARR_WORKER_POLL_INTERVAL_SECONDS", 5);
    let reconcile_interval = env_duration("KUBARR_WORKER_RECONCILE_INTERVAL_SECONDS", 30);
    let domain_reconcile_interval = env_duration("KUBARR_DOMAIN_RECONCILE_INTERVAL_SECONDS", 60);
    manager.run_worker(poll_interval, reconcile_interval).await;
    domain_reconciler.run(domain_reconcile_interval);

    tracing::info!(
        poll_interval_seconds = poll_interval.as_secs(),
        reconcile_interval_seconds = reconcile_interval.as_secs(),
        domain_reconcile_interval_seconds = domain_reconcile_interval.as_secs(),
        "Kubarr worker started"
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("Kubarr worker shutting down");
    Ok(())
}

fn env_duration(name: &str, default_seconds: u64) -> std::time::Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(default_seconds))
}

/// Initialize tracing/logging
fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("kubarr={}", CONFIG.log_level).into()),
        )
        .with(tracing_subscriber::fmt::layer().with_ansi(false))
        .init();
}

/// Initialize all application services
async fn init_services() -> anyhow::Result<AppState> {
    let k8s_client = init_kubernetes().await;
    let catalog = init_catalog();

    // Create chart sync service. Run the initial sync in the background so
    // slow registry/network calls cannot block the health endpoint on startup.
    let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
    tokio::spawn({
        let chart_sync = chart_sync.clone();
        async move {
            if let Err(e) = chart_sync.sync_on_blocking_thread().await {
                tracing::warn!("Initial chart sync failed: {}", e);
            }
        }
    });

    let conn = init_database(&k8s_client).await?;

    let audit = AuditService::new();
    let notification = NotificationService::new();

    audit.set_db(conn.clone()).await;
    notification.set_db(conn.clone()).await;
    if let Err(e) = notification.init_providers().await {
        tracing::warn!("Failed to initialize notification providers: {}", e);
    }

    if let Err(e) = init_jwt_keys(&conn).await {
        tracing::warn!("Failed to initialize JWT keys: {}", e);
    } else {
        tracing::info!("JWT signing keys initialized");
    }

    scheduler::start_scheduler(Arc::new(conn.clone()), chart_sync.clone());

    Ok(AppState::new(
        Some(conn),
        k8s_client,
        catalog,
        chart_sync,
        audit,
        notification,
    ))
}

/// Initialize the database connection (runs migrations automatically).
async fn init_database(
    k8s_client: &Arc<RwLock<Option<K8sClient>>>,
) -> anyhow::Result<sea_orm::DatabaseConnection> {
    // Prefer database credentials from the in-cluster secret when available.
    let database_url = if CONFIG.kubernetes.in_cluster {
        let k8s_guard = k8s_client.read().await;
        if let Some(ref k8s) = *k8s_guard {
            match k8s.get_database_url("kubarr-database").await {
                Ok(url) => Some(url),
                Err(_) => {
                    tracing::info!("No database secret found, falling back to configured URL");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    if let Some(url) = database_url {
        tracing::info!("Found database credentials in K8s secret, connecting...");
        let conn = db::connect_with_url(&url).await?;
        tracing::info!("Database connection established from K8s secret");
        return Ok(conn);
    }

    let conn = db::connect().await?;
    tracing::info!("Database connection established");
    Ok(conn)
}

/// Initialize the Kubernetes client
async fn init_kubernetes() -> Arc<RwLock<Option<K8sClient>>> {
    match K8sClient::new().await {
        Ok(client) => {
            tracing::info!("Kubernetes client initialized");
            Arc::new(RwLock::new(Some(client)))
        }
        Err(e) => {
            tracing::warn!(
                "Failed to initialize Kubernetes client: {}. Some features will be unavailable.",
                e
            );
            Arc::new(RwLock::new(None))
        }
    }
}

/// Initialize the app catalog
fn init_catalog() -> Arc<RwLock<AppCatalog>> {
    let catalog = Arc::new(RwLock::new(AppCatalog::new()));
    tracing::info!("App catalog loaded");
    catalog
}

/// Create the main application router
fn create_app(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    endpoints::create_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// init_kubernetes() must return an Arc without panicking, even when no
    /// kubeconfig is present — it logs a warning and returns None.
    #[tokio::test]
    async fn init_kubernetes_returns_arc_even_without_cluster() {
        let result = init_kubernetes().await;
        // We cannot assert Some/None because a valid kubeconfig may or may not
        // be present in the test environment; we just need it not to panic.
        let _ = result.read().await;
    }

    /// init_catalog() must return a non-empty catalog wrapped in Arc<RwLock>.
    #[test]
    fn init_catalog_returns_catalog() {
        let catalog = init_catalog();
        // Just verify the Arc is usable; AppCatalog::new() is already tested elsewhere
        let _ = catalog.try_read();
    }

    /// create_app() must return a Router without panicking.
    #[tokio::test]
    async fn create_app_returns_router() {
        use crate::services::{
            audit::AuditService, catalog::AppCatalog, chart_sync::ChartSyncService,
            notification::NotificationService,
        };
        use crate::state::{AppState, SharedK8sClient};
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let k8s: SharedK8sClient = Arc::new(RwLock::new(None));
        let catalog = Arc::new(RwLock::new(AppCatalog::default()));
        let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
        let audit = AuditService::new();
        let notification = NotificationService::new();
        let state = AppState::new(None, k8s, catalog, chart_sync, audit, notification);

        // Should not panic
        let _router = create_app(state);
    }
}

/// Start the HTTP server
async fn serve(app: Router) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], CONFIG.server.port));
    tracing::info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
