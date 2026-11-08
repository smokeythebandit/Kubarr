//! Kubarr as Jellyfin's identity provider.
//!
//! The Kubarr plugin inside Jellyfin owns every account it creates there and
//! asks Kubarr two things:
//!
//! - **verify**: is this username and password a Kubarr user allowed into
//!   Jellyfin? Native clients (TV, phone) sign in this way, since they cannot
//!   carry Kubarr's session cookie or answer a TOTP prompt. A user who has
//!   two-factor authentication must use an app password here.
//! - **snapshot**: which Kubarr users belong in Jellyfin, and which of them
//!   are administrators. The plugin disables any account it owns that is
//!   missing from the snapshot.
//!
//! Kubarr also pushes the snapshot to the plugin whenever users or roles
//! change, so a disabled user loses Jellyfin within seconds rather than at
//! the plugin's next poll.
//!
//! Both directions authenticate with [`managed_key::JELLYFIN_IDENTITY`], which
//! only Jellyfin's namespace holds.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};
use crate::models::prelude::*;
use crate::models::{app_password, role, role_app_permission, role_permission, user, user_role};
use crate::services::app_identity::is_app_admin;
use crate::services::managed_key;
use crate::services::security::verify_password;
use crate::state::AppState;

/// The app name Kubarr permissions use for Jellyfin (`app.jellyfin`).
pub const APP: &str = "jellyfin";

/// Path of the plugin's sync endpoint, relative to Jellyfin's base URL.
pub const PLUGIN_SYNC_PATH: &str = "/Kubarr/users/sync";

/// Prefix of generated app passwords, so a leaked one is recognisable.
pub const APP_PASSWORD_PREFIX: &str = "kbr_";

/// One Kubarr user as Jellyfin should see them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct IdentityUser {
    /// Kubarr user id; stable across renames, so the plugin links on it.
    pub user_id: String,
    pub username: String,
    pub email: String,
    pub is_admin: bool,
    /// False for a user who still has access through a role but whose Kubarr
    /// account is disabled or not yet approved.
    pub enabled: bool,
}

/// What the plugin receives on every sync, pushed or pulled.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct IdentitySnapshot {
    pub users: Vec<IdentityUser>,
}

/// Why a native sign-in was refused. Each maps to a status the plugin turns
/// into Jellyfin's generic "invalid username or password".
#[derive(Debug, PartialEq, Eq)]
pub enum VerifyRefusal {
    InvalidCredentials,
    NoAccess,
    Throttled,
}

impl From<VerifyRefusal> for AppError {
    fn from(refusal: VerifyRefusal) -> Self {
        match refusal {
            VerifyRefusal::InvalidCredentials => {
                AppError::Unauthorized("Invalid credentials".to_string())
            }
            VerifyRefusal::NoAccess => {
                AppError::Forbidden("No access to app: jellyfin".to_string())
            }
            VerifyRefusal::Throttled => {
                AppError::TooManyRequests("Too many failed sign-ins; try again later".to_string())
            }
        }
    }
}

// ----------------------------------------------------------------------------
// Snapshot
// ----------------------------------------------------------------------------

/// Build the list of Kubarr users who belong in Jellyfin. Four queries
/// regardless of user count, since the plugin polls this.
pub async fn snapshot(db: &DatabaseConnection) -> Result<IdentitySnapshot> {
    let users = User::find().all(db).await?;
    let user_roles = UserRole::find().all(db).await?;
    let role_permissions = RolePermission::find().all(db).await?;
    let role_apps = RoleAppPermission::find().all(db).await?;
    Ok(build_snapshot(
        users,
        &user_roles,
        &role_permissions,
        &role_apps,
    ))
}

