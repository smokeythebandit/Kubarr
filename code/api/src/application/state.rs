use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use sea_orm::DatabaseConnection;

use crate::services::audit::AuditService;
use crate::services::catalog::AppCatalog;
use crate::services::chart_sync::ChartSyncService;
use crate::services::k8s::K8sClient;
use crate::services::notification::NotificationService;

/// Database connection type alias
pub type DbConn = DatabaseConnection;

/// Cached app endpoint with expiration
#[derive(Clone)]
pub struct CachedEndpoint {
    pub base_url: String,
    pub base_path: Option<String>,
    pub landing_path: Option<String>,
    pub expires_at: Instant,
}

/// Cache for app service endpoints (avoids K8s API calls on every request)
#[derive(Clone, Default)]
pub struct EndpointCache {
    cache: Arc<RwLock<HashMap<String, CachedEndpoint>>>,
    ttl: Duration,
}

impl EndpointCache {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            ttl: Duration::from_secs(ttl_seconds),
        }
    }

    /// Get cached endpoint for an app (base_url, base_path, landing_path)
    pub async fn get(&self, app_name: &str) -> Option<(String, Option<String>, Option<String>)> {
        let cache = self.cache.read().await;
        if let Some(entry) = cache.get(app_name) {
            if entry.expires_at > Instant::now() {
                return Some((
                    entry.base_url.clone(),
                    entry.base_path.clone(),
                    entry.landing_path.clone(),
                ));
            }
        }
        None
    }

    /// Cache an endpoint for an app
    pub async fn set(
        &self,
        app_name: &str,
        base_url: String,
        base_path: Option<String>,
        landing_path: Option<String>,
    ) {
        let mut cache = self.cache.write().await;
        cache.insert(
            app_name.to_string(),
            CachedEndpoint {
                base_url,
                base_path,
                landing_path,
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
                Some("/web/".to_string()),
            )
            .await;
        let result = cache.get("myapp").await;
        assert!(result.is_some());
        let (url, path, landing_path) = result.unwrap();
        assert_eq!(url, "http://myapp.default.svc");
        assert_eq!(path, Some("/web".to_string()));
        assert_eq!(landing_path, Some("/web/".to_string()));
    }

    #[tokio::test]
    async fn endpoint_cache_set_without_path() {
        let cache = EndpointCache::new(60);
        cache
            .set("otherapp", "http://otherapp.ns.svc".to_string(), None, None)
            .await;
        let result = cache.get("otherapp").await;
        assert!(result.is_some());
        let (url, path, landing_path) = result.unwrap();
        assert_eq!(url, "http://otherapp.ns.svc");
        assert_eq!(path, None);
        assert_eq!(landing_path, None);
    }

    #[tokio::test]
    async fn endpoint_cache_invalidate() {
        let cache = EndpointCache::new(60);
        cache
            .set("myapp", "http://myapp.default.svc".to_string(), None, None)
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
    pub endpoint_cache: EndpointCache,
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
        Self {
            db: Arc::new(RwLock::new(db)),
            k8s_client,
            catalog,
            chart_sync,
            audit,
            notification,
            endpoint_cache: EndpointCache::new(60), // Cache endpoints for 60 seconds
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
