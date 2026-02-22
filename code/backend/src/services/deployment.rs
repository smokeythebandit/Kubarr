use std::collections::HashMap;
use std::process::Command;

use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment};
use k8s_openapi::api::core::v1::Namespace;
use kube::api::{Api, DeleteParams, ListParams};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::services::catalog::AppCatalog;
use crate::services::vpn;
use crate::services::K8sClient;

/// Deployment request
#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentRequest {
    pub app_name: String,
    #[serde(default)]
    pub custom_config: HashMap<String, String>,
}

/// Deployment status response
#[derive(Debug, Clone, Serialize)]
pub struct DeploymentStatus {
    pub app_name: String,
    pub namespace: String,
    pub status: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

/// Deployment manager for applications
pub struct DeploymentManager<'a> {
    k8s: &'a K8sClient,
    catalog: &'a AppCatalog,
    db: Option<&'a DatabaseConnection>,
}

impl<'a> DeploymentManager<'a> {
    pub fn new(k8s: &'a K8sClient, catalog: &'a AppCatalog) -> Self {
        Self {
            k8s,
            catalog,
            db: None,
        }
    }

    /// Create with database connection for VPN support
    pub fn with_db(
        k8s: &'a K8sClient,
        catalog: &'a AppCatalog,
        db: &'a DatabaseConnection,
    ) -> Self {
        Self {
            k8s,
            catalog,
            db: Some(db),
        }
    }

    /// Get the OCI chart reference for an app
    fn get_chart_ref(&self, app_name: &str) -> String {
        format!("{}/{}", CONFIG.charts.registry, app_name)
    }

