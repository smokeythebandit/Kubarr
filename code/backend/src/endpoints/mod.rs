pub mod apps;
pub mod audit;
pub mod auth;
pub mod cloudflare;
pub mod extractors;
pub mod frontend;
pub mod logs;
pub mod monitoring;
pub mod networking;
pub mod notifications;
pub mod oauth;
pub mod proxy;
pub mod roles;
pub mod settings;
pub mod storage;
pub mod users;
pub mod vpn;

use axum::{extract::State, middleware as axum_middleware, response::Html, Router};
use utoipa::OpenApi;

use crate::config::CONFIG;
use crate::middleware::require_auth;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Kubarr API",
        description = "Kubernetes application management dashboard API",
        version = "0.1.0"
    ),
    paths(
        // Health
        health_check,
        health_check_detailed,
        get_version,
        // Auth
        auth::login,
        auth::logout,
        auth::list_sessions,
        auth::revoke_session,
        auth::switch_session,
        auth::list_accounts,
        // Users
        users::list_users,
        users::get_current_user_info,
        users::update_own_profile,
        users::delete_own_account,
        users::get_my_preferences,
        users::update_my_preferences,
        users::change_own_password,
        users::setup_2fa,
        users::enable_2fa,
        users::disable_2fa,
        users::get_2fa_status,
        users::list_pending_users,
        users::list_invites,
        users::create_invite,
        users::delete_invite,
        users::get_user,
        users::update_user,
        users::delete_user,
        users::approve_user,
        users::reject_user,
        users::admin_reset_password,
        // Roles
        roles::list_roles,
        roles::create_role,
        roles::list_all_permissions,
        roles::get_role,
        roles::update_role,
        roles::delete_role,
        roles::set_role_apps,
        roles::get_role_permissions,
        roles::set_role_permissions,
        // Apps
        apps::list_catalog,
        apps::get_app_from_catalog,
        apps::get_app_icon,
        apps::list_installed_apps,
        apps::install_app,
        apps::delete_app,
        apps::restart_app,
        apps::list_categories,
        apps::get_apps_by_category,
        apps::check_app_health,
        apps::check_app_exists,
        apps::get_app_status,
        apps::sync_charts,
        apps::log_app_access,
        // Monitoring
        monitoring::get_app_metrics,
        monitoring::get_cluster_metrics,
        monitoring::get_app_detail_metrics,
        monitoring::get_cluster_network_history,
        monitoring::get_cluster_metrics_history,
        monitoring::check_vm_available,
        monitoring::get_pods,
        monitoring::get_metrics,
        monitoring::get_app_health,
        monitoring::get_endpoints,
        monitoring::check_metrics_available,
        // Networking
        networking::get_network_topology,
        networking::get_network_stats,
        // Logs
        logs::get_pod_logs,
        logs::get_app_logs,
        logs::get_raw_pod_logs,
        logs::get_vlogs_namespaces,
        logs::get_vlogs_labels,
        logs::get_vlogs_label_values,
        logs::query_vlogs,
        // Audit
        audit::list_audit_logs,
        audit::audit_stats,
        audit::clear_audit_logs,
        // Notifications
        notifications::get_inbox,
        notifications::get_unread_count,
        notifications::mark_as_read,
        notifications::mark_all_as_read,
        notifications::delete_notification,
        notifications::list_channels,
        notifications::get_channel,
        notifications::update_channel,
        notifications::test_channel,
        notifications::list_events,
        notifications::update_event,
        notifications::get_preferences,
        notifications::update_preference,
        notifications::list_logs,
        // Storage
        storage::browse_directory,
        storage::get_storage_stats,
        storage::get_file_info,
        storage::create_directory,
        storage::delete_path,
        storage::download_file,
        // Settings
        settings::list_settings,
        settings::get_setting,
        settings::update_setting,
        // OAuth
        oauth::list_available_providers,
        oauth::list_providers,
        oauth::get_provider,
        oauth::update_provider,
        oauth::oauth_login,
        oauth::oauth_callback,
        oauth::list_linked_accounts,
        oauth::unlink_account,
        oauth::link_account_start,
        // VPN
        vpn::list_providers,
        vpn::get_provider,
        vpn::create_provider,
        vpn::update_provider,
        vpn::delete_provider,
        vpn::test_provider,
        vpn::list_app_configs,
        vpn::get_app_config,
        vpn::assign_vpn,
        vpn::remove_vpn,
        vpn::get_forwarded_port,
        vpn::list_supported_providers,
        // Cloudflare
        cloudflare::get_config,
        cloudflare::save_config,
        cloudflare::delete_config,
        cloudflare::get_status,
        cloudflare::validate_token,
    ),
    tags(
        (name = "Health", description = "Health check and version endpoints"),
        (name = "Auth", description = "Authentication and session management"),
        (name = "Users", description = "User management, preferences, and 2FA"),
        (name = "Roles", description = "Role-based access control"),
        (name = "Apps", description = "Application catalog and deployment"),
        (name = "Monitoring", description = "Metrics and cluster monitoring"),
        (name = "Networking", description = "Network topology and statistics"),
        (name = "Logs", description = "Log viewing and VictoriaLogs integration"),
        (name = "Audit", description = "Audit log management"),
        (name = "Notifications", description = "Notification channels, events, and inbox"),
        (name = "Storage", description = "File storage management"),
        (name = "Settings", description = "System settings"),
        (name = "OAuth", description = "OAuth provider configuration and login"),
        (name = "VPN", description = "VPN provider and app VPN configuration"),
        (name = "Cloudflare", description = "Cloudflare Tunnel configuration"),
    )
)]
pub struct ApiDoc;

