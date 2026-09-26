use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::models::{app_operation, app_state, audit_outbox, user};
use crate::services::audit::log_on_transaction;
use crate::services::catalog::AppCatalog;
use crate::services::deployment::{DeploymentManager, DeploymentRequest};
use crate::services::gpu::{self, GpuSelection, QUEUED_GPU_KEY};
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
pub const STATUS_PAUSED: &str = "paused";
pub const STATUS_CANCELLED: &str = "cancelled";
pub const STATUS_RETRIED: &str = "retried";
const INTERRUPTED_LABEL: &str = "outcome_indeterminate_after_worker_interruption";
const STALE_RUNNING_AFTER: chrono::Duration = chrono::Duration::minutes(15);
const RUNNING_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(250);
const STOP_ERROR: &str = "Stop requested; external outcome indeterminate";

pub const DESIRED_INSTALLED: &str = "installed";
pub const DESIRED_REMOVED: &str = "removed";

pub const OBS_NOT_INSTALLED: &str = "not_installed";
pub const OBS_INSTALLING: &str = "installing";
pub const OBS_INSTALLED: &str = "installed";
pub const OBS_UNHEALTHY: &str = "unhealthy";
pub const OBS_DELETING: &str = "deleting";
pub const OBS_FAILED: &str = "failed";

pub struct AppWorkerTasks {
    operation: JoinHandle<()>,
    reconciliation: JoinHandle<()>,
    audit_delivery: JoinHandle<()>,
}