fn build_snapshot(
    users: Vec<user::Model>,
    user_roles: &[user_role::Model],
    role_permissions: &[role_permission::Model],
    role_apps: &[role_app_permission::Model],
) -> IdentitySnapshot {
    let mut permissions_by_role: HashMap<i64, Vec<String>> = HashMap::new();
    for p in role_permissions {
        permissions_by_role
            .entry(p.role_id)
            .or_default()
            .push(p.permission.clone());
    }
    for a in role_apps {
        permissions_by_role
            .entry(a.role_id)
            .or_default()
            .push(format!("app.{}", a.app_name));
    }
    let mut roles_by_user: HashMap<i64, Vec<i64>> = HashMap::new();
    for ur in user_roles {
        roles_by_user
            .entry(ur.user_id)
            .or_default()
            .push(ur.role_id);
    }

    let mut out: Vec<IdentityUser> = users
        .into_iter()
        .filter_map(|u| {
            let permissions: Vec<String> = roles_by_user
                .get(&u.id)
                .into_iter()
                .flatten()
                .filter_map(|role_id| permissions_by_role.get(role_id))
                .flatten()
                .cloned()
                .collect();
            if !has_jellyfin_access(&permissions) {
                return None;
            }
            Some(IdentityUser {
                user_id: u.id.to_string(),
                username: u.username,
                email: u.email,
                is_admin: is_app_admin(&permissions),
                enabled: u.is_active && u.is_approved,
            })
        })
        .collect();
    out.sort_by(|a, b| a.username.cmp(&b.username));
    IdentitySnapshot { users: out }
}

fn has_jellyfin_access(permissions: &[String]) -> bool {
    permissions
        .iter()
        .any(|p| p == "app.*" || p == "app.jellyfin")
}

// ----------------------------------------------------------------------------
// Verify
// ----------------------------------------------------------------------------

/// Check a native client's credentials. On success returns the user as the
/// snapshot would list them.
pub async fn verify(
    db: &DatabaseConnection,
    state: &AppState,
    username: &str,
    password: &str,
) -> std::result::Result<IdentityUser, VerifyRefusal> {
    let throttle_key = username.trim().to_lowercase();
    if THROTTLE.is_blocked(&throttle_key) {
        return Err(VerifyRefusal::Throttled);
    }

    let outcome = verify_unthrottled(db, state, username, password).await;
    match &outcome {
        Err(VerifyRefusal::InvalidCredentials) => THROTTLE.record_failure(&throttle_key),
        Ok(_) => THROTTLE.clear(&throttle_key),
        _ => {}
    }
    outcome
}

async fn verify_unthrottled(
    db: &DatabaseConnection,
    state: &AppState,
    username: &str,
    password: &str,
) -> std::result::Result<IdentityUser, VerifyRefusal> {
    let found = User::find()
        .filter(
            user::Column::Username
                .eq(username)
                .or(user::Column::Email.eq(username)),
        )
        .one(db)
        .await
        .map_err(|_| VerifyRefusal::InvalidCredentials)?;
    let Some(found) = found else {
        // Spend a bcrypt verification anyway so timing does not reveal
        // whether the username exists.
        let _ = verify_password(password, &DUMMY_BCRYPT_HASH);
        return Err(VerifyRefusal::InvalidCredentials);
    };

    let needs_app_password = found.totp_enabled || role_requires_2fa(db, found.id).await;
    let credentials_ok = if needs_app_password {
        check_app_password(db, found.id, password).await
    } else {
        verify_password(password, &found.hashed_password)
            || check_app_password(db, found.id, password).await
    };
    if !credentials_ok {
        tracing::info!(
            user_id = found.id,
            "Jellyfin sign-in refused: bad credentials"
        );
        return Err(VerifyRefusal::InvalidCredentials);
    }

    // Credentials are right; only now reveal account state.
    if !found.is_active || !found.is_approved {
        return Err(VerifyRefusal::NoAccess);
    }
    let permissions = crate::middleware::auth::fetch_user_permissions(state, found.id).await;
    if !has_jellyfin_access(&permissions) {
        return Err(VerifyRefusal::NoAccess);
    }

    tracing::info!(user_id = found.id, "Jellyfin sign-in verified");
    Ok(IdentityUser {
        user_id: found.id.to_string(),
        username: found.username,
        email: found.email,
        is_admin: is_app_admin(&permissions),
        enabled: true,
    })
}

