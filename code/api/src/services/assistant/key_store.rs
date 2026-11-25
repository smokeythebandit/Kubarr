//! Where the Anthropic API key lives: only in the Kubernetes Secret
//! `kubarr-anthropic-api-key` (data key `api-key`) in Kubarr's own namespace.
//! Never in the database, never in a response, never in a log line.

use std::collections::BTreeMap;

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, DeleteParams, ObjectMeta, Patch, PatchParams};
use parking_lot::Mutex;

use crate::error::{AppError, Result};
use crate::state::SharedK8sClient;

/// Name of the Secret holding the key.
pub const SECRET_NAME: &str = "kubarr-anthropic-api-key";
/// Data key inside the Secret.
pub const SECRET_KEY: &str = "api-key";

const KEY_PREFIX: &str = "sk-ant-";
const KEY_MIN_LEN: usize = 32;
const KEY_MAX_LEN: usize = 256;

/// Storage for the API key.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait KeyStore: Send + Sync {
    /// The key, if one is stored.
    async fn get(&self) -> Result<Option<String>>;
    /// Store (or replace) the key.
    async fn set(&self, key: &str) -> Result<()>;
    /// Remove the key; succeeds when none is stored.
    async fn delete(&self) -> Result<()>;
}

/// Trim and check the shape of a key someone pasted. The error never repeats
/// the input.
pub fn normalize_key(raw: &str) -> Result<String> {
    let key = raw.trim();
    let shape_ok = key.starts_with(KEY_PREFIX)
        && (KEY_MIN_LEN..=KEY_MAX_LEN).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if shape_ok {
        Ok(key.to_string())
    } else {
        Err(AppError::BadRequest(format!(
            "api_key: expected an Anthropic API key starting with '{KEY_PREFIX}' \
             ({KEY_MIN_LEN}-{KEY_MAX_LEN} letters, digits, '-' or '_')"
        )))
    }
}

/// Replace the key, and anything shaped like an Anthropic key, in `text`.
pub fn redact(text: &str, key: &str) -> String {
    let mut out = if key.is_empty() {
        text.to_string()
    } else {
        text.replace(key, "[redacted]")
    };
    while let Some(start) = out.find(KEY_PREFIX) {
        let end = out[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .map_or(out.len(), |offset| start + offset);
        out.replace_range(start..end, "[redacted]");
    }
    out
}

/// The production store: a Secret in the backend's namespace.
pub struct SecretKeyStore {
    k8s: SharedK8sClient,
    /// Explicit namespace; `None` uses the client's own namespace (in a
    /// cluster, the namespace of the backend's service account).
    namespace: Option<String>,
}

impl SecretKeyStore {
    pub fn new(k8s: SharedK8sClient, namespace: Option<String>) -> Self {
        Self { k8s, namespace }
    }

    /// `KUBARR_NAMESPACE` or `POD_NAMESPACE` when set, else the client's
    /// namespace.
    pub fn from_env(k8s: SharedK8sClient) -> Self {
        let namespace = ["KUBARR_NAMESPACE", "POD_NAMESPACE"]
            .into_iter()
            .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
        Self::new(k8s, namespace)
    }

    async fn api(&self) -> Result<(Api<Secret>, String)> {
        let guard = self.k8s.read().await;
        let client = guard.as_ref().ok_or_else(|| {
            AppError::ServiceUnavailable("Kubernetes client not available".into())
        })?;
        let namespace = self
            .namespace
            .clone()
            .unwrap_or_else(|| client.client().default_namespace().to_string());
        Ok((
            Api::namespaced(client.client().clone(), &namespace),
            namespace,
        ))
    }
}

/// Kubernetes errors for the Secret, without the server's message (it never
/// contains Secret data, but the status code is all a caller needs).
fn kube_error(action: &str, error: kube::Error) -> AppError {
    let code = match &error {
        kube::Error::Api(status) => status.code.to_string(),
        _ => "transport".to_string(),
    };
    tracing::warn!(secret = SECRET_NAME, code = %code, "Failed to {action} the assistant API key Secret");
    AppError::Internal(format!("Failed to {action} the assistant API key ({code})"))
}

#[async_trait]
impl KeyStore for SecretKeyStore {
    async fn get(&self) -> Result<Option<String>> {
        let (api, _) = self.api().await?;
        let secret = api
            .get_opt(SECRET_NAME)
            .await
            .map_err(|e| kube_error("read", e))?;
        Ok(secret
            .and_then(|secret| secret.data)
            .and_then(|mut data| data.remove(SECRET_KEY))
            .and_then(|bytes| String::from_utf8(bytes.0).ok())
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty()))
    }

    async fn set(&self, key: &str) -> Result<()> {
        let (api, namespace) = self.api().await?;
        let secret = Secret {
            metadata: ObjectMeta {
                name: Some(SECRET_NAME.to_string()),
                namespace: Some(namespace),
                labels: Some(BTreeMap::from([(
                    "app.kubernetes.io/managed-by".to_string(),
                    "kubarr".to_string(),
                )])),
                ..Default::default()
            },
            string_data: Some(BTreeMap::from([(SECRET_KEY.to_string(), key.to_string())])),
            type_: Some("Opaque".to_string()),
            ..Default::default()
        };
        api.patch(
            SECRET_NAME,
            &PatchParams::apply("kubarr").force(),
            &Patch::Apply(&secret),
        )
        .await
        .map_err(|e| kube_error("store", e))?;
        Ok(())
    }

    async fn delete(&self) -> Result<()> {
        let (api, _) = self.api().await?;
        match api.delete(SECRET_NAME, &DeleteParams::default()).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(status)) if status.code == 404 => Ok(()),
            Err(e) => Err(kube_error("remove", e)),
        }
    }
}

