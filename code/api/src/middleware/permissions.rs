//! Permission system with type-safe authorization extractors
//!
//! Usage in handlers:
//! ```ignore
//! use crate::middleware::{Authorized, permissions::*};
//!
//! async fn list_users(
//!     Authorized(user): Authorized<UsersView>,
//!     State(state): State<AppState>,
//! ) -> Result<Json<Vec<User>>> {
//!     // Permission already verified - just use user
//! }
//! ```

use std::marker::PhantomData;

use axum::{extract::FromRequestParts, http::request::Parts};

use crate::error::AppError;
use crate::middleware::AuthenticatedUser;
use crate::models::user;

/// Trait for permission marker types
pub trait Permission: Send + Sync + 'static {
    /// The permission string (e.g., "users.view")
    const NAME: &'static str;
}

/// Macro to define permission types
///
/// Creates zero-sized marker types that implement `Permission`
macro_rules! define_permissions {
    ($($(#[$meta:meta])* $name:ident => $perm:expr),* $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(Debug, Clone, Copy)]
            pub struct $name;

            impl Permission for $name {
                const NAME: &'static str = $perm;
            }
        )*
    };
}

// Define all application permissions
define_permissions! {
    // User management
    /// View users list and details
    UsersView => "users.view",
    /// Create, update, delete users
    UsersManage => "users.manage",
    /// Reset other users' passwords
    UsersResetPassword => "users.reset_password",

    // Role management
    /// View roles list and details
    RolesView => "roles.view",
    /// Create, update, delete roles
    RolesManage => "roles.manage",

    // App management
    /// View installed apps
    AppsView => "apps.view",
    /// Install new apps
    AppsInstall => "apps.install",
    /// Delete/uninstall apps
    AppsDelete => "apps.delete",
    /// Restart apps
    AppsRestart => "apps.restart",

    // Storage management
    /// Browse and view storage
    StorageView => "storage.view",
    /// Create directories, upload files
    StorageWrite => "storage.write",
    /// Delete files and directories
    StorageDelete => "storage.delete",
    /// Download files
    StorageDownload => "storage.download",

    // Logs
    /// View application logs
    LogsView => "logs.view",

    // Monitoring
    /// View metrics and monitoring data
    MonitoringView => "monitoring.view",

    // Settings
    /// View system settings
    SettingsView => "settings.view",
    /// Modify system settings
    SettingsManage => "settings.manage",

    // Audit
    /// View audit logs
    AuditView => "audit.view",
    /// Manage audit logs (clear old entries)
    AuditManage => "audit.manage",

    // Notifications
    /// View notifications
    NotificationsView => "notifications.view",
    /// Manage notification settings
    NotificationsManage => "notifications.manage",

    // Networking
    /// View network topology
    NetworkingView => "networking.view",

    // VPN
    /// View VPN providers and app VPN configurations
    VpnView => "vpn.view",
    /// Manage VPN providers and assign VPN to apps
    VpnManage => "vpn.manage",

}

/// Extractor that requires a specific permission
///
/// This extractor verifies that the authenticated user has the required
/// permission before the handler is called. If the permission check fails,
/// a 403 Forbidden error is returned.
///
/// # Example
/// ```ignore
/// async fn delete_user(
///     Authorized(user): Authorized<UsersManage>,
///     Path(id): Path<i64>,
/// ) -> Result<()> {
///     // User is guaranteed to have "users.manage" permission
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Authorized<P: Permission>(pub user::Model, PhantomData<P>);

impl<P: Permission> Authorized<P> {
    /// Get the authenticated user
    pub fn user(&self) -> &user::Model {
        &self.0
    }

    /// Get the user ID
    pub fn user_id(&self) -> i64 {
        self.0.id
    }
}

impl<S, P> FromRequestParts<S> for Authorized<P>
where
    S: Send + Sync,
    P: Permission,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Get authenticated user from extensions (set by auth middleware)
        let auth_user = parts
            .extensions
            .get::<AuthenticatedUser>()
            .ok_or_else(|| AppError::Unauthorized("Authentication required".to_string()))?;

        // Check if user has the required permission
        if !auth_user.has_permission(P::NAME) {
            return Err(AppError::Forbidden(format!(
                "Permission denied: {} required",
                P::NAME
            )));
        }

        Ok(Authorized(auth_user.user.clone(), PhantomData))
    }
}

/// Extractor for any authenticated user (no specific permission required)
///
/// Use this when you just need to verify the user is authenticated
/// but don't need a specific permission.
#[derive(Debug, Clone)]
pub struct Authenticated(pub user::Model);

impl Authenticated {
    /// Get the authenticated user
    pub fn user(&self) -> &user::Model {
        &self.0
    }

