//! Gateway app proxy authorization endpoints.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::middleware::Authenticated;
use crate::models::prelude::*;
use crate::models::{app_domain_assignment, domain};
use crate::state::AppState;

/// Create lightweight gateway authorization routes.
pub fn proxy_auth_routes(state: AppState) -> Router {
    Router::new()
        .route("/route", get(resolve_gateway_route))
        .route("/{app_name}", get(authorize_gateway_app_proxy))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct RouteLookupQuery {
    host: String,
    path: String,
}

async fn resolve_gateway_route(
    State(state): State<AppState>,
    Query(query): Query<RouteLookupQuery>,
    auth: Authenticated,
) -> Result<impl IntoResponse> {
    let resolved = resolve_app_assignment(&state, &query.host, &query.path).await?;
    let Some(resolved) = resolved else {
        return Err(AppError::NotFound(
            "No app route assignment matched request".to_string(),
        ));
    };

    // Authentication already resolved this user's permissions; the gateway
    // asks for a route on every proxied request, so do not fetch them again.
    if !auth.allows_app(&resolved.app_name) {
        return Err(AppError::Forbidden(format!(
            "No access to app: {}",
            resolved.app_name
        )));
    }

    let (upstream, base_path, landing_path) =
        get_app_upstream_url(&state, &resolved.app_name).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    insert_header(&mut response, "x-kubarr-app-name", &resolved.app_name)?;
    insert_header(&mut response, "x-kubarr-route-mode", &resolved.route_mode)?;
    insert_header(&mut response, "x-kubarr-target-path", &resolved.target_path)?;
    insert_header(&mut response, "x-kubarr-upstream", &upstream)?;
    if let Some(base_path) = base_path {
        insert_header(&mut response, "x-kubarr-base-path", &base_path)?;
    }
    if let Some(landing_path) = landing_path {
        insert_header(&mut response, "x-kubarr-landing-path", &landing_path)?;
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    Ok(response)
}

async fn authorize_gateway_app_proxy(
    State(state): State<AppState>,
    Path(app_name): Path<String>,
    auth: Authenticated,
) -> Result<impl IntoResponse> {
    if !auth.allows_app(&app_name) {
        return Err(AppError::Forbidden(format!(
            "No access to app: {}",
            app_name
        )));
    }

    let (upstream, base_path, landing_path) = get_app_upstream_url(&state, &app_name).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    insert_header(&mut response, "x-kubarr-upstream", &upstream)?;
    if let Some(base_path) = base_path {
        insert_header(&mut response, "x-kubarr-base-path", &base_path)?;
    }
    if let Some(landing_path) = landing_path {
        insert_header(&mut response, "x-kubarr-landing-path", &landing_path)?;
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    Ok(response)
}

fn insert_header(
    response: &mut axum::response::Response,
    name: &'static str,
    value: &str,
) -> Result<()> {
    let value = HeaderValue::from_str(value)
        .map_err(|e| AppError::Internal(format!("Invalid {} header: {}", name, e)))?;
    response.headers_mut().insert(name, value);
    Ok(())
}

#[derive(Debug)]
struct ResolvedRoute {
    app_name: String,
    route_mode: String,
    target_path: String,
}

async fn resolve_app_assignment(
    state: &AppState,
    host: &str,
    path: &str,
) -> Result<Option<ResolvedRoute>> {
    let db = state.get_db().await?;
    let host = normalize_host(host);
    let path = normalize_request_path(path);

    let assignments = AppDomainAssignment::find()
        .filter(app_domain_assignment::Column::Enabled.eq(true))
        .all(&db)
        .await?;

    for assignment in assignments {
        let Some(domain) = Domain::find_by_id(assignment.domain_id).one(&db).await? else {
            continue;
        };
        if !domain.enabled {
            continue;
        }

        match assignment.route_mode.as_str() {
            "exact_host" | "subdomain" => {
                let expected = assignment_host(&assignment, &domain);
                if expected.as_deref() == Some(host.as_str()) {
                    return Ok(Some(ResolvedRoute {
                        app_name: assignment.app_name,
                        route_mode: assignment.route_mode,
                        target_path: path,
                    }));
                }
            }
            "path" => {
                let Some(prefix) = assignment.path_prefix.as_deref() else {
                    continue;
                };
                let prefix = normalize_request_path(prefix);
                if host_matches_domain(&host, &domain.domain) && path_matches_prefix(&path, &prefix)
                {
                    let target_path = strip_path_prefix(&path, &prefix);
                    return Ok(Some(ResolvedRoute {
                        app_name: assignment.app_name,
                        route_mode: assignment.route_mode,
                        target_path,
                    }));
                }
            }
            _ => {}
        }
    }

    Ok(None)
}

fn assignment_host(
    assignment: &app_domain_assignment::Model,
    domain: &domain::Model,
) -> Option<String> {
    let host = assignment.hostname.as_deref()?.trim().to_lowercase();
    if host.is_empty() {
        return None;
    }
    if assignment.route_mode == "exact_host" || host.contains('.') {
        return Some(host);
    }
    Some(format!(
        "{}.{}",
        host,
        domain.domain.trim_start_matches("*.")
    ))
}

fn host_matches_domain(host: &str, domain: &str) -> bool {
    let domain = domain.trim_start_matches("*.");
    host == domain || host.ends_with(&format!(".{}", domain))
}

fn normalize_host(host: &str) -> String {
    host.split(':').next().unwrap_or(host).trim().to_lowercase()
}

fn normalize_request_path(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path).trim();
    if path.is_empty() {
        "/".to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{}/", prefix.trim_end_matches('/')))
}

fn strip_path_prefix(path: &str, prefix: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    let stripped = path.strip_prefix(prefix).unwrap_or(path);
    if stripped.is_empty() {
        "/".to_string()
    } else if stripped.starts_with('/') {
        stripped.to_string()
    } else {
        format!("/{}", stripped)
    }
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
    use crate::services::{
        audit::AuditService, catalog::AppCatalog, chart_sync::ChartSyncService,
        notification::NotificationService,
    };
    use crate::state::{SharedCatalog, SharedK8sClient};
    use sea_orm::Database;
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
