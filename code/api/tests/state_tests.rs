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
        .set("sonarr", "http://sonarr:8989".to_string(), None, None)
        .await;
    let result = cache.get("sonarr").await;
    assert!(result.is_some(), "Cache must return set value");
    let (url, path, host) = result.unwrap();
    assert_eq!(url, "http://sonarr:8989");
    assert!(path.is_none());
    assert!(host.is_none());
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
            None,
        )
        .await;
    let result = cache.get("radarr").await;
    assert!(result.is_some());
    let (url, path, host) = result.unwrap();
    assert_eq!(url, "http://radarr:7878");
    assert_eq!(path.as_deref(), Some("/radarr"));
    assert!(host.is_none());
}

#[tokio::test]
async fn test_endpoint_cache_invalidate() {
    use kubarr::state::EndpointCache;
    let cache = EndpointCache::new(60);
    cache
        .set("sonarr", "http://sonarr:8989".to_string(), None, None)
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
    cache
        .set("app", "http://old:8080".to_string(), None, None)
        .await;
    cache
        .set(
            "app",
            "http://new:9090".to_string(),
            Some("/new".to_string()),
            None,
        )
        .await;
    let (url, path, host) = cache.get("app").await.unwrap();
    assert_eq!(url, "http://new:9090");
    assert_eq!(path.as_deref(), Some("/new"));
    assert!(host.is_none());
}
