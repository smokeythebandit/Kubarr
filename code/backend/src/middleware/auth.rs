//! Authentication middleware for API routes
//!
//! Requires valid session cookie for all endpoints except `/auth/*`.
//! Session tokens contain only a session ID - user data is looked up from the database.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::models::prelude::*;
use crate::models::{role_app_permission, role_permission, session, user, user_role};
use crate::services::security::decode_session_token;
use crate::state::AppState;

/// Base cookie name for session tokens (indexed as kubarr_session_0, kubarr_session_1, etc.)
pub const SESSION_COOKIE_BASE: &str = "kubarr_session";
/// Cookie name for active session index
pub const ACTIVE_SESSION_COOKIE: &str = "kubarr_active";
/// Maximum number of simultaneous sessions
pub const MAX_SESSIONS: usize = 5;

/// Legacy cookie name (for backwards compatibility)
pub const SESSION_COOKIE_NAME: &str = "kubarr_session";

/// Authenticated user with permissions, stored in request extensions
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub user: user::Model,
    pub permissions: Vec<String>,
}

impl AuthenticatedUser {
    /// Check if user has a specific permission
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.contains(&permission.to_string())
    }

    /// Check if user has access to a specific app
    pub fn has_app_access(&self, app_name: &str) -> bool {
        // Check for app.* wildcard or specific app permission
        self.permissions.contains(&"app.*".to_string())
            || self.permissions.contains(&format!("app.{}", app_name))
    }
}

/// Auth middleware that validates session cookies and fetches permissions
///
/// Skips authentication for `/auth/*` routes.
/// Returns 401 Unauthorized if session is missing or invalid.
pub async fn require_auth(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path();

    // Skip authentication for /auth/* routes
    if path.starts_with("/auth") {
        return next.run(req).await;
    }

    // Extract session token from cookie
    let token = match extract_token(&req) {
        Some(t) => t,
        None => {
            return unauthorized_response("Missing or invalid session");
        }
    };

    // Validate session and get user with permissions
    let auth_user = match authenticate_session(&state, &token).await {
        Ok(u) => u,
        Err(msg) => {
            return unauthorized_response(&msg);
        }
    };

    // Add authenticated user to request extensions
    req.extensions_mut().insert(auth_user);

    next.run(req).await
}

/// Extract session token from cookie (supports multi-session)
fn extract_token(req: &Request) -> Option<String> {
    let cookies = req.headers().get(header::COOKIE)?;
    let cookie_str = cookies.to_str().ok()?;

    // Parse all cookies
    let mut active_slot: Option<usize> = None;
    let mut session_cookies: std::collections::HashMap<usize, String> =
        std::collections::HashMap::new();
    let mut legacy_token: Option<String> = None;

    for cookie in cookie_str.split(';') {
        let cookie = cookie.trim();

        // Check for active session cookie
        if let Some(value) = cookie.strip_prefix(&format!("{}=", ACTIVE_SESSION_COOKIE)) {
            active_slot = value.parse().ok();
        }
        // Check for indexed session cookies (kubarr_session_0, kubarr_session_1, etc.)
        else if cookie.starts_with(SESSION_COOKIE_BASE) {
            for i in 0..MAX_SESSIONS {
                let prefix = format!("{}_{}=", SESSION_COOKIE_BASE, i);
                if let Some(value) = cookie.strip_prefix(&prefix) {
                    session_cookies.insert(i, value.to_string());
                    break;
                }
            }
            // Also check for legacy cookie (kubarr_session without index)
            if let Some(value) = cookie.strip_prefix(&format!("{}=", SESSION_COOKIE_NAME)) {
                // Only use legacy if it doesn't match an indexed pattern
                if !cookie.contains(&format!("{}_", SESSION_COOKIE_BASE)) {
                    legacy_token = Some(value.to_string());
                }
            }
        }
    }

    // If we have indexed sessions, use the active one
    if !session_cookies.is_empty() {
        let slot = active_slot.unwrap_or(0);
        if let Some(token) = session_cookies.get(&slot) {
            return Some(token.clone());
        }
        // Fallback to first available session
        if let Some((_, token)) = session_cookies.iter().next() {
            return Some(token.clone());
        }
    }

    // Fallback to legacy cookie
    legacy_token
}

/// Authenticate using session token (from cookie)
/// Validates the signed JWT, looks up session in database, and updates last_accessed_at
async fn authenticate_session(state: &AppState, token: &str) -> Result<AuthenticatedUser, String> {
    // Decode and validate the session token
    let claims =
        decode_session_token(token).map_err(|_| "Invalid or expired session".to_string())?;

    // Get database connection
    let db = state
        .get_db()
        .await
        .map_err(|_| "Database not available".to_string())?;

    // Look up session in database
    let session = Session::find_by_id(&claims.sid)
        .one(&db)
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| "Session not found".to_string())?;

    // Check if session is revoked
    if session.is_revoked {
        return Err("Session has been revoked".to_string());
    }

    // Check if session is expired (double-check against DB value)
    if session.expires_at < Utc::now() {
        return Err("Session has expired".to_string());
    }

    // Fetch user from database
    let user = User::find_by_id(session.user_id)
        .filter(user::Column::IsActive.eq(true))
        .filter(user::Column::IsApproved.eq(true))
        .one(&db)
        .await
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| "User not found or inactive".to_string())?;

    // Update last_accessed_at (fire and forget - don't block on this)
    let session_id = session.id.clone();
    let shared_db = state.db.clone();
    tokio::spawn(async move {
        if let Some(db) = shared_db.read().await.clone() {
            let update = session::ActiveModel {
                id: Set(session_id),
                last_accessed_at: Set(Utc::now()),
                ..Default::default()
            };
            let _ = update.update(&db).await;
        }
    });

    // Fetch all permissions for this user from their roles
    let permissions = fetch_user_permissions(state, session.user_id).await;

    Ok(AuthenticatedUser { user, permissions })
}

