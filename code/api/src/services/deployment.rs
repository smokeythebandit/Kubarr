use std::collections::HashMap;
use std::fs;
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{Namespace, Secret, Service};
use kube::api::{Api, DeleteParams, ListParams};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};

use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::models::{app_state, app_vpn_config};
use crate::services::catalog::{kubarr_system_component, lifecycle_for_app_name, AppCatalog};
use crate::services::gpu::{self, GpuSelection};
use crate::services::helm_process;
use crate::services::storage_config::{self, PersistedStorageConfig};
use crate::services::vpn;
use crate::services::K8sClient;
use tokio_util::sync::CancellationToken;

/// Deployment request
#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentRequest {
    pub app_name: String,
    #[serde(default)]
    pub custom_config: HashMap<String, String>,
    #[serde(default, deserialize_with = "gpu::gpu_field")]
    pub gpu: Option<Option<GpuSelection>>,
    #[serde(default)]
    pub reuse_values: bool,
    #[serde(default = "default_wait")]
    pub wait: bool,
}

fn default_wait() -> bool {
    true
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct ReleaseMetadata {
    version: String,
    status: String,
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
        self.run_helm_command_with_stop(args, None)
    }

    fn run_helm_command_with_stop(
        &self,
        args: &[&str],
        stop: Option<&CancellationToken>,
    ) -> Result<String> {
        let timeout = if args.first() == Some(&"uninstall") {
            Duration::from_secs(120)
        } else {
            Duration::from_secs(12 * 60)
        };
        let output =
            tokio::task::block_in_place(|| helm_process::run_with_stop(args, timeout, stop))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Internal(format!(
                "Helm command failed: {}",
                stderr
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub fn installed_chart_version(&self, app_name: &str) -> Option<String> {
        self.catalog.get_app(app_name)?;
        let lifecycle = lifecycle_for_app_name(app_name);
        installed_version_from_metadata(self.release_metadata(&lifecycle).ok().flatten())
    }

    fn release_metadata(
        &self,
        lifecycle: &crate::services::catalog::AppLifecycleConfig,
    ) -> Result<Option<ReleaseMetadata>> {
        self.release_metadata_with_command(lifecycle, |args| {
            let output =
                tokio::task::block_in_place(|| helm_process::run(args, Duration::from_secs(30)))?;
            if output.status.success() {
                return Ok(Some(output.stdout));
            }

            let stderr = String::from_utf8_lossy(&output.stderr);
            if helm_release_not_found(&stderr) {
                Ok(None)
            } else {
                Err(AppError::Internal(format!(
                    "Helm metadata command failed: {stderr}"
                )))
            }
        })
    }

    fn release_metadata_with_command(
        &self,
        lifecycle: &crate::services::catalog::AppLifecycleConfig,
        run_helm: impl FnOnce(&[&str]) -> Result<Option<Vec<u8>>>,
    ) -> Result<Option<ReleaseMetadata>> {
        let output = run_helm(&[
            "get",
            "metadata",
            lifecycle.release_name.as_str(),
            "-n",
            lifecycle.release_namespace.as_str(),
            "-o",
            "json",
        ])?;
        output
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|error| {
                    AppError::Internal(format!("Invalid Helm release metadata: {error}"))
                })
            })
            .transpose()
    }

    /// Deploy an application using Helm
    pub async fn deploy_app(
        &self,
        request: &DeploymentRequest,
        storage_config: Option<&PersistedStorageConfig>,
    ) -> Result<DeploymentStatus> {
        self.deploy_app_with_command(request, storage_config, |args| self.run_helm_command(args))
            .await
    }

    pub async fn deploy_app_with_stop(
        &self,
        request: &DeploymentRequest,
        storage_config: Option<&PersistedStorageConfig>,
        stop: Option<&CancellationToken>,
    ) -> Result<DeploymentStatus> {
        let result = self
            .deploy_app_with_command(request, storage_config, |args| {
                self.run_helm_command_with_stop(args, stop)
            })
            .await?;
        if stop.is_some_and(CancellationToken::is_cancelled) {
            return Err(AppError::Internal(
                "Stop requested; external outcome indeterminate".into(),
            ));
        }
        Ok(result)
    }

    async fn deploy_app_with_command(
        &self,
        request: &DeploymentRequest,
        storage_config: Option<&PersistedStorageConfig>,
        run_helm: impl FnOnce(&[&str]) -> Result<String>,
    ) -> Result<DeploymentStatus> {
        // Get app config from catalog
        let app_config = self.catalog.get_app(&request.app_name).ok_or_else(|| {
            AppError::NotFound(format!("App '{}' not found in catalog", request.app_name))
        })?;

        validate_custom_config(&request.custom_config)?;
        if request.gpu.is_some() && !matches!(request.app_name.as_str(), "plex" | "jellyfin") {
            return Err(AppError::BadRequest(
                "GPU is supported only for Plex and Jellyfin".into(),
            ));
        }
        if request.gpu.is_some() {
            gpu::validate_gpu_scheduling_config(&request.custom_config)?;
        }
        let gpu_hostname = if let Some(Some(selection)) = &request.gpu {
            Some(gpu::validate_selection(self.k8s, selection).await?)
        } else {
            None
        };

        // Resolve VPN state before making any Kubernetes changes. An assigned
        // VPN that cannot be resolved must fail closed rather than deploying
        // the application without its network protection.
        let vpn_config = match self.db {
            Some(db) => vpn::get_vpn_deployment_config(db, &request.app_name).await?,
            None => None,
        };
        let cleanup_unassigned_vpn_secret = self.db.is_some() && vpn_config.is_none();

        let chart_ref = self.get_chart_ref(&request.app_name);
        let lifecycle = lifecycle_for_app_name(&request.app_name);
        let release = lifecycle.release_name.as_str();
        let release_namespace = lifecycle.release_namespace.as_str();
        let namespace = lifecycle.namespace.as_str();
        let chart_version = if let Some(db) = self.db {
            app_state::Entity::find_by_id(request.app_name.clone())
                .one(db)
                .await
                .ok()
                .flatten()
                .and_then(|state| state.available_chart_version)
        } else {
            None
        }
        .or_else(|| self.catalog.chart_version(&request.app_name));

        // VPN credentials are a namespaced Secret and must exist before Helm
        // renders the release. Helm's --create-namespace happens too late.
        self.k8s.ensure_namespace(namespace).await?;

        // Keep Helm 3's client-side apply and readiness checks explicit under Helm 4.
        let mut helm_args = helm_upgrade_install_args(
            release,
            &chart_ref,
            release_namespace,
            CONFIG.charts.plain_http,
        );
        if let Some(ref version) = chart_version {
            helm_args.extend(["--version", version]);
        }
        if request.reuse_values {
            helm_args.push("--reuse-values");
        }
        if request.wait {
            helm_args.extend(["--wait=legacy", "--rollback-on-failure", "--timeout", "10m"]);
        }

        // Collect --set arguments
        // The API creates the namespace above, so the chart must not render and
        // make Helm adopt that cluster-scoped resource during upgrades.
        let mut set_args: Vec<String> = namespace_helm_values(namespace).into();
        if matches!(request.app_name.as_str(), "plex" | "jellyfin") {
            // An omitted GPU on update preserves the release; install and explicit null
            // both reset previously reused managed values.
            if !request.reuse_values || request.gpu.is_some() {
                set_args.extend(gpu::helm_values(
                    request
                        .gpu
                        .as_ref()
                        .and_then(Option::as_ref)
                        .zip(gpu_hostname.as_deref()),
                ));
            }
        }
        let mut vpn_values_file = None;

        // Add storage configuration using the shared NFS-backed media PVC.
        if let Some(storage) = storage_config {
            if !storage.validated() {
                return Err(AppError::BadRequest(
                    "Storage must be validated before installing apps".to_string(),
                ));
            }
            set_args.extend(media_storage_helm_values(storage)?);
            if storage.mode == storage_config::StorageMode::ManagedNfs {
                let server = self.managed_nfs_cluster_ip().await?;
                set_args.push(format!("storage.media.nfs.server={server}"));
            }
        }

        // LinuxServer-based app images use s6 init to switch to PUID/PGID and
        // adjust mounted config directories. The app charts are intentionally
        // user-tunable, but these capabilities are required for the default
        // catalog images to boot on restricted clusters.
        if !app_config.is_system {
            set_args.push("securityContext.allowPrivilegeEscalation=true".to_string());
            set_args.push(
                "securityContext.capabilities.add={CHOWN,DAC_OVERRIDE,FOWNER,SETUID,SETGID}"
                    .to_string(),
            );
        }

        if let Some(vpn_config) = vpn_config {
            let db = self.db.ok_or_else(|| {
                AppError::Internal("VPN configuration resolved without a database".to_string())
            })?;
            // Secret creation is part of the deployment boundary: never invoke
            // Helm if credentials could not be installed.
            let secret_name =
                vpn::create_vpn_secret_for_app(self.k8s, db, &request.app_name).await?;
            tracing::info!(
                "Created VPN secret {} for app {}",
                secret_name,
                request.app_name
            );
            set_args.push("vpn.enabled=true".to_string());
            set_args.push(format!("vpn.secretName={}", secret_name));
            set_args.push(format!("vpn.killSwitch={}", vpn_config.kill_switch));
            set_args.push(format!(
                "vpn.portForwarding.enabled={}",
                vpn_config.port_forwarding
            ));
            // Avoid Helm's comma-sensitive --set parser for the CIDR list.
            let path = std::env::temp_dir()
                .join(format!("kubarr-vpn-values-{}.yaml", uuid::Uuid::new_v4()));
            let values = serde_yaml::to_string(&serde_json::json!({
                "vpn": {
                    "firewallOutboundSubnets": vpn_config.firewall_outbound_subnets
                }
            }))?;
            fs::write(&path, values).map_err(|error| {
                AppError::Internal(format!("Failed to write Helm values file: {error}"))
            })?;
            vpn_values_file = Some(path);
        } else {
            // These must be explicit even with --reuse-values, otherwise a
            // previous assignment can leave the VPN sidecar enabled.
            set_args.push("vpn.enabled=false".to_string());
            set_args.push("vpn.secretName=".to_string());
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

        let vpn_values_path = vpn_values_file
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        if let Some(path) = vpn_values_path.as_deref() {
            helm_args.extend(["--values", path]);
        }

        // Run helm command
        let args_str: Vec<&str> = helm_args.iter().map(|s| s.as_ref()).collect();
        let result = run_helm(&args_str);
        if let Some(path) = vpn_values_file {
            let _ = fs::remove_file(path);
        }
        result?;

        if cleanup_unassigned_vpn_secret {
            self.cleanup_unassigned_vpn_secret(&request.app_name, namespace)
                .await?;
        }

        Ok(DeploymentStatus {
            app_name: request.app_name.clone(),
            namespace: namespace.to_string(),
            status: "installing".to_string(),
            message: format!("Deploying {}", app_config.display_name),
            timestamp: Utc::now(),
        })
    }

    async fn cleanup_unassigned_vpn_secret(&self, app_name: &str, namespace: &str) -> Result<()> {
        let Some(db) = self.db else {
            return Ok(());
        };

        // The assignment may have changed while Helm was running. Never remove
        // credentials needed by a newly queued assignment.
        let assigned = app_vpn_config::Entity::find_by_id(app_name.to_string())
            .one(db)
            .await
            .map_err(|error| {
                AppError::Internal(format!(
                    "VPN sidecar was removed, but failed to confirm Secret cleanup safety: {error}"
                ))
            })?
            .is_some();
        if assigned {
            return Ok(());
        }

        let secret_name = format!("vpn-{app_name}");
        let secrets: Api<Secret> = Api::namespaced(self.k8s.client().clone(), namespace);
        match secrets.delete(&secret_name, &DeleteParams::default()).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(error)) if error.code == 404 => Ok(()),
            Err(error) => Err(AppError::Internal(format!(
                "VPN sidecar was removed, but failed to delete managed Secret '{secret_name}' in namespace '{namespace}': {error}"
            ))),
        }
    }

    async fn managed_nfs_cluster_ip(&self) -> Result<String> {
        let services: Api<Service> = Api::namespaced(self.k8s.client().clone(), "kubarr-storage");
        let service = services.get("kubarr-managed-nfs").await.map_err(|e| {
            AppError::Internal(format!("Failed to resolve managed NFS service: {e}"))
        })?;
        service
            .spec
            .and_then(|spec| spec.cluster_ip)
            .filter(|ip| !ip.is_empty() && ip != "None")
            .ok_or_else(|| AppError::Internal("Managed NFS service has no ClusterIP".to_string()))
    }

    /// Remove an application
    pub async fn remove_app(&self, app_name: &str) -> Result<bool> {
        self.remove_app_with_stop(app_name, None).await
    }

    pub async fn remove_app_with_stop(
        &self,
        app_name: &str,
        stop: Option<&CancellationToken>,
    ) -> Result<bool> {
        if self.catalog.get_app(app_name).is_none() {
            return Err(AppError::NotFound(format!(
                "App '{}' not found in catalog",
                app_name
            )));
        }
        let lifecycle = lifecycle_for_app_name(app_name);
        let namespace = lifecycle.namespace.as_str();
        let release = lifecycle.release_name.as_str();
        let release_namespace = lifecycle.release_namespace.as_str();

        // Try to uninstall with Helm
        if let Err(error) =
            self.run_helm_command_with_stop(&["uninstall", release, "-n", release_namespace], stop)
        {
            // Do not report removal as successful when Helm timed out or failed: its
            // external outcome is unknown, and deleting the namespace could mask it.
            // An already absent release is the one safe exception.
            if stop.is_some_and(CancellationToken::is_cancelled)
                || !helm_release_not_found(&error.to_string())
            {
                return Err(error);
            }
        }

        if stop.is_some_and(CancellationToken::is_cancelled) {
            return Err(AppError::Internal(
                "Stop requested; external outcome indeterminate".into(),
            ));
        }

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

    /// Get list of deployed app names.
    pub async fn get_deployed_apps(&self) -> Vec<String> {
        let mut deployed_apps = Vec::new();
        for app in self
            .catalog
            .get_all_apps()
            .into_iter()
            .filter(|app| !app.is_hidden)
        {
            if self.check_app_deployed(&app.name).await {
                deployed_apps.push(app.name.clone());
            }
        }

        deployed_apps
    }

    pub async fn check_app_deployed(&self, app_name: &str) -> bool {
        match self.app_health(app_name).await {
            Ok(health) => health["status"].as_str() != Some("not_found"),
            Err(_) => false,
        }
    }

    pub async fn check_kubarr_system_component(&self, app_name: &str) -> bool {
        match kubarr_system_component(app_name) {
            Some(component) => match component.workload_kind {
                crate::services::catalog::SystemComponentWorkloadKind::Static => {
                    CONFIG.charts.dir.exists()
                }
                crate::services::catalog::SystemComponentWorkloadKind::Deployment => {
                    match component.workload_name {
                        Some(workload_name) => self
                            .check_deployment_ready(component.namespace, workload_name)
                            .await
                            .unwrap_or(false),
                        None => false,
                    }
                }
                crate::services::catalog::SystemComponentWorkloadKind::DaemonSet => {
                    match component.workload_name {
                        Some(workload_name) => self
                            .check_daemonset_ready(component.namespace, workload_name)
                            .await
                            .unwrap_or(false),
                        None => false,
                    }
                }
                crate::services::catalog::SystemComponentWorkloadKind::StatefulSet => {
                    match component.workload_name {
                        Some(workload_name) => self
                            .check_statefulset_ready(component.namespace, workload_name)
                            .await
                            .unwrap_or(false),
                        None => false,
                    }
                }
            },
            None => false,
        }
    }

    pub async fn app_health(&self, app_name: &str) -> Result<serde_json::Value> {
        self.app_health_with_metadata_command(app_name, |lifecycle| {
            self.release_metadata(lifecycle)
        })
        .await
    }

    async fn app_health_with_metadata_command(
        &self,
        app_name: &str,
        get_metadata: impl FnOnce(
            &crate::services::catalog::AppLifecycleConfig,
        ) -> Result<Option<ReleaseMetadata>>,
    ) -> Result<serde_json::Value> {
        if self.catalog.get_app(app_name).is_none() {
            return Err(AppError::NotFound(format!(
                "App '{}' not found in catalog",
                app_name
            )));
        }
        let lifecycle = lifecycle_for_app_name(app_name);
        let namespace = lifecycle.namespace.as_str();

        if !self.check_namespace_exists(namespace).await {
            return Ok(serde_json::json!({
                "status": "not_found",
                "healthy": false,
                "message": "Namespace does not exist"
            }));
        }

        // A ready workload can belong to the previous revision while Helm is
        // still applying (or has failed) the current revision. Keep checking
        // workload readiness, but never call such a release healthy.
        let release_metadata = get_metadata(&lifecycle)?;

        let workload_health = match (
            lifecycle.workload_kind.as_deref(),
            lifecycle.workload_name.as_deref(),
        ) {
            (Some("deployment"), Some(name)) => {
                let healthy = self.check_deployment_ready(namespace, name).await?;
                Ok(single_workload_health("Deployment", name, healthy))
            }
            (Some("daemonset"), Some(name)) => {
                let healthy = self.check_daemonset_ready(namespace, name).await?;
                Ok(single_workload_health("DaemonSet", name, healthy))
            }
            (Some("statefulset"), Some(name)) => {
                let healthy = self.check_statefulset_ready(namespace, name).await?;
                Ok(single_workload_health("StatefulSet", name, healthy))
            }
            (Some("static"), _) => Ok(serde_json::json!({
                "status": "healthy",
                "healthy": true,
                "message": "Static component is available"
            })),
            _ => self.check_namespace_health(namespace).await,
        }?;

        Ok(health_with_release_status(
            workload_health,
            release_metadata.as_ref(),
        ))
    }

    pub async fn check_deployment_ready(
        &self,
        namespace: &str,
        deployment_name: &str,
    ) -> Result<bool> {
        let deployments: Api<Deployment> = Api::namespaced(self.k8s.client().clone(), namespace);
        let deployment = deployments.get(deployment_name).await?;
        let desired = deployment
            .spec
            .as_ref()
            .and_then(|s| s.replicas)
            .unwrap_or(1);
        let available = deployment
            .status
            .as_ref()
            .and_then(|s| s.available_replicas)
            .unwrap_or(0);
        Ok(available >= desired && desired > 0)
    }

    pub async fn check_statefulset_ready(
        &self,
        namespace: &str,
        statefulset_name: &str,
    ) -> Result<bool> {
        let statefulsets: Api<StatefulSet> = Api::namespaced(self.k8s.client().clone(), namespace);
        let statefulset = statefulsets.get(statefulset_name).await?;
        let desired = statefulset
            .spec
            .as_ref()
            .and_then(|s| s.replicas)
            .unwrap_or(1);
        let ready = statefulset
            .status
            .as_ref()
            .and_then(|s| s.ready_replicas)
            .unwrap_or(0);
        Ok(ready >= desired && desired > 0)
    }

    pub async fn check_daemonset_ready(
        &self,
        namespace: &str,
        daemonset_name: &str,
    ) -> Result<bool> {
        let daemonsets: Api<DaemonSet> = Api::namespaced(self.k8s.client().clone(), namespace);
        let daemonset = daemonsets.get(daemonset_name).await?;
        let desired = daemonset
            .status
            .as_ref()
            .map(|s| s.desired_number_scheduled)
            .unwrap_or(1);
        let ready = daemonset
            .status
            .as_ref()
            .map(|s| s.number_ready)
            .unwrap_or(0);
        let available = daemonset
            .status
            .as_ref()
            .and_then(|s| s.number_available)
            .unwrap_or(0);
        Ok(ready >= desired && available >= desired && desired > 0)
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

pub fn media_storage_helm_values(storage: &PersistedStorageConfig) -> Result<Vec<String>> {
    let mut values = vec![
        format!(
            "storage.media.existingClaim={}",
            storage_config::MEDIA_PVC_NAME
        ),
        format!(
            "storage.media.mountPath={}",
            storage_config::STORAGE_MOUNT_PATH
        ),
    ];

    match storage.mode {
        storage_config::StorageMode::ExternalNfs => {
            let config: storage_config::ExternalNfsConfig =
                serde_json::from_value(storage.config_json.clone()).map_err(|e| {
                    AppError::Internal(format!("Invalid external NFS storage config: {e}"))
                })?;
            values.push(format!("storage.media.nfs.server={}", config.server));
            values.push(format!("storage.media.nfs.path={}", config.export_path));
        }
        storage_config::StorageMode::ManagedNfs => {
            let config: storage_config::ManagedNfsConfig =
                serde_json::from_value(storage.config_json.clone()).map_err(|e| {
                    AppError::Internal(format!("Invalid managed NFS storage config: {e}"))
                })?;
            values.push(format!("storage.media.nfs.size={}", config.size));
        }
    }

    Ok(values)
}

fn single_workload_health(kind: &str, name: &str, healthy: bool) -> serde_json::Value {
    serde_json::json!({
        "status": if healthy { "healthy" } else { "unhealthy" },
        "healthy": healthy,
        "deployments": [{
            "name": name,
            "kind": kind,
            "healthy": healthy
        }],
        "message": if healthy { "Workload healthy" } else { "Workload is not ready" }
    })
}

impl ReleaseMetadata {
    fn is_deployed(&self) -> bool {
        self.status.trim().eq_ignore_ascii_case("deployed")
    }
}

fn installed_version_from_metadata(metadata: Option<ReleaseMetadata>) -> Option<String> {
    metadata.and_then(|metadata| metadata.is_deployed().then_some(metadata.version))
}

fn health_with_release_status(
    mut health: serde_json::Value,
    metadata: Option<&ReleaseMetadata>,
) -> serde_json::Value {
    let Some(metadata) = metadata.filter(|metadata| !metadata.is_deployed()) else {
        return health;
    };

    health["status"] = serde_json::Value::String("unhealthy".to_string());
    health["healthy"] = serde_json::Value::Bool(false);
    health["helm_status"] = serde_json::Value::String(metadata.status.clone());
    health["message"] = serde_json::Value::String(format!(
        "Helm release is {}; workload readiness does not mark the app healthy",
        metadata.status
    ));
    health
}

fn helm_release_not_found(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("release: not found") || stderr.contains("release not found")
}

fn namespace_helm_values(namespace: &str) -> [String; 2] {
    [
        format!("namespace.name={namespace}"),
        "namespace.create=false".to_string(),
    ]
}

fn validate_custom_config(custom_config: &HashMap<String, String>) -> Result<()> {
    gpu::validate_custom_gpu_keys(custom_config)?;
    if let Some(key) = custom_config.keys().find(|key| is_vpn_helm_key(key)) {
        return Err(AppError::BadRequest(format!(
            "custom_config key '{key}' is managed by Kubarr; use VPN settings instead"
        )));
    }
    if let Some((key, _)) = custom_config
        .iter()
        .find(|(_, value)| value_injects_vpn_helm_assignment(value))
    {
        return Err(AppError::BadRequest(format!(
            "custom_config key '{key}' contains a managed VPN assignment; use VPN settings instead"
        )));
    }
    Ok(())
}

fn is_vpn_helm_key(key: &str) -> bool {
    // Helm accepts both dotted paths and list-index paths. A comma can begin a
    // second assignment in --set, so inspect each unescaped assignment too.
    key.split(',').any(|part| {
        let part = part.trim_start();
        part == "vpn" || part.starts_with("vpn.") || part.starts_with("vpn[")
    })
}

fn value_injects_vpn_helm_assignment(value: &str) -> bool {
    // Helm's --set parser treats commas as assignment separators, including in
    // list-like values. Be conservative around escaped commas too: accepting a
    // literal is less important than allowing a managed VPN override.
    value.split(',').skip(1).any(|segment| {
        let segment = segment.trim_start().trim_start_matches('{').trim_start();
        let Some((key, _)) = segment.split_once('=') else {
            return false;
        };
        is_vpn_helm_key(key.trim())
    })
}

fn helm_upgrade_install_args<'a>(
    release: &'a str,
    chart_ref: &'a str,
    release_namespace: &'a str,
    plain_http: bool,
) -> Vec<&'a str> {
    let mut args = vec![
        "upgrade",
        "--install",
        "--server-side=false",
        release,
        chart_ref,
        "-n",
        release_namespace,
        "--create-namespace",
    ];
    if plain_http {
        args.push("--plain-http");
    }
    args
}