/// Create the main API router
pub fn create_router(state: AppState) -> Router {
    // Health/version routes that need state (separate so we can apply state properly)
    let health_routes = Router::new()
        .route("/api/health", axum::routing::get(health_check))
        .route(
            "/api/system/health",
            axum::routing::get(health_check_detailed),
        )
        .route("/api/system/version", axum::routing::get(get_version))
        .with_state(state.clone());

    // Public routes (no auth required) - these already have state applied internally
    let public_routes = Router::new().nest("/auth", auth::auth_routes(state.clone()));

    // Protected API routes (auth required)
    let protected_api_routes = Router::new().nest("/api", api_routes(state.clone())).layer(
        axum_middleware::from_fn_with_state(state.clone(), require_auth),
    );

    // Note: App proxy routes (e.g., /qbittorrent/) are handled by the frontend fallback
    // which checks if the path is an installed app and proxies to it if authenticated

    // Frontend fallback router (unauthenticated)
    let fallback_router = Router::new()
        .fallback(frontend::proxy_frontend)
        .with_state(state);

    // OpenAPI spec and Swagger UI routes (no auth required)
    let openapi_routes = Router::new()
        .route("/api/openapi.json", axum::routing::get(openapi_json))
        .route("/api/docs", axum::routing::get(swagger_ui));

    // Merge all routes, with frontend proxy as fallback
    // The frontend fallback handles app proxying (e.g., /qbittorrent/) for authenticated users
    health_routes
        .merge(public_routes)
        .merge(openapi_routes)
        .merge(protected_api_routes)
        .merge(fallback_router)
}

/// API routes under /api/* (protected by auth middleware)
fn api_routes(state: AppState) -> Router {
    Router::new()
        .nest("/users", users::users_routes(state.clone()))
        .nest("/roles", roles::roles_routes(state.clone()))
        .nest("/settings", settings::settings_routes(state.clone()))
        .nest("/monitoring", monitoring::monitoring_routes(state.clone()))
        .nest("/networking", networking::networking_routes(state.clone()))
        .nest("/apps", apps::apps_routes(state.clone()))
        .nest("/storage", storage::storage_routes(state.clone()))
        .nest("/logs", logs::logs_routes(state.clone()))
        .nest("/audit", audit::audit_routes(state.clone()))
        .nest(
            "/notifications",
            notifications::notifications_routes(state.clone()),
        )
        .nest("/oauth", oauth::oauth_routes(state.clone()))
        .nest("/vpn", vpn::vpn_routes(state.clone()))
        .nest("/cloudflare", cloudflare::cloudflare_routes(state.clone()))
}

#[utoipa::path(get, path = "/api/health", tag = "Health", responses((status = 200, description = "OK")))]
async fn health_check() -> &'static str {
    "OK"
}

#[utoipa::path(get, path = "/api/system/health", tag = "Health", responses((status = 200, body = serde_json::Value)))]
async fn health_check_detailed(State(state): State<AppState>) -> axum::Json<serde_json::Value> {
    let db_connected = state.is_db_connected().await;

    axum::Json(serde_json::json!({
        "status": "ok",
        "ready": db_connected,
        "database_connected": db_connected
    }))
}

#[utoipa::path(get, path = "/api/system/version", tag = "Health", responses((status = 200, body = serde_json::Value)))]
async fn get_version() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "version": CONFIG.version,
        "channel": CONFIG.channel,
        "commit_hash": CONFIG.commit_hash,
        "build_time": CONFIG.build_time,
        "rust_version": "1.83",
        "backend": "rust"
    }))
}

/// Serve the OpenAPI JSON spec
async fn openapi_json() -> axum::Json<utoipa::openapi::OpenApi> {
    axum::Json(ApiDoc::openapi())
}

/// Serve Swagger UI
async fn swagger_ui() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>Kubarr API - Swagger UI</title>
    <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script>
        SwaggerUIBundle({
            url: '/api/openapi.json',
            dom_id: '#swagger-ui',
            presets: [SwaggerUIBundle.presets.apis, SwaggerUIBundle.SwaggerUIStandalonePreset],
            layout: 'BaseLayout'
        });
    </script>
</body>
</html>"#,
    )
}