    /// Run a Helm command
    fn run_helm_command(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("helm")
            .args(args)
            .output()
            .map_err(|e| AppError::Internal(format!("Failed to run helm: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Internal(format!(
                "Helm command failed: {}",
                stderr
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Deploy an application using Helm
    pub async fn deploy_app(
        &self,
        request: &DeploymentRequest,
        storage_path: Option<&str>,
    ) -> Result<DeploymentStatus> {
        // Get app config from catalog
        let app_config = self.catalog.get_app(&request.app_name).ok_or_else(|| {
            AppError::NotFound(format!("App '{}' not found in catalog", request.app_name))
        })?;

        let chart_ref = self.get_chart_ref(&request.app_name);
        let namespace = &request.app_name;

        // Build helm upgrade --install command
        let mut helm_args = vec![
            "upgrade",
            "--install",
            &request.app_name,
            &chart_ref,
            "-n",
            namespace,
            "--create-namespace",
        ];

        // Collect --set arguments
        let mut set_args: Vec<String> = Vec::new();
        let mut set_string_args: Vec<String> = Vec::new();

        // Add storage configuration
        if let Some(path) = storage_path {
            set_args.push("storage.hostPath.enabled=true".to_string());
            set_args.push(format!("storage.hostPath.rootPath={}", path));
        }

        // Check for VPN configuration
        if let Some(db) = self.db {
            if let Ok(Some(vpn_config)) =
                vpn::get_vpn_deployment_config(db, &request.app_name).await
            {
                // Create K8s secret with VPN credentials
                match vpn::create_vpn_secret_for_app(self.k8s, db, &request.app_name).await {
                    Ok(secret_name) => {
                        tracing::info!(
                            "Created VPN secret {} for app {}",
                            secret_name,
                            request.app_name
                        );
                        set_args.push("vpn.enabled=true".to_string());
                        set_args.push(format!("vpn.secretName={}", secret_name));
                        set_args.push(format!("vpn.killSwitch={}", vpn_config.kill_switch));
                        if vpn_config.port_forwarding {
                            set_args.push("vpn.portForwarding.enabled=true".to_string());
                        }
                        // Use --set-string for subnets since CIDR notation contains
                        // commas and slashes that Helm's --set parser misinterprets
                        set_string_args.push(format!(
                            "vpn.firewallOutboundSubnets={}",
                            vpn_config.firewall_outbound_subnets
                        ));
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to create VPN secret for app {}: {}",
                            request.app_name,
                            e
                        );
                    }
                }
            }
        }

        // Add custom config
        for (key, value) in &request.custom_config {
            set_args.push(format!("{}={}", key, value));
        }

        // Add --set arguments
        for arg in &set_args {
            helm_args.push("--set");
            helm_args.push(arg);
        }

        // Add --set-string arguments (for values containing special chars like commas/slashes)
        for arg in &set_string_args {
            helm_args.push("--set-string");
            helm_args.push(arg);
        }

        // Run helm command
        let args_str: Vec<&str> = helm_args.iter().map(|s| s.as_ref()).collect();
        self.run_helm_command(&args_str)?;

        Ok(DeploymentStatus {
            app_name: request.app_name.clone(),
            namespace: namespace.to_string(),
            status: "installing".to_string(),
            message: format!("Deploying {}", app_config.display_name),
            timestamp: Utc::now(),
        })
    }

    /// Remove an application
    pub async fn remove_app(&self, app_name: &str) -> Result<bool> {
        let namespace = app_name;

        // Try to uninstall with Helm
        let _ = self.run_helm_command(&["uninstall", app_name, "-n", namespace]);

        // Delete the namespace
        let namespaces: Api<Namespace> = Api::all(self.k8s.client().clone());

        match namespaces.delete(namespace, &DeleteParams::default()).await {
            Ok(_) => Ok(true),
            Err(kube::Error::Api(ae)) if ae.code == 404 => Ok(true),
            Err(e) => Err(AppError::Internal(format!(
                "Failed to delete namespace: {}",
                e
            ))),
        }
    }

    /// Get list of deployed app names (excludes hidden/system apps like kubarr itself)
    pub async fn get_deployed_apps(&self) -> Vec<String> {
        let namespaces: Api<Namespace> = Api::all(self.k8s.client().clone());

        // Get all catalog app names (excluding hidden apps)
        let catalog_apps: std::collections::HashSet<_> = self
            .catalog
            .get_all_apps()
            .iter()
            .filter(|app| !app.is_hidden)
            .map(|app| app.name.clone())
            .collect();

        let mut deployed_apps = Vec::new();

        if let Ok(ns_list) = namespaces.list(&ListParams::default()).await {
            for ns in ns_list {
                if let Some(name) = ns.metadata.name {
                    if catalog_apps.contains(&name) {
                        // Check if there are deployments in this namespace
                        if let Ok(health) = self.check_namespace_health(&name).await {
                            if health.get("deployments").is_some() {
                                deployed_apps.push(name);
                            }
                        }
                    }
                }
            }
        }

        deployed_apps
    }

    /// Check if a namespace exists
    pub async fn check_namespace_exists(&self, namespace: &str) -> bool {
        let namespaces: Api<Namespace> = Api::all(self.k8s.client().clone());
        namespaces.get(namespace).await.is_ok()
    }

    /// Check if all deployments in a namespace are healthy
    pub async fn check_namespace_health(&self, namespace: &str) -> Result<serde_json::Value> {
        let namespaces: Api<Namespace> = Api::all(self.k8s.client().clone());

        // Check if namespace exists
        if namespaces.get(namespace).await.is_err() {
            return Ok(serde_json::json!({
                "status": "not_found",
                "healthy": false,
                "message": "Namespace does not exist"
            }));
        }

        // Get deployments
        let deployments: Api<Deployment> = Api::namespaced(self.k8s.client().clone(), namespace);
        let deploy_list = deployments.list(&ListParams::default()).await?;

        // Get daemonsets
        let daemonsets: Api<DaemonSet> = Api::namespaced(self.k8s.client().clone(), namespace);
        let ds_list = daemonsets.list(&ListParams::default()).await?;

        if deploy_list.items.is_empty() && ds_list.items.is_empty() {
            return Ok(serde_json::json!({
                "status": "no_workloads",
                "healthy": false,
                "message": "No deployments or daemonsets found in namespace"
            }));
        }

        let mut all_healthy = true;
        let mut workload_statuses = Vec::new();

        // Check deployments
        for deploy in &deploy_list.items {
            let name = deploy.metadata.name.clone().unwrap_or_default();
            let spec = deploy.spec.as_ref();
            let status = deploy.status.as_ref();

            let replicas = spec.and_then(|s| s.replicas).unwrap_or(1);
            let ready_replicas = status.and_then(|s| s.ready_replicas).unwrap_or(0);
            let available_replicas = status.and_then(|s| s.available_replicas).unwrap_or(0);

            let is_healthy = ready_replicas >= replicas && available_replicas >= replicas;

            workload_statuses.push(serde_json::json!({
                "name": name,
                "kind": "Deployment",
                "replicas": replicas,
                "ready_replicas": ready_replicas,
                "available_replicas": available_replicas,
                "healthy": is_healthy
            }));

            if !is_healthy {
                all_healthy = false;
            }
        }

        // Check daemonsets
        for ds in &ds_list.items {
            let name = ds.metadata.name.clone().unwrap_or_default();
            let status = ds.status.as_ref();

            let desired = status.map(|s| s.desired_number_scheduled).unwrap_or(1);
            let ready = status.map(|s| s.number_ready).unwrap_or(0);
            let available = status.and_then(|s| s.number_available).unwrap_or(0);

            let is_healthy = ready >= desired && available >= desired && desired > 0;

            workload_statuses.push(serde_json::json!({
                "name": name,
                "kind": "DaemonSet",
                "desired": desired,
                "ready": ready,
                "available": available,
                "healthy": is_healthy
            }));

            if !is_healthy {
                all_healthy = false;
            }
        }

        Ok(serde_json::json!({
            "status": if all_healthy { "healthy" } else { "unhealthy" },
            "healthy": all_healthy,
            "deployments": workload_statuses,
            "message": if all_healthy { "All workloads healthy" } else { "Some workloads are not healthy" }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // DeploymentRequest deserialization
    // -------------------------------------------------------------------------

    #[test]
    fn deployment_request_deserializes_minimal() {
        let json = r#"{"app_name": "radarr"}"#;
        let req: DeploymentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.app_name, "radarr");
        assert!(req.custom_config.is_empty());
    }

    #[test]
    fn deployment_request_deserializes_with_custom_config() {
        let json = r#"{
            "app_name": "sonarr",
            "custom_config": {
                "image.tag": "latest",
                "replicaCount": "2"
            }
        }"#;
        let req: DeploymentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.app_name, "sonarr");
        assert_eq!(
            req.custom_config.get("image.tag").map(|s| s.as_str()),
            Some("latest")
        );
        assert_eq!(
            req.custom_config.get("replicaCount").map(|s| s.as_str()),
            Some("2")
        );
    }

    #[test]
    fn deployment_request_custom_config_defaults_empty() {
        // The #[serde(default)] on custom_config means it defaults to empty map
        let json = r#"{"app_name": "prowlarr"}"#;
        let req: DeploymentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.custom_config.len(), 0);
    }

    // -------------------------------------------------------------------------
    // DeploymentStatus serialization
    // -------------------------------------------------------------------------

    #[test]
    fn deployment_status_serializes() {
        let now = Utc::now();
        let status = DeploymentStatus {
            app_name: "radarr".to_string(),
            namespace: "radarr".to_string(),
            status: "installing".to_string(),
            message: "Deploying Radarr".to_string(),
            timestamp: now,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["app_name"], "radarr");
        assert_eq!(json["namespace"], "radarr");
        assert_eq!(json["status"], "installing");
        assert_eq!(json["message"], "Deploying Radarr");
        // timestamp should be a string (RFC 3339)
        assert!(json["timestamp"].is_string());
    }

    #[test]
    fn deployment_status_serializes_removing_status() {
        let now = Utc::now();
        let status = DeploymentStatus {
            app_name: "sonarr".to_string(),
            namespace: "sonarr".to_string(),
            status: "removing".to_string(),
            message: "Removing Sonarr".to_string(),
            timestamp: now,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["status"], "removing");
        assert_eq!(json["app_name"], "sonarr");
    }

    #[test]
    fn deployment_request_clone() {
        let req = DeploymentRequest {
            app_name: "bazarr".to_string(),
            custom_config: {
                let mut m = HashMap::new();
                m.insert("key".to_string(), "value".to_string());
                m
            },
        };
        let cloned = req.clone();
        assert_eq!(cloned.app_name, req.app_name);
        assert_eq!(cloned.custom_config, req.custom_config);
    }

    #[test]
    fn deployment_status_clone() {
        let now = Utc::now();
        let status = DeploymentStatus {
            app_name: "lidarr".to_string(),
            namespace: "lidarr".to_string(),
            status: "healthy".to_string(),
            message: "All good".to_string(),
            timestamp: now,
        };
        let cloned = status.clone();
        assert_eq!(cloned.app_name, status.app_name);
        assert_eq!(cloned.status, status.status);
    }
}
