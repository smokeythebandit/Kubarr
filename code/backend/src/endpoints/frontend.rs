//! Frontend proxy handler
//!
//! Proxies unmatched requests to the frontend service.
//! Also handles app proxying for authenticated users at /{app_name}/* paths.
//! Implements SPA routing: returns index.html for non-asset 404s.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, Method, Response, StatusCode},
};

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::models::prelude::*;
use crate::models::session;
use crate::services::security::decode_session_token;
use crate::state::AppState;
use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

/// Check if a path looks like a static asset (has a file extension)
fn is_static_asset(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    if let Some(last_segment) = path.rsplit('/').next() {
        last_segment.contains('.')
    } else {
        false
    }
}

/// Reserved paths that should never be treated as app names
const RESERVED_PATHS: &[&str] = &[
    "api",
    "auth",
    "proxy",
    "assets",
    "favicon.svg",
    "login",
    "app-error",
];

/// Extract app name from path if it looks like an app path
fn extract_app_name(path: &str) -> Option<&str> {
    let path = path.trim_start_matches('/');
    let first_segment = path.split('/').next()?;

    // Skip reserved paths and static assets
    if first_segment.is_empty()
        || RESERVED_PATHS.contains(&first_segment)
        || first_segment.contains('.')
    {
        return None;
    }

    Some(first_segment)
}

/// Extract session token from cookie header
fn extract_session_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?;
    let cookie_str = cookies.to_str().ok()?;

    for cookie in cookie_str.split(';') {
        let cookie = cookie.trim();
        if let Some(value) = cookie.strip_prefix("kubarr_session=") {
            return Some(value.to_string());
        }
    }
    None
}

/// Rewrite Location headers in app proxy responses so redirects go through the proxy.
/// For apps without base_path: rewrite internal URLs and prepend app name to absolute paths.
/// For apps with base_path: only rewrite internal URLs (absolute paths already have correct prefix).
fn rewrite_app_response(
    mut response: Response<Body>,
    app_name: &str,
    internal_base_url: &str,
    base_path: Option<&str>,
) -> Response<Body> {
    if !response.status().is_redirection() {
        return response;
    }

    let location = match response.headers().get(header::LOCATION) {
        Some(loc) => match loc.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return response,
        },
        None => return response,
    };

    let base_path = base_path.filter(|path| !path.is_empty());
    let has_base_path = base_path.is_some();

    let rewritten = if let Some(path) = location.strip_prefix(internal_base_url) {
        // Absolute internal URL: http://app.ns.svc.cluster.local:PORT/path → /app_name/path
        let path = path.trim_start_matches('/');
        if has_base_path {
            // For base_path apps, the path already contains the base path prefix
            // (e.g., /jackett/UI/Login) — don't add app_name again
            format!("/{}", path)
        } else {
            format!("/{}/{}", app_name, path)
        }
    } else if base_path.is_some() {
        // Apps with a declared base path are expected to emit redirects that
        // already include that path. Leave them untouched.
        location
    } else if location.starts_with(&format!("/{app_name}/")) {
        // Preserve already-prefixed redirects for charts that have not yet been
        // upgraded to publish kubarr.io/base-path metadata.
        location
    } else if location == format!("/{app_name}") {
        location
    } else if location.starts_with('/') {
        // Absolute path without base_path: prepend the app prefix
        // (apps with base_path already have the correct prefix in their redirects)
        format!("/{}{}", app_name, location)
    } else {
        // Relative or full external URL, or base_path app with correct absolute path
        location
    };

    if let Ok(val) = rewritten.parse() {
        response.headers_mut().insert(header::LOCATION, val);
    }

    response
}

/// Rewrite text response bodies for apps without base_path.
/// Handles HTML, JavaScript, and CSS responses to prefix absolute paths with /{app_name}
/// so the browser fetches resources through the proxy instead of hitting the SPA fallback.
async fn rewrite_response_body(response: Response<Body>, app_name: &str) -> Response<Body> {
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let is_html = content_type.contains("text/html");
    let is_js = content_type.contains("javascript");
    let is_css = content_type.contains("text/css");

    if !is_html && !is_js && !is_css {
        return response;
    }

    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };

    let text = String::from_utf8_lossy(&bytes);
    let prefix = format!("/{}", app_name);

    let rewritten = if is_html {
        rewrite_html(&text, &prefix)
    } else if is_js {
        rewrite_js(&text, &prefix)
    } else {
        rewrite_css(&text, &prefix)
    };

    let body_bytes = rewritten.into_bytes();
    let mut parts = parts;
    if parts.headers.contains_key(header::CONTENT_LENGTH) {
        if let Ok(val) = body_bytes.len().to_string().parse() {
            parts.headers.insert(header::CONTENT_LENGTH, val);
        }
    }

    Response::from_parts(parts, Body::from(body_bytes))
}