impl AppWorkerTasks {
    pub async fn wait(self) {
        for (name, task) in [
            ("operation", self.operation),
            ("reconciliation", self.reconciliation),
            ("audit delivery", self.audit_delivery),
        ] {
            if let Err(error) = task.await {
                tracing::error!(task = name, %error, "App worker task stopped unexpectedly");
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AppOperationResponse {
    pub id: String,
    pub app_name: String,
    pub operation: String,
    pub status: String,
    pub stop_requested: bool,
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
        gpu::validate_custom_gpu_keys(&custom_config)?;
        self.enqueue_operation_unchecked(app_name, operation, custom_config, created_by)
            .await
    }

    async fn persist_operation(
        &self,
        app_name: &str,
        operation: &str,
        custom_config: HashMap<String, String>,
        created_by: Option<i64>,
    ) -> Result<AppOperationResponse> {
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
            stop_requested: Set(false),
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

        let transaction = self.db.begin().await?;
        let inserted = operation_model.insert(&transaction).await?;
        self.mark_state_for_operation(&transaction, app_name, operation, &inserted.id)
            .await?;
        log_queued(&transaction, &inserted).await?;
        transaction.commit().await?;
        Ok(inserted.into())
    }

    pub async fn enqueue_gpu_operation(
        &self,
        app_name: &str,
        operation: &str,
        mut custom_config: HashMap<String, String>,
        gpu: Option<Option<GpuSelection>>,
        created_by: Option<i64>,
    ) -> Result<AppOperationResponse> {
        gpu::validate_custom_gpu_keys(&custom_config)?;
        if gpu.is_some() && !matches!(app_name, "plex" | "jellyfin") {
            return Err(AppError::BadRequest(
                "GPU is supported only for Plex and Jellyfin".into(),
            ));
        }
        if gpu.is_some() {
            gpu::validate_gpu_scheduling_config(&custom_config)?;
        }
        if let Some(selection) = gpu.as_ref().and_then(Option::as_ref) {
            let guard = self.k8s_client.read().await;
            let client = guard
                .as_ref()
                .ok_or_else(|| AppError::Internal("Kubernetes client not available".into()))?;
            gpu::validate_selection(client, selection).await?;
        }
        if gpu.is_some() {
            custom_config.insert(QUEUED_GPU_KEY.into(), serde_json::to_string(&gpu)?);
        }
        // Only this method may insert the reserved queue key.
        self.enqueue_operation_unchecked(app_name, operation, custom_config, created_by)
            .await
    }

    async fn enqueue_operation_unchecked(
        &self,
        app_name: &str,
        operation: &str,
        custom_config: HashMap<String, String>,
        created_by: Option<i64>,
    ) -> Result<AppOperationResponse> {
        validate_operation(operation)?;
        self.persist_operation(app_name, operation, custom_config, created_by)
            .await
    }

    /// Queue an app update using the caller's transaction.
    ///
    /// This is intentionally narrow: callers that change deployment inputs can commit the
    /// input and its operation together, so a queue failure never leaves unapplied config.
    pub async fn enqueue_update_in_transaction(
        &self,
        transaction: &DatabaseTransaction,
        app_name: &str,
        created_by: Option<i64>,
    ) -> Result<AppOperationResponse> {
        let available_chart_version = {
            let catalog = self.catalog.read().await;
            let app = catalog.get_app(app_name).ok_or_else(|| {
                AppError::NotFound(format!("App '{}' not found in catalog", app_name))
            })?;
            if app.is_system {
                return Err(AppError::BadRequest(format!(
                    "VPN is not supported for system app '{}'",
                    app_name
                )));
            }
            catalog.chart_version(app_name)
        };

        let now = Utc::now();
        let id = Uuid::new_v4().to_string();
        let operation = app_operation::ActiveModel {
            id: Set(id.clone()),
            app_name: Set(app_name.to_string()),
            operation: Set(OP_UPDATE.to_string()),
            status: Set(STATUS_QUEUED.to_string()),
            stop_requested: Set(false),
            message: Set(Some(format!("Queued {} for {}", OP_UPDATE, app_name))),
            error: Set(None),
            custom_config: Set(Some("{}".to_string())),
            attempts: Set(0),
            created_by: Set(created_by),
            created_at: Set(now),
            started_at: Set(None),
            finished_at: Set(None),
            updated_at: Set(now),
        }
        .insert(transaction)
        .await?;

        if let Some(existing) = app_state::Entity::find_by_id(app_name.to_string())
            .one(transaction)
            .await?
        {
            let mut active: app_state::ActiveModel = existing.into();
            active.namespace =
                Set(crate::services::catalog::lifecycle_for_app_name(app_name).namespace);
            active.desired_state = Set(DESIRED_INSTALLED.to_string());
            active.observed_state = Set(OBS_INSTALLING.to_string());
            active.healthy = Set(false);
            active.message = Set(Some(format!("Queued {}", OP_UPDATE)));
            active.last_operation_id = Set(Some(id));
            active.last_checked_at = Set(Some(now));
            active.updated_at = Set(now);
            active.update(transaction).await?;
        } else {
            app_state::ActiveModel {
                app_name: Set(app_name.to_string()),
                namespace: Set(crate::services::catalog::lifecycle_for_app_name(app_name).namespace),
                desired_state: Set(DESIRED_INSTALLED.to_string()),
                observed_state: Set(OBS_INSTALLING.to_string()),
                healthy: Set(false),
                message: Set(Some(format!("Queued {}", OP_UPDATE))),
                installed_chart_version: Set(None),
                available_chart_version: Set(available_chart_version),
                update_available: Set(false),
                last_operation_id: Set(Some(id)),
                last_checked_at: Set(Some(now)),
                updated_at: Set(now),
            }
            .insert(transaction)
            .await?;
        }

        log_queued(transaction, &operation).await?;
        Ok(operation.into())
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

    /// Queued controls are transactional CAS updates. Running cancel records a
    /// stop request; the worker alone resolves the external outcome.
    pub async fn control_operation(
        &self,
        id: &str,
        action: &str,
        actor_id: i64,
    ) -> Result<AppOperationResponse> {
        let (from, to) = match action {
            "pause" => (STATUS_QUEUED, STATUS_PAUSED),
            "resume" => (STATUS_PAUSED, STATUS_QUEUED),
            "cancel" => (STATUS_QUEUED, STATUS_CANCELLED),
            _ => return Err(AppError::BadRequest("Unsupported operation control".into())),
        };
        let transaction = self.db.begin().await?;
        let operation = app_operation::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Operation '{id}' not found")))?;
        if action == "cancel" && operation.status == STATUS_RUNNING {
            let now = Utc::now();
            let changed = app_operation::Entity::update_many()
                .col_expr(app_operation::Column::StopRequested, true.into())
                .col_expr(app_operation::Column::Message, "Stop requested".into())
                .col_expr(app_operation::Column::UpdatedAt, now.into())
                .filter(app_operation::Column::Id.eq(id))
                .filter(app_operation::Column::Status.eq(STATUS_RUNNING))
                .filter(app_operation::Column::StopRequested.eq(false))
                .exec(&transaction)
                .await?;
            if changed.rows_affected != 1 {
                return Err(AppError::Conflict(
                    "Operation already stopped or completed".into(),
                ));
            }
            log_control(&transaction, &operation, "stop_requested", actor_id).await?;
            let result = app_operation::Entity::find_by_id(id)
                .one(&transaction)
                .await?
                .unwrap();
            transaction.commit().await?;
            return Ok(result.into());
        }
        let now = Utc::now();
        let changed = app_operation::Entity::update_many()
            .col_expr(
                app_operation::Column::Status,
                sea_orm::sea_query::Expr::value(to),
            )
            .col_expr(
                app_operation::Column::Message,
                sea_orm::sea_query::Expr::value(format!(
                    "Operation {} by user {actor_id}",
                    match action {
                        "cancel" => "cancelled",
                        "pause" => "paused",
                        _ => "resumed",
                    }
                )),
            )
            .col_expr(
                app_operation::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .filter(app_operation::Column::Id.eq(id))
            .filter(if action == "cancel" {
                app_operation::Column::Status.is_in([STATUS_QUEUED, STATUS_PAUSED])
            } else {
                app_operation::Column::Status.eq(from)
            });
        let changed = if action == "cancel" {
            changed
                .col_expr(
                    app_operation::Column::FinishedAt,
                    sea_orm::sea_query::Expr::value(now),
                )
                .exec(&transaction)
                .await?
        } else {
            changed.exec(&transaction).await?
        };
        if changed.rows_affected != 1 {
            return Err(AppError::Conflict(if operation.status == STATUS_RUNNING {
                "Operation already running; cannot safely interrupt Helm".into()
            } else {
                format!(
                    "Operation must be {from} to {action}; current status: {}",
                    operation.status
                )
            }));
        }
        log_control(&transaction, &operation, action, actor_id).await?;
        if action == "cancel" {
            app_state::Entity::update_many()
                .col_expr(
                    app_state::Column::Message,
                    sea_orm::sea_query::Expr::value("Queued operation cancelled"),
                )
                .col_expr(
                    app_state::Column::UpdatedAt,
                    sea_orm::sea_query::Expr::value(now),
                )
                .filter(app_state::Column::AppName.eq(&operation.app_name))
                .filter(app_state::Column::LastOperationId.eq(id))
                .exec(&transaction)
                .await?;
        }
        let result = app_operation::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .unwrap();
        transaction.commit().await?;
        Ok(result.into())
    }

    pub async fn retry_operation(&self, id: &str, actor_id: i64) -> Result<AppOperationResponse> {
        let transaction = self.db.begin().await?;
        let original = app_operation::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Operation '{id}' not found")))?;
        if original.status != STATUS_FAILED {
            return Err(AppError::Conflict(format!(
                "Retry requires failed operation; current status: {}",
                original.status
            )));
        }
        // Validate stored input before copying it, including the reserved internal GPU
        // marker. It must never be accepted from a new public custom-config request.
        validate_operation(&original.operation)?;
        if original.operation == OP_DELETE
            && self
                .catalog
                .read()
                .await
                .get_app(&original.app_name)
                .is_some_and(|app| app.is_system)
        {
            return Err(AppError::Forbidden("Cannot delete system app".into()));
        }
        let config: HashMap<String, String> =
            serde_json::from_str(original.custom_config.as_deref().unwrap_or("{}"))
                .map_err(|_| AppError::BadRequest("Invalid stored operation config".into()))?;
        let mut public_config = config.clone();
        public_config.remove(QUEUED_GPU_KEY);
        gpu::validate_custom_gpu_keys(&public_config)?;
        if let Some(gpu) = config.get(QUEUED_GPU_KEY) {
            if !matches!(original.app_name.as_str(), "plex" | "jellyfin") {
                return Err(AppError::BadRequest("Invalid stored GPU app".into()));
            }
            serde_json::from_str::<Option<GpuSelection>>(gpu)
                .map_err(|_| AppError::BadRequest("Invalid stored GPU selection".into()))?;
        }
        let now = Utc::now();
        let new_id = Uuid::new_v4().to_string();
        let changed = app_operation::Entity::update_many()
            .col_expr(
                app_operation::Column::Status,
                sea_orm::sea_query::Expr::value(STATUS_RETRIED),
            )
            .col_expr(
                app_operation::Column::Message,
                sea_orm::sea_query::Expr::value(format!("Retried as {new_id}")),
            )
            .col_expr(
                app_operation::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .filter(app_operation::Column::Id.eq(id))
            .filter(app_operation::Column::Status.eq(STATUS_FAILED))
            .exec(&transaction)
            .await?;
        if changed.rows_affected != 1 {
            return Err(AppError::Conflict("Operation already retried".into()));
        }
        let inserted = app_operation::ActiveModel {
            id: Set(new_id.clone()),
            app_name: Set(original.app_name.clone()),
            operation: Set(original.operation.clone()),
            status: Set(STATUS_QUEUED.into()),
            stop_requested: Set(false),
            message: Set(Some(format!("Retry of {id}"))),
            error: Set(None),
            custom_config: Set(original.custom_config.clone()),
            attempts: Set(0),
            created_by: Set(Some(actor_id)),
            created_at: Set(now),
            started_at: Set(None),
            finished_at: Set(None),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await?;
        self.mark_state_for_operation(
            &transaction,
            &original.app_name,
            &original.operation,
            &new_id,
        )
        .await?;
        log_control(&transaction, &original, "retry", actor_id).await?;
        log_queued(&transaction, &inserted).await?;
        transaction.commit().await?;
        Ok(inserted.into())
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

    /// Refresh the catalog version fields without waiting for a worker reconcile.
    pub async fn refresh_available_chart_versions(&self) -> Result<()> {
        let catalog = self.catalog.read().await;
        self.refresh_available_chart_versions_with(|app_name| catalog.chart_version(app_name))
            .await
    }

    async fn refresh_available_chart_versions_with(
        &self,
        version_for: impl Fn(&str) -> Option<String>,
    ) -> Result<()> {
        let now = Utc::now();

        for initial_state in app_state::Entity::find().all(&self.db).await? {
            let observed_available = version_for(&initial_state.app_name);
            let mut state = initial_state;
            loop {
                // A successful catalog sync is the authoritative producer for availability.
                // A missing chart is not evidence that an existing version was withdrawn.
                let available = observed_available
                    .clone()
                    .or_else(|| state.available_chart_version.clone());
                let update_available = versions_differ(
                    state.installed_chart_version.as_deref(),
                    available.as_deref(),
                );
                let installed_before = state.installed_chart_version.clone();
                let available_before = state.available_chart_version.clone();
                let app_name = state.app_name.clone();
                let mut active: app_state::ActiveModel = state.into();
                active.available_chart_version = Set(available);
                active.update_available = Set(update_available);
                active.last_checked_at = Set(Some(now));
                active.updated_at = Set(now);
                let mut update = app_state::Entity::update_many()
                    .set(active)
                    .filter(app_state::Column::AppName.eq(&app_name));
                update = filter_optional_string(
                    update,
                    app_state::Column::InstalledChartVersion,
                    installed_before.as_deref(),
                );
                update = filter_optional_string(
                    update,
                    app_state::Column::AvailableChartVersion,
                    available_before.as_deref(),
                );
                if update.exec(&self.db).await?.rows_affected != 0 {
                    break;
                }
                let Some(current) = app_state::Entity::find_by_id(app_name)
                    .one(&self.db)
                    .await?
                else {
                    break;
                };
                state = current;
            }
        }

        Ok(())
    }

    pub fn run_worker(
        self: Arc<Self>,
        poll_interval: Duration,
        reconcile_interval: Duration,
        cancellation: CancellationToken,
    ) -> AppWorkerTasks {
        let operation_worker = self.clone();
        let operation_cancellation = cancellation.clone();
        let operation = tokio::spawn(async move {
            operation_worker
                .run_operation_loop(poll_interval, operation_cancellation)
                .await;
        });

        let reconcile_worker = self.clone();
        let audit_cancellation = cancellation.clone();
        let reconciliation = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(reconcile_interval);
            loop {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => break,
                    _ = ticker.tick() => {
                        if cancellation.is_cancelled() {
                            break;
                        }
                        if let Err(e) = reconcile_worker.reconcile_states().await {
                            tracing::error!(error = %e, "App worker reconcile loop failed");
                        }
                    }
                }
            }
        });

        let audit_worker = self.clone();
        let audit_delivery = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    biased;
                    _ = audit_cancellation.cancelled() => break,
                    _ = ticker.tick() => {
                        if audit_cancellation.is_cancelled() { break; }
                        if let Err(error) = audit_worker.drain_audit_outbox().await {
                            tracing::error!(%error, "App terminal audit delivery failed; will retry");
                        }
                    }
                }
            }
        });

        AppWorkerTasks {
            operation,
            reconciliation,
            audit_delivery,
        }
    }

    async fn run_operation_loop(
        self: Arc<Self>,
        poll_interval: Duration,
        cancellation: CancellationToken,
    ) {
        self.run_operation_loop_with(poll_interval, cancellation, {
            let manager = self.clone();
            move |operation, stop| {
                let manager = manager.clone();
                async move { manager.execute_operation(&operation, &stop).await }
            }
        })
        .await;
    }

    async fn run_operation_loop_with<F, Fut>(
        &self,
        poll_interval: Duration,
        cancellation: CancellationToken,
        execute: F,
    ) where
        F: Fn(app_operation::Model, CancellationToken) -> Fut,
        Fut: Future<Output = Result<String>>,
    {
        let mut ticker = tokio::time::interval(poll_interval);
        loop {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                _ = ticker.tick() => {
                    if cancellation.is_cancelled() {
                        break;
                    }
                    // This loop is sequential: it never classifies its own in-flight Helm
                    // execution as abandoned. Recreate deployment keeps predecessor workers gone.
                    if let Err(e) = self.recover_stale_operations().await {
                        tracing::error!(error = %e, "Failed to recover interrupted app operations");
                        continue;
                    }
                    // Once claimed, an operation is deliberately allowed to finish during shutdown.
                    if let Err(e) = self.process_next_operation_with_stop(&execute, RUNNING_HEARTBEAT_INTERVAL).await {
                        tracing::error!(error = %e, "App worker operation loop failed");
                    }
                }
            }
        }
    }

