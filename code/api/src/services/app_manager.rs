use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::{app_operation, app_state};
use crate::services::catalog::AppCatalog;
use crate::services::deployment::{DeploymentManager, DeploymentRequest};
use crate::services::k8s::K8sClient;
use crate::services::storage_config;
use crate::state::{SharedCatalog, SharedK8sClient};

pub const OP_INSTALL: &str = "install";
pub const OP_UPDATE: &str = "update";
pub const OP_DELETE: &str = "delete";
pub const OP_RESTART: &str = "restart";

pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_SUCCEEDED: &str = "succeeded";
pub const STATUS_FAILED: &str = "failed";

pub const DESIRED_INSTALLED: &str = "installed";
pub const DESIRED_REMOVED: &str = "removed";

pub const OBS_NOT_INSTALLED: &str = "not_installed";
pub const OBS_INSTALLING: &str = "installing";
pub const OBS_INSTALLED: &str = "installed";
pub const OBS_UNHEALTHY: &str = "unhealthy";
pub const OBS_DELETING: &str = "deleting";
pub const OBS_FAILED: &str = "failed";

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AppOperationResponse {
    pub id: String,
    pub app_name: String,
    pub operation: String,
    pub status: String,
    pub message: Option<String>,
    pub error: Option<String>,
    pub attempts: i32,
    pub created_by: Option<i64>,
    pub created_at: chrono::DateTime<Utc>,
    pub started_at: Option<chrono::DateTime<Utc>>,
    pub finished_at: Option<chrono::DateTime<Utc>>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AppStateResponse {
    pub app_name: String,
    pub namespace: String,
    pub desired_state: String,
    pub observed_state: String,
    pub healthy: bool,
    pub message: Option<String>,
    pub installed_chart_version: Option<String>,
    pub available_chart_version: Option<String>,
    pub update_available: bool,
    pub last_operation_id: Option<String>,
    pub last_checked_at: Option<chrono::DateTime<Utc>>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct AppManager {
    db: DatabaseConnection,
    k8s_client: SharedK8sClient,
    catalog: SharedCatalog,
}

impl AppManager {
    pub fn new(
        db: DatabaseConnection,
        k8s_client: SharedK8sClient,
        catalog: SharedCatalog,
    ) -> Self {
        Self {
            db,
            k8s_client,
            catalog,
        }
    }

    pub async fn enqueue_operation(
        &self,
        app_name: &str,
        operation: &str,
        custom_config: HashMap<String, String>,
        created_by: Option<i64>,
    ) -> Result<AppOperationResponse> {
        validate_operation(operation)?;

        if operation != OP_DELETE {
            let catalog = self.catalog.read().await;
            if catalog.get_app(app_name).is_none() {
                return Err(AppError::NotFound(format!(
                    "App '{}' not found in catalog",
                    app_name
                )));
            }
        }

        let now = Utc::now();
        let operation_model = app_operation::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            app_name: Set(app_name.to_string()),
            operation: Set(operation.to_string()),
            status: Set(STATUS_QUEUED.to_string()),
            message: Set(Some(format!("Queued {} for {}", operation, app_name))),
            error: Set(None),
            custom_config: Set(Some(serde_json::to_string(&custom_config).map_err(
                |e| AppError::Internal(format!("Failed to serialize operation config: {}", e)),
            )?)),
            attempts: Set(0),
            created_by: Set(created_by),
            created_at: Set(now),
            started_at: Set(None),
            finished_at: Set(None),
            updated_at: Set(now),
        };

        let inserted = operation_model.insert(&self.db).await?;
        self.mark_state_for_operation(app_name, operation, &inserted.id)
            .await?;
        Ok(inserted.into())
    }

    pub async fn list_operations(&self) -> Result<Vec<AppOperationResponse>> {
        let operations = app_operation::Entity::find()
            .order_by_desc(app_operation::Column::CreatedAt)
            .all(&self.db)
            .await?;
        Ok(operations.into_iter().map(Into::into).collect())
    }

    pub async fn get_operation(&self, id: &str) -> Result<AppOperationResponse> {
        let operation = app_operation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Operation '{}' not found", id)))?;
        Ok(operation.into())
    }

    pub async fn list_states(&self) -> Result<Vec<AppStateResponse>> {
        let states = app_state::Entity::find()
            .order_by_asc(app_state::Column::AppName)
            .all(&self.db)
            .await?;
        Ok(states.into_iter().map(Into::into).collect())
    }

    pub async fn get_state(&self, app_name: &str) -> Result<AppStateResponse> {
        let state = app_state::Entity::find_by_id(app_name.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("State for '{}' not found", app_name)))?;
        Ok(state.into())
    }

    pub async fn run_worker(
        self: Arc<Self>,
        poll_interval: Duration,
        reconcile_interval: Duration,
    ) {
        let operation_worker = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(poll_interval);
            loop {
                ticker.tick().await;
                if let Err(e) = operation_worker.process_next_operation().await {
                    tracing::error!(error = %e, "App worker operation loop failed");
                }
            }
        });

        let reconcile_worker = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(reconcile_interval);
            loop {
                ticker.tick().await;
                if let Err(e) = reconcile_worker.reconcile_states().await {
                    tracing::error!(error = %e, "App worker reconcile loop failed");
                }
            }
        });
    }

    async fn process_next_operation(&self) -> Result<()> {
        let Some(operation) = self.claim_next_operation().await? else {
            return Ok(());
        };

        let result = self.execute_operation(&operation).await;
        match result {
            Ok(message) => {
                self.finish_operation(&operation.id, STATUS_SUCCEEDED, Some(message), None)
                    .await?;
                self.reconcile_app(&operation.app_name).await?;
            }
            Err(e) => {
                let error = e.to_string();
                self.finish_operation(
                    &operation.id,
                    STATUS_FAILED,
                    Some(format!("{} failed", operation.operation)),
                    Some(error.clone()),
                )
                .await?;
                self.upsert_state(
                    &operation.app_name,
                    &operation.app_name,
                    desired_state_for_operation(&operation.operation),
                    OBS_FAILED,
                    false,
                    Some(error),
                    Some(operation.id.clone()),
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn claim_next_operation(&self) -> Result<Option<app_operation::Model>> {
        let Some(operation) = app_operation::Entity::find()
            .filter(app_operation::Column::Status.eq(STATUS_QUEUED))
            .order_by_asc(app_operation::Column::CreatedAt)
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };

        let now = Utc::now();
        let attempts = operation.attempts + 1;
        let mut active: app_operation::ActiveModel = operation.into();
        active.status = Set(STATUS_RUNNING.to_string());
        active.message = Set(Some("Worker started operation".to_string()));
        active.started_at = Set(Some(now));
        active.updated_at = Set(now);
        active.attempts = Set(attempts);

        Ok(Some(active.update(&self.db).await?))
    }

    async fn execute_operation(&self, operation: &app_operation::Model) -> Result<String> {
        let k8s_guard = self.k8s_client.read().await;
        let client = k8s_guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;
        let catalog = self.catalog.read().await;

        match operation.operation.as_str() {
            OP_INSTALL | OP_UPDATE => {
                self.execute_install_or_update(operation, client, &catalog)
                    .await
            }
            OP_DELETE => {
                let manager = DeploymentManager::new(client, &catalog);
                manager.remove_app(&operation.app_name).await?;
                self.upsert_state(
                    &operation.app_name,
                    &operation.app_name,
                    DESIRED_REMOVED,
                    OBS_NOT_INSTALLED,
                    false,
                    Some("Removed".to_string()),
                    Some(operation.id.clone()),
                )
                .await?;
                Ok(format!("Removed {}", operation.app_name))
            }
            OP_RESTART => {
                self.restart_app(client, &operation.app_name).await?;
                Ok(format!("Restarted {}", operation.app_name))
            }
            _ => Err(AppError::BadRequest(format!(
                "Unsupported operation '{}'",
                operation.operation
            ))),
        }
    }

    async fn execute_install_or_update(
        &self,
        operation: &app_operation::Model,
        client: &K8sClient,
        catalog: &AppCatalog,
    ) -> Result<String> {
        let custom_config: HashMap<String, String> = operation
            .custom_config
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();

        let app_config = catalog.get_app(&operation.app_name).ok_or_else(|| {
            AppError::NotFound(format!("App '{}' not found in catalog", operation.app_name))
        })?;

        let storage = storage_config::get_storage_config_from_db(&self.db)
            .await?
            .map(|(config, _)| config);
        if !app_config.is_system {
            let storage = storage.as_ref().ok_or_else(|| {
                AppError::BadRequest(
                    "Storage must be configured and validated before installing apps".to_string(),
                )
            })?;
            if !storage.validated() {
                return Err(AppError::BadRequest(
                    "Storage must be validated before installing apps".to_string(),
                ));
            }
        }

        let manager = DeploymentManager::with_db(client, catalog, &self.db);
        let request = DeploymentRequest {
            app_name: operation.app_name.clone(),
            custom_config,
            reuse_values: operation.operation == OP_UPDATE,
            wait: !(operation.operation == OP_UPDATE && operation.app_name == "kubarr-worker"),
        };
        let deployment_storage = if app_config.is_system {
            None
        } else {
            storage.as_ref()
        };
        let status = manager.deploy_app(&request, deployment_storage).await?;

        self.upsert_state(
            &operation.app_name,
            &status.namespace,
            DESIRED_INSTALLED,
            OBS_INSTALLING,
            false,
            Some(status.message.clone()),
            Some(operation.id.clone()),
        )
        .await?;

        Ok(status.message)
    }

    async fn restart_app(&self, client: &K8sClient, app_name: &str) -> Result<()> {
        let pods = client.get_pod_status(app_name, Some(app_name)).await?;
        let pod_api: Api<Pod> = Api::namespaced(client.client().clone(), app_name);

        for pod in &pods {
            let _ = pod_api.delete(&pod.name, &DeleteParams::default()).await;
        }

        Ok(())
    }

    async fn finish_operation(
        &self,
        id: &str,
        status: &str,
        message: Option<String>,
        error: Option<String>,
    ) -> Result<()> {
        let operation = app_operation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Operation '{}' not found", id)))?;
        let now = Utc::now();
        let mut active: app_operation::ActiveModel = operation.into();
        active.status = Set(status.to_string());
        active.message = Set(message);
        active.error = Set(error);
        active.finished_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&self.db).await?;
        Ok(())
    }

    async fn mark_state_for_operation(
        &self,
        app_name: &str,
        operation: &str,
        id: &str,
    ) -> Result<()> {
        let observed = match operation {
            OP_DELETE => OBS_DELETING,
            OP_INSTALL | OP_UPDATE => OBS_INSTALLING,
            OP_RESTART => OBS_INSTALLED,
            _ => OBS_INSTALLING,
        };

        self.upsert_state(
            app_name,
            &crate::services::catalog::lifecycle_for_app_name(app_name).namespace,
            desired_state_for_operation(operation),
            observed,
            false,
            Some(format!("Queued {}", operation)),
            Some(id.to_string()),
        )
        .await
    }

    async fn reconcile_states(&self) -> Result<()> {
        let app_names: Vec<String> = {
            let catalog = self.catalog.read().await;
            catalog
                .get_all_apps()
                .into_iter()
                .filter(|app| !app.is_hidden)
                .map(|app| app.name.clone())
                .collect()
        };

        for app_name in app_names {
            if let Err(e) = self.reconcile_app(&app_name).await {
                tracing::warn!(app = app_name, error = %e, "Failed to reconcile app state");
            }
        }

        Ok(())
    }

    async fn reconcile_app(&self, app_name: &str) -> Result<()> {
        let k8s_guard = self.k8s_client.read().await;
        let Some(client) = k8s_guard.as_ref() else {
            return Ok(());
        };
        let catalog = self.catalog.read().await;
        let manager = DeploymentManager::new(client, &catalog);

        if catalog.get_app(app_name).is_none() {
            return Err(AppError::NotFound(format!(
                "App '{}' not found in catalog",
                app_name
            )));
        }
        let namespace = crate::services::catalog::lifecycle_for_app_name(app_name).namespace;

        if !manager.check_namespace_exists(&namespace).await {
            self.upsert_state(
                app_name,
                &namespace,
                DESIRED_REMOVED,
                OBS_NOT_INSTALLED,
                false,
                Some("Not installed".to_string()),
                None,
            )
            .await?;
            return Ok(());
        }

        let health = manager.app_health(app_name).await?;
        let healthy = health["healthy"].as_bool().unwrap_or(false);
        let message = health["message"]
            .as_str()
            .map(ToString::to_string)
            .or_else(|| Some("State reconciled".to_string()));
        let observed = if healthy {
            OBS_INSTALLED
        } else {
            OBS_UNHEALTHY
        };

        self.upsert_state(
            app_name,
            &namespace,
            DESIRED_INSTALLED,
            observed,
            healthy,
            message,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn upsert_state(
        &self,
        app_name: &str,
        namespace: &str,
        desired_state: &str,
        observed_state: &str,
        healthy: bool,
        message: Option<String>,
        operation_id: Option<String>,
    ) -> Result<()> {
        let now = Utc::now();
        let (available_chart_version, installed_chart_version) = {
            let k8s_guard = self.k8s_client.read().await;
            let catalog = self.catalog.read().await;
            let available = catalog.chart_version(app_name);
            let installed = k8s_guard.as_ref().and_then(|client| {
                let manager = DeploymentManager::new(client, &catalog);
                manager.installed_chart_version(app_name)
            });
            (available, installed)
        };
        let update_available = matches!(
            (&installed_chart_version, &available_chart_version),
            (Some(installed), Some(available)) if installed != available
        );
        if let Some(existing) = app_state::Entity::find_by_id(app_name.to_string())
            .one(&self.db)
            .await?
        {
            let mut active: app_state::ActiveModel = existing.into();
            active.namespace = Set(namespace.to_string());
            active.desired_state = Set(desired_state.to_string());
            active.observed_state = Set(observed_state.to_string());
            active.healthy = Set(healthy);
            active.message = Set(message);
            if operation_id.is_some() {
                active.last_operation_id = Set(operation_id);
            }
            active.installed_chart_version = Set(installed_chart_version);
            active.available_chart_version = Set(available_chart_version);
            active.update_available = Set(update_available);
            active.last_checked_at = Set(Some(now));
            active.updated_at = Set(now);
            active.update(&self.db).await?;
        } else {
            app_state::ActiveModel {
                app_name: Set(app_name.to_string()),
                namespace: Set(namespace.to_string()),
                desired_state: Set(desired_state.to_string()),
                observed_state: Set(observed_state.to_string()),
                healthy: Set(healthy),
                message: Set(message),
                installed_chart_version: Set(installed_chart_version),
                available_chart_version: Set(available_chart_version),
                update_available: Set(update_available),
                last_operation_id: Set(operation_id),
                last_checked_at: Set(Some(now)),
                updated_at: Set(now),
            }
            .insert(&self.db)
            .await?;
        }

        Ok(())
    }
}

fn validate_operation(operation: &str) -> Result<()> {
    match operation {
        OP_INSTALL | OP_UPDATE | OP_DELETE | OP_RESTART => Ok(()),
        _ => Err(AppError::BadRequest(format!(
            "Unsupported operation '{}'",
            operation
        ))),
    }
}

fn desired_state_for_operation(operation: &str) -> &'static str {
    if operation == OP_DELETE {
        DESIRED_REMOVED
    } else {
        DESIRED_INSTALLED
    }
}

impl From<app_operation::Model> for AppOperationResponse {
    fn from(model: app_operation::Model) -> Self {
        Self {
            id: model.id,
            app_name: model.app_name,
            operation: model.operation,
            status: model.status,
            message: model.message,
            error: model.error,
            attempts: model.attempts,
            created_by: model.created_by,
            created_at: model.created_at,
            started_at: model.started_at,
            finished_at: model.finished_at,
            updated_at: model.updated_at,
        }
    }
}

impl From<app_state::Model> for AppStateResponse {
    fn from(model: app_state::Model) -> Self {
        Self {
            app_name: model.app_name,
            namespace: model.namespace,
            desired_state: model.desired_state,
            observed_state: model.observed_state,
            healthy: model.healthy,
            message: model.message,
            installed_chart_version: model.installed_chart_version,
            available_chart_version: model.available_chart_version,
            update_available: model.update_available,
            last_operation_id: model.last_operation_id,
            last_checked_at: model.last_checked_at,
            updated_at: model.updated_at,
        }
    }
}
