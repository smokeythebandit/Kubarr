use http::Request;
use serde_json::{json, Value};

use super::DomainReconciler;
use crate::error::{AppError, Result};

impl DomainReconciler {
    pub(super) async fn apply_opaque_secret(
        &self,
        client: &kube::Client,
        namespace: &str,
        name: &str,
        string_data: Value,
    ) -> Result<()> {
        apply_json(
            client,
            &format!("/api/v1/namespaces/{}/secrets/{}", namespace, name),
            json!({
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": name,
                    "namespace": namespace,
                    "labels": { "app.kubernetes.io/managed-by": "kubarr" }
                },
                "type": "Opaque",
                "stringData": string_data
            }),
        )
        .await
    }
}

pub(super) async fn apply_json(client: &kube::Client, path: &str, manifest: Value) -> Result<()> {
    let body = serde_json::to_vec(&manifest)?;
    let separator = if path.contains('?') { '&' } else { '?' };
    let path = format!("{}{}fieldManager=kubarr&force=true", path, separator);
    let request = Request::patch(&path)
        .header("content-type", "application/apply-patch+yaml")
        .header("accept", "application/json")
        .body(body)
        .map_err(|error| {
            AppError::Internal(format!("Failed to build Kubernetes request: {}", error))
        })?;
    client.request::<Value>(request).await?;
    Ok(())
}

pub(super) async fn cert_manager_available(client: &kube::Client) -> bool {
    let request = match Request::get("/apis/cert-manager.io/v1").body(Vec::new()) {
        Ok(request) => request,
        Err(_) => return false,
    };
    client.request::<Value>(request).await.is_ok()
}
