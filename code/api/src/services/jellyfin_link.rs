//! One Jellyfin API key, shared by Jellyfin and the apps that refresh its library.
//!
//! Jellyfin issues API keys only to a signed-in administrator, and a fresh
//! install has none until its setup wizard is finished. Kubarr therefore
//! generates the key itself, keeps it in `system_settings`, and places it in a
//! Secret in each participating app namespace. The Jellyfin chart registers it
//! in Jellyfin's database; the Sonarr and Radarr charts add a Jellyfin
//! connection with it. The key never travels as a Helm value.
//!
//! The mechanism is shared with the other keys Kubarr owns on an
//! application's behalf; see [`crate::services::managed_key`]. This module is
//! the Jellyfin link's name for it.

use sea_orm::DatabaseConnection;

use crate::error::Result;
use crate::services::managed_key::{ManagedKey, JELLYFIN};
use crate::services::K8sClient;

/// Secret created in each participating namespace; the charts read `api-key`.
pub const SECRET_NAME: &str = JELLYFIN.secret_name;
/// The Helm value tree Kubarr manages for this link.
pub const HELM_ROOT: &str = JELLYFIN.helm_root;

/// The link itself, for callers that handle every managed key alike.
pub const LINK: &ManagedKey = &JELLYFIN;

/// Apps whose charts take part in the Jellyfin link.
pub fn applies_to(app_name: &str) -> bool {
    JELLYFIN.applies_to(app_name)
}

/// Return the shared key, creating it on first use.
pub async fn api_key(db: &DatabaseConnection) -> Result<String> {
    JELLYFIN.api_key(db).await
}

/// Create or update the Secret in an app namespace before Helm runs.
pub async fn ensure_secret(k8s: &K8sClient, namespace: &str, key: &str) -> Result<()> {
    JELLYFIN.ensure_secret(k8s, namespace, key).await
}

/// `--set` values that switch the link on (Some) or off (None).
pub fn helm_values(secret_name: Option<&str>) -> String {
    JELLYFIN.helm_values(secret_name)
}
