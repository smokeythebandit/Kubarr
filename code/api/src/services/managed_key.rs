//! Shared machinery for an API key Kubarr owns on an application's behalf.
//!
//! Some applications need a key that two charts must agree on before either
//! has run: Jellyfin and the managers that refresh its library, SABnzbd and
//! the managers that queue downloads with it. Shipping a fixed key in the
//! chart makes it a published default credential, and letting each
//! application generate its own leaves the other side unable to guess it.
//!
//! So Kubarr generates the key, keeps it in `system_settings`, and writes it
//! into a Secret in each participating namespace before Helm renders a Pod
//! that mounts it. Only the Secret's name travels as a Helm value; the key
//! itself never appears in an argument list, a log line or a chart.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ObjectMeta, Patch, PatchParams};
use sea_orm::sea_query::OnConflict;
use sea_orm::{DatabaseConnection, EntityTrait, Set};

use crate::error::{AppError, Result};
use crate::models::system_setting;
use crate::services::K8sClient;

/// The data key inside every managed Secret; the charts read this name.
pub const SECRET_KEY: &str = "api-key";

/// One key, the applications that share it, and where it is kept.
pub struct ManagedKey {
    /// Row in `system_settings` holding the generated key.
    pub setting_key: &'static str,
    /// Secret created in each participating namespace.
    pub secret_name: &'static str,
    /// The Helm value tree Kubarr manages for this link.
    pub helm_root: &'static str,
    /// What the key is for, stored alongside it and shown in diagnostics.
    pub description: &'static str,
    /// Charts that take part. An app outside this list gets no Secret and no
    /// value, so an unrelated chart cannot be handed a credential.
    pub participants: &'static [&'static str],
    /// Sentence naming this link in the error that refuses a hand-set value.
    pub refusal: &'static str,
}

/// Jellyfin issues API keys only to a signed-in administrator, and a fresh
/// install has none until its setup wizard is finished, so the key cannot be
/// read out of the application.
pub const JELLYFIN: ManagedKey = ManagedKey {
    setting_key: "jellyfin_api_key",
    secret_name: "kubarr-jellyfin-api-key",
    helm_root: "jellyfinIntegration",
    description: "API key Sonarr and Radarr use to refresh the Jellyfin library",
    participants: &["jellyfin", "sonarr", "radarr"],
    refusal: "sets the Jellyfin link, which Kubarr manages",
};

/// SABnzbd generates its own key on first start and keeps it in `sabnzbd.ini`,
/// where no other pod can read it, so the managers could never authenticate
/// without a key both sides are told in advance.
pub const SABNZBD: ManagedKey = ManagedKey {
    setting_key: "sabnzbd_api_key",
    secret_name: "kubarr-sabnzbd-api-key",
    helm_root: "sabnzbdIntegration",
    description: "API key Radarr, Sonarr and Lidarr use to queue downloads with SABnzbd",
    participants: &["sabnzbd", "radarr", "sonarr", "lidarr"],
    refusal: "sets the SABnzbd link, which Kubarr manages",
};

/// Every managed key, so a new one is wired into deployment by adding it here.
pub const ALL: &[&ManagedKey] = &[&JELLYFIN, &SABNZBD];

impl ManagedKey {
    pub fn applies_to(&self, app_name: &str) -> bool {
        self.participants.contains(&app_name)
    }

