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
use crate::models::{app_operation, app_state, app_vpn_config, audit_outbox, user};
use crate::services::app_storage;
use crate::services::audit::log_on_transaction;
use crate::services::auto_update_settings;
use crate::services::catalog::AppCatalog;
use crate::services::deployment::{DeploymentManager, DeploymentRequest};
use crate::services::gpu::{self, GpuSelection, QUEUED_GPU_KEY};
use crate::services::install_options::{self, InstallStorage};
use crate::services::k8s::K8sClient;
use crate::services::notification::NotificationService;
use crate::services::storage_config;
use crate::services::update_diagnostics::{self, RestartSnapshot};
use crate::services::update_policy::{update_kind, UpdateKind};
use crate::state::{SharedCatalog, SharedK8sClient};

pub const OP_INSTALL: &str = "install";
pub const OP_UPDATE: &str = "update";
pub const OP_DELETE: &str = "delete";
pub const OP_RESTART: &str = "restart";
const QUEUED_INSTALL_KEY: &str = "__kubarr_install_choices";
const QUEUED_STORAGE_KEY: &str = "__kubarr_storage_snapshot";
/// Chart version an update operation was queued for (recorded on every update,
/// whoever started it); never reaches Helm.
pub(crate) const AUTO_TARGET_KEY: &str = "__kubarr_auto_target_version";

#[derive(Serialize, Deserialize)]
struct QueuedInstall {
    chart_version: String,
    storage: InstallStorage,
    vpn_provider_id: Option<i64>,
    #[serde(default)]
    fresh_private_config: bool,
}

#[derive(Serialize, Deserialize)]
struct QueuedStorage {
    chart_version: Option<String>,
    storage: Option<InstallStorage>,
    #[serde(default)]
    fresh_private_config: bool,
}

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
/// Why an automatic update was dropped before Helm ran.
pub(crate) const AUTO_SKIPPED_MESSAGE: &str =
    "Skipped: available version changed since it was queued";
const STOP_ERROR: &str = "Stop requested; external outcome indeterminate";
const DEFAULT_UPDATE_HEALTH_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const UPDATE_HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(10);
/// An updated app must stay healthy this long (inside the health timeout)
/// before the update counts as good: readiness alone misses late crashes.
const UPDATE_SETTLE: Duration = Duration::from_secs(180);
/// Appended to failure notifications (spec: rollback limits).
const MIGRATION_NOTE: &str = "A rollback restores the previous chart; it cannot undo a one-way data migration the new version may have run.";
/// Notification details for an update the worker lost track of.
const INTERRUPTED_DETAILS: &str = "interrupted during verification; state unknown — check the app";
/// A container restarting more often than this since the update started fails it.
const UPDATE_RESTART_LIMIT: i32 = 2;
/// Longest summary line placed above the captured diagnostics.
const SUMMARY_LINE_MAX: usize = 300;

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
    pub auto_update: bool,
    /// Classification of the pending update; `None` when none is available.
    pub available_update_kind: Option<UpdateKind>,
    pub last_operation_id: Option<String>,
    pub last_checked_at: Option<chrono::DateTime<Utc>>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// What happened to an app after an update was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    Healthy,
    RolledBack {
        diagnostics: String,
    },
    RollbackFailed {
        diagnostics: String,
    },
    /// Helm failed and no rollback was attempted because the app was not
    /// changed. `confirmed`: Helm history proved no new revision exists.
    NotApplied {
        diagnostics: String,
        confirmed: bool,
    },
}

impl UpdateOutcome {
    fn diagnostics(&self) -> Option<&str> {
        match self {
            UpdateOutcome::Healthy => None,
            UpdateOutcome::RolledBack { diagnostics }
            | UpdateOutcome::RollbackFailed { diagnostics }
            | UpdateOutcome::NotApplied { diagnostics, .. } => Some(diagnostics),
        }
    }
}

struct UpdateVersions {
    from: String,
    to: String,
}

struct UpdateReport {
    from: String,
    to: String,
    outcome: UpdateOutcome,
}

impl UpdateReport {
    fn message(&self, app: &str) -> String {
        let (from, to) = (&self.from, &self.to);
        match self.outcome {
            UpdateOutcome::Healthy => format!("Updated {app} {from} → {to}"),
            UpdateOutcome::RolledBack { .. } => {
                format!("Update to {to} failed; rolled back to {from}")
            }
            UpdateOutcome::RollbackFailed { .. } => {
                format!("Update to {to} failed; rollback to {from} failed")
            }
            UpdateOutcome::NotApplied {
                confirmed: true, ..
            } => format!("Update to {to} failed; {app} unchanged at {from}"),
            UpdateOutcome::NotApplied {
                confirmed: false, ..
            } => format!("Update to {to} failed; Helm reported an error"),
        }
    }

    fn audit_action(&self) -> AuditAction {
        match self.outcome {
            UpdateOutcome::Healthy => AuditAction::AppUpdated,
            UpdateOutcome::RolledBack { .. } | UpdateOutcome::NotApplied { .. } => {
                AuditAction::AppUpdateFailed
            }
            UpdateOutcome::RollbackFailed { .. } => AuditAction::AppUpdateRollbackFailed,
        }
    }

    /// Appended to the event's fixed notification prefix. Diagnostics are
    /// already scrubbed.
    fn notification_details(&self, app: &str) -> String {
        let versions = format!("app={app} from={} to={}", self.from, self.to);
        let (what, note) = match self.outcome {
            UpdateOutcome::Healthy => return versions,
            UpdateOutcome::RolledBack { .. } => {
                (format!("rolled back to {}; ", self.from), true)
            }
            UpdateOutcome::NotApplied {
                confirmed: true, ..
            } => (
                format!(
                    "Helm reported an error; the app is still running {}; ",
                    self.from
                ),
                false,
            ),
            UpdateOutcome::NotApplied {
                confirmed: false, ..
            } => (
                "Helm reported an error; the app is healthy but Helm history could not confirm which version is running; "
                    .to_string(),
                false,
            ),
            UpdateOutcome::RollbackFailed { .. } => (String::new(), true),
        };
        let line = self
            .outcome
            .diagnostics()
            .map(first_line)
            .unwrap_or_default();
        if note {
            format!("{versions}; {what}{line}. {MIGRATION_NOTE}")
        } else {
            format!("{versions}; {what}{line}")
        }
    }
}

/// The result an executed operation hands to the worker's completion step.
enum OperationOutput {
    Done(String),
    Update(UpdateReport),
}

impl From<String> for OperationOutput {
    fn from(message: String) -> Self {
        OperationOutput::Done(message)
    }
}