/// Fetch all permissions for a user from their roles
async fn fetch_user_permissions(state: &AppState, user_id: i64) -> Vec<String> {
    // Get database connection
    let db = match state.get_db().await {
        Ok(db) => db,
        Err(_) => return Vec::new(),
    };

    // Get all role IDs for this user
    let user_roles = UserRole::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .all(&db)
        .await
        .unwrap_or_default();

    let role_ids: Vec<i64> = user_roles.iter().map(|ur| ur.role_id).collect();

    if role_ids.is_empty() {
        return Vec::new();
    }

    // Get all permissions from all roles
    let permissions = RolePermission::find()
        .filter(role_permission::Column::RoleId.is_in(role_ids.clone()))
        .all(&db)
        .await
        .unwrap_or_default();

    let mut perms: Vec<String> = permissions.iter().map(|p| p.permission.clone()).collect();

    // Get app permissions and convert to app.{name} format
    let app_permissions = RoleAppPermission::find()
        .filter(role_app_permission::Column::RoleId.is_in(role_ids))
        .all(&db)
        .await
        .unwrap_or_default();

    for app_perm in app_permissions {
        perms.push(format!("app.{}", app_perm.app_name));
    }

    // Deduplicate
    perms.sort();
    perms.dedup();
    perms
}

/// Create a 401 Unauthorized JSON response
fn unauthorized_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "detail": message
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_user(id: i64) -> user::Model {
        use chrono::Utc;
        user::Model {
            id,
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            hashed_password: "hash".to_string(),
            is_active: true,
            is_approved: true,
            totp_secret: None,
            totp_enabled: false,
            totp_verified_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_has_permission_present() {
        let user = AuthenticatedUser {
            user: fake_user(1),
            permissions: vec!["users.view".to_string(), "apps.view".to_string()],
        };
        assert!(user.has_permission("users.view"));
        assert!(user.has_permission("apps.view"));
    }

    #[test]
    fn test_has_permission_absent() {
        let user = AuthenticatedUser {
            user: fake_user(1),
            permissions: vec!["users.view".to_string()],
        };
        assert!(!user.has_permission("users.manage"));
        assert!(!user.has_permission("apps.install"));
    }

    #[test]
    fn test_has_permission_empty() {
        let user = AuthenticatedUser {
            user: fake_user(1),
            permissions: vec![],
        };
        assert!(!user.has_permission("any.permission"));
    }

    #[test]
    fn test_has_app_access_via_wildcard() {
        let user = AuthenticatedUser {
            user: fake_user(1),
            permissions: vec!["app.*".to_string()],
        };
        assert!(user.has_app_access("sonarr"));
        assert!(user.has_app_access("radarr"));
    }

    #[test]
    fn test_has_app_access_via_specific() {
        let user = AuthenticatedUser {
            user: fake_user(1),
            permissions: vec!["app.sonarr".to_string()],
        };
        assert!(user.has_app_access("sonarr"));
        assert!(!user.has_app_access("radarr"));
    }

    #[test]
    fn test_has_app_access_no_perms() {
        let user = AuthenticatedUser {
            user: fake_user(1),
            permissions: vec!["users.view".to_string()],
        };
        assert!(!user.has_app_access("sonarr"));
    }

    #[tokio::test]
    async fn test_unauthorized_response_status() {
        use http_body_util::BodyExt;
        let response = unauthorized_response("not logged in");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["detail"], "not logged in");
    }

    #[test]
    fn test_session_cookie_constants() {
        assert_eq!(SESSION_COOKIE_BASE, "kubarr_session");
        assert_eq!(SESSION_COOKIE_NAME, "kubarr_session");
        assert_eq!(ACTIVE_SESSION_COOKIE, "kubarr_active");
        assert!(MAX_SESSIONS > 0);
    }

    fn request_with_cookie(cookie: &str) -> Request {
        Request::builder()
            .header(header::COOKIE, cookie)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[test]
    fn extract_token_no_cookie() {
        let req = Request::builder().body(axum::body::Body::empty()).unwrap();
        assert_eq!(extract_token(&req), None);
    }

    #[test]
    fn extract_token_legacy_cookie() {
        let req = request_with_cookie("kubarr_session=legacytok");
        assert_eq!(extract_token(&req), Some("legacytok".to_string()));
    }

    #[test]
    fn extract_token_indexed_slot_zero_no_active() {
        // kubarr_session_0 present, no active cookie → falls back to slot 0
        let req = request_with_cookie("kubarr_session_0=tok0");
        assert_eq!(extract_token(&req), Some("tok0".to_string()));
    }

    #[test]
    fn extract_token_indexed_slot_with_active() {
        // slots 0 and 1 present, active = 1 → returns slot 1
        let req =
            request_with_cookie("kubarr_session_0=tok0; kubarr_session_1=tok1; kubarr_active=1");
        assert_eq!(extract_token(&req), Some("tok1".to_string()));
    }

    #[test]
    fn extract_token_active_slot_not_present_falls_back_to_first() {
        // active=2 but only slot 0 is present → falls back to first available
        let req = request_with_cookie("kubarr_session_0=tok0; kubarr_active=2");
        assert_eq!(extract_token(&req), Some("tok0".to_string()));
    }
}
