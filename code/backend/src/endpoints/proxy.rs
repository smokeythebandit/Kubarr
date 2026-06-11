//! App proxy endpoints
//!
//! Reverse proxy requests to installed apps under /proxy/{app_name}/*

use axum::{
    extract::ws::WebSocketUpgrade,
    extract::{FromRequestParts, Path, Request, State},
    response::Response,
    routing::any,
    Router,
};

use crate::error::{AppError, Result};
use crate::middleware::Authenticated;
use crate::services::{is_websocket_upgrade, proxy_websocket};
use crate::state::AppState;

/// Create proxy routes
pub fn proxy_routes(state: AppState) -> Router {
    Router::new()
        .route("/{app_name}", any(proxy_app_root))
        .route("/{app_name}/", any(proxy_app_root))
        .route("/{app_name}/{*path}", any(proxy_app_with_path))
        .with_state(state)
}

/// Proxy requests to an installed app (root path)
async fn proxy_app_root(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authenticated,
    request: Request,
) -> Result<Response> {
    proxy_app_request(state, app_name, String::new(), auth, request).await
}

/// Proxy requests to an installed app (with path)
async fn proxy_app_with_path(
    State(state): State<AppState>,
    Path((app_name, path)): Path<(String, String)>,
    auth: Authenticated,
    request: Request,
) -> Result<Response> {
    proxy_app_request(state, app_name, path, auth, request).await
}

/// Inner proxy implementation
async fn proxy_app_request(
    state: AppState,
    app_name: String,
    path: String,
    auth: Authenticated,
    request: Request,
) -> Result<Response> {
    let user = auth.user();

    // Check if user has access to this app
    if !check_app_permission(&state, user.id, &app_name).await {
        return Err(AppError::Forbidden(format!(
            "No access to app: {}",
            app_name
        )));
    }

    // Get the app's service endpoint from Kubernetes
    let target_url = get_app_target_url(&state, &app_name, &path).await?;

    let method = request.method().clone();
    let headers = request.headers().clone();

    // Handle WebSocket upgrade
    if is_websocket_upgrade(&headers) {
        let (mut parts, body) = request.into_parts();
        if let Ok(ws) = WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
            return Ok(proxy_websocket(ws, target_url).await);
        }
        // If WebSocket extraction failed, reconstruct and fall through to HTTP proxy
        let request = Request::from_parts(parts, body);
        let body = request.into_body();
        return state
            .proxy
            .proxy_http(&target_url, method, headers, body)
            .await;
    }

    // Proxy HTTP request
    let body = request.into_body();
    state
        .proxy
        .proxy_http(&target_url, method, headers, body)
        .await
}

/// Check if user has permission to access the app
async fn check_app_permission(state: &AppState, user_id: i64, app_name: &str) -> bool {
    use crate::endpoints::extractors::get_user_permissions;

    let db = match state.get_db().await {
        Ok(db) => db,
        Err(_) => return false,
    };
    let permissions = get_user_permissions(&db, user_id).await;

    // Check for app.* wildcard or specific app.{name} permission
    permissions.contains(&"app.*".to_string()) || permissions.contains(&format!("app.{}", app_name))
}