/// An in-memory store for tests and for running without a cluster.
#[derive(Default)]
pub struct MemoryKeyStore {
    key: Mutex<Option<String>>,
}

impl MemoryKeyStore {
    pub fn with_key(key: &str) -> Self {
        Self {
            key: Mutex::new(Some(key.to_string())),
        }
    }
}

#[async_trait]
impl KeyStore for MemoryKeyStore {
    async fn get(&self) -> Result<Option<String>> {
        Ok(self.key.lock().clone())
    }

    async fn set(&self, key: &str) -> Result<()> {
        *self.key.lock() = Some(key.to_string());
        Ok(())
    }

    async fn delete(&self) -> Result<()> {
        *self.key.lock() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_key() -> String {
        // Assembled at runtime: no credential-shaped literal in the source.
        format!("{KEY_PREFIX}api03-{}", "x".repeat(40))
    }

    #[test]
    fn normalize_accepts_trimmed_anthropic_keys() {
        let key = sample_key();
        assert_eq!(normalize_key(&format!("  {key}\n")).unwrap(), key);
    }

    #[test]
    fn normalize_rejects_wrong_shapes_without_echoing_them() {
        let too_long = format!("{KEY_PREFIX}{}", "a".repeat(300));
        let with_space = format!("{KEY_PREFIX}abc def{}", "a".repeat(30));
        for bad in [
            "",
            "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789",
            "sk-ant-short",
            too_long.as_str(),
            with_space.as_str(),
        ] {
            let err = normalize_key(bad).unwrap_err().to_string();
            assert!(err.contains("api_key"), "{err}");
            if !bad.is_empty() {
                assert!(!err.contains(bad), "error echoes the input: {err}");
            }
        }
    }

    #[test]
    fn redact_hides_the_key_and_key_shaped_text() {
        let key = sample_key();
        let other = format!("{KEY_PREFIX}admin01-{}", "y".repeat(20));
        let text = format!("bad key {key}; also {other}.");
        let out = redact(&text, &key);
        assert_eq!(out, "bad key [redacted]; also [redacted].");
        assert_eq!(redact("nothing here", ""), "nothing here");
    }

    #[tokio::test]
    async fn memory_store_round_trips() {
        let store = MemoryKeyStore::default();
        assert_eq!(store.get().await.unwrap(), None);
        store.set("k").await.unwrap();
        assert_eq!(store.get().await.unwrap().as_deref(), Some("k"));
        store.delete().await.unwrap();
        store.delete().await.unwrap();
        assert_eq!(store.get().await.unwrap(), None);
    }

    #[tokio::test]
    async fn secret_store_without_cluster_is_unavailable() {
        let k8s: SharedK8sClient = std::sync::Arc::new(tokio::sync::RwLock::new(None));
        let store = SecretKeyStore::new(k8s, Some("kubarr".into()));
        assert!(matches!(
            store.get().await,
            Err(AppError::ServiceUnavailable(_))
        ));
    }
}