    /// Return the shared key, creating it on first use. Concurrent first
    /// installs agree on one key: the insert is a no-op if another caller won
    /// the race, and every caller then reads back what was stored.
    pub async fn api_key(&self, db: &DatabaseConnection) -> Result<String> {
        if let Some(existing) = system_setting::Entity::find_by_id(self.setting_key)
            .one(db)
            .await?
        {
            return self.checked(existing.value);
        }
        let candidate = system_setting::ActiveModel {
            key: Set(self.setting_key.to_string()),
            value: Set(generate_key()),
            description: Set(Some(self.description.to_string())),
            updated_at: Set(chrono::Utc::now()),
        };
        system_setting::Entity::insert(candidate)
            .on_conflict(
                OnConflict::column(system_setting::Column::Key)
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(db)
            .await?;
        let stored = system_setting::Entity::find_by_id(self.setting_key)
            .one(db)
            .await?
            .ok_or_else(|| {
                AppError::Internal(format!("{} was not stored", self.setting_key))
            })?;
        self.checked(stored.value)
    }

    fn checked(&self, key: String) -> Result<String> {
        if valid_key(&key) {
            Ok(key)
        } else {
            Err(AppError::Internal(format!(
                "Stored {} is malformed",
                self.setting_key
            )))
        }
    }

    fn secret(&self, namespace: &str, key: &str) -> Secret {
        Secret {
            metadata: ObjectMeta {
                name: Some(self.secret_name.to_string()),
                namespace: Some(namespace.to_string()),
                labels: Some(BTreeMap::from([(
                    "app.kubernetes.io/managed-by".to_string(),
                    "kubarr".to_string(),
                )])),
                ..Default::default()
            },
            string_data: Some(BTreeMap::from([(SECRET_KEY.to_string(), key.to_string())])),
            type_: Some("Opaque".to_string()),
            ..Default::default()
        }
    }

    /// Create or update the Secret in an app namespace before Helm runs.
    pub async fn ensure_secret(&self, k8s: &K8sClient, namespace: &str, key: &str) -> Result<()> {
        let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), namespace);
        secrets
            .patch(
                self.secret_name,
                &PatchParams::apply("kubarr").force(),
                &Patch::Apply(&self.secret(namespace, key)),
            )
            .await?;
        Ok(())
    }

    /// `--set` value that switches the link on (Some) or off (None). Both are
    /// explicit so `--reuse-values` cannot keep a stale assignment.
    pub fn helm_values(&self, secret_name: Option<&str>) -> String {
        format!("{}.secretName={}", self.helm_root, secret_name.unwrap_or(""))
    }
}

/// A GUID without separators: Jellyfin's own key format, and what SABnzbd
/// generates for itself, so neither application sees anything unusual.
fn generate_key() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn valid_key(key: &str) -> bool {
    key.len() == 32
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_match_the_applications_format() {
        let key = generate_key();
        assert!(valid_key(&key), "{key}");
        assert_ne!(key, generate_key());
    }

    #[test]
    fn rejects_malformed_keys() {
        assert!(!valid_key(""));
        assert!(!valid_key(&"A".repeat(32)));
        assert!(!valid_key(&"a".repeat(31)));
        assert!(!valid_key("0123456789abcdef0123456789abcde,"));
        assert!(JELLYFIN.checked("x".into()).is_err());
        assert!(SABNZBD.checked("x".into()).is_err());
    }

    #[test]
    fn each_link_covers_only_the_charts_that_share_its_key() {
        for app in ["jellyfin", "sonarr", "radarr"] {
            assert!(JELLYFIN.applies_to(app), "{app}");
        }
        for app in ["plex", "jellyseerr", "prowlarr", "qbittorrent", "lidarr"] {
            assert!(!JELLYFIN.applies_to(app), "{app}");
        }
        for app in ["sabnzbd", "radarr", "sonarr", "lidarr"] {
            assert!(SABNZBD.applies_to(app), "{app}");
        }
        // The other download clients need no key: they are reached over their
        // own credentials or none at all.
        for app in ["qbittorrent", "transmission", "deluge", "rutorrent", "jellyfin"] {
            assert!(!SABNZBD.applies_to(app), "{app}");
        }
    }

    #[test]
    fn links_never_share_a_setting_secret_or_helm_root() {
        for field in [
            ALL.iter().map(|link| link.setting_key).collect::<Vec<_>>(),
            ALL.iter().map(|link| link.secret_name).collect::<Vec<_>>(),
            ALL.iter().map(|link| link.helm_root).collect::<Vec<_>>(),
        ] {
            let mut unique = field.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), field.len(), "{field:?}");
        }
    }

    #[test]
    fn secret_holds_only_the_key_under_the_chart_contract() {
        let secret = SABNZBD.secret("radarr", "0123456789abcdef0123456789abcdef");
        assert_eq!(secret.metadata.name.as_deref(), Some("kubarr-sabnzbd-api-key"));
        assert_eq!(secret.metadata.namespace.as_deref(), Some("radarr"));
        let data = secret.string_data.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[SECRET_KEY], "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn helm_values_never_carry_the_key() {
        assert_eq!(
            JELLYFIN.helm_values(Some(JELLYFIN.secret_name)),
            "jellyfinIntegration.secretName=kubarr-jellyfin-api-key"
        );
        assert_eq!(
            SABNZBD.helm_values(Some(SABNZBD.secret_name)),
            "sabnzbdIntegration.secretName=kubarr-sabnzbd-api-key"
        );
        for link in ALL {
            let off = link.helm_values(None);
            assert!(off.ends_with(".secretName="), "{off}");
        }
    }
}