    /// Get the user ID
    pub fn user_id(&self) -> i64 {
        self.0.id
    }
}

impl<S> FromRequestParts<S> for Authenticated
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_user = parts
            .extensions
            .get::<AuthenticatedUser>()
            .ok_or_else(|| AppError::Unauthorized("Authentication required".to_string()))?;

        Ok(Authenticated(auth_user.user.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_user(id: i64, username: &str) -> crate::models::user::Model {
        use chrono::Utc;
        crate::models::user::Model {
            id,
            username: username.to_string(),
            email: format!("{username}@test.com"),
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
    fn test_permission_names() {
        assert_eq!(UsersView::NAME, "users.view");
        assert_eq!(UsersManage::NAME, "users.manage");
        assert_eq!(UsersResetPassword::NAME, "users.reset_password");
        assert_eq!(RolesView::NAME, "roles.view");
        assert_eq!(RolesManage::NAME, "roles.manage");
        assert_eq!(AppsView::NAME, "apps.view");
        assert_eq!(AppsInstall::NAME, "apps.install");
        assert_eq!(AppsDelete::NAME, "apps.delete");
        assert_eq!(AppsRestart::NAME, "apps.restart");
        assert_eq!(StorageView::NAME, "storage.view");
        assert_eq!(StorageWrite::NAME, "storage.write");
        assert_eq!(StorageDelete::NAME, "storage.delete");
        assert_eq!(StorageDownload::NAME, "storage.download");
        assert_eq!(LogsView::NAME, "logs.view");
        assert_eq!(MonitoringView::NAME, "monitoring.view");
        assert_eq!(SettingsView::NAME, "settings.view");
        assert_eq!(SettingsManage::NAME, "settings.manage");
        assert_eq!(AuditView::NAME, "audit.view");
        assert_eq!(AuditManage::NAME, "audit.manage");
        assert_eq!(NotificationsView::NAME, "notifications.view");
        assert_eq!(NotificationsManage::NAME, "notifications.manage");
        assert_eq!(NetworkingView::NAME, "networking.view");
        assert_eq!(VpnView::NAME, "vpn.view");
        assert_eq!(VpnManage::NAME, "vpn.manage");
    }

    #[test]
    fn test_authorized_user_accessors() {
        let user = fake_user(42, "testuser");
        let auth: Authorized<UsersView> = Authorized(user.clone(), PhantomData);
        assert_eq!(auth.user_id(), 42);
        assert_eq!(auth.user().username, "testuser");
    }

    #[test]
    fn test_authenticated_user_accessors() {
        let user = fake_user(7, "viewer");
        let auth = Authenticated(user.clone());
        assert_eq!(auth.user_id(), 7);
        assert_eq!(auth.user().username, "viewer");
    }

    #[test]
    fn test_permission_type_is_copy() {
        // Permission marker types should be Copy and Clone
        let _p1 = UsersView;
        let _p2 = _p1;
        let _p3 = _p2;
    }

    fn make_auth_user(
        user: crate::models::user::Model,
        permissions: Vec<&str>,
    ) -> crate::middleware::AuthenticatedUser {
        crate::middleware::AuthenticatedUser {
            user,
            permissions: permissions.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn authorized_extractor_ok_when_permission_present() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        let user = fake_user(1, "alice");
        let auth_user = make_auth_user(user, vec!["users.view"]);

        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(auth_user);

        let (mut parts, _) = req.into_parts();
        let result = Authorized::<UsersView>::from_request_parts(&mut parts, &()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().user_id(), 1);
    }

    #[tokio::test]
    async fn authorized_extractor_forbidden_when_permission_missing() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        let user = fake_user(2, "bob");
        let auth_user = make_auth_user(user, vec!["roles.view"]);

        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(auth_user);

        let (mut parts, _) = req.into_parts();
        let result = Authorized::<UsersManage>::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn authorized_extractor_unauthorized_when_no_extension() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        let req = Request::builder().body(()).unwrap();
        let (mut parts, _) = req.into_parts();
        let result = Authorized::<UsersView>::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::AppError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn authenticated_extractor_ok_when_user_present() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        let user = fake_user(3, "carol");
        let auth_user = make_auth_user(user, vec![]);

        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(auth_user);

        let (mut parts, _) = req.into_parts();
        let result = Authenticated::from_request_parts(&mut parts, &()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().user_id(), 3);
    }

    #[tokio::test]
    async fn authenticated_extractor_unauthorized_when_no_extension() {
        use axum::extract::FromRequestParts;
        use axum::http::Request;

        let req = Request::builder().body(()).unwrap();
        let (mut parts, _) = req.into_parts();
        let result = Authenticated::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::AppError::Unauthorized(_)));
    }
}