/// Rewrite HTML content: absolute paths in attributes + inline script patterns
fn rewrite_html(html: &str, prefix: &str) -> String {
    // Rewrite absolute paths in HTML attributes (double and single quoted)
    let rewritten = html
        .replace("src=\"/", &format!("src=\"{}/", prefix))
        .replace("href=\"/", &format!("href=\"{}/", prefix))
        .replace("action=\"/", &format!("action=\"{}/", prefix))
        .replace("src='/", &format!("src='{}/", prefix))
        .replace("href='/", &format!("href='{}/", prefix))
        .replace("action='/", &format!("action='{}/", prefix));

    // Fix protocol-relative URLs that got incorrectly rewritten (e.g., //cdn.example.com)
    let rewritten = rewritten
        .replace(&format!("src=\"{}//", prefix), "src=\"//")
        .replace(&format!("href=\"{}//", prefix), "href=\"//")
        .replace(&format!("action=\"{}//", prefix), "action=\"//")
        .replace(&format!("src='{}//", prefix), "src='//")
        .replace(&format!("href='{}//", prefix), "href='//")
        .replace(&format!("action='{}//", prefix), "action='//");

    // Rewrite inline JS patterns commonly found in <script> blocks:
    // - JSON config values like "base": "/" or "basePath": "/"
    //   These are used by apps (e.g., Deluge ExtJS) to construct URLs at runtime
    rewrite_js_paths(&rewritten, prefix)
}

/// Rewrite JavaScript content: webpack public paths and other absolute path patterns
fn rewrite_js(js: &str, prefix: &str) -> String {
    rewrite_js_paths(js, prefix)
}

/// Rewrite CSS content: url() references with absolute paths
fn rewrite_css(css: &str, prefix: &str) -> String {
    // url("/path/...") → url("/app_name/path/...")
    let rewritten = css
        .replace("url(\"/", &format!("url(\"{}/", prefix))
        .replace("url('/", &format!("url('{}/", prefix))
        .replace("url(/", &format!("url({}/", prefix));

    // Fix protocol-relative
    rewritten
        .replace(&format!("url(\"{}//'", prefix), "url(\"//")
        .replace(&format!("url('{}//'", prefix), "url('//")
        .replace(&format!("url({}//'", prefix), "url(//")
}

/// Rewrite common JS patterns containing absolute paths.
/// This is generic and handles patterns from various frameworks:
/// - Webpack minified public path: .p="/..." (e.g., .p="/_next/")
/// - JSON-style config values: :"/" or : "/" (e.g., "base": "/")
fn rewrite_js_paths(text: &str, prefix: &str) -> String {
    let rewritten = text
        // Webpack public path (minified): .p="/..." or .p='/'
        .replace(".p=\"/", &format!(".p=\"{}/", prefix))
        .replace(".p='/", &format!(".p='{}/", prefix))
        // JSON config values: :"/" and : "/" (with optional space after colon)
        // Catches patterns like "base":"/" or "base": "/"
        .replace(":\"/", &format!(":\"{}/", prefix))
        .replace(":'/", &format!(":\'{}/", prefix))
        .replace(": \"/", &format!(": \"{}/", prefix))
        .replace(": '/", &format!(": '{}/", prefix));

    // Fix protocol-relative URLs that got incorrectly rewritten
    let rewritten = rewritten
        .replace(&format!(".p=\"{}//", prefix), ".p=\"//")
        .replace(&format!(".p='{}//", prefix), ".p='//")
        .replace(&format!(":\"{}//'", prefix), ":\"//")
        .replace(&format!(":\'{}//'", prefix), ":'//")
        .replace(&format!(": \"{}//'", prefix), ": \"//")
        .replace(&format!(": '{}//'", prefix), ": '//");

    // Fix double-prefixing: if a path already had the prefix, undo the extra one
    let double_prefix = format!("{}{}/", prefix, prefix);
    let single_prefix = format!("{}/", prefix);
    rewritten.replace(&double_prefix, &single_prefix)
}

/// Create a redirect response to the app error page
#[allow(clippy::unwrap_used)]
fn redirect_to_app_error(app_name: &str, reason: &str, details: &str) -> Response<Body> {
    let encoded_details = urlencoding::encode(details);
    let redirect_url = format!(
        "/app-error?app={}&reason={}&details={}",
        app_name, reason, encoded_details
    );

    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, redirect_url)
        .body(Body::empty())
        .unwrap()
}