/// A bcrypt hash, at the cost real passwords use, of a string nobody knows.
static DUMMY_BCRYPT_HASH: Lazy<String> = Lazy::new(|| {
    crate::services::security::hash_password(&crate::services::security::generate_random_string(16))
        .unwrap_or_default()
});

async fn role_requires_2fa(db: &DatabaseConnection, user_id: i64) -> bool {
    Role::find()
        .inner_join(UserRole)
        .filter(user_role::Column::UserId.eq(user_id))
        .filter(role::Column::Requires2fa.eq(true))
        .one(db)
        .await
        .ok()
        .flatten()
        .is_some()
}

// ----------------------------------------------------------------------------
// App passwords
// ----------------------------------------------------------------------------

/// Generate an app password and the digest to store for it.
pub fn generate_app_password() -> (String, String) {
    let plain = format!(
        "{}{}",
        APP_PASSWORD_PREFIX,
        crate::services::security::generate_random_string(16)
    );
    let digest = digest_app_password(&plain);
    (plain, digest)
}

pub fn digest_app_password(plain: &str) -> String {
    hex::encode(Sha256::digest(plain.trim().as_bytes()))
}

async fn check_app_password(db: &DatabaseConnection, user_id: i64, password: &str) -> bool {
    if !password.trim().starts_with(APP_PASSWORD_PREFIX) {
        return false;
    }
    let digest = digest_app_password(password);
    let Ok(Some(found)) = AppPassword::find()
        .filter(app_password::Column::TokenHash.eq(digest))
        .filter(app_password::Column::UserId.eq(user_id))
        .one(db)
        .await
    else {
        return false;
    };
    let touch = app_password::ActiveModel {
        id: Set(found.id),
        last_used_at: Set(Some(Utc::now())),
        ..Default::default()
    };
    let _ = touch.update(db).await;
    true
}

// ----------------------------------------------------------------------------
// Throttling
// ----------------------------------------------------------------------------

/// Failed native sign-ins allowed per username within [`THROTTLE_WINDOW`].
/// Jellyfin is reachable without a Kubarr session, so this is what stands
/// between the internet and password guessing. Per replica: with N API
/// replicas an attacker gets at most N times this.
const THROTTLE_LIMIT: u32 = 5;
const THROTTLE_WINDOW: Duration = Duration::from_secs(300);

static THROTTLE: Lazy<Throttle> = Lazy::new(Throttle::default);

#[derive(Default)]
struct Throttle {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

impl Throttle {
    fn is_blocked(&self, key: &str) -> bool {
        let mut map = self.failures.lock();
        match map.get(key) {
            Some((_, since)) if since.elapsed() > THROTTLE_WINDOW => {
                map.remove(key);
                false
            }
            Some((count, _)) => *count >= THROTTLE_LIMIT,
            None => false,
        }
    }

    fn record_failure(&self, key: &str) {
        let mut map = self.failures.lock();
        // Bound memory against a flood of distinct usernames.
        if map.len() > 10_000 {
            map.retain(|_, (_, since)| since.elapsed() <= THROTTLE_WINDOW);
        }
        let entry = map.entry(key.to_string()).or_insert((0, Instant::now()));
        if entry.1.elapsed() > THROTTLE_WINDOW {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
    }

    fn clear(&self, key: &str) {
        self.failures.lock().remove(key);
    }
}

// ----------------------------------------------------------------------------
// Service authentication and push
// ----------------------------------------------------------------------------

/// Whether `presented` is the Jellyfin identity key. Constant-time in the
/// key's length.
pub async fn is_identity_key(db: &DatabaseConnection, presented: &str) -> Result<bool> {
    let expected = managed_key::JELLYFIN_IDENTITY.api_key(db).await?;
    Ok(constant_time_eq(expected.as_bytes(), presented.as_bytes()))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Push the current snapshot to Jellyfin in the background. Errors are
/// logged, never returned: the plugin also polls, so a missed push only
/// delays a change.
pub fn notify_changed(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        // Collapse a burst of changes (a role edit touches several rows).
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Err(e) = push(&state).await {
            tracing::debug!("Jellyfin identity push skipped: {}", e);
        }
    });
}

async fn push(state: &AppState) -> Result<()> {
    let db = state.get_db().await?;
    // Never mint the key just to push: no key means Jellyfin was never
    // installed with the plugin.
    if SystemSetting::find_by_id(managed_key::JELLYFIN_IDENTITY.setting_key)
        .one(&db)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let (base_url, base_path, _) =
        crate::endpoints::proxy::get_app_upstream_url(state, APP).await?;
    let key = managed_key::JELLYFIN_IDENTITY.api_key(&db).await?;
    let body = snapshot(&db).await?;
    let url = format!(
        "{}{}{}",
        base_url,
        base_path.unwrap_or_default().trim_end_matches('/'),
        PLUGIN_SYNC_PATH
    );
    let response = push_client()
        .post(&url)
        .header("X-Kubarr-Key", key)
        .json(&body)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(AppError::Internal(format!(
            "Jellyfin plugin answered {}",
            response.status()
        )));
    }
    tracing::debug!(
        users = body.users.len(),
        "Pushed identity snapshot to Jellyfin"
    );
    Ok(())
}

