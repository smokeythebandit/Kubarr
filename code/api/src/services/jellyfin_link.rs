//! One Jellyfin API key, shared by Jellyfin and the apps that refresh its library.
//!
//! Jellyfin issues API keys only to a signed-in administrator, and a fresh
//! install has none until its setup wizard is finished. Kubarr therefore
//! generates the key itself, keeps it in `system_settings`, and places it in a
//! Secret in each participating app namespace. The Jellyfin chart registers it
//! in Jellyfin's database; the Sonarr and Radarr charts add a Jellyfin
//! connection with it. The key never travels as a Helm value.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ObjectMeta, Patch, PatchParams};
use sea_orm::sea_query::OnConflict;
use sea_orm::{DatabaseConnection, EntityTrait, Set};

use crate::error::{AppError, Result};
use crate::models::system_setting;
use crate::services::K8sClient;

/// Secret created in each participating namespace; the charts read `api-key`.
pub const SECRET_NAME: &str = "kubarr-jellyfin-api-key";
const SECRET_KEY: &str = "api-key";
const SETTING_KEY: &str = "jellyfin_api_key";
/// The Helm value tree Kubarr manages for this link.
pub const HELM_ROOT: &str = "jellyfinIntegration";

/// Apps whose charts take part in the Jellyfin link.
pub fn applies_to(app_name: &str) -> bool {
    matches!(app_name, "jellyfin" | "sonarr" | "radarr")
}

/// Jellyfin's own key format: a GUID without separators.
fn generate_key() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn valid_key(key: &str) -> bool {
    key.len() == 32
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Return the shared key, creating it on first use. Concurrent first installs
/// agree on one key: the insert is a no-op if another caller won the race.
pub async fn api_key(db: &DatabaseConnection) -> Result<String> {
    if let Some(existing) = system_setting::Entity::find_by_id(SETTING_KEY)
        .one(db)
        .await?
    {
        return checked(existing.value);
    }
    let candidate = system_setting::ActiveModel {
        key: Set(SETTING_KEY.to_string()),
        value: Set(generate_key()),
        description: Set(Some(
            "API key Sonarr and Radarr use to refresh the Jellyfin library".to_string(),
        )),
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
    let stored = system_setting::Entity::find_by_id(SETTING_KEY)
        .one(db)
        .await?
        .ok_or_else(|| AppError::Internal("Jellyfin API key was not stored".into()))?;
    checked(stored.value)
}

fn checked(key: String) -> Result<String> {
    if valid_key(&key) {
        Ok(key)
    } else {
        Err(AppError::Internal(
            "Stored Jellyfin API key is malformed".into(),
        ))
    }
}

fn secret(namespace: &str, key: &str) -> Secret {
    Secret {
        metadata: ObjectMeta {
            name: Some(SECRET_NAME.to_string()),
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
pub async fn ensure_secret(k8s: &K8sClient, namespace: &str, key: &str) -> Result<()> {
    let secrets: Api<Secret> = Api::namespaced(k8s.client().clone(), namespace);
    secrets
        .patch(
            SECRET_NAME,
            &PatchParams::apply("kubarr").force(),
            &Patch::Apply(&secret(namespace, key)),
        )
        .await?;
    Ok(())
}

/// `--set` values that switch the link on (Some) or off (None). Both are
/// explicit so `--reuse-values` cannot keep a stale assignment.
pub fn helm_values(secret_name: Option<&str>) -> String {
    format!("{HELM_ROOT}.secretName={}", secret_name.unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_match_jellyfins_format() {
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
        assert!(checked("x".into()).is_err());
    }

    #[test]
    fn only_jellyfin_and_its_library_writers_take_part() {
        for app in ["jellyfin", "sonarr", "radarr"] {
            assert!(applies_to(app), "{app}");
        }
        for app in ["plex", "jellyseerr", "prowlarr", "qbittorrent"] {
            assert!(!applies_to(app), "{app}");
        }
    }

    #[test]
    fn secret_holds_only_the_key_under_the_chart_contract() {
        let secret = secret("sonarr", "0123456789abcdef0123456789abcdef");
        assert_eq!(secret.metadata.name.as_deref(), Some(SECRET_NAME));
        assert_eq!(secret.metadata.namespace.as_deref(), Some("sonarr"));
        let data = secret.string_data.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[SECRET_KEY], "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn helm_values_never_carry_the_key() {
        assert_eq!(
            helm_values(Some(SECRET_NAME)),
            "jellyfinIntegration.secretName=kubarr-jellyfin-api-key"
        );
        assert_eq!(helm_values(None), "jellyfinIntegration.secretName=");
    }
}