    #[cfg(test)]
    async fn process_next_operation_with<F, Fut>(&self, execute: F) -> Result<()>
    where
        F: FnOnce(app_operation::Model) -> Fut,
        Fut: Future<Output = Result<String>>,
    {
        self.process_next_operation_with_heartbeat(execute, RUNNING_HEARTBEAT_INTERVAL)
            .await
    }

    #[cfg(test)]
    async fn process_next_operation_with_heartbeat<F, Fut>(
        &self,
        execute: F,
        heartbeat_interval: Duration,
    ) -> Result<()>
    where
        F: FnOnce(app_operation::Model) -> Fut,
        Fut: Future<Output = Result<String>>,
    {
        self.process_next_operation_with_stop(|operation, _| execute(operation), heartbeat_interval)
            .await
    }

    async fn process_next_operation_with_stop<F, Fut>(
        &self,
        execute: F,
        heartbeat_interval: Duration,
    ) -> Result<()>
    where
        F: FnOnce(app_operation::Model, CancellationToken) -> Fut,
        Fut: Future<Output = Result<String>>,
    {
        let Some(operation) = self.claim_next_operation().await? else {
            return Ok(());
        };

        tracing::info!(
            operation_id = %operation.id,
            app = %operation.app_name,
            operation = %operation.operation,
            attempts = operation.attempts,
            "App operation claimed"
        );

        // JoinSet aborts the heartbeat if this future is dropped (e.g. a panic).
        // Normal shutdown lets an already-claimed external operation finish.
        let heartbeat_stop = CancellationToken::new();
        let stop = CancellationToken::new();
        let mut heartbeat = JoinSet::new();
        heartbeat.spawn(heartbeat_running_operation(
            self.db.clone(),
            operation.id.clone(),
            heartbeat_interval,
            heartbeat_stop.clone(),
        ));
        heartbeat.spawn(watch_stop(
            self.db.clone(),
            operation.id.clone(),
            stop.clone(),
        ));
        let result = execute(operation.clone(), stop.clone()).await;
        heartbeat_stop.cancel();
        // Do not stop the watcher until finalization has resolved the stop/success race.
        let stopped = stop.is_cancelled();
        let result = if stopped {
            Err(AppError::Internal(STOP_ERROR.into()))
        } else {
            result
        };
        // The watcher exits when the row becomes terminal, or when we finish below.
        // The heartbeat task exits immediately; the watcher is aborted on drop.
        heartbeat.abort_all();
        if let Some(Err(error)) = heartbeat.join_next().await {
            tracing::error!(operation_id = %operation.id, %error, "App operation heartbeat task stopped unexpectedly");
        }
        let stop_requested = app_operation::Entity::find_by_id(&operation.id)
            .one(&self.db)
            .await?
            .is_some_and(|row| row.stop_requested);
        if stop_requested {
            self.finish_stopped(&operation).await?;
        } else if stopped {
            if !self
                .finish_operation_if_unstopped(
                    &operation.id,
                    STATUS_FAILED,
                    Some(STOP_ERROR.into()),
                    Some(STOP_ERROR.into()),
                )
                .await?
            {
                self.finish_stopped(&operation).await?;
            }
        } else {
            match result {
                Ok(message) => {
                    let won = self
                        .finish_operation_if_unstopped(
                            &operation.id,
                            STATUS_SUCCEEDED,
                            Some(message.clone()),
                            None,
                        )
                        .await?;
                    if won && operation.operation != OP_RESTART {
                        if let Err(error) = self
                            .upsert_state(
                                &operation.app_name,
                                &crate::services::catalog::lifecycle_for_app_name(
                                    &operation.app_name,
                                )
                                .namespace,
                                desired_state_for_operation(&operation.operation),
                                if operation.operation == OP_DELETE {
                                    OBS_NOT_INSTALLED
                                } else {
                                    OBS_INSTALLING
                                },
                                false,
                                Some(if operation.operation == OP_DELETE {
                                    "Removed".to_string()
                                } else {
                                    message.clone()
                                }),
                                Some(operation.id.clone()),
                                false,
                            )
                            .await
                        {
                            tracing::error!(operation_id = %operation.id, %error, "App state update failed after external success");
                        }
                    }
                    if won {
                        if let Err(error) = self.reconcile_app(&operation.app_name).await {
                            tracing::error!(operation_id = %operation.id, %error, "App reconcile failed after terminal commit");
                        }
                    } else {
                        self.finish_stopped(&operation).await?;
                    }
                    tracing::info!(
                        operation_id = %operation.id,
                        app = %operation.app_name,
                        operation = %operation.operation,
                        "App operation finished"
                    );
                }
                Err(e) => {
                    let error = e.to_string();
                    let message = if error.contains("external outcome indeterminate") {
                        "Helm deadline exceeded; external outcome indeterminate".to_string()
                    } else {
                        format!("{} failed", operation.operation)
                    };
                    let won = self
                        .finish_operation_if_unstopped(
                            &operation.id,
                            STATUS_FAILED,
                            Some(message),
                            Some(error.clone()),
                        )
                        .await?;
                    if !won {
                        self.finish_stopped(&operation).await?;
                    }
                    if won {
                        if let Err(state_error) = self
                            .upsert_state(
                                &operation.app_name,
                                &crate::services::catalog::lifecycle_for_app_name(
                                    &operation.app_name,
                                )
                                .namespace,
                                desired_state_for_operation(&operation.operation),
                                OBS_FAILED,
                                false,
                                Some(error),
                                Some(operation.id.clone()),
                                false,
                            )
                            .await
                        {
                            tracing::error!(operation_id = %operation.id, %state_error, "App failure state update failed after terminal commit");
                        }
                    }
                    tracing::warn!(
                        operation_id = %operation.id,
                        app = %operation.app_name,
                        operation = %operation.operation,
                        "App operation failed"
                    );
                }
            }
        }

        if let Err(error) = self.deliver_audit(&operation.id).await {
            tracing::error!(operation_id = %operation.id, %error, "Terminal app audit pending retry");
        }

        Ok(())
    }