/// Get the target URL for an app
async fn get_app_target_url(state: &AppState, app_name: &str, path: &str) -> Result<String> {
    // Check cache first
    let (base_url, _base_path) = if let Some(cached) = state.endpoint_cache.get(app_name).await {
        cached
    } else {
        // Get K8s client
        let k8s_guard = state.k8s_client.read().await;
        let k8s = k8s_guard
            .as_ref()
            .ok_or_else(|| AppError::ServiceUnavailable("Kubernetes not available".to_string()))?;

        // Get service endpoints for the app
        // Apps are deployed in namespaces named after the app
        let endpoints = k8s.get_service_endpoints(app_name, app_name).await?;

        if endpoints.is_empty() {
            return Err(AppError::NotFound(format!(
                "App {} not found or not ready",
                app_name
            )));
        }

        // Use the first endpoint
        let endpoint = &endpoints[0];

        // Build the internal URL
        // Format: http://{service_name}.{namespace}.svc.cluster.local:{port}/{path}
        let base_url = format!(
            "http://{}.{}.svc.cluster.local:{}",
            endpoint.name, endpoint.namespace, endpoint.port
        );

        let base_path = endpoint.base_path.clone();

        // Cache the endpoint
        state
            .endpoint_cache
            .set(app_name, base_url.clone(), base_path.clone())
            .await;

        (base_url, base_path)
    };

    // Append path
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        Ok(base_url)
    } else {
        Ok(format!("{}/{}", base_url, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{role, role_permission, user, user_role};
    use crate::services::{
        audit::AuditService, catalog::AppCatalog, chart_sync::ChartSyncService,
        notification::NotificationService,
    };
    use crate::state::{SharedCatalog, SharedK8sClient};
    use sea_orm::{ActiveModelTrait, Database, Set};
    use sea_orm_migration::MigratorTrait;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    async fn make_test_db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        crate::migrations::Migrator::up(&db, None)
            .await
            .expect("migrate");
        db
    }

    async fn make_test_state(db: sea_orm::DatabaseConnection) -> AppState {
        let k8s: SharedK8sClient = Arc::new(RwLock::new(None));
        let catalog: SharedCatalog = Arc::new(RwLock::new(AppCatalog::default()));
        let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
        AppState::new(
            Some(db),
            k8s,
            catalog,
            chart_sync,
            AuditService::new(),
            NotificationService::new(),
        )
    }

    // ── check_app_permission ─────────────────────────────────────────────

    #[tokio::test]
    async fn check_app_permission_no_db_returns_false() {
        // State with no DB → get_db() fails → returns false
        let k8s: SharedK8sClient = Arc::new(RwLock::new(None));
        let catalog: SharedCatalog = Arc::new(RwLock::new(AppCatalog::default()));
        let chart_sync = Arc::new(ChartSyncService::new(catalog.clone()));
        let state = AppState::new(
            None,
            k8s,
            catalog,
            chart_sync,
            AuditService::new(),
            NotificationService::new(),
        );
        let result = check_app_permission(&state, 1, "myapp").await;
        assert!(!result);
    }

    #[tokio::test]
    async fn check_app_permission_no_permissions_returns_false() {
        let db = make_test_db().await;
        // Insert a user with no app permissions
        let now = chrono::Utc::now();
        let user_model = user::ActiveModel {
            username: Set("noperm".to_string()),
            email: Set("noperm@test.com".to_string()),
            hashed_password: Set("hash".to_string()),
            is_active: Set(true),
            is_approved: Set(true),
            totp_secret: Set(None),
            totp_enabled: Set(false),
            totp_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let inserted = user_model.insert(&db).await.unwrap();
        let state = make_test_state(db).await;
        let result = check_app_permission(&state, inserted.id, "myapp").await;
        assert!(!result);
    }

    #[tokio::test]
    async fn check_app_permission_wildcard_returns_true() {
        let db = make_test_db().await;
        let now = chrono::Utc::now();

        // Create user
        let user_model = user::ActiveModel {
            username: Set("wildcard_user".to_string()),
            email: Set("wc@test.com".to_string()),
            hashed_password: Set("hash".to_string()),
            is_active: Set(true),
            is_approved: Set(true),
            totp_secret: Set(None),
            totp_enabled: Set(false),
            totp_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let u = user_model.insert(&db).await.unwrap();

        // Create role with app.* permission
        let role_model = role::ActiveModel {
            name: Set("app_admin".to_string()),
            description: Set(None),
            is_system: Set(false),
            requires_2fa: Set(false),
            created_at: Set(now),
            ..Default::default()
        };
        let r = role_model.insert(&db).await.unwrap();

        let perm = role_permission::ActiveModel {
            role_id: Set(r.id),
            permission: Set("app.*".to_string()),
            ..Default::default()
        };
        perm.insert(&db).await.unwrap();

        let ur = user_role::ActiveModel {
            user_id: Set(u.id),
            role_id: Set(r.id),
        };
        ur.insert(&db).await.unwrap();

        let state = make_test_state(db).await;
        assert!(check_app_permission(&state, u.id, "jellyfin").await);
        assert!(check_app_permission(&state, u.id, "qbittorrent").await);
    }

    #[tokio::test]
    async fn check_app_permission_specific_app_returns_true_for_that_app() {
        let db = make_test_db().await;
        let now = chrono::Utc::now();

        let user_model = user::ActiveModel {
            username: Set("specific_user".to_string()),
            email: Set("sp@test.com".to_string()),
            hashed_password: Set("hash".to_string()),
            is_active: Set(true),
            is_approved: Set(true),
            totp_secret: Set(None),
            totp_enabled: Set(false),
            totp_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let u = user_model.insert(&db).await.unwrap();

        let role_model = role::ActiveModel {
            name: Set("jellyfin_role".to_string()),
            description: Set(None),
            is_system: Set(false),
            requires_2fa: Set(false),
            created_at: Set(now),
            ..Default::default()
        };
        let r = role_model.insert(&db).await.unwrap();

        let perm = role_permission::ActiveModel {
            role_id: Set(r.id),
            permission: Set("app.jellyfin".to_string()),
            ..Default::default()
        };
        perm.insert(&db).await.unwrap();

        let ur = user_role::ActiveModel {
            user_id: Set(u.id),
            role_id: Set(r.id),
        };
        ur.insert(&db).await.unwrap();

        let state = make_test_state(db).await;
        assert!(check_app_permission(&state, u.id, "jellyfin").await);
        assert!(!check_app_permission(&state, u.id, "qbittorrent").await);
    }

    // ── get_app_target_url ───────────────────────────────────────────────

    #[tokio::test]
    async fn get_app_target_url_returns_cached_url() {
        let db = make_test_db().await;
        let state = make_test_state(db).await;

        // Pre-seed the cache
        state
            .endpoint_cache
            .set(
                "myapp",
                "http://myapp.ns.svc.cluster.local:8080".to_string(),
                Some(String::new()),
            )
            .await;

        let url = get_app_target_url(&state, "myapp", "").await.unwrap();
        assert_eq!(url, "http://myapp.ns.svc.cluster.local:8080");
    }

    #[tokio::test]
    async fn get_app_target_url_appends_path() {
        let db = make_test_db().await;
        let state = make_test_state(db).await;

        state
            .endpoint_cache
            .set(
                "myapp",
                "http://myapp.ns.svc.cluster.local:8080".to_string(),
                Some(String::new()),
            )
            .await;

        let url = get_app_target_url(&state, "myapp", "sub/page")
            .await
            .unwrap();
        assert_eq!(url, "http://myapp.ns.svc.cluster.local:8080/sub/page");
    }

    #[tokio::test]
    async fn get_app_target_url_strips_leading_slash_from_path() {
        let db = make_test_db().await;
        let state = make_test_state(db).await;

        state
            .endpoint_cache
            .set(
                "myapp",
                "http://myapp.ns.svc.cluster.local:8080".to_string(),
                Some(String::new()),
            )
            .await;

        let url = get_app_target_url(&state, "myapp", "/sub/page")
            .await
            .unwrap();
        assert_eq!(url, "http://myapp.ns.svc.cluster.local:8080/sub/page");
    }

    #[tokio::test]
    async fn get_app_target_url_no_cache_no_k8s_returns_error() {
        let db = make_test_db().await;
        let state = make_test_state(db).await;
        // No cache, K8s is None → ServiceUnavailable
        let err = get_app_target_url(&state, "unknownapp", "")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ServiceUnavailable(_)));
    }

    // ── proxy_routes registration ────────────────────────────────────────

    #[tokio::test]
    async fn proxy_routes_creates_router_without_panic() {
        let db = make_test_db().await;
        let state = make_test_state(db).await;
        // Just verify proxy_routes() can be called without panicking
        let _router = proxy_routes(state);
    }
}