fn push_client() -> Arc<reqwest::Client> {
    static CLIENT: Lazy<Arc<reqwest::Client>> = Lazy::new(|| {
        Arc::new(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        )
    });
    CLIENT.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: i64, name: &str, active: bool) -> user::Model {
        user::Model {
            id,
            username: name.into(),
            email: format!("{name}@example.com"),
            hashed_password: String::new(),
            is_active: active,
            is_approved: true,
            totp_secret: None,
            totp_enabled: false,
            totp_verified_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn ur(user_id: i64, role_id: i64) -> user_role::Model {
        user_role::Model { user_id, role_id }
    }

    fn perm(role_id: i64, permission: &str) -> role_permission::Model {
        role_permission::Model {
            id: 0,
            role_id,
            permission: permission.into(),
        }
    }

    fn app(role_id: i64, app_name: &str) -> role_app_permission::Model {
        role_app_permission::Model {
            id: 0,
            role_id,
            app_name: app_name.into(),
        }
    }

    #[test]
    fn snapshot_lists_only_users_with_jellyfin_and_maps_admin() {
        // role 1 = admin (app.* + settings.manage), 2 = viewer (jellyfin),
        // 3 = downloader (qbittorrent only)
        let snapshot = build_snapshot(
            vec![
                user(1, "root", true),
                user(2, "viewer", true),
                user(3, "dl", true),
                user(4, "gone", false),
            ],
            &[ur(1, 1), ur(2, 2), ur(3, 3), ur(4, 2)],
            &[perm(1, "app.*"), perm(1, "settings.manage")],
            &[app(2, "jellyfin"), app(3, "qbittorrent")],
        );
        let by_name: HashMap<_, _> = snapshot
            .users
            .iter()
            .map(|u| (u.username.as_str(), u))
            .collect();
        assert_eq!(by_name.len(), 3, "{snapshot:?}");
        assert!(by_name["root"].is_admin);
        assert!(!by_name["viewer"].is_admin);
        assert!(by_name["viewer"].enabled);
        assert!(!by_name["gone"].enabled);
        assert!(!by_name.contains_key("dl"));
    }

    #[test]
    fn app_passwords_are_prefixed_and_digest_stably() {
        let (plain, digest) = generate_app_password();
        assert!(plain.starts_with(APP_PASSWORD_PREFIX));
        assert_eq!(plain.len(), APP_PASSWORD_PREFIX.len() + 32);
        assert_eq!(digest, digest_app_password(&plain));
        assert_eq!(digest, digest_app_password(&format!(" {plain} ")));
        assert_ne!(digest, generate_app_password().1);
    }

    #[test]
    fn throttle_blocks_after_limit_and_clears_on_success() {
        let t = Throttle::default();
        for _ in 0..THROTTLE_LIMIT {
            assert!(!t.is_blocked("bob"));
            t.record_failure("bob");
        }
        assert!(t.is_blocked("bob"));
        assert!(!t.is_blocked("carol"));
        t.clear("bob");
        assert!(!t.is_blocked("bob"));
    }

    #[test]
    fn constant_time_eq_compares_exactly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