    async fn claim_next_operation(&self) -> Result<Option<app_operation::Model>> {
        let Some(operation) = app_operation::Entity::find()
            .filter(app_operation::Column::Status.eq(STATUS_QUEUED))
            .filter(app_operation::Column::StopRequested.eq(false))
            .order_by_asc(app_operation::Column::CreatedAt)
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };

        self.claim_operation(operation).await
    }

    async fn claim_operation(
        &self,
        operation: app_operation::Model,
    ) -> Result<Option<app_operation::Model>> {
        let now = Utc::now();
        let attempts = operation.attempts + 1;
        let id = operation.id.clone();
        let mut active: app_operation::ActiveModel = operation.into();
        active.status = Set(STATUS_RUNNING.to_string());
        active.message = Set(Some("Worker started operation".to_string()));
        active.started_at = Set(Some(now));
        active.updated_at = Set(now);
        active.attempts = Set(attempts);

        // Competing workers may have selected the same row. Only one may execute it.
        let claimed = app_operation::Entity::update_many()
            .set(active)
            .filter(app_operation::Column::Id.eq(&id))
            .filter(app_operation::Column::Status.eq(STATUS_QUEUED))
            .filter(app_operation::Column::StopRequested.eq(false))
            .exec(&self.db)
            .await?;
        if claimed.rows_affected == 0 {
            return Ok(None);
        }
        Ok(app_operation::Entity::find_by_id(id).one(&self.db).await?)
    }

    async fn execute_operation(
        &self,
        operation: &app_operation::Model,
        stop: &CancellationToken,
    ) -> Result<String> {
        check_stop(stop)?;
        let k8s_guard = self.k8s_client.read().await;
        let client = k8s_guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;
        let catalog = self.catalog.read().await;

        match operation.operation.as_str() {
            OP_INSTALL | OP_UPDATE => {
                self.execute_install_or_update(operation, client, &catalog, stop)
                    .await
            }
            OP_DELETE => {
                let manager = DeploymentManager::new(client, &catalog);
                manager
                    .remove_app_with_stop(&operation.app_name, Some(stop))
                    .await?;
                Ok(format!("Removed {}", operation.app_name))
            }
            OP_RESTART => {
                check_stop(stop)?;
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
        stop: &CancellationToken,
    ) -> Result<String> {
        // A malformed persisted request must fail rather than silently deploying
        // with defaults (which can reset a queued GPU or other chart settings).
        let mut custom_config = parse_queued_config(operation.custom_config.as_deref())?;
        let gpu: Option<Option<GpuSelection>> = custom_config
            .remove(QUEUED_GPU_KEY)
            .map(|raw| {
                serde_json::from_str(&raw)
                    .map_err(|_| AppError::BadRequest("Invalid queued GPU selection".into()))
            })
            .transpose()?;

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
            gpu,
            reuse_values: operation.operation == OP_UPDATE,
            wait: !(operation.operation == OP_UPDATE && operation.app_name == "kubarr-worker"),
        };
        let deployment_storage = if app_config.is_system {
            None
        } else {
            storage.as_ref()
        };
        check_stop(stop)?;
        let status = manager
            .deploy_app_with_stop(&request, deployment_storage, Some(stop))
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

    #[cfg(test)]
    async fn finish_operation(
        &self,
        id: &str,
        status: &str,
        message: Option<String>,
        error: Option<String>,
    ) -> Result<()> {
        if !self
            .finish_operation_if_unstopped(id, status, message, error)
            .await?
        {
            return Err(AppError::Conflict(
                "Operation stop requested or already completed".into(),
            ));
        }
        Ok(())
    }

    async fn finish_stopped(&self, operation: &app_operation::Model) -> Result<()> {
        let transaction = self.db.begin().await?;
        let now = Utc::now();
        let changed = app_operation::Entity::update_many()
            .col_expr(app_operation::Column::Status, STATUS_CANCELLED.into())
            .col_expr(
                app_operation::Column::Message,
                "Stopped; external outcome indeterminate".into(),
            )
            .col_expr(app_operation::Column::Error, STOP_ERROR.into())
            .col_expr(app_operation::Column::FinishedAt, now.into())
            .col_expr(app_operation::Column::UpdatedAt, now.into())
            .filter(app_operation::Column::Id.eq(&operation.id))
            .filter(app_operation::Column::Status.eq(STATUS_RUNNING))
            .filter(app_operation::Column::StopRequested.eq(true))
            .exec(&transaction)
            .await?;
        if changed.rows_affected == 1 {
            self.queue_terminal_audit(&transaction, operation, false, "indeterminate")
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn finish_operation_if_unstopped(
        &self,
        id: &str,
        status: &str,
        message: Option<String>,
        error: Option<String>,
    ) -> Result<bool> {
        let transaction = self.db.begin().await?;
        let operation = app_operation::Entity::find_by_id(id.to_string())
            .one(&transaction)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Operation '{}' not found", id)))?;
        let now = Utc::now();
        let mut active: app_operation::ActiveModel = operation.clone().into();
        active.status = Set(status.to_string());
        active.message = Set(message);
        let phase = if error
            .as_deref()
            .is_some_and(|e| e.contains("external outcome indeterminate"))
        {
            "indeterminate"
        } else {
            "completed"
        };
        active.error = Set(error);
        active.finished_at = Set(Some(now));
        active.updated_at = Set(now);
        let changed = app_operation::Entity::update_many()
            .set(active)
            .filter(app_operation::Column::Id.eq(id))
            .filter(app_operation::Column::Status.eq(STATUS_RUNNING))
            .filter(app_operation::Column::StopRequested.eq(false))
            .exec(&transaction)
            .await?;
        if changed.rows_affected != 1 {
            return Ok(false);
        }
        self.queue_terminal_audit(&transaction, &operation, status == STATUS_SUCCEEDED, phase)
            .await?;
        transaction.commit().await?;
        Ok(true)
    }

    async fn queue_terminal_audit(
        &self,
        transaction: &DatabaseTransaction,
        operation: &app_operation::Model,
        success: bool,
        phase: &str,
    ) -> Result<()> {
        let action = match operation.operation.as_str() {
            OP_INSTALL => AuditAction::AppInstalled,
            OP_DELETE => AuditAction::AppUninstalled,
            OP_RESTART => AuditAction::AppRestarted,
            _ => AuditAction::AppConfigured,
        };
        let (_, username) = actor(transaction, operation.created_by).await?;
        audit_outbox::ActiveModel {
            operation_id: Set(operation.id.clone()),
            action: Set(action.to_string()),
            resource_type: Set(ResourceType::App.to_string()),
            resource_id: Set(operation.app_name.clone()),
            created_by: Set(operation.created_by),
            username: Set(username),
            details: Set(serde_json::json!({
                "operation_id": operation.id, "attempts": operation.attempts, "phase": phase
            })
            .to_string()),
            success: Set(success),
            outcome: Set(if phase == "completed" {
                if success {
                    STATUS_SUCCEEDED
                } else {
                    STATUS_FAILED
                }
            } else {
                "indeterminate"
            }
            .into()),
            error_label: Set(if success {
                None
            } else {
                Some(
                    if phase == "completed" {
                        "app_operation_failed"
                    } else {
                        INTERRUPTED_LABEL
                    }
                    .into(),
                )
            }),
            created_at: Set(Utc::now()),
            processed_at: Set(None),
        }
        .insert(transaction)
        .await?;
        Ok(())
    }

    /// Recover only work whose worker has not updated its running row for a long time.
    /// The external outcome cannot be inferred after a crash; never replay the operation.
    pub async fn recover_stale_operations(&self) -> Result<()> {
        let cutoff = Utc::now() - STALE_RUNNING_AFTER;
        let candidates = app_operation::Entity::find()
            .filter(app_operation::Column::Status.eq(STATUS_RUNNING))
            .filter(app_operation::Column::UpdatedAt.lt(cutoff))
            .all(&self.db)
            .await?;
        for operation in candidates {
            self.recover_stale_operation(operation, cutoff).await?;
        }
        Ok(())
    }

    async fn recover_stale_operation(
        &self,
        operation: app_operation::Model,
        cutoff: chrono::DateTime<Utc>,
    ) -> Result<()> {
        let transaction = self.db.begin().await?;
        let now = Utc::now();
        let mut active: app_operation::ActiveModel = operation.clone().into();
        active.status = Set(if operation.stop_requested {
            STATUS_CANCELLED
        } else {
            STATUS_FAILED
        }
        .into());
        active.message = Set(Some(
            if operation.stop_requested {
                "Stop requested; outcome indeterminate after worker interruption"
            } else {
                "Outcome indeterminate after worker interruption"
            }
            .into(),
        ));
        active.error = Set(Some(
            if operation.stop_requested {
                STOP_ERROR
            } else {
                INTERRUPTED_LABEL
            }
            .into(),
        ));
        active.finished_at = Set(Some(now));
        active.updated_at = Set(now);
        let changed = app_operation::Entity::update_many()
            .set(active)
            .filter(app_operation::Column::Id.eq(&operation.id))
            .filter(app_operation::Column::Status.eq(STATUS_RUNNING))
            // A concurrent cancel changes both this flag and updated_at. Never
            // overwrite that request with the stale snapshot read above.
            .filter(app_operation::Column::StopRequested.eq(operation.stop_requested))
            .filter(app_operation::Column::UpdatedAt.lt(cutoff))
            .exec(&transaction)
            .await?;
        if changed.rows_affected == 0 {
            return Ok(());
        }
        self.queue_terminal_audit(&transaction, &operation, false, "indeterminate")
            .await?;
        transaction.commit().await?;
        tracing::error!(operation_id = %operation.id, "Recovered interrupted app operation with unknown external outcome");
        Ok(())
    }

    /// Both insert and acknowledgement are one transaction. The conditional update
    /// is the delivery claim: a competing dispatcher rolls its audit insert back.
    async fn deliver_audit(&self, id: &str) -> Result<()> {
        let transaction = self.db.begin().await?;
        let Some(event) = audit_outbox::Entity::find_by_id(id)
            .one(&transaction)
            .await?
        else {
            return Ok(());
        };
        if event.processed_at.is_some() {
            return Ok(());
        }
        let claimed = audit_outbox::Entity::update_many()
            .col_expr(audit_outbox::Column::ProcessedAt, Utc::now().into())
            .filter(audit_outbox::Column::OperationId.eq(id))
            .filter(audit_outbox::Column::ProcessedAt.is_null())
            .exec(&transaction)
            .await?;
        if claimed.rows_affected != 1 {
            return Ok(());
        }
        let action = match event.action.as_str() {
            "app_installed" => AuditAction::AppInstalled,
            "app_uninstalled" => AuditAction::AppUninstalled,
            "app_restarted" => AuditAction::AppRestarted,
            "app_configured" => AuditAction::AppConfigured,
            _ => return Err(AppError::Internal("Invalid app audit outbox action".into())),
        };
        log_on_transaction(
            &transaction,
            action,
            ResourceType::App,
            Some(event.resource_id),
            event.created_by,
            event.username,
            Some(serde_json::from_str(&event.details).map_err(|e| {
                AppError::Internal(format!("Invalid app audit outbox details: {e}"))
            })?),
            None,
            None,
            event.success,
            event.error_label,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn drain_audit_outbox(&self) -> Result<()> {
        let pending = audit_outbox::Entity::find()
            .filter(audit_outbox::Column::ProcessedAt.is_null())
            .order_by_asc(audit_outbox::Column::CreatedAt)
            .limit(100)
            .all(&self.db)
            .await?;
        for event in pending {
            // Retry on the next tick if the audit sink is unavailable.
            if let Err(error) = self.deliver_audit(&event.operation_id).await {
                tracing::error!(operation_id = %event.operation_id, %error, "App audit event still pending");
                return Err(error);
            }
        }
        Ok(())
    }

    async fn mark_state_for_operation(
        &self,
        transaction: &DatabaseTransaction,
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

        let now = Utc::now();
        let namespace = crate::services::catalog::lifecycle_for_app_name(app_name).namespace;
        if let Some(existing) = app_state::Entity::find_by_id(app_name)
            .one(transaction)
            .await?
        {
            let mut active: app_state::ActiveModel = existing.into();
            active.namespace = Set(namespace);
            active.desired_state = Set(desired_state_for_operation(operation).into());
            active.observed_state = Set(observed.into());
            active.healthy = Set(false);
            active.message = Set(Some(format!("Queued {}", operation)));
            active.last_operation_id = Set(Some(id.into()));
            active.last_checked_at = Set(Some(now));
            active.updated_at = Set(now);
            active.update(transaction).await?;
        } else {
            let version = self.catalog.read().await.chart_version(app_name);
            app_state::ActiveModel {
                app_name: Set(app_name.into()),
                namespace: Set(namespace),
                desired_state: Set(desired_state_for_operation(operation).into()),
                observed_state: Set(observed.into()),
                healthy: Set(false),
                message: Set(Some(format!("Queued {}", operation))),
                installed_chart_version: Set(None),
                available_chart_version: Set(version),
                update_available: Set(false),
                last_operation_id: Set(Some(id.into())),
                last_checked_at: Set(Some(now)),
                updated_at: Set(now),
            }
            .insert(transaction)
            .await?;
        }
        Ok(())
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
                false,
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
            false,
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
        enqueue: bool,
    ) -> Result<()> {
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
        self.upsert_state_with_chart_versions(
            app_name,
            namespace,
            desired_state,
            observed_state,
            healthy,
            message,
            operation_id,
            enqueue,
            available_chart_version,
            installed_chart_version,
        )
        .await
    }

    /// Version inputs are explicit so persistence can be tested without a Kubernetes client.
    #[allow(clippy::too_many_arguments)]
    async fn upsert_state_with_chart_versions(
        &self,
        app_name: &str,
        namespace: &str,
        desired_state: &str,
        observed_state: &str,
        healthy: bool,
        message: Option<String>,
        operation_id: Option<String>,
        enqueue: bool,
        available_chart_version: Option<String>,
        installed_chart_version: Option<String>,
    ) -> Result<()> {
        let now = Utc::now();
        while let Some(existing) = app_state::Entity::find_by_id(app_name.to_string())
            .one(&self.db)
            .await?
        {
            // A completed operation must not replace a newer queued intent.
            if !enqueue && operation_id.is_some() && operation_id != existing.last_operation_id {
                return Ok(());
            }
            let previous_operation_id = existing.last_operation_id.clone();
            let stored_available = existing.available_chart_version.clone();
            let update_available = versions_differ(
                installed_chart_version.as_deref(),
                stored_available.as_deref(),
            );
            let mut active: app_state::ActiveModel = existing.into();
            active.namespace = Set(namespace.to_string());
            // Reconciliation observes reality; it does not change requested intent.
            if enqueue || operation_id.is_some() {
                active.desired_state = Set(desired_state.to_string());
            }
            active.observed_state = Set(observed_state.to_string());
            active.healthy = Set(healthy);
            active.message = Set(message.clone());
            if operation_id.is_some() {
                active.last_operation_id = Set(operation_id.clone());
            }
            active.installed_chart_version = Set(installed_chart_version.clone());
            // Reconciliation never publishes its process-local catalog observation.
            active.update_available = Set(update_available);
            active.last_checked_at = Set(Some(now));
            active.updated_at = Set(now);
            let mut update = app_state::Entity::update_many()
                .set(active)
                .filter(app_state::Column::AppName.eq(app_name));
            if !enqueue {
                update = update.filter(match previous_operation_id {
                    Some(id) => app_state::Column::LastOperationId.eq(id),
                    None => app_state::Column::LastOperationId.is_null(),
                });
            }
            update = filter_optional_string(
                update,
                app_state::Column::AvailableChartVersion,
                stored_available.as_deref(),
            );
            if update.exec(&self.db).await?.rows_affected != 0 {
                return Ok(());
            }
            // Catalog persistence or a newer operation won the race. Re-read so the
            // derived flag is calculated against the current authoritative target.
        }

        let update_available = versions_differ(
            installed_chart_version.as_deref(),
            available_chart_version.as_deref(),
        );
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

        Ok(())
    }
}

/// Only refresh the liveness clock: never replace terminal status, message,
/// attempts or the terminal timestamp with a stale snapshot of the row.
async fn heartbeat_running_operation(
    db: DatabaseConnection,
    id: String,
    interval: Duration,
    cancellation: CancellationToken,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await; // The claim has just set updated_at; wait one full interval.
    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            _ = ticker.tick() => {
                match app_operation::Entity::update_many()
                    .col_expr(app_operation::Column::UpdatedAt, sea_orm::sea_query::Expr::value(Utc::now()))
                    .filter(app_operation::Column::Id.eq(&id))
                    .filter(app_operation::Column::Status.eq(STATUS_RUNNING))
                    .exec(&db).await {
                        Ok(result) if result.rows_affected == 0 => break,
                        Ok(_) => {},
                        Err(error) => tracing::error!(operation_id = %id, %error, "App operation heartbeat failed"),
                    }
            }
        }
    }
}

fn check_stop(stop: &CancellationToken) -> Result<()> {
    if stop.is_cancelled() {
        Err(AppError::Internal(STOP_ERROR.into()))
    } else {
        Ok(())
    }
}

fn parse_queued_config(config: Option<&str>) -> Result<HashMap<String, String>> {
    serde_json::from_str(config.unwrap_or("{}"))
        .map_err(|_| AppError::BadRequest("Invalid queued operation config".into()))
}

async fn watch_stop(db: DatabaseConnection, id: String, stop: CancellationToken) {
    let mut ticker = tokio::time::interval(STOP_POLL_INTERVAL);
    loop {
        ticker.tick().await;
        match app_operation::Entity::find_by_id(&id).one(&db).await {
            Ok(Some(row)) if row.stop_requested => {
                stop.cancel();
                break;
            }
            Ok(Some(row)) if row.status != STATUS_RUNNING => break,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                // Losing DB visibility cannot be interpreted as permission to continue Helm.
                stop.cancel();
                break;
            }
        }
    }
}

#[cfg(test)]
#[path = "app_manager_tests.rs"]
mod tests;

async fn actor<C: ConnectionTrait>(
    db: &C,
    id: Option<i64>,
) -> Result<(Option<i64>, Option<String>)> {
    let username = if let Some(id) = id {
        user::Entity::find_by_id(id)
            .one(db)
            .await?
            .map(|user| user.username)
    } else {
        None
    };
    Ok((id, username))
}

async fn log_queued<C: ConnectionTrait>(db: &C, operation: &app_operation::Model) -> Result<()> {
    let (user_id, username) = actor(db, operation.created_by).await?;
    log_on_transaction(
        db,
        AuditAction::AppConfigured,
        ResourceType::App,
        Some(operation.app_name.clone()),
        user_id,
        username,
        Some(serde_json::json!({
            "operation_id": operation.id, "operation": operation.operation, "phase": "queued"
        })),
        None,
        None,
        true,
        None,
    )
    .await?;
    Ok(())
}

async fn log_control<C: ConnectionTrait>(
    db: &C,
    operation: &app_operation::Model,
    action: &str,
    actor_id: i64,
) -> Result<()> {
    let (_, username) = actor(db, Some(actor_id)).await?;
    log_on_transaction(db, AuditAction::AppConfigured, ResourceType::App,
        Some(operation.app_name.clone()), Some(actor_id), username,
        Some(serde_json::json!({"operation_id": operation.id, "operation": operation.operation, "phase": action})),
        None, None, true, None).await?;
    Ok(())
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

fn versions_differ(installed: Option<&str>, available: Option<&str>) -> bool {
    matches!((installed, available), (Some(installed), Some(available)) if installed != available)
}

fn filter_optional_string<C>(
    update: sea_orm::UpdateMany<app_state::Entity>,
    column: C,
    value: Option<&str>,
) -> sea_orm::UpdateMany<app_state::Entity>
where
    C: ColumnTrait,
{
    update.filter(match value {
        Some(value) => column.eq(value),
        None => column.is_null(),
    })
}

impl From<app_operation::Model> for AppOperationResponse {
    fn from(model: app_operation::Model) -> Self {
        Self {
            id: model.id,
            app_name: model.app_name,
            operation: model.operation,
            status: model.status,
            stop_requested: model.stop_requested,
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