#[cfg(test)]
#[path = "deployment_tests.rs"]
mod deployment_tests;

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
        assert_eq!(req.gpu, None);
    }

    #[test]
    fn gpu_disable_is_distinct_from_omitted_and_unknown_vendor_is_rejected() {
        let disabled: DeploymentRequest =
            serde_json::from_str(r#"{"app_name":"plex","gpu":null}"#).unwrap();
        assert_eq!(disabled.gpu, Some(None));
        let selected: DeploymentRequest = serde_json::from_str(
            r#"{"app_name":"plex","gpu":{"vendor":"intel","node_name":"gpu-node"}}"#,
        )
        .unwrap();
        let selection = selected.gpu.unwrap().unwrap();
        assert_eq!(selection.node_name, "gpu-node");
        assert_eq!(selection.resource_name, None);
        assert!(serde_json::from_str::<DeploymentRequest>(
            r#"{"app_name":"plex","gpu":{"vendor":"unknown","node_name":"gpu-node"}}"#
        )
        .is_err());
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
            reuse_values: false,
            gpu: None,
            wait: true,
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

    #[test]
    fn release_metadata_preserves_semver() {
        for version in ["1.29.2", "1.29.2-rc.1", "1.29.2+5.1", "1.29.2-rc.1+5.1"] {
            let metadata = serde_json::to_vec(&serde_json::json!({
                "chart": "my-hyphenated-chart",
                "version": version,
                "status": "deployed",
                "appVersion": "9.0.0",
                "revision": 7
            }))
            .unwrap();
            let parsed: ReleaseMetadata = serde_json::from_slice(&metadata).unwrap();
            assert!(parsed.is_deployed());
            assert_eq!(parsed.version, version);
        }
    }

    #[test]
    fn release_metadata_rejects_missing_or_invalid_deployed_version() {
        for metadata in [
            "not json",
            "{}",
            r#"{"version": null}"#,
            r#"{"version": 3}"#,
            r#"{"version": "1.29.2"}"#,
            r#"{"version": "1.29.2", "status": "pending-upgrade"}"#,
            r#"{"chart": "radarr-1.29.2", "manifest": "helm.sh/chart: radarr-1.29.2"}"#,
        ] {
            let parsed = serde_json::from_slice::<ReleaseMetadata>(metadata.as_bytes());
            assert!(
                !parsed
                    .map(|metadata| metadata.is_deployed())
                    .unwrap_or(false),
                "unexpectedly accepted metadata: {metadata}"
            );
        }
    }

    #[test]
    fn installed_version_requires_deployed_release() {
        for status in ["pending-install", "pending-upgrade", "failed", "superseded"] {
            assert_eq!(
                installed_version_from_metadata(Some(ReleaseMetadata {
                    version: "1.29.2-rc.1+5.1".to_string(),
                    status: status.to_string(),
                })),
                None,
                "status {status} must not count as installed"
            );
        }
        assert_eq!(
            installed_version_from_metadata(Some(ReleaseMetadata {
                version: "1.29.2-rc.1+5.1".to_string(),
                status: "deployed".to_string(),
            }))
            .as_deref(),
            Some("1.29.2-rc.1+5.1")
        );
    }

    #[test]
    fn helm_metadata_not_found_is_distinct_from_command_failure() {
        assert!(helm_release_not_found("Error: release: not found"));
        assert!(helm_release_not_found("RELEASE NOT FOUND"));
        assert!(!helm_release_not_found(
            "Error: Kubernetes cluster unreachable"
        ));
    }

    #[test]
    fn namespace_values_disable_chart_managed_namespaces() {
        assert_eq!(
            namespace_helm_values("victoriametrics"),
            [
                "namespace.name=victoriametrics".to_string(),
                "namespace.create=false".to_string(),
            ]
        );
    }

    #[test]
    fn upgrade_install_plain_http_is_opt_in() {
        let secure = helm_upgrade_install_args("app", "oci://registry/app", "app", false);
        assert!(!secure.contains(&"--plain-http"));
        let plain = helm_upgrade_install_args("app", "oci://registry/app", "app", true);
        assert_eq!(plain.last(), Some(&"--plain-http"));
    }
}