/// A terminal write: `action` overrides the audit action derived from the operation.
struct Terminal {
    status: &'static str,
    message: Option<String>,
    error: Option<String>,
    diagnostics: Option<String>,
    action: Option<AuditAction>,
}

enum HealthVerdict {
    Healthy,
    Unhealthy(String),
}

/// Helm history reduced to what verification needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReleaseRevisions {
    /// Highest revision, whatever its status.
    latest: Option<u32>,
    /// Highest revision Helm reports as `deployed` (the live one).
    deployed: Option<u32>,
}

impl ReleaseRevisions {
    fn from_history(history: &[(u32, String)]) -> Self {
        Self {
            latest: history.iter().map(|(revision, _)| *revision).max(),
            deployed: history
                .iter()
                .filter(|(_, status)| status == "deployed")
                .map(|(revision, _)| *revision)
                .max(),
        }
    }
}

/// The cluster reads and Helm actions the update verifier needs, behind a seam
/// so the decision flow is testable without a cluster.
// async_trait adds #[must_use] to methods that already return Result.
#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
trait UpdateProbe: Send + Sync {
    async fn restart_snapshot(&self) -> Result<RestartSnapshot>;
    /// `app_health`'s `healthy` flag and message.
    async fn health(&self) -> Result<(bool, String)>;
    /// `helm history` as (revision, status).
    fn history(&self) -> Result<Vec<(u32, String)>>;
    async fn capture(&self, failed: Option<u32>, previous: Option<u32>) -> String;
    fn rollback(&self, revision: u32) -> Result<()>;
    /// Mask secrets (namespace Secret values and inline patterns) in text that
    /// leaves the worker.
    async fn scrub(&self, text: &str) -> String;
}

struct ClusterUpdateProbe<'a, 'm> {
    k8s: &'a K8sClient,
    deploy: &'a DeploymentManager<'m>,
    app: &'a str,
    namespace: String,
}

impl<'a, 'm> ClusterUpdateProbe<'a, 'm> {
    fn new(k8s: &'a K8sClient, deploy: &'a DeploymentManager<'m>, app: &'a str) -> Self {
        Self {
            k8s,
            deploy,
            app,
            namespace: crate::services::catalog::lifecycle_for_app_name(app).namespace,
        }
    }
}

#[async_trait::async_trait]
impl UpdateProbe for ClusterUpdateProbe<'_, '_> {
    async fn restart_snapshot(&self) -> Result<RestartSnapshot> {
        update_diagnostics::restart_snapshot(self.k8s, &self.namespace).await
    }

    async fn health(&self) -> Result<(bool, String)> {
        let health = self.deploy.app_health(self.app).await?;
        Ok((
            health["healthy"].as_bool().unwrap_or(false),
            health["message"].as_str().unwrap_or_default().to_string(),
        ))
    }

    fn history(&self) -> Result<Vec<(u32, String)>> {
        self.deploy.release_revisions(self.app)
    }

    async fn scrub(&self, text: &str) -> String {
        update_diagnostics::scrub_for_namespace(self.k8s, &self.namespace, text).await
    }

    async fn capture(&self, failed: Option<u32>, previous: Option<u32>) -> String {
        update_diagnostics::capture(
            self.k8s,
            self.deploy,
            self.app,
            &self.namespace,
            failed,
            previous,
        )
        .await
    }

    fn rollback(&self, revision: u32) -> Result<()> {
        self.deploy.rollback_release(self.app, revision)
    }
}

