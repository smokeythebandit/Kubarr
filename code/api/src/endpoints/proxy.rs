//! Gateway app proxy authorization endpoints.

use axum::{
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};

use crate::error::{AppError, Result};
use crate::middleware::Authenticated;
use crate::state::AppState;

/// Create lightweight gateway authorization routes.
pub fn proxy_auth_routes(state: AppState) -> Router {
    Router::new()
        .route("/{app_name}", get(authorize_gateway_app_proxy))
        .with_state(state)
}

async fn authorize_gateway_app_proxy(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authenticated,
) -> Result<impl IntoResponse> {
    let user = auth.user();
    if !check_app_permission(&state, user.id, &app_name).await {
        return Err(AppError::Forbidden(format!(
            "No access to app: {}",
            app_name
        )));
    }

    let (upstream, base_path, landing_path) = get_app_upstream_url(&state, &app_name).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    let upstream = HeaderValue::from_str(&upstream)
        .map_err(|e| AppError::Internal(format!("Invalid upstream header: {}", e)))?;
    response.headers_mut().insert("x-kubarr-upstream", upstream);
    if let Some(base_path) = base_path {
        let base_path = HeaderValue::from_str(&base_path)
            .map_err(|e| AppError::Internal(format!("Invalid base path header: {}", e)))?;
        response
            .headers_mut()
            .insert("x-kubarr-base-path", base_path);
    }
    if let Some(landing_path) = landing_path {
        let landing_path = HeaderValue::from_str(&landing_path)
            .map_err(|e| AppError::Internal(format!("Invalid landing path header: {}", e)))?;
        response
            .headers_mut()
            .insert("x-kubarr-landing-path", landing_path);
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    Ok(response)
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

/// Get the upstream base URL for an app. The gateway appends the request path.
async fn get_app_upstream_url(
    state: &AppState,
    app_name: &str,
) -> Result<(String, Option<String>, Option<String>)> {
    // Check cache first
    let (base_url, base_path, landing_path) =
        if let Some(cached) = state.endpoint_cache.get(app_name).await {
            cached
        } else {
            // Get K8s client
            let k8s_guard = state.k8s_client.read().await;
            let k8s = k8s_guard.as_ref().ok_or_else(|| {
                AppError::ServiceUnavailable("Kubernetes not available".to_string())
            })?;

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
            let landing_path = endpoint.landing_path.clone();

            // Cache the endpoint
            state
                .endpoint_cache
                .set(
                    app_name,
                    base_url.clone(),
                    base_path.clone(),
                    landing_path.clone(),
                )
                .await;

            (base_url, base_path, landing_path)
        };

    Ok((base_url, base_path, landing_path))
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

    // ── get_app_upstream_url ─────────────────────────────────────────────

    #[tokio::test]
    async fn get_app_upstream_url_returns_cached_url() {
        let db = make_test_db().await;
        let state = make_test_state(db).await;

        // Pre-seed the cache
        state
            .endpoint_cache
            .set(
                "myapp",
                "http://myapp.ns.svc.cluster.local:8080".to_string(),
                Some(String::new()),
                Some("/web/".to_string()),
            )
            .await;

        let url = get_app_upstream_url(&state, "myapp").await.unwrap();
        assert_eq!(
            url,
            (
                "http://myapp.ns.svc.cluster.local:8080".to_string(),
                Some(String::new()),
                Some("/web/".to_string())
            )
        );
    }

    #[tokio::test]
    async fn get_app_upstream_url_no_cache_no_k8s_returns_error() {
        let db = make_test_db().await;
        let state = make_test_state(db).await;
        // No cache, K8s is None → ServiceUnavailable
        let err = get_app_upstream_url(&state, "unknownapp")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ServiceUnavailable(_)));
    }
}