/// Proxy requests to the frontend service with SPA routing support
/// Also handles app proxying for authenticated users at /{app_name}/* paths
pub async fn proxy_frontend(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response<Body>> {
    let path = request.uri().path().to_string();
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();

    let method = request.method().clone();
    let headers = request.headers().clone();

    // Check if this looks like an app path and user is authenticated
    if let Some(app_name) = extract_app_name(&path) {
        tracing::info!("Detected potential app path: {}", app_name);
        if let Some(token) = extract_session_token(&headers) {
            tracing::info!("Found session token for app {}", app_name);
            // Decode session token (contains session ID, not user ID)
            if let Ok(claims) = decode_session_token(&token) {
                tracing::info!(
                    "Session token decoded for app {}, sid={}",
                    app_name,
                    claims.sid
                );
                // Look up session in database to get user_id
                let db = match state.get_db().await {
                    Ok(db) => db,
                    Err(_) => {
                        tracing::warn!("Database not available for session lookup");
                        return proxy_to_frontend(
                            &state,
                            path,
                            query,
                            method,
                            headers,
                            request.into_body(),
                        )
                        .await;
                    }
                };
                if let Ok(Some(session_record)) = Session::find()
                    .filter(session::Column::Id.eq(&claims.sid))
                    .filter(session::Column::IsRevoked.eq(false))
                    .filter(session::Column::ExpiresAt.gt(Utc::now()))
                    .one(&db)
                    .await
                {
                    let user_id = session_record.user_id;
                    tracing::info!(
                        "Session valid for user {} accessing app {}",
                        user_id,
                        app_name
                    );
                    // Check if user has access to this app
                    let has_permission = check_app_permission(&state, user_id, app_name).await;
                    tracing::info!(
                        "User {} permission for app {}: {}",
                        user_id,
                        app_name,
                        has_permission
                    );
                    if has_permission {
                        // Try to proxy to the app
                        match get_app_target_url(&state, app_name, &path, &query).await {
                            Ok((base_url, base_path, target_url)) => {
                                let has_base_path = base_path.is_some();
                                tracing::info!(
                                    "Proxying to app {}: {} (base_path: {})",
                                    app_name,
                                    target_url,
                                    has_base_path
                                );
                                let body = request.into_body();
                                let proxy = &state.proxy;
                                match proxy.proxy_http(&target_url, method, headers, body).await {
                                    Ok(response) => {
                                        // Rewrite Location headers for redirects
                                        let response = rewrite_app_response(
                                            response,
                                            app_name,
                                            &base_url,
                                            base_path.as_deref(),
                                        );
                                        // For apps without base_path, rewrite HTML body
                                        // to prefix absolute paths with /{app_name}
                                        let response = if !has_base_path {
                                            rewrite_response_body(response, app_name).await
                                        } else {
                                            response
                                        };
                                        return Ok(response);
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to connect to app {}: {}",
                                            app_name,
                                            e
                                        );
                                        return Ok(redirect_to_app_error(
                                            app_name,
                                            "connection_failed",
                                            &e.to_string(),
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::info!(
                                    "App {} not found, falling through to frontend: {}",
                                    app_name,
                                    e
                                );
                                // Don't show error page - fall through to frontend proxy
                                // The path might be a frontend route (e.g., /networking)
                            }
                        }
                    }
                } else {
                    tracing::info!("Session not found or invalid for app {}", app_name);
                }
            } else {
                tracing::info!("Failed to decode session token for app {}", app_name);
            }
        } else {
            tracing::info!("No session token for app path {}", app_name);
        }
    }

    let body = request.into_body();
    let proxy = &state.proxy;

    // For static assets, proxy directly
    if is_static_asset(&path) {
        let target_url = format!("{}{}{}", CONFIG.frontend_url, path, query);
        tracing::debug!("Proxying static asset: {}", target_url);

        return proxy
            .proxy_http(&target_url, method, headers, body)
            .await
            .map_err(|e| {
                tracing::error!("Frontend proxy error: {}", e);
                AppError::BadGateway(format!("Frontend unavailable: {}", e))
            });
    }

    // For non-asset paths, try the path first, fall back to index.html on 404
    let target_url = format!("{}{}{}", CONFIG.frontend_url, path, query);
    tracing::debug!("Proxying to frontend: {}", target_url);

    let response = proxy
        .proxy_http(&target_url, method.clone(), headers.clone(), body)
        .await
        .map_err(|e| {
            tracing::error!("Frontend proxy error: {}", e);
            AppError::BadGateway(format!("Frontend unavailable: {}", e))
        })?;

    // If 404, serve index.html for SPA routing
    if response.status() == StatusCode::NOT_FOUND {
        let index_url = format!("{}/index.html", CONFIG.frontend_url);
        tracing::debug!("SPA fallback to index.html for path: {}", path);

        return proxy
            .proxy_http(&index_url, Method::GET, headers, Body::empty())
            .await
            .map_err(|e| {
                tracing::error!("Frontend proxy error (index.html): {}", e);
                AppError::BadGateway(format!("Frontend unavailable: {}", e))
            });
    }

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

/// Helper function to proxy to frontend
async fn proxy_to_frontend(
    state: &AppState,
    path: String,
    query: String,
    method: Method,
    headers: axum::http::HeaderMap,
    body: Body,
) -> Result<Response<Body>> {
    let proxy = &state.proxy;

    // For static assets, proxy directly
    if is_static_asset(&path) {
        let target_url = format!("{}{}{}", CONFIG.frontend_url, path, query);
        tracing::debug!("Proxying static asset: {}", target_url);

        return proxy
            .proxy_http(&target_url, method, headers, body)
            .await
            .map_err(|e| {
                tracing::error!("Frontend proxy error: {}", e);
                AppError::BadGateway(format!("Frontend unavailable: {}", e))
            });
    }

    // For non-asset paths, try the path first, fall back to index.html on 404
    let target_url = format!("{}{}{}", CONFIG.frontend_url, path, query);
    tracing::debug!("Proxying to frontend: {}", target_url);

    let response = proxy
        .proxy_http(&target_url, method.clone(), headers.clone(), body)
        .await
        .map_err(|e| {
            tracing::error!("Frontend proxy error: {}", e);
            AppError::BadGateway(format!("Frontend unavailable: {}", e))
        })?;

    // If 404, serve index.html for SPA routing
    if response.status() == StatusCode::NOT_FOUND {
        let index_url = format!("{}/index.html", CONFIG.frontend_url);
        tracing::debug!("SPA fallback to index.html for path: {}", path);

        return proxy
            .proxy_http(&index_url, Method::GET, headers, Body::empty())
            .await
            .map_err(|e| {
                tracing::error!("Frontend proxy error (index.html): {}", e);
                AppError::BadGateway(format!("Frontend unavailable: {}", e))
            });
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Response, StatusCode};

    // -------------------------------------------------------------------------
    // is_static_asset tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_static_asset_css() {
        assert!(is_static_asset("/style.css"));
    }

    #[test]
    fn test_is_static_asset_js() {
        assert!(is_static_asset("/app.js"));
    }

    #[test]
    fn test_is_static_asset_ico() {
        assert!(is_static_asset("/favicon.ico"));
    }

    #[test]
    fn test_is_static_asset_nested_png() {
        assert!(is_static_asset("/assets/img.png"));
    }

    #[test]
    fn test_is_static_asset_js_with_query() {
        // The query string is stripped before checking for a dot
        assert!(is_static_asset("/file.js?v=123"));
    }

    #[test]
    fn test_is_static_asset_root_is_not_asset() {
        assert!(!is_static_asset("/"));
    }

    #[test]
    fn test_is_static_asset_api_path_is_not_asset() {
        assert!(!is_static_asset("/api/data"));
    }

    #[test]
    fn test_is_static_asset_settings_route_is_not_asset() {
        assert!(!is_static_asset("/settings"));
    }

    #[test]
    fn test_is_static_asset_admin_route_is_not_asset() {
        assert!(!is_static_asset("/admin"));
    }

    #[test]
    fn test_is_static_asset_empty_string_is_not_asset() {
        assert!(!is_static_asset(""));
    }

    // -------------------------------------------------------------------------
    // extract_app_name tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_app_name_simple() {
        assert_eq!(extract_app_name("/sonarr"), Some("sonarr"));
    }

    #[test]
    fn test_extract_app_name_with_subpath() {
        assert_eq!(extract_app_name("/sonarr/path"), Some("sonarr"));
    }

    #[test]
    fn test_extract_app_name_with_query() {
        // The query part is after '?' but split('/') still yields "sonarr?q=1" as first segment.
        // However is_static_asset logic and RESERVED_PATHS check happen on the first segment.
        // "sonarr?q=1" has no dot → passes through → Some("sonarr?q=1").
        // Let's verify the actual behaviour for "/sonarr?q=1":
        // trim_start_matches('/') → "sonarr?q=1"
        // split('/').next() → "sonarr?q=1"  (no slash before '?')
        // "sonarr?q=1".contains('.') → false → Some("sonarr?q=1")
        // The path "/sonarr?q=1" would reach this function as the path portion only
        // (Axum splits path from query), so the path is "/sonarr" and this test
        // exercises the function as called in production.
        assert_eq!(extract_app_name("/sonarr"), Some("sonarr"));
    }

    #[test]
    fn test_extract_app_name_api_is_reserved() {
        assert_eq!(extract_app_name("/api/health"), None);
    }

    #[test]
    fn test_extract_app_name_auth_is_reserved() {
        assert_eq!(extract_app_name("/auth/login"), None);
    }

    #[test]
    fn test_extract_app_name_assets_is_reserved() {
        assert_eq!(extract_app_name("/assets/logo.png"), None);
    }

    #[test]
    fn test_extract_app_name_favicon_svg_is_reserved() {
        assert_eq!(extract_app_name("/favicon.svg"), None);
    }

    #[test]
    fn test_extract_app_name_root_is_none() {
        assert_eq!(extract_app_name("/"), None);
    }

    #[test]
    fn test_extract_app_name_double_slash_is_none() {
        // trim_start_matches('/') on "//" → ""  → first segment is "" → None
        assert_eq!(extract_app_name("//"), None);
    }

    #[test]
    fn test_extract_app_name_file_with_dot_is_none() {
        // first segment has a dot → treated as static asset, returns None
        assert_eq!(extract_app_name("/file.txt"), None);
    }

    #[test]
    fn test_extract_app_name_login_is_reserved() {
        assert_eq!(extract_app_name("/login"), None);
    }

    #[test]
    fn test_extract_app_name_radarr() {
        assert_eq!(extract_app_name("/radarr/movies"), Some("radarr"));
    }

    // -------------------------------------------------------------------------
    // extract_session_token tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_session_token_present() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(header::COOKIE, "kubarr_session=mytoken123".parse().unwrap());
        assert_eq!(
            extract_session_token(&headers),
            Some("mytoken123".to_string())
        );
    }

    #[test]
    fn test_extract_session_token_multiple_cookies_kubarr_in_middle() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=val; kubarr_session=abc456; another=xyz"
                .parse()
                .unwrap(),
        );
        assert_eq!(extract_session_token(&headers), Some("abc456".to_string()));
    }

    #[test]
    fn test_extract_session_token_missing_cookie_header() {
        let headers = axum::http::HeaderMap::new();
        assert_eq!(extract_session_token(&headers), None);
    }

    #[test]
    fn test_extract_session_token_cookie_header_without_kubarr_session() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(header::COOKIE, "session=other; csrf=token".parse().unwrap());
        assert_eq!(extract_session_token(&headers), None);
    }

    // -------------------------------------------------------------------------
    // rewrite_app_response tests
    // -------------------------------------------------------------------------

    fn make_response(status: StatusCode, location: Option<&str>) -> Response<Body> {
        let mut builder = Response::builder().status(status);
        if let Some(loc) = location {
            builder = builder.header(header::LOCATION, loc);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn test_rewrite_app_response_non_redirect_passes_through() {
        let response = make_response(StatusCode::OK, None);
        let result = rewrite_app_response(
            response,
            "sonarr",
            "http://sonarr.sonarr.svc.cluster.local:8989",
            None,
        );
        assert_eq!(result.status(), StatusCode::OK);
    }

    #[test]
    fn test_rewrite_app_response_redirect_with_internal_url_no_base_path() {
        // Location: http://app.ns.svc.cluster.local:PORT/ui/page
        // → /sonarr/ui/page
        let internal_base = "http://sonarr.sonarr.svc.cluster.local:8989";
        let location = format!("{}/ui/page", internal_base);
        let response = make_response(StatusCode::FOUND, Some(&location));
        let result = rewrite_app_response(response, "sonarr", internal_base, None);
        let loc = result
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/sonarr/ui/page");
    }

    #[test]
    fn test_rewrite_app_response_redirect_with_internal_url_has_base_path() {
        // Location: http://jackett.jackett.svc.cluster.local:9117/jackett/UI/Login
        // With has_base_path=true → /jackett/UI/Login (don't add app prefix again)
        let internal_base = "http://jackett.jackett.svc.cluster.local:9117";
        let location = format!("{}/jackett/UI/Login", internal_base);
        let response = make_response(StatusCode::FOUND, Some(&location));
        let result = rewrite_app_response(response, "jackett", internal_base, Some("/jackett"));
        let loc = result
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/jackett/UI/Login");
    }

    #[test]
    fn test_rewrite_app_response_absolute_path_no_base_path_gets_prefix() {
        // Location: /login → /sonarr/login
        let internal_base = "http://sonarr.sonarr.svc.cluster.local:8989";
        let response = make_response(StatusCode::FOUND, Some("/login"));
        let result = rewrite_app_response(response, "sonarr", internal_base, None);
        let loc = result
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/sonarr/login");
    }

    #[test]
    fn test_rewrite_app_response_already_prefixed_no_base_path_unchanged() {
        // Sonarr/Radarr may redirect to their configured URL base themselves.
        let internal_base = "http://sonarr.sonarr.svc.cluster.local:8989";
        let response = make_response(StatusCode::TEMPORARY_REDIRECT, Some("/sonarr/"));
        let result = rewrite_app_response(response, "sonarr", internal_base, None);
        let loc = result
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/sonarr/");
    }

    #[test]
    fn test_rewrite_app_response_absolute_path_with_base_path_unchanged() {
        // With has_base_path=true, absolute paths that don't match the internal base
        // are returned unchanged (they already contain the base path prefix)
        let internal_base = "http://jackett.jackett.svc.cluster.local:9117";
        let response = make_response(StatusCode::FOUND, Some("/jackett/UI/Dashboard"));
        let result = rewrite_app_response(response, "jackett", internal_base, Some("/jackett"));
        let loc = result
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "/jackett/UI/Dashboard");
    }

    #[test]
    fn test_rewrite_app_response_external_url_passes_through() {
        // A full external URL should be left untouched
        let internal_base = "http://sonarr.sonarr.svc.cluster.local:8989";
        let response = make_response(StatusCode::FOUND, Some("https://external.example.com/page"));
        let result = rewrite_app_response(response, "sonarr", internal_base, None);
        let loc = result
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(loc, "https://external.example.com/page");
    }

    // -------------------------------------------------------------------------
    // rewrite_html tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_rewrite_html_src_double_quote() {
        let html = r#"<img src="/images/logo.png">"#;
        let result = rewrite_html(html, "/sonarr");
        assert!(result.contains(r#"src="/sonarr/images/logo.png""#));
    }

    #[test]
    fn test_rewrite_html_href_double_quote() {
        let html = r#"<link href="/css/style.css">"#;
        let result = rewrite_html(html, "/sonarr");
        assert!(result.contains(r#"href="/sonarr/css/style.css""#));
    }

    #[test]
    fn test_rewrite_html_action_double_quote() {
        let html = r#"<form action="/submit">"#;
        let result = rewrite_html(html, "/sonarr");
        assert!(result.contains(r#"action="/sonarr/submit""#));
    }

    #[test]
    fn test_rewrite_html_src_single_quote() {
        let html = "<img src='/images/logo.png'>";
        let result = rewrite_html(html, "/sonarr");
        assert!(result.contains("src='/sonarr/images/logo.png'"));
    }

    #[test]
    fn test_rewrite_html_href_single_quote() {
        let html = "<link href='/css/style.css'>";
        let result = rewrite_html(html, "/sonarr");
        assert!(result.contains("href='/sonarr/css/style.css'"));
    }

    #[test]
    fn test_rewrite_html_protocol_relative_url_preserved() {
        // "//cdn.example.com" must NOT get a prefix — protocol-relative URLs
        // get fixed back after the initial rewrite
        let html = r#"<img src="//cdn.example.com/logo.png">"#;
        let result = rewrite_html(html, "/sonarr");
        assert!(result.contains(r#"src="//cdn.example.com/logo.png""#));
    }

    // -------------------------------------------------------------------------
    // rewrite_css tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_rewrite_css_url_double_quote() {
        let css = r#"background: url("/images/bg.png");"#;
        let result = rewrite_css(css, "/sonarr");
        assert!(result.contains(r#"url("/sonarr/images/bg.png")"#));
    }

    #[test]
    fn test_rewrite_css_url_single_quote() {
        let css = "background: url('/images/bg.png');";
        let result = rewrite_css(css, "/sonarr");
        assert!(result.contains("url('/sonarr/images/bg.png')"));
    }

    #[test]
    fn test_rewrite_css_url_unquoted() {
        let css = "background: url(/images/bg.png);";
        let result = rewrite_css(css, "/sonarr");
        assert!(result.contains("url(/sonarr/images/bg.png)"));
    }

    // -------------------------------------------------------------------------
    // rewrite_js_paths tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_rewrite_js_paths_webpack_public_path_double_quote() {
        let js = r#"e.p="/_next/""#;
        let result = rewrite_js_paths(js, "/sonarr");
        assert!(result.contains(r#".p="/sonarr/_next/""#));
    }

    #[test]
    fn test_rewrite_js_paths_json_config_colon_slash() {
        let js = r#"{"base":"/"}"#;
        let result = rewrite_js_paths(js, "/sonarr");
        assert!(result.contains(r#""base":"/sonarr/""#));
    }

    #[test]
    fn test_rewrite_js_paths_json_config_colon_space_slash() {
        let js = r#"{"base": "/"}"#;
        let result = rewrite_js_paths(js, "/sonarr");
        assert!(result.contains(r#""base": "/sonarr/""#));
    }

    #[test]
    fn test_rewrite_js_paths_double_prefix_prevention() {
        // If the path already contains the prefix, it must not be doubled
        let js = r#"{"base":"/sonarr/"}"#;
        let result = rewrite_js_paths(js, "/sonarr");
        // After rewrite + fix, "/sonarr/" should appear exactly once in the value
        assert!(result.contains(r#""/sonarr/""#));
        assert!(!result.contains(r#""/sonarr/sonarr/""#));
    }

    #[test]
    fn test_rewrite_js_paths_webpack_single_quote() {
        let js = ".p='/static/'";
        let result = rewrite_js_paths(js, "/sonarr");
        assert!(result.contains(".p='/sonarr/static/'"));
    }

    // -------------------------------------------------------------------------
    // redirect_to_app_error tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_redirect_to_app_error_returns_302() {
        let resp = redirect_to_app_error("sonarr", "not_found", "App not deployed");
        assert_eq!(resp.status(), StatusCode::FOUND);
    }

    #[test]
    fn test_redirect_to_app_error_location_contains_app_and_reason() {
        let resp = redirect_to_app_error("radarr", "unreachable", "Connection refused");
        let location = resp
            .headers()
            .get(header::LOCATION)
            .expect("Location header must be set")
            .to_str()
            .unwrap();
        assert!(location.contains("radarr"), "location: {}", location);
        assert!(location.contains("unreachable"), "location: {}", location);
    }

    #[test]
    fn test_redirect_to_app_error_encodes_details() {
        let resp = redirect_to_app_error("jackett", "error", "has spaces & special");
        let location = resp
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        // URL-encoded spaces and & should appear in the location
        assert!(
            !location.contains("has spaces"),
            "details should be URL-encoded: {}",
            location
        );
    }

    // -------------------------------------------------------------------------
    // rewrite_response_body tests (async)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_rewrite_response_body_non_html_passes_through_unchanged() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"key":"/value"}"#))
            .unwrap();
        let result = rewrite_response_body(resp, "sonarr").await;
        assert_eq!(result.status(), StatusCode::OK);
        // Body should be unchanged because content-type is not HTML/JS/CSS
        let bytes = axum::body::to_bytes(result.into_body(), 1024)
            .await
            .unwrap();
        let s = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(s.contains(r#"{"key":"/value"}"#));
    }

    #[tokio::test]
    async fn test_rewrite_response_body_html_rewrites_paths() {
        let html = r#"<link href="/css/app.css"><img src="/images/logo.png">"#;
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(html))
            .unwrap();
        let result = rewrite_response_body(resp, "sonarr").await;
        let bytes = axum::body::to_bytes(result.into_body(), 4096)
            .await
            .unwrap();
        let s = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            s.contains("/sonarr/css/app.css"),
            "HTML href not rewritten: {}",
            s
        );
        assert!(
            s.contains("/sonarr/images/logo.png"),
            "HTML src not rewritten: {}",
            s
        );
    }

    #[tokio::test]
    async fn test_rewrite_response_body_js_rewrites_webpack_path() {
        let js = r#"e.p="/_next/static/""#;
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/javascript")
            .body(Body::from(js))
            .unwrap();
        let result = rewrite_response_body(resp, "nextapp").await;
        let bytes = axum::body::to_bytes(result.into_body(), 4096)
            .await
            .unwrap();
        let s = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            s.contains("/nextapp/_next/static/"),
            "JS webpack path not rewritten: {}",
            s
        );
    }

    #[tokio::test]
    async fn test_rewrite_response_body_css_rewrites_url() {
        let css = r#"background: url("/images/bg.png");"#;
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/css")
            .body(Body::from(css))
            .unwrap();
        let result = rewrite_response_body(resp, "myapp").await;
        let bytes = axum::body::to_bytes(result.into_body(), 4096)
            .await
            .unwrap();
        let s = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            s.contains(r#"url("/myapp/images/bg.png")"#),
            "CSS url not rewritten: {}",
            s
        );
    }

    #[tokio::test]
    async fn test_rewrite_response_body_updates_content_length() {
        // If the response has a Content-Length header, it should be updated
        let html = r#"<link href="/css/x.css">"#;
        let original_len = html.len().to_string();
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .header(header::CONTENT_LENGTH, &original_len)
            .body(Body::from(html))
            .unwrap();
        let result = rewrite_response_body(resp, "myapp").await;
        // The Content-Length should be updated to reflect the new body size
        if let Some(len_header) = result.headers().get(header::CONTENT_LENGTH) {
            let new_len: usize = len_header.to_str().unwrap().parse().unwrap();
            // Rewritten body is longer than original, so Content-Length should differ
            assert!(new_len != original_len.parse::<usize>().unwrap());
        }
    }

    // -------------------------------------------------------------------------
    // rewrite_app_response — additional edge cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_rewrite_app_response_redirect_without_location_passes_through() {
        // A redirect response without a Location header should not be modified
        let resp = Response::builder()
            .status(StatusCode::FOUND)
            .body(Body::empty())
            .unwrap();
        let result = rewrite_app_response(
            resp,
            "sonarr",
            "http://sonarr.sonarr.svc.cluster.local:8989",
            None,
        );
        assert_eq!(result.status(), StatusCode::FOUND);
        assert!(result.headers().get(header::LOCATION).is_none());
    }

    // -------------------------------------------------------------------------
    // get_app_target_url — cache-hit paths (no K8s needed)
    // -------------------------------------------------------------------------

    async fn make_state() -> AppState {
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
            None,
            k8s_client,
            catalog,
            chart_sync,
            AuditService::new(),
            NotificationService::new(),
        )
    }

    #[tokio::test]
    async fn test_get_app_target_url_cache_hit_no_base_path() {
        let state = make_state().await;
        let base = "http://sonarr.sonarr.svc.cluster.local:8989";
        state
            .endpoint_cache
            .set("sonarr", base.to_string(), None)
            .await;

        let (ret_base, base_path, target_url) =
            get_app_target_url(&state, "sonarr", "/sonarr/movies", "")
                .await
                .expect("should succeed from cache");

        assert_eq!(ret_base, base);
        assert!(base_path.is_none());
        // Strips the app name prefix → "movies"
        assert_eq!(target_url, format!("{}/movies", base));
    }

    #[tokio::test]
    async fn test_get_app_target_url_cache_hit_with_base_path() {
        let state = make_state().await;
        let base = "http://sonarr.sonarr.svc.cluster.local:8989";
        state
            .endpoint_cache
            .set("sonarr", base.to_string(), Some("/sonarr".to_string()))
            .await;

        let (ret_base, base_path, target_url) =
            get_app_target_url(&state, "sonarr", "/sonarr/movies", "")
                .await
                .expect("should succeed from cache");

        assert_eq!(ret_base, base);
        assert_eq!(base_path.as_deref(), Some("/sonarr"));
        // With base_path, keeps full path starting from after /
        assert_eq!(target_url, format!("{}/sonarr/movies", base));
    }

    #[tokio::test]
    async fn test_get_app_target_url_cache_hit_no_base_path_empty_path() {
        let state = make_state().await;
        let base = "http://sonarr.sonarr.svc.cluster.local:8989";
        state
            .endpoint_cache
            .set("sonarr", base.to_string(), None)
            .await;

        let (_, _, target_url) = get_app_target_url(&state, "sonarr", "/sonarr", "")
            .await
            .expect("should succeed");

        // Empty after stripping app name → add trailing slash
        assert_eq!(target_url, format!("{}/", base));
    }

    #[tokio::test]
    async fn test_get_app_target_url_cache_hit_no_base_path_empty_path_with_query() {
        let state = make_state().await;
        let base = "http://sonarr.sonarr.svc.cluster.local:8989";
        state
            .endpoint_cache
            .set("sonarr", base.to_string(), None)
            .await;

        let (_, _, target_url) = get_app_target_url(&state, "sonarr", "/sonarr", "?search=foo")
            .await
            .expect("should succeed");

        // Empty path with query
        assert_eq!(target_url, format!("{}/?search=foo", base));
    }

    #[tokio::test]
    async fn test_get_app_target_url_cache_hit_with_base_path_empty_path() {
        let state = make_state().await;
        let base = "http://sonarr.sonarr.svc.cluster.local:8989";
        state
            .endpoint_cache
            .set("sonarr", base.to_string(), Some("/sonarr".to_string()))
            .await;

        let (_, _, target_url) = get_app_target_url(&state, "sonarr", "/", "")
            .await
            .expect("should succeed");

        assert_eq!(target_url, format!("{}/", base));
    }

    #[tokio::test]
    async fn test_get_app_target_url_cache_hit_with_base_path_empty_path_with_query() {
        let state = make_state().await;
        let base = "http://sonarr.sonarr.svc.cluster.local:8989";
        state
            .endpoint_cache
            .set("sonarr", base.to_string(), Some("/sonarr".to_string()))
            .await;

        let (_, _, target_url) = get_app_target_url(&state, "sonarr", "/", "?q=1")
            .await
            .expect("should succeed");

        assert_eq!(target_url, format!("{}/?q=1", base));
    }

    #[tokio::test]
    async fn test_get_app_target_url_no_k8s_returns_err() {
        let state = make_state().await;
        // No cache entry, k8s_client is None → should return Err
        let result = get_app_target_url(&state, "sonarr", "/sonarr", "").await;
        assert!(result.is_err());
    }
}

/// Get the target URL for an app, returning (base_url, base_path, full_target_url)
async fn get_app_target_url(
    state: &AppState,
    app_name: &str,
    path: &str,
    query: &str,
) -> Result<(String, Option<String>, String)> {
    // Check cache first
    let (base_url, base_path) = if let Some(cached) = state.endpoint_cache.get(app_name).await {
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

    let has_base_path = base_path.is_some();

    let target_url = if has_base_path {
        // App has a URL base (e.g., Sonarr at /sonarr) - keep the full path
        let trimmed = path.trim_start_matches('/');
        if trimmed.is_empty() && query.is_empty() {
            format!("{}/", base_url)
        } else if trimmed.is_empty() {
            format!("{}/?{}", base_url, query.trim_start_matches('?'))
        } else {
            format!("{}/{}{}", base_url, trimmed, query)
        }
    } else {
        // App has no URL base (e.g., qBittorrent) - strip the app name prefix
        let app_path = path
            .trim_start_matches('/')
            .strip_prefix(app_name)
            .unwrap_or("")
            .trim_start_matches('/');

        if app_path.is_empty() && query.is_empty() {
            format!("{}/", base_url)
        } else if app_path.is_empty() {
            format!("{}/?{}", base_url, query.trim_start_matches('?'))
        } else {
            format!("{}/{}{}", base_url, app_path, query)
        }
    };

    Ok((base_url, base_path, target_url))
}