#[derive(Clone)]
pub struct AppManager {
    pub(super) db: DatabaseConnection,
    pub(super) k8s_client: SharedK8sClient,
    pub(super) catalog: SharedCatalog,
    /// Serializes Helm writes between queued operations and the HA rescaler.
    pub(super) helm_lock: Arc<tokio::sync::Mutex<()>>,
    /// Update outcomes are announced here; absent means no notifications.
    notifications: Option<Arc<NotificationService>>,
    /// How long an updated app may take to become healthy before rollback.
    /// Fixed override; when absent the owner's setting is read per update.
    update_health_timeout: Option<Duration>,
    /// Health poll interval and settle period (constants outside tests).
    update_poll: Duration,
    update_settle: Duration,
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
            helm_lock: Arc::default(),
            notifications: None,
            update_health_timeout: None,
            update_poll: UPDATE_HEALTH_POLL_INTERVAL,
            update_settle: UPDATE_SETTLE,
        }
    }

    /// Shrink the health poll interval and settle period so tests run fast.
    #[cfg(test)]
    fn with_update_timing(mut self, poll: Duration, settle: Duration) -> Self {
        self.update_poll = poll;
        self.update_settle = settle;
        self
    }

    /// Announce update outcomes through `notifications`.
    pub fn with_notifications(mut self, notifications: Arc<NotificationService>) -> Self {
        self.notifications = Some(notifications);
        self
    }

    /// Override how long an updated app may take to become healthy.
    pub fn with_update_health_timeout(mut self, timeout: Duration) -> Self {
        self.update_health_timeout = Some(timeout);
        self
    }

    pub async fn enqueue_operation(
        &self,
        app_name: &str,
        operation: &str,
        custom_config: HashMap<String, String>,
        created_by: Option<i64>,
    ) -> Result<AppOperationResponse> {
        gpu::validate_custom_gpu_keys(&custom_config)?;
        install_options::validate_custom_storage_keys(&custom_config)?;
        if custom_config.contains_key(QUEUED_INSTALL_KEY)
            || custom_config.contains_key(QUEUED_STORAGE_KEY)
            || custom_config.contains_key(AUTO_TARGET_KEY)
        {
            return Err(AppError::BadRequest("Reserved install key".into()));
        }
        self.enqueue_operation_unchecked(app_name, operation, custom_config, created_by)
            .await
    }

    /// Queue an update started by the auto-update loop (no actor, trigger
    /// "auto"). The target version is recorded so a failure can be tied to it.
    pub async fn enqueue_auto_update(
        &self,
        app_name: &str,
        target_version: &str,
    ) -> Result<AppOperationResponse> {
        let config = HashMap::from([(AUTO_TARGET_KEY.to_string(), target_version.to_string())]);
        self.persist_operation(app_name, OP_UPDATE, config, None, "auto")
            .await
    }

    async fn persist_operation(
        &self,
        app_name: &str,
        operation: &str,
        mut custom_config: HashMap<String, String>,
        created_by: Option<i64>,
        trigger: &str,
    ) -> Result<AppOperationResponse> {
        if operation != OP_DELETE {
            let catalog = self.catalog.read().await;
            if catalog.get_app(app_name).is_none() {
                return Err(AppError::NotFound(format!(
                    "App '{}' not found in catalog",
                    app_name
                )));
            }
            if operation == OP_UPDATE && !custom_config.contains_key(AUTO_TARGET_KEY) {
                let version = match catalog.chart_version(app_name) {
                    Some(version) => Some(version),
                    None => app_state::Entity::find_by_id(app_name.to_string())
                        .one(&self.db)
                        .await?
                        .and_then(|row| row.available_chart_version),
                };
                if let Some(version) = version {
                    custom_config.insert(AUTO_TARGET_KEY.into(), version);
                }
            }
        }

        let transaction = self.db.begin().await?;
        if matches!(operation, OP_INSTALL | OP_UPDATE) {
            let snapshot = self.storage_snapshot(&transaction, app_name).await?;
            if snapshot.storage.is_some() {
                custom_config.insert(QUEUED_STORAGE_KEY.into(), serde_json::to_string(&snapshot)?);
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
            trigger: Set(Some(trigger.to_string())),
            diagnostics: Set(None),
            created_at: Set(now),
            started_at: Set(None),
            finished_at: Set(None),
            updated_at: Set(now),
        };

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
        install_options::validate_custom_storage_keys(&custom_config)?;
        if custom_config.contains_key(QUEUED_INSTALL_KEY)
            || custom_config.contains_key(QUEUED_STORAGE_KEY)
            || custom_config.contains_key(AUTO_TARGET_KEY)
        {
            return Err(AppError::BadRequest("Reserved install key".into()));
        }
        if gpu.is_some() && !crate::services::gpu::chart_supports_gpu(app_name) {
            return Err(AppError::BadRequest(
                "This application's chart offers no GPU selection".into(),
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

    async fn storage_snapshot(
        &self,
        tx: &DatabaseTransaction,
        app_name: &str,
    ) -> Result<QueuedStorage> {
        let saved = app_storage::get(tx, app_name).await?;
        let Some(saved) = saved else {
            return Ok(QueuedStorage {
                chart_version: None,
                storage: None,
                fresh_private_config: false,
            });
        };
        let catalog = self.catalog.read().await;
        let options = install_options::load(&catalog, app_name)?;
        install_options::validate(&options, &saved.storage)?;
        Ok(QueuedStorage {
            chart_version: Some(options.chart_version),
            storage: Some(saved.storage),
            fresh_private_config: saved.fresh_private_config,
        })
    }

    /// The VPN assignment and install operation share the caller's transaction.
    #[allow(clippy::too_many_arguments)]
    pub async fn enqueue_install_in_transaction(
        &self,
        tx: &DatabaseTransaction,
        app_name: &str,
        version: &str,
        storage: InstallStorage,
        fresh_private_config: bool,
        gpu: Option<Option<GpuSelection>>,
        actor: i64,
    ) -> Result<AppOperationResponse> {
        if gpu.is_some() && !crate::services::gpu::chart_supports_gpu(app_name) {
            return Err(AppError::BadRequest(
                "This application's chart offers no GPU selection".into(),
            ));
        }
        if let Some(selection) = gpu.as_ref().and_then(Option::as_ref) {
            let guard = self.k8s_client.read().await;
            let client = guard
                .as_ref()
                .ok_or_else(|| AppError::Internal("Kubernetes client not available".into()))?;
            gpu::validate_selection(client, selection).await?;
        }
        let catalog = self.catalog.read().await;
        let app = catalog
            .get_app(app_name)
            .ok_or_else(|| AppError::NotFound("App not found".into()))?;
        if app.is_system
            || app.is_hidden
            || catalog.chart_version(app_name).as_deref() != Some(version)
        {
            return Err(AppError::Conflict(
                "Chart changed; review install options again".into(),
            ));
        }
        drop(catalog);
        let vpn_provider_id = app_vpn_config::Entity::find_by_id(app_name)
            .one(tx)
            .await?
            .map(|assignment| assignment.vpn_provider_id);
        let mut config = HashMap::from([(
            QUEUED_INSTALL_KEY.to_string(),
            serde_json::to_string(&QueuedInstall {
                chart_version: version.into(),
                storage,
                vpn_provider_id,
                fresh_private_config,
            })?,
        )]);
        if gpu.is_some() {
            config.insert(QUEUED_GPU_KEY.into(), serde_json::to_string(&gpu)?);
        }
        let now = Utc::now();
        let operation = app_operation::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            app_name: Set(app_name.into()),
            operation: Set(OP_INSTALL.into()),
            status: Set(STATUS_QUEUED.into()),
            stop_requested: Set(false),
            message: Set(Some(format!("Queued install for {app_name}"))),
            error: Set(None),
            custom_config: Set(Some(serde_json::to_string(&config)?)),
            attempts: Set(0),
            created_by: Set(Some(actor)),
            trigger: Set(Some("manual".to_string())),
            diagnostics: Set(None),
            created_at: Set(now),
            started_at: Set(None),
            finished_at: Set(None),
            updated_at: Set(now),
        }
        .insert(tx)
        .await?;
        self.mark_state_for_operation(tx, app_name, OP_INSTALL, &operation.id)
            .await?;
        log_queued(tx, &operation).await?;
        Ok(operation.into())
    }

    async fn enqueue_operation_unchecked(
        &self,
        app_name: &str,
        operation: &str,
        custom_config: HashMap<String, String>,
        created_by: Option<i64>,
    ) -> Result<AppOperationResponse> {
        validate_operation(operation)?;
        self.persist_operation(app_name, operation, custom_config, created_by, "manual")
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
        if app_operation::Entity::find()
            .filter(app_operation::Column::AppName.eq(app_name))
            .filter(app_operation::Column::Status.is_in([
                STATUS_QUEUED,
                STATUS_RUNNING,
                STATUS_PAUSED,
            ]))
            .one(transaction)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict(
                "An app operation is already in progress".into(),
            ));
        }

        let now = Utc::now();
        let id = Uuid::new_v4().to_string();
        let snapshot = self.storage_snapshot(transaction, app_name).await?;
        let mut queued_config = HashMap::new();
        if let Some(version) = &available_chart_version {
            queued_config.insert(AUTO_TARGET_KEY.to_string(), version.clone());
        }
        if snapshot.storage.is_some() {
            queued_config.insert(
                QUEUED_STORAGE_KEY.to_string(),
                serde_json::to_string(&snapshot)?,
            );
        }
        let operation = app_operation::ActiveModel {
            id: Set(id.clone()),
            app_name: Set(app_name.to_string()),
            operation: Set(OP_UPDATE.to_string()),
            status: Set(STATUS_QUEUED.to_string()),
            stop_requested: Set(false),
            message: Set(Some(format!("Queued {} for {}", OP_UPDATE, app_name))),
            error: Set(None),
            custom_config: Set(Some(serde_json::to_string(&queued_config)?)),
            attempts: Set(0),
            created_by: Set(created_by),
            trigger: Set(Some("manual".to_string())),
            diagnostics: Set(None),
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
                auto_update: Set(true),
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
                .ok_or_else(|| AppError::NotFound(format!("Operation '{id}' not found")))?;
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
            .ok_or_else(|| AppError::NotFound(format!("Operation '{id}' not found")))?;
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
        if app_operation::Entity::find()
            .filter(app_operation::Column::AppName.eq(&original.app_name))
            .filter(app_operation::Column::Status.is_in([
                STATUS_QUEUED,
                STATUS_RUNNING,
                STATUS_PAUSED,
            ]))
            .one(&transaction)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict(
                "An app operation is already in progress".into(),
            ));
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
        let mut config: HashMap<String, String> =
            serde_json::from_str(original.custom_config.as_deref().unwrap_or("{}"))
                .map_err(|_| AppError::BadRequest("Invalid stored operation config".into()))?;
        let mut public_config = config.clone();
        let mut promoted_config: Option<String> = None;
        public_config.remove(QUEUED_GPU_KEY);
        public_config.remove(AUTO_TARGET_KEY);
        if let Some(raw) = public_config.remove(QUEUED_STORAGE_KEY) {
            let snapshot: QueuedStorage = serde_json::from_str(&raw)
                .map_err(|_| AppError::BadRequest("Invalid stored storage snapshot".into()))?;
            if let Some(storage) = snapshot.storage {
                let saved = app_storage::get(&transaction, &original.app_name).await?;
                if saved.as_ref().is_none_or(|saved| {
                    saved.storage != storage
                        || saved.fresh_private_config != snapshot.fresh_private_config
                }) {
                    return Err(AppError::Conflict(
                        "Saved storage changed since this operation; review it before retrying"
                            .into(),
                    ));
                }
                let catalog = self.catalog.read().await;
                let options = install_options::load(&catalog, &original.app_name)?;
                if snapshot.chart_version.as_deref() != Some(options.chart_version.as_str()) {
                    return Err(AppError::Conflict(
                        "Chart changed; review saved storage settings".into(),
                    ));
                }
                install_options::validate(&options, &storage)?;
            }
        }
        if let Some(raw) = public_config.remove(QUEUED_INSTALL_KEY) {
            let mut pinned: QueuedInstall = serde_json::from_str(&raw)
                .map_err(|_| AppError::BadRequest("Invalid stored install options".into()))?;
            let catalog = self.catalog.read().await;
            let options = install_options::load(&catalog, &original.app_name)?;
            if options.chart_version != pinned.chart_version {
                return Err(AppError::Conflict(
                    "Chart changed; review install options again".into(),
                ));
            }
            install_options::validate(&options, &pinned.storage)?;
            let saved = app_storage::get(&transaction, &original.app_name).await?;
            let proof_promoted = saved.as_ref().is_some_and(|saved| {
                original.operation == OP_INSTALL
                    && !pinned.fresh_private_config
                    && saved.fresh_private_config
                    && saved.storage == pinned.storage
                    && saved.chart_version == pinned.chart_version
                    && pinned
                        .storage
                        .config
                        .as_ref()
                        .is_some_and(|config| config.create && config.existing_claim.is_empty())
            });
            if saved.as_ref().is_none_or(|saved| {
                saved.storage != pinned.storage
                    || (saved.fresh_private_config != pinned.fresh_private_config
                        && !proof_promoted)
            }) {
                return Err(AppError::Conflict(
                    "Saved storage changed since this install; review it before retrying".into(),
                ));
            }
            if proof_promoted {
                // The original request remains immutable. A new retry can use
                // the separately persisted proof of its successful Helm install.
                pinned.fresh_private_config = true;
                config.insert(QUEUED_INSTALL_KEY.into(), serde_json::to_string(&pinned)?);
                promoted_config = Some(serde_json::to_string(&config)?);
            }
        }
        if matches!(original.operation.as_str(), OP_INSTALL | OP_UPDATE)
            && !config.contains_key(QUEUED_INSTALL_KEY)
            && !config.contains_key(QUEUED_STORAGE_KEY)
            && app_storage::get(&transaction, &original.app_name)
                .await?
                .is_some()
        {
            return Err(AppError::Conflict(
                "Storage was configured after this legacy operation; review before retrying".into(),
            ));
        }
        gpu::validate_custom_gpu_keys(&public_config)?;
        install_options::validate_custom_storage_keys(&public_config)?;
        if let Some(gpu) = config.get(QUEUED_GPU_KEY) {
            if !crate::services::gpu::chart_supports_gpu(&original.app_name) {
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
            custom_config: Set(promoted_config.or_else(|| original.custom_config.clone())),
            attempts: Set(0),
            created_by: Set(Some(actor_id)),
            trigger: Set(Some("manual".to_string())),
            diagnostics: Set(None),
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

    /// Turn automatic updates on or off for one app.
    pub async fn set_auto_update(&self, app_name: &str, enabled: bool) -> Result<AppStateResponse> {
        let state = app_state::Entity::find_by_id(app_name.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("State for '{}' not found", app_name)))?;
        let mut active: app_state::ActiveModel = state.into();
        active.auto_update = Set(enabled);
        Ok(active.update(&self.db).await?.into())
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
        Fut: Future<Output = Result<OperationOutput>>,
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
                    // execution as abandoned. Only the Lease-elected leader runs it, and a
                    // leader that loses its Lease exits, so no other worker runs it concurrently.
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
        self.process_next_operation_with_stop(
            |operation, _| {
                let output = execute(operation);
                async move { output.await.map(OperationOutput::from) }
            },
            heartbeat_interval,
        )
        .await
    }

    async fn process_next_operation_with_stop<F, Fut>(
        &self,
        execute: F,
        heartbeat_interval: Duration,
    ) -> Result<()>
    where
        F: FnOnce(app_operation::Model, CancellationToken) -> Fut,
        Fut: Future<Output = Result<OperationOutput>>,
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
                Ok(OperationOutput::Done(message)) => {
                    let action = terminal_action(&operation.operation, true);
                    self.complete_succeeded(&operation, message, action).await?;
                }
                Ok(OperationOutput::Update(report)) => {
                    let message = report.message(&operation.app_name);
                    let action = report.audit_action();
                    let won = match report.outcome.diagnostics() {
                        None => {
                            self.complete_succeeded(&operation, message.clone(), action.clone())
                                .await?
                        }
                        Some(diagnostics) => {
                            let summary = first_line(diagnostics).to_string();
                            self.complete_failed(
                                &operation,
                                Terminal {
                                    status: STATUS_FAILED,
                                    message: Some(message.clone()),
                                    error: Some(summary),
                                    diagnostics: Some(diagnostics.to_string()),
                                    action: Some(action.clone()),
                                },
                                message.clone(),
                            )
                            .await?
                        }
                    };
                    if won {
                        self.notify_update(
                            &operation,
                            &action,
                            &report.notification_details(&operation.app_name),
                        )
                        .await;
                    }
                }
                Err(e) => {
                    let error = e.to_string();
                    let message = if error.contains("external outcome indeterminate") {
                        "Helm deadline exceeded; external outcome indeterminate".to_string()
                    } else {
                        format!("{} failed", operation.operation)
                    };
                    let won = self
                        .complete_failed(
                            &operation,
                            Terminal {
                                status: STATUS_FAILED,
                                message: Some(message),
                                error: Some(error.clone()),
                                diagnostics: None,
                                action: None,
                            },
                            error.clone(),
                        )
                        .await?;
                    // An update that failed before it could be verified (normally
                    // before Helm ran). A dropped stale auto target stays silent.
                    if won
                        && operation.operation == OP_UPDATE
                        && !error.contains(AUTO_SKIPPED_MESSAGE)
                    {
                        let details = format!(
                            "{}; the update failed before Kubarr could verify it: {}",
                            update_subject(&operation),
                            update_diagnostics::scrub_inline(first_line(&error))
                        );
                        self.notify_update(&operation, &AuditAction::AppUpdateFailed, &details)
                            .await;
                    }
                }
            }
        }

        if let Err(error) = self.deliver_audit(&operation.id).await {
            tracing::error!(operation_id = %operation.id, %error, "Terminal app audit pending retry");
        }

        Ok(())
    }

    /// Record success, then publish state. Returns whether this worker won the terminal write.
    async fn complete_succeeded(
        &self,
        operation: &app_operation::Model,
        message: String,
        action: AuditAction,
    ) -> Result<bool> {
        let won = self
            .finish_operation_with(
                &operation.id,
                Terminal {
                    status: STATUS_SUCCEEDED,
                    message: Some(message.clone()),
                    error: None,
                    diagnostics: None,
                    action: Some(action),
                },
            )
            .await?;
        if won && operation.operation != OP_RESTART {
            if let Err(error) = self
                .upsert_state(
                    &operation.app_name,
                    &crate::services::catalog::lifecycle_for_app_name(&operation.app_name)
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
                        message
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
            self.finish_stopped(operation).await?;
        }
        tracing::info!(
            operation_id = %operation.id,
            app = %operation.app_name,
            operation = %operation.operation,
            "App operation finished"
        );
        Ok(won)
    }

    /// Record failure, then publish `state_message` on the app. Returns whether
    /// this worker won the terminal write.
    async fn complete_failed(
        &self,
        operation: &app_operation::Model,
        terminal: Terminal,
        state_message: String,
    ) -> Result<bool> {
        let won = self.finish_operation_with(&operation.id, terminal).await?;
        if won {
            if let Err(state_error) = self
                .upsert_state(
                    &operation.app_name,
                    &crate::services::catalog::lifecycle_for_app_name(&operation.app_name)
                        .namespace,
                    desired_state_for_operation(&operation.operation),
                    OBS_FAILED,
                    false,
                    Some(state_message),
                    Some(operation.id.clone()),
                    false,
                )
                .await
            {
                tracing::error!(operation_id = %operation.id, %state_error, "App failure state update failed after terminal commit");
            }
        } else {
            self.finish_stopped(operation).await?;
        }
        tracing::warn!(
            operation_id = %operation.id,
            app = %operation.app_name,
            operation = %operation.operation,
            "App operation failed"
        );
        Ok(won)
    }

    /// Best effort: a notification failure never changes the operation's outcome.
    async fn notify_update(
        &self,
        operation: &app_operation::Model,
        action: &AuditAction,
        details: &str,
    ) {
        let Some(notifications) = &self.notifications else {
            return;
        };
        let username = match actor(&self.db, operation.created_by).await {
            Ok((_, username)) => username,
            Err(error) => {
                tracing::warn!(operation_id = %operation.id, %error, "Update notification actor lookup failed");
                None
            }
        };
        if let Err(error) = notifications
            .notify_event(
                action,
                operation.created_by,
                username.as_deref(),
                Some(details),
            )
            .await
        {
            tracing::warn!(operation_id = %operation.id, %error, "Update notification failed");
        }
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
    ) -> Result<OperationOutput> {
        check_stop(stop)?;
        let k8s_guard = self.k8s_client.read().await;
        let client = k8s_guard
            .as_ref()
            .ok_or_else(|| AppError::Internal("Kubernetes client not available".to_string()))?;
        let catalog = self.catalog.read().await;
        // Taken after the read guards, as reconcile_ha does: acquiring a read
        // while holding this lock could queue behind a catalog writer that is
        // itself waiting on the other holder's read.
        let _helm = self.helm_lock.lock().await;

        let outcome = match operation.operation.as_str() {
            OP_INSTALL | OP_UPDATE => {
                self.execute_install_or_update(operation, client, &catalog, stop)
                    .await
            }
            OP_DELETE => {
                let manager = DeploymentManager::new(client, &catalog);
                manager
                    .remove_app_with_stop(&operation.app_name, Some(stop))
                    .await?;
                Ok(format!("Removed {}", operation.app_name).into())
            }
            OP_RESTART => {
                check_stop(stop)?;
                self.restart_app(client, &operation.app_name).await?;
                Ok(format!("Restarted {}", operation.app_name).into())
            }
            _ => Err(AppError::BadRequest(format!(
                "Unsupported operation '{}'",
                operation.operation
            ))),
        };

        // These operations change pods on purpose, and whoever triggered one is
        // about to look. Dropping the cached row closes the gap between the
        // action landing and the watch reporting it, so the dashboard cannot
        // show the pre-action state back to them.
        if let Err(error) =
            crate::services::pod_cache::invalidate(&self.db, &operation.app_name).await
        {
            tracing::warn!(%error, app = %operation.app_name, "failed to invalidate pod status cache");
        }

        outcome
    }

    /// An automatic update only deploys the version the loop vetted: the chart
    /// on disk must still be that version and the update still auto-eligible.
    pub(super) async fn guard_auto_target(
        &self,
        operation: &app_operation::Model,
        target: Option<&str>,
        current: Option<&str>,
    ) -> Result<()> {
        let installed = app_state::Entity::find_by_id(operation.app_name.clone())
            .one(&self.db)
            .await?
            .and_then(|row| row.installed_chart_version);
        let still_vetted = match (target, current, installed.as_deref()) {
            (Some(target), Some(current), Some(installed)) => {
                target == current
                    && crate::services::update_policy::auto_eligible(
                        crate::services::update_policy::update_kind(installed, target),
                    )
            }
            _ => false,
        };
        if still_vetted {
            Ok(())
        } else {
            Err(AppError::Conflict(AUTO_SKIPPED_MESSAGE.into()))
        }
    }

    async fn execute_install_or_update(
        &self,
        operation: &app_operation::Model,
        client: &K8sClient,
        catalog: &AppCatalog,
        stop: &CancellationToken,
    ) -> Result<OperationOutput> {
        // A malformed persisted request must fail rather than silently deploying
        // with defaults (which can reset a queued GPU or other chart settings).
        let mut custom_config = parse_queued_config(operation.custom_config.as_deref())?;
        let queued_target = custom_config.remove(AUTO_TARGET_KEY);
        if operation.operation == OP_UPDATE && operation.trigger.as_deref() == Some("auto") {
            self.guard_auto_target(
                operation,
                queued_target.as_deref(),
                catalog.chart_version(&operation.app_name).as_deref(),
            )
            .await?;
        }
        let gpu: Option<Option<GpuSelection>> = custom_config
            .remove(QUEUED_GPU_KEY)
            .map(|raw| {
                serde_json::from_str(&raw)
                    .map_err(|_| AppError::BadRequest("Invalid queued GPU selection".into()))
            })
            .transpose()?;
        let install: Option<QueuedInstall> = custom_config
            .remove(QUEUED_INSTALL_KEY)
            .map(|raw| {
                serde_json::from_str(&raw)
                    .map_err(|_| AppError::BadRequest("Invalid queued install choices".into()))
            })
            .transpose()?;
        let saved_snapshot: Option<QueuedStorage> = custom_config
            .remove(QUEUED_STORAGE_KEY)
            .map(|raw| {
                serde_json::from_str(&raw)
                    .map_err(|_| AppError::BadRequest("Invalid queued storage snapshot".into()))
            })
            .transpose()?;
        if let Some(ref snapshot) = saved_snapshot {
            if let Some(storage) = &snapshot.storage {
                let options = install_options::load(catalog, &operation.app_name)?;
                if snapshot.chart_version.as_deref() != Some(options.chart_version.as_str()) {
                    return Err(AppError::Conflict(
                        "Chart changed since storage settings were queued".into(),
                    ));
                }
                install_options::validate(&options, storage)?;
                let current = app_storage::get(&self.db, &operation.app_name).await?;
                if current.as_ref().is_none_or(|row| {
                    row.storage != *storage
                        || row.fresh_private_config != snapshot.fresh_private_config
                }) {
                    return Err(AppError::Conflict(
                        "Saved storage changed after this job was queued".into(),
                    ));
                }
            }
        }
        if let Some(ref pinned) = install {
            let options = install_options::load(catalog, &operation.app_name)?;
            if options.chart_version != pinned.chart_version {
                return Err(AppError::Conflict(
                    "Chart changed since install was queued; review options again".into(),
                ));
            }
            install_options::validate(&options, &pinned.storage)?;
            let current = app_storage::get(&self.db, &operation.app_name).await?;
            if current.as_ref().is_none_or(|row| {
                row.storage != pinned.storage
                    || row.fresh_private_config != pinned.fresh_private_config
            }) {
                return Err(AppError::Conflict(
                    "Saved storage is missing or changed since this install was queued".into(),
                ));
            }
            let assigned = app_vpn_config::Entity::find_by_id(&operation.app_name)
                .one(&self.db)
                .await?
                .map(|config| config.vpn_provider_id);
            if assigned != pinned.vpn_provider_id {
                return Err(AppError::Conflict(
                    "VPN assignment changed since install was queued; review before retrying"
                        .into(),
                ));
            }
        }

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
        let proven_fresh = if operation.operation == OP_INSTALL
            && install
                .as_ref()
                .and_then(|choice| choice.storage.config.as_ref())
                .is_some_and(|config| config.create && config.existing_claim.is_empty())
        {
            if manager.release_absent(&operation.app_name)? {
                let options = install_options::load(catalog, &operation.app_name)?;
                match options.private_claim {
                    Some(claim) => {
                        manager
                            .private_claim_absent(&operation.app_name, &claim)
                            .await?
                    }
                    None => false,
                }
            } else {
                false
            }
        } else {
            false
        };
        // The worker cannot verify its own replacement (it is deployed without --wait).
        let verify = operation.operation == OP_UPDATE && request.wait;
        let versions = if verify {
            Some(
                self.update_versions(&manager, catalog, &operation.app_name)
                    .await?,
            )
        } else {
            None
        };
        let deploy = manager.deploy_install_with_stop(
            &request,
            deployment_storage,
            Some(stop),
            install
                .as_ref()
                .map(|choice| {
                    (
                        choice.chart_version.as_str(),
                        &choice.storage,
                        Some(choice.vpn_provider_id),
                        choice.fresh_private_config,
                    )
                })
                .or_else(|| {
                    saved_snapshot.as_ref().and_then(|snapshot| {
                        snapshot
                            .storage
                            .as_ref()
                            .zip(snapshot.chart_version.as_deref())
                            .map(|(storage, version)| {
                                (version, storage, None, snapshot.fresh_private_config)
                            })
                    })
                }),
            proven_fresh,
        );

        let Some(versions) = versions else {
            return Ok(deploy.await?.message.into());
        };
        let probe = ClusterUpdateProbe::new(client, &manager, &operation.app_name);
        self.run_verified_update(&probe, versions, stop, async {
            deploy.await.map(|status| status.message)
        })
        .await
    }

    /// The release's deployed chart version (falling back to the recorded one)
    /// and the catalog version the update installs.
    async fn update_versions(
        &self,
        manager: &DeploymentManager<'_>,
        catalog: &AppCatalog,
        app_name: &str,
    ) -> Result<UpdateVersions> {
        let from = match manager.installed_chart_version(app_name) {
            Some(version) => Some(version),
            None => app_state::Entity::find_by_id(app_name)
                .one(&self.db)
                .await?
                .and_then(|state| state.installed_chart_version),
        };
        Ok(UpdateVersions {
            from: from.unwrap_or_else(|| "unknown".into()),
            to: catalog
                .chart_version(app_name)
                .unwrap_or_else(|| "unknown".into()),
        })
    }

    /// The configured health timeout, or the default when settings cannot be read.
    async fn health_timeout(&self) -> Duration {
        if let Some(timeout) = self.update_health_timeout {
            return timeout;
        }
        match auto_update_settings::load(&self.db).await {
            Ok(settings) => settings.health_timeout(),
            Err(error) => {
                tracing::warn!(%error, "Auto-update settings unavailable; using the default health timeout");
                DEFAULT_UPDATE_HEALTH_TIMEOUT
            }
        }
    }

    /// Snapshot restarts, run Helm, then verify the app and roll back if it is
    /// unhealthy. The caller holds `helm_lock` throughout.
    async fn run_verified_update<H>(
        &self,
        probe: &dyn UpdateProbe,
        versions: UpdateVersions,
        stop: &CancellationToken,
        helm: H,
    ) -> Result<OperationOutput>
    where
        H: Future<Output = Result<String>>,
    {
        // Read per operation so a settings change applies without a restart.
        let timeout = self.health_timeout().await;
        let before = probe.restart_snapshot().await?;
        // The only safe rollback target is what was live before this update.
        let before_revisions = probe
            .history()
            .map(|history| ReleaseRevisions::from_history(&history))
            .map_err(|error| error.to_string());
        let helm_result = helm.await;
        if stop.is_cancelled() {
            // A stop wins regardless; do not keep the lock for a verdict nobody records.
            return helm_result.map(OperationOutput::from);
        }
        // A stop during verification abandons it (no rollback); the worker records the stop.
        let outcome = tokio::select! {
            outcome = self.verify_or_roll_back(probe, &before, before_revisions, helm_result, timeout) => outcome,
            _ = stop.cancelled() => return Err(AppError::Internal(STOP_ERROR.into())),
        };
        Ok(OperationOutput::Update(UpdateReport {
            from: versions.from,
            to: versions.to,
            outcome,
        }))
    }

    /// Wait up to `timeout` for the app to be healthy continuously for the
    /// settle period (3 min), with no container restarted more than twice since
    /// `before`. A restart breach at any point fails at once; a timeout shorter
    /// than time-to-healthy plus settle fails. Polls every 10 s.
    async fn verify_update(
        &self,
        probe: &dyn UpdateProbe,
        before: &RestartSnapshot,
        timeout: Duration,
    ) -> HealthVerdict {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut healthy_since: Option<tokio::time::Instant> = None;
        loop {
            let reason = match probe.restart_snapshot().await {
                Ok(now)
                    if update_diagnostics::restarts_exceeded(
                        before,
                        &now,
                        UPDATE_RESTART_LIMIT,
                    ) =>
                {
                    return HealthVerdict::Unhealthy(format!(
                        "A container restarted more than {UPDATE_RESTART_LIMIT} times since the update started"
                    ));
                }
                Ok(_) => match probe.health().await {
                    Ok((true, _)) => {
                        let now = tokio::time::Instant::now();
                        let since = *healthy_since.get_or_insert(now);
                        if now.duration_since(since) >= self.update_settle {
                            return HealthVerdict::Healthy;
                        }
                        None
                    }
                    Ok((false, message)) if message.is_empty() => {
                        Some("app health check failing".to_string())
                    }
                    Ok((false, message)) => Some(message),
                    Err(error) => Some(format!("health check error: {error}")),
                },
                Err(error) => Some(format!("restart counts unavailable: {error}")),
            };
            if reason.is_some() {
                healthy_since = None;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                let within = describe_duration(timeout);
                return HealthVerdict::Unhealthy(match reason {
                    Some(reason) => format!("Not healthy within {within}: {reason}"),
                    None => format!(
                        "Not healthy for the {} settle period within {within}",
                        describe_duration(self.update_settle)
                    ),
                });
            }
            tokio::time::sleep(self.update_poll.min(deadline - now)).await;
        }
    }

    /// Decide the outcome after Helm returned. Rolls back only to the revision
    /// that was deployed before the update, and only when Helm created a newer one.
    async fn verify_or_roll_back(
        &self,
        probe: &dyn UpdateProbe,
        before: &RestartSnapshot,
        before_revisions: std::result::Result<ReleaseRevisions, String>,
        helm_result: Result<String>,
        timeout: Duration,
    ) -> UpdateOutcome {
        let verdict = self.verify_update(probe, before, timeout).await;
        let helm_error = helm_result.err().map(|error| error.to_string());
        if matches!(verdict, HealthVerdict::Healthy) && helm_error.is_none() {
            return UpdateOutcome::Healthy;
        }
        let after_revisions = probe
            .history()
            .map(|history| ReleaseRevisions::from_history(&history))
            .map_err(|error| error.to_string());
        // `None` when either side of the comparison is unknown.
        let applied = match (&before_revisions, &after_revisions) {
            (Ok(before), Ok(after)) => Some(after.latest > before.latest),
            _ => None,
        };
        let helm_line = helm_error
            .as_deref()
            .map(|error| format!("Helm upgrade failed: {}", first_line(error)));

        let reason = match (verdict, &helm_line) {
            (HealthVerdict::Healthy, Some(helm_line)) => {
                if applied == Some(true) {
                    // A new revision is deployed and healthy; Helm's error was spurious.
                    tracing::warn!(error = %helm_line, "Helm reported an error but the updated app is healthy");
                    return UpdateOutcome::Healthy;
                }
                let confirmed = applied == Some(false);
                let note = if confirmed {
                    "No new revision is running; nothing to roll back"
                } else {
                    "Helm history unavailable; the app is healthy, so no rollback was attempted"
                };
                return UpdateOutcome::NotApplied {
                    diagnostics: self.compose(probe, &[helm_line, note], "").await,
                    confirmed,
                };
            }
            (HealthVerdict::Healthy, None) => return UpdateOutcome::Healthy,
            (HealthVerdict::Unhealthy(reason), None) => reason,
            (HealthVerdict::Unhealthy(reason), Some(helm_line)) => format!("{helm_line}; {reason}"),
        };

        let failed_revision = after_revisions.as_ref().ok().and_then(|r| r.latest);
        let target = before_revisions.as_ref().ok().and_then(|r| r.deployed);
        // Evidence first: the rollback replaces the pods whose logs explain the failure.
        let captured = probe.capture(failed_revision, target).await;

        if applied == Some(false) {
            let note = "No new Helm revision was created; nothing to roll back";
            return UpdateOutcome::NotApplied {
                diagnostics: self.compose(probe, &[&reason, note], &captured).await,
                confirmed: true,
            };
        }

        let rollback: std::result::Result<u32, String> = match (applied, target) {
            (None, _) => Err(format!(
                "Rollback not attempted: Helm history unavailable: {}",
                before_revisions
                    .err()
                    .or(after_revisions.err())
                    .unwrap_or_default()
            )),
            (_, None) => {
                Err("Rollback not attempted: no deployed revision before the update".into())
            }
            (_, Some(revision)) => {
                let before_rollback = match probe.restart_snapshot().await {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        tracing::warn!(%error, "Restart snapshot before rollback failed; using the pre-update baseline");
                        before.clone()
                    }
                };
                match probe.rollback(revision) {
                    Err(error) => Err(format!("Rollback to revision {revision} failed: {error}")),
                    Ok(()) => match self.verify_update(probe, &before_rollback, timeout).await {
                        HealthVerdict::Healthy => Ok(revision),
                        HealthVerdict::Unhealthy(reason) => Err(format!(
                            "Still unhealthy after rollback to revision {revision}: {reason}"
                        )),
                    },
                }
            }
        };

        let rollback_line = match &rollback {
            Ok(revision) => format!("Rolled back to revision {revision}"),
            Err(problem) => problem.clone(),
        };
        let diagnostics = self
            .compose(probe, &[&reason, &rollback_line], &captured)
            .await;
        match rollback {
            Ok(_) => UpdateOutcome::RolledBack { diagnostics },
            Err(_) => UpdateOutcome::RollbackFailed { diagnostics },
        }
    }

    /// Summary lines (scrubbed, one line each) above the already-scrubbed capture.
    async fn compose(&self, probe: &dyn UpdateProbe, lines: &[&str], captured: &str) -> String {
        let summary: Vec<&str> = lines.iter().map(|line| first_line(line)).collect();
        let scrubbed = probe.scrub(&summary.join("\n")).await;
        compose_diagnostics(&scrubbed, captured)
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
        status: &'static str,
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
            self.queue_terminal_audit(
                &transaction,
                operation,
                terminal_action(&operation.operation, false),
                false,
                "indeterminate",
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn finish_operation_if_unstopped(
        &self,
        id: &str,
        status: &'static str,
        message: Option<String>,
        error: Option<String>,
    ) -> Result<bool> {
        self.finish_operation_with(
            id,
            Terminal {
                status,
                message,
                error,
                diagnostics: None,
                action: None,
            },
        )
        .await
    }

    async fn finish_operation_with(&self, id: &str, terminal: Terminal) -> Result<bool> {
        let Terminal {
            status,
            message,
            error,
            diagnostics,
            action,
        } = terminal;
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
        if diagnostics.is_some() {
            active.diagnostics = Set(diagnostics);
        }
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
        let success = status == STATUS_SUCCEEDED;
        let action = action.unwrap_or_else(|| terminal_action(&operation.operation, success));
        self.queue_terminal_audit(&transaction, &operation, action, success, phase)
            .await?;
        transaction.commit().await?;
        Ok(true)
    }

    async fn queue_terminal_audit(
        &self,
        transaction: &DatabaseTransaction,
        operation: &app_operation::Model,
        action: AuditAction,
        success: bool,
        phase: &str,
    ) -> Result<()> {
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
        self.queue_terminal_audit(
            &transaction,
            &operation,
            terminal_action(&operation.operation, false),
            false,
            "indeterminate",
        )
        .await?;
        transaction.commit().await?;
        tracing::error!(operation_id = %operation.id, "Recovered interrupted app operation with unknown external outcome");
        // The worker died holding an update: the app may be on a broken new
        // revision or half rolled back, and nobody else will say so. No
        // diagnostics are stored, so the failed-version skip ignores it and the
        // next window re-evaluates the app.
        if operation.operation == OP_UPDATE && !operation.stop_requested {
            let details = format!("{}; {INTERRUPTED_DETAILS}", update_subject(&operation));
            self.notify_update(&operation, &AuditAction::AppUpdateRollbackFailed, &details)
                .await;
        }
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
            "app_updated" => AuditAction::AppUpdated,
            "app_update_failed" => AuditAction::AppUpdateFailed,
            "app_update_rollback_failed" => AuditAction::AppUpdateRollbackFailed,
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
                auto_update: Set(true),
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
            auto_update: Set(true),
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

fn terminal_action(operation: &str, success: bool) -> AuditAction {
    match operation {
        OP_INSTALL => AuditAction::AppInstalled,
        OP_DELETE => AuditAction::AppUninstalled,
        OP_RESTART => AuditAction::AppRestarted,
        OP_UPDATE if success => AuditAction::AppUpdated,
        OP_UPDATE => AuditAction::AppUpdateFailed,
        _ => AuditAction::AppConfigured,
    }
}

/// `app=<app>`, plus `to=<version>` when the operation recorded its target.
fn update_subject(operation: &app_operation::Model) -> String {
    let target = parse_queued_config(operation.custom_config.as_deref())
        .ok()
        .and_then(|mut config| config.remove(AUTO_TARGET_KEY));
    match target {
        Some(target) => format!("app={} to={target}", operation.app_name),
        None => format!("app={}", operation.app_name),
    }
}

/// "10 min" for whole minutes, otherwise seconds ("90 s").
fn describe_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs > 0 && secs.is_multiple_of(60) {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// One line, at most `SUMMARY_LINE_MAX` bytes.
fn summary_line(text: &str) -> &str {
    let line = first_line(text);
    let mut end = line.len().min(SUMMARY_LINE_MAX);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// The scrubbed summary lines lead, so the first one is the "first diagnostic
/// line" notifications quote. Each is capped at `SUMMARY_LINE_MAX` bytes.
fn compose_diagnostics(summary: &str, captured: &str) -> String {
    let mut report = summary
        .lines()
        .map(summary_line)
        .collect::<Vec<_>>()
        .join("\n");
    if !captured.is_empty() {
        report.push_str("\n\n");
        report.push_str(captured);
    }
    update_diagnostics::truncate_report(report)
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
        let available_update_kind = match (
            model.update_available,
            model.installed_chart_version.as_deref(),
            model.available_chart_version.as_deref(),
        ) {
            (true, Some(installed), Some(available)) => Some(update_kind(installed, available)),
            _ => None,
        };
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
            auto_update: model.auto_update,
            available_update_kind,
            last_operation_id: model.last_operation_id,
            last_checked_at: model.last_checked_at,
            updated_at: model.updated_at,
        }
    }
}
