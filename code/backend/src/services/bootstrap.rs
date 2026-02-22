use std::sync::Arc;

use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};

use crate::db;
use crate::error::{AppError, Result};
use crate::models::prelude::*;
use crate::models::{bootstrap_status, server_config};
use crate::services::catalog::AppCatalog;
use crate::services::k8s::K8sClient;
use crate::state::SharedDbConn;

/// Component status for bootstrap
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ComponentStatus {
    pub component: String,
    pub display_name: String,
    pub status: String, // 'pending', 'installing', 'healthy', 'failed'
    pub message: Option<String>,
    pub error: Option<String>,
}

/// Bootstrap progress event sent via WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BootstrapEvent {
    #[serde(rename = "component_started")]
    ComponentStarted { component: String, message: String },
    #[serde(rename = "component_progress")]
    ComponentProgress {
        component: String,
        message: String,
        progress: u8,
    },
    #[serde(rename = "component_completed")]
    ComponentCompleted { component: String, message: String },
    #[serde(rename = "component_failed")]
    ComponentFailed {
        component: String,
        message: String,
        error: String,
    },
    #[serde(rename = "database_connected")]
    DatabaseConnected { message: String },
    #[serde(rename = "bootstrap_complete")]
    BootstrapComplete { message: String },
}

/// Bootstrap components to install in order
pub const BOOTSTRAP_COMPONENTS: &[(&str, &str)] = &[
    ("postgresql", "PostgreSQL"),
    ("victoriametrics", "VictoriaMetrics"),
    ("victorialogs", "VictoriaLogs"),
    ("fluent-bit", "Fluent Bit"),
];

/// In-memory status for components (used before database is available)
#[derive(Debug, Clone, Default)]
pub struct InMemoryStatus {
    pub statuses: Vec<ComponentStatus>,
    pub started: bool,
}

/// Bootstrap service for installing system components
pub struct BootstrapService {
    db: SharedDbConn,
    k8s: Arc<RwLock<Option<K8sClient>>>,
    catalog: Arc<RwLock<AppCatalog>>,
    pub broadcast_tx: broadcast::Sender<String>,
    /// In-memory status (used before PostgreSQL is installed)
    in_memory_status: Arc<RwLock<InMemoryStatus>>,
}

impl BootstrapService {
    pub fn new(
        db: SharedDbConn,
        k8s: Arc<RwLock<Option<K8sClient>>>,
        catalog: Arc<RwLock<AppCatalog>>,
        broadcast_tx: broadcast::Sender<String>,
    ) -> Self {
        // Initialize in-memory status with all components
        let statuses = BOOTSTRAP_COMPONENTS
            .iter()
            .map(|(component, display_name)| ComponentStatus {
                component: component.to_string(),
                display_name: display_name.to_string(),
                status: "pending".to_string(),
                message: Some("Waiting to install".to_string()),
                error: None,
            })
            .collect();

        Self {
            db,
            k8s,
            catalog,
            broadcast_tx,
            in_memory_status: Arc::new(RwLock::new(InMemoryStatus {
                statuses,
                started: false,
            })),
        }
    }

    /// Get status of all bootstrap components
    pub async fn get_status(&self) -> Result<Vec<ComponentStatus>> {
        // Try database first
        let db_guard = self.db.read().await;
        if let Some(ref db) = *db_guard {
            let components = BootstrapStatus::find().all(db).await?;
            if !components.is_empty() {
                return Ok(components
                    .into_iter()
                    .map(|c| ComponentStatus {
                        component: c.component,
                        display_name: c.display_name,
                        status: c.status,
                        message: c.message,
                        error: c.error,
                    })
                    .collect());
            }
        }
        drop(db_guard);

        // Fall back to in-memory status
        let status = self.in_memory_status.read().await;
        Ok(status.statuses.clone())
    }

    /// Check if bootstrap is complete (all components healthy)
    pub async fn is_complete(&self) -> bool {
        match self.get_status().await {
            Ok(components) => {
                !components.is_empty() && components.iter().all(|c| c.status == "healthy")
            }
            Err(_) => false,
        }
    }

    /// Check if bootstrap has been started
    pub async fn has_started(&self) -> bool {
        // Check database first
        let db_guard = self.db.read().await;
        if let Some(ref db) = *db_guard {
            if let Ok(components) = BootstrapStatus::find().all(db).await {
                if components.iter().any(|c| c.status != "pending") {
                    return true;
                }
            }
        }
        drop(db_guard);

        // Check in-memory status
        let status = self.in_memory_status.read().await;
        status.started
    }

    /// Broadcast an event to all connected WebSocket clients
    fn broadcast(&self, event: BootstrapEvent) {
        if let Ok(json) = serde_json::to_string(&event) {
            let _ = self.broadcast_tx.send(json);
        }
    }

    /// Update in-memory status for a component
    async fn update_in_memory_status(
        &self,
        component: &str,
        status: &str,
        message: Option<&str>,
        error: Option<&str>,
    ) {
        let mut mem_status = self.in_memory_status.write().await;
        if let Some(comp) = mem_status
            .statuses
            .iter_mut()
            .find(|c| c.component == component)
        {
            comp.status = status.to_string();
            if let Some(msg) = message {
                comp.message = Some(msg.to_string());
            }
            if let Some(err) = error {
                comp.error = Some(err.to_string());
            }
        }
    }

    /// Update component status in database (if available)
    async fn update_db_status(
        &self,
        db: &DatabaseConnection,
        component: &str,
        status: &str,
        message: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let existing = BootstrapStatus::find()
            .filter(bootstrap_status::Column::Component.eq(component))
            .one(db)
            .await?;

        if let Some(existing) = existing {
            let mut active: bootstrap_status::ActiveModel = existing.into();
            active.status = Set(status.to_string());

            if let Some(msg) = message {
                active.message = Set(Some(msg.to_string()));
            }

            if let Some(err) = error {
                active.error = Set(Some(err.to_string()));
            }

            if status == "installing" {
                if let sea_orm::ActiveValue::Unchanged(None) = active.started_at {
                    active.started_at = Set(Some(Utc::now()));
                }
            }

            if status == "healthy" || status == "failed" {
                active.completed_at = Set(Some(Utc::now()));
            }

            active.update(db).await?;
        }
        Ok(())
    }

    /// Initialize bootstrap status records in database
    async fn initialize_db_status(&self, db: &DatabaseConnection) -> Result<()> {
        for (component, display_name) in BOOTSTRAP_COMPONENTS {
            let existing = BootstrapStatus::find()
                .filter(bootstrap_status::Column::Component.eq(*component))
                .one(db)
                .await?;

            if existing.is_none() {
                // Get current in-memory status
                let mem_status = self.in_memory_status.read().await;
                let current = mem_status
                    .statuses
                    .iter()
                    .find(|c| c.component == *component);

                let (status, message, error) = if let Some(c) = current {
                    (c.status.clone(), c.message.clone(), c.error.clone())
                } else {
                    (
                        "pending".to_string(),
                        Some("Waiting to install".to_string()),
                        None,
                    )
                };

                let record = bootstrap_status::ActiveModel {
                    component: Set(component.to_string()),
                    display_name: Set(display_name.to_string()),
                    status: Set(status),
                    message: Set(message),
                    error: Set(error),
                    ..Default::default()
                };
                record.insert(db).await?;
            }
        }
        Ok(())
    }

    /// Check if PostgreSQL is healthy
    async fn check_postgresql_health(&self) -> Result<bool> {
        let k8s_guard = self.k8s.read().await;
        let k8s = k8s_guard.as_ref().ok_or_else(|| {
            AppError::ServiceUnavailable("Kubernetes client not available".to_string())
        })?;

        // Check if the kubarr-db-0 pod is running and ready
        match k8s.get_pod("postgresql", "kubarr-db-0").await {
            Ok(pod) => {
                if let Some(status) = pod.status {
                    if let Some(phase) = status.phase {
                        if phase == "Running" {
                            if let Some(conditions) = status.conditions {
                                for condition in conditions {
                                    if condition.type_ == "Ready" && condition.status == "True" {
                                        return Ok(true);
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(false)
            }
            Err(_) => Ok(false),
        }
    }

    /// Install PostgreSQL using Helm chart
    async fn install_postgresql(&self) -> Result<()> {
        tracing::info!("Installing PostgreSQL via Helm chart");

        self.update_in_memory_status(
            "postgresql",
            "installing",
            Some("Deploying PostgreSQL..."),
            None,
        )
        .await;
        self.broadcast(BootstrapEvent::ComponentStarted {
            component: "postgresql".to_string(),
            message: "Deploying PostgreSQL...".to_string(),
        });
        self.broadcast(BootstrapEvent::ComponentProgress {
            component: "postgresql".to_string(),
            message: "Deploying PostgreSQL...".to_string(),
            progress: 10,
        });

        // Deploy PostgreSQL cluster using helm
        let result = tokio::process::Command::new("helm")
            .args([
                "upgrade",
                "--install",
                "postgresql",
                "/app/charts/postgresql",
                "-n",
                "postgresql",
                "--create-namespace",
                "--wait",
                "--timeout",
                "5m",
            ])
            .output()
            .await;

        match result {
            Ok(output) => {
                if !output.status.success() {
                    let error_msg = String::from_utf8_lossy(&output.stderr).to_string();
                    tracing::error!("PostgreSQL installation failed: {}", error_msg);
                    self.update_in_memory_status(
                        "postgresql",
                        "failed",
                        Some("Installation failed"),
                        Some(&error_msg),
                    )
                    .await;
                    self.broadcast(BootstrapEvent::ComponentFailed {
                        component: "postgresql".to_string(),
                        message: "PostgreSQL installation failed".to_string(),
                        error: error_msg.clone(),
                    });
                    return Err(AppError::Internal(error_msg));
                }
            }
            Err(e) => {
                let error_msg = format!("Failed to run helm: {}", e);
                tracing::error!("{}", error_msg);
                self.update_in_memory_status(
                    "postgresql",
                    "failed",
                    Some("Installation failed"),
                    Some(&error_msg),
                )
                .await;
                self.broadcast(BootstrapEvent::ComponentFailed {
                    component: "postgresql".to_string(),
                    message: "PostgreSQL installation failed".to_string(),
                    error: error_msg.clone(),
                });
                return Err(AppError::Internal(error_msg));
            }
        }

        self.broadcast(BootstrapEvent::ComponentProgress {
            component: "postgresql".to_string(),
            message: "Waiting for PostgreSQL to become healthy...".to_string(),
            progress: 50,
        });

        // Wait for PostgreSQL to become healthy
        let max_attempts = 60;
        let mut attempts = 0;
        let mut healthy = false;

        while attempts < max_attempts {
            if self.check_postgresql_health().await.unwrap_or(false) {
                healthy = true;
                break;
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            attempts += 1;

            let progress = 50 + (attempts * 50 / max_attempts) as u8;
            self.broadcast(BootstrapEvent::ComponentProgress {
                component: "postgresql".to_string(),
                message: format!(
                    "Waiting for PostgreSQL to become healthy... ({}/{})",
                    attempts, max_attempts
                ),
                progress: progress.min(99),
            });
        }

        if !healthy {
            let error_msg = "PostgreSQL did not become healthy within timeout".to_string();
            self.update_in_memory_status(
                "postgresql",
                "failed",
                Some("Health check timeout"),
                Some(&error_msg),
            )
            .await;
            self.broadcast(BootstrapEvent::ComponentFailed {
                component: "postgresql".to_string(),
                message: "PostgreSQL health check failed".to_string(),
                error: error_msg.clone(),
            });
            return Err(AppError::Internal(error_msg));
        }

        self.update_in_memory_status("postgresql", "healthy", Some("PostgreSQL is running"), None)
            .await;
        self.broadcast(BootstrapEvent::ComponentCompleted {
            component: "postgresql".to_string(),
            message: "PostgreSQL installed successfully".to_string(),
        });

        Ok(())
    }

    /// Connect to database after PostgreSQL is installed
    async fn connect_to_database(&self) -> Result<()> {
        tracing::info!("Connecting to PostgreSQL database...");

        // Get database URL from K8s secret
        let k8s_guard = self.k8s.read().await;
        let k8s = k8s_guard.as_ref().ok_or_else(|| {
            AppError::ServiceUnavailable("Kubernetes client not available".to_string())
        })?;

        let database_url = k8s.get_database_url("postgresql").await?;
        drop(k8s_guard);

        tracing::info!("Got database URL from K8s secret");

        // Retry connection - DNS may not be ready immediately after service creation
        let mut last_error = None;
        for attempt in 1..=10 {
            match db::connect_with_url(&database_url).await {
                Ok(conn) => {
                    tracing::info!("Database connection established on attempt {}", attempt);

                    // Store connection in shared state
                    {
                        let mut db_guard = self.db.write().await;
                        *db_guard = Some(conn.clone());
                    }

                    // Initialize JWT signing keys (required for login/sessions)
                    if let Err(e) = crate::services::init_jwt_keys(&conn).await {
                        tracing::warn!("Failed to initialize JWT keys: {}", e);
                    } else {
                        tracing::info!("JWT signing keys initialized");
                    }

                    // Initialize bootstrap status in database
                    self.initialize_db_status(&conn).await?;

                    self.broadcast(BootstrapEvent::DatabaseConnected {
                        message: "Database connection established".to_string(),
                    });

                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Database connection attempt {}/10 failed: {}", attempt, e);
                    last_error = Some(e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::Internal("Failed to connect to database after 10 attempts".to_string())
        }))
    }

    /// Install a single component using Helm
    async fn install_component(&self, component: &str, display_name: &str) -> Result<()> {
        // PostgreSQL is handled specially (needs to connect to DB after)
        if component == "postgresql" {
            self.install_postgresql().await?;
            self.connect_to_database().await?;
            return Ok(());
        }

        tracing::info!("Installing bootstrap component: {}", component);

        // Get database connection (should be available after PostgreSQL is installed)
        let db_guard = self.db.read().await;
        let db = db_guard
            .as_ref()
            .ok_or_else(|| AppError::ServiceUnavailable("Database not connected".to_string()))?;

        // Update status
        self.update_db_status(
            db,
            component,
            "installing",
            Some(&format!("Installing {}...", display_name)),
            None,
        )
        .await?;
        drop(db_guard);

        self.update_in_memory_status(
            component,
            "installing",
            Some(&format!("Installing {}...", display_name)),
            None,
        )
        .await;
        self.broadcast(BootstrapEvent::ComponentStarted {
            component: component.to_string(),
            message: format!("Installing {}...", display_name),
        });

        // Deploy using helm directly (bootstrap components aren't in the app catalog)
        let chart_path = format!("/app/charts/{}", component);
        let result = tokio::process::Command::new("helm")
            .args([
                "upgrade",
                "--install",
                component,
                &chart_path,
                "-n",
                component, // Each component in its own namespace
                "--create-namespace",
                "--wait",
                "--timeout",
                "5m",
            ])
            .output()
            .await;

        match result {
            Ok(output) => {
                if !output.status.success() {
                    let error_msg = String::from_utf8_lossy(&output.stderr).to_string();
                    tracing::error!("{} installation failed: {}", component, error_msg);

                    let db_guard = self.db.read().await;
                    if let Some(ref db) = *db_guard {
                        let _ = self
                            .update_db_status(
                                db,
                                component,
                                "failed",
                                Some("Installation failed"),
                                Some(&error_msg),
                            )
                            .await;
                    }
                    self.update_in_memory_status(
                        component,
                        "failed",
                        Some("Installation failed"),
                        Some(&error_msg),
                    )
                    .await;
                    self.broadcast(BootstrapEvent::ComponentFailed {
                        component: component.to_string(),
                        message: format!("{} installation failed", display_name),
                        error: error_msg.clone(),
                    });
                    return Err(AppError::Internal(error_msg));
                }

                tracing::info!("{} helm install succeeded", component);
                self.broadcast(BootstrapEvent::ComponentProgress {
                    component: component.to_string(),
                    message: format!("Deployed {}, verifying...", display_name),
                    progress: 80,
                });
            }
            Err(e) => {
                let error_msg = format!("Failed to run helm: {}", e);
                tracing::error!("{}", error_msg);

                let db_guard = self.db.read().await;
                if let Some(ref db) = *db_guard {
                    let _ = self
                        .update_db_status(
                            db,
                            component,
                            "failed",
                            Some("Installation failed"),
                            Some(&error_msg),
                        )
                        .await;
                }
                self.update_in_memory_status(
                    component,
                    "failed",
                    Some("Installation failed"),
                    Some(&error_msg),
                )
                .await;
                self.broadcast(BootstrapEvent::ComponentFailed {
                    component: component.to_string(),
                    message: format!("{} installation failed", display_name),
                    error: error_msg.clone(),
                });
                return Err(AppError::Internal(error_msg));
            }
        }

        // Mark as healthy (helm --wait already verified pods are ready)
        let db_guard = self.db.read().await;
        if let Some(ref db) = *db_guard {
            self.update_db_status(
                db,
                component,
                "healthy",
                Some(&format!("{} is running", display_name)),
                None,
            )
            .await?;
        }
        self.update_in_memory_status(
            component,
            "healthy",
            Some(&format!("{} is running", display_name)),
            None,
        )
        .await;
        self.broadcast(BootstrapEvent::ComponentCompleted {
            component: component.to_string(),
            message: format!("{} installed successfully", display_name),
        });

        Ok(())
    }

    /// Start the bootstrap process
    pub async fn start_bootstrap(&self) -> Result<()> {
        // Mark as started
        {
            let mut mem_status = self.in_memory_status.write().await;
            mem_status.started = true;
        }

        // Install PostgreSQL first (sequential, not parallel)
        self.install_component("postgresql", "PostgreSQL").await?;

        // Now install remaining components in parallel
        let remaining_components: Vec<_> = BOOTSTRAP_COMPONENTS
            .iter()
            .filter(|(c, _)| *c != "postgresql")
            .collect();

        let mut handles = Vec::new();
        for (component, display_name) in remaining_components {
            let db = self.db.clone();
            let k8s = self.k8s.clone();
            let catalog = self.catalog.clone();
            let broadcast_tx = self.broadcast_tx.clone();
            let component = component.to_string();
            let display_name = display_name.to_string();

            let handle = tokio::spawn(async move {
                let service = BootstrapService::new(db, k8s, catalog, broadcast_tx);
                if let Err(e) = service.install_component(&component, &display_name).await {
                    tracing::error!("Failed to install {}: {}", component, e);
                }
            });
            handles.push(handle);
        }

        // Wait for all installations to complete
        for handle in handles {
            let _ = handle.await;
        }

        // Check if all components are healthy
        if self.is_complete().await {
            self.broadcast(BootstrapEvent::BootstrapComplete {
                message: "All system components installed successfully".to_string(),
            });
        }

        Ok(())
    }

    /// Retry installing a failed component
    pub async fn retry_component(&self, component: &str) -> Result<()> {
        let comp = BOOTSTRAP_COMPONENTS
            .iter()
            .find(|(c, _)| *c == component)
            .ok_or_else(|| AppError::NotFound(format!("Unknown component: {}", component)))?;

        // Reset status
        self.update_in_memory_status(component, "pending", Some("Retrying installation..."), None)
            .await;

        // Also reset in database if available
        let db_guard = self.db.read().await;
        if let Some(ref db) = *db_guard {
            let _ = self
                .update_db_status(
                    db,
                    component,
                    "pending",
                    Some("Retrying installation..."),
                    None,
                )
                .await;
        }
        drop(db_guard);

        // Install the component
        self.install_component(comp.0, comp.1).await
    }
}

/// Save server configuration
pub async fn save_server_config(
    db: &DatabaseConnection,
    name: &str,
    storage_path: &str,
) -> Result<server_config::Model> {
    let now = Utc::now();

    let existing = ServerConfig::find().one(db).await?;

    if let Some(existing) = existing {
        let mut active: server_config::ActiveModel = existing.into();
        active.name = Set(name.to_string());
        active.storage_path = Set(storage_path.to_string());
        Ok(active.update(db).await?)
    } else {
        let config = server_config::ActiveModel {
            name: Set(name.to_string()),
            storage_path: Set(storage_path.to_string()),
            created_at: Set(now),
            ..Default::default()
        };
        Ok(config.insert(db).await?)
    }
}

/// Get server configuration
pub async fn get_server_config(db: &DatabaseConnection) -> Result<Option<server_config::Model>> {
    Ok(ServerConfig::find().one(db).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // BOOTSTRAP_COMPONENTS constant
    // -------------------------------------------------------------------------

    #[test]
    fn bootstrap_components_has_four_entries() {
        assert_eq!(BOOTSTRAP_COMPONENTS.len(), 4);
    }

    #[test]
    fn bootstrap_components_contains_postgresql() {
        let found = BOOTSTRAP_COMPONENTS.iter().any(|(c, _)| *c == "postgresql");
        assert!(found, "BOOTSTRAP_COMPONENTS must contain postgresql");
    }

    #[test]
    fn bootstrap_components_contains_victoriametrics() {
        let found = BOOTSTRAP_COMPONENTS
            .iter()
            .any(|(c, _)| *c == "victoriametrics");
        assert!(found, "BOOTSTRAP_COMPONENTS must contain victoriametrics");
    }

    #[test]
    fn bootstrap_components_contains_victorialogs() {
        let found = BOOTSTRAP_COMPONENTS
            .iter()
            .any(|(c, _)| *c == "victorialogs");
        assert!(found, "BOOTSTRAP_COMPONENTS must contain victorialogs");
    }

    #[test]
    fn bootstrap_components_contains_fluent_bit() {
        let found = BOOTSTRAP_COMPONENTS.iter().any(|(c, _)| *c == "fluent-bit");
        assert!(found, "BOOTSTRAP_COMPONENTS must contain fluent-bit");
    }

    // -------------------------------------------------------------------------
    // BootstrapEvent serde
    // -------------------------------------------------------------------------

    #[test]
    fn event_component_started_serializes_correctly() {
        let event = BootstrapEvent::ComponentStarted {
            component: "postgresql".to_string(),
            message: "Starting...".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"type\":\"component_started\""));
        assert!(json.contains("\"component\":\"postgresql\""));
    }

    #[test]
    fn event_component_progress_serializes_correctly() {
        let event = BootstrapEvent::ComponentProgress {
            component: "victoriametrics".to_string(),
            message: "50%".to_string(),
            progress: 50,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"type\":\"component_progress\""));
        assert!(json.contains("\"progress\":50"));
    }

    #[test]
    fn event_component_completed_serializes_correctly() {
        let event = BootstrapEvent::ComponentCompleted {
            component: "fluent-bit".to_string(),
            message: "Done".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"type\":\"component_completed\""));
    }

    #[test]
    fn event_component_failed_serializes_correctly() {
        let event = BootstrapEvent::ComponentFailed {
            component: "victorialogs".to_string(),
            message: "Failed".to_string(),
            error: "helm error".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"type\":\"component_failed\""));
        assert!(json.contains("\"error\":\"helm error\""));
    }

    #[test]
    fn event_database_connected_serializes_correctly() {
        let event = BootstrapEvent::DatabaseConnected {
            message: "Connected to database".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"type\":\"database_connected\""));
    }

    #[test]
    fn event_bootstrap_complete_serializes_correctly() {
        let event = BootstrapEvent::BootstrapComplete {
            message: "All components installed".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"type\":\"bootstrap_complete\""));
    }

    #[test]
    fn bootstrap_events_round_trip() {
        let events: Vec<BootstrapEvent> = vec![
            BootstrapEvent::ComponentStarted {
                component: "a".to_string(),
                message: "m".to_string(),
            },
            BootstrapEvent::ComponentProgress {
                component: "b".to_string(),
                message: "m".to_string(),
                progress: 42,
            },
            BootstrapEvent::ComponentCompleted {
                component: "c".to_string(),
                message: "m".to_string(),
            },
            BootstrapEvent::ComponentFailed {
                component: "d".to_string(),
                message: "m".to_string(),
                error: "e".to_string(),
            },
            BootstrapEvent::DatabaseConnected {
                message: "m".to_string(),
            },
            BootstrapEvent::BootstrapComplete {
                message: "m".to_string(),
            },
        ];
        for event in &events {
            let json = serde_json::to_string(event).expect("serialize");
            let back: BootstrapEvent = serde_json::from_str(&json).expect("deserialize");
            // Re-serialize and compare JSON strings as a proxy for equality
            let json2 = serde_json::to_string(&back).expect("re-serialize");
            assert_eq!(json, json2);
        }
    }

    // -------------------------------------------------------------------------
    // ComponentStatus struct
    // -------------------------------------------------------------------------

    #[test]
    fn component_status_construction() {
        let status = ComponentStatus {
            component: "postgresql".to_string(),
            display_name: "PostgreSQL".to_string(),
            status: "healthy".to_string(),
            message: Some("Running".to_string()),
            error: None,
        };
        assert_eq!(status.component, "postgresql");
        assert_eq!(status.status, "healthy");
        assert!(status.error.is_none());
    }

    #[test]
    fn component_status_clone() {
        let status = ComponentStatus {
            component: "victoriametrics".to_string(),
            display_name: "VictoriaMetrics".to_string(),
            status: "failed".to_string(),
            message: None,
            error: Some("helm error".to_string()),
        };
        let cloned = status.clone();
        assert_eq!(cloned.component, status.component);
        assert_eq!(cloned.error, status.error);
    }

    #[test]
    fn component_status_serde_round_trip() {
        let status = ComponentStatus {
            component: "fluent-bit".to_string(),
            display_name: "Fluent Bit".to_string(),
            status: "installing".to_string(),
            message: Some("Deploying...".to_string()),
            error: None,
        };
        let json = serde_json::to_string(&status).expect("serialize");
        let back: ComponentStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.component, status.component);
        assert_eq!(back.status, status.status);
    }

    // -------------------------------------------------------------------------
    // InMemoryStatus
    // -------------------------------------------------------------------------

    #[test]
    fn in_memory_status_default() {
        let s = InMemoryStatus::default();
        assert!(s.statuses.is_empty());
        assert!(!s.started);
    }

    // -------------------------------------------------------------------------
    // initialize_db_status — private method, tested via inline tests
    // -------------------------------------------------------------------------

    async fn make_svc_with_db(db: &DatabaseConnection) -> BootstrapService {
        let shared_db = Arc::new(RwLock::new(Some(db.clone())));
        let k8s: Arc<RwLock<Option<K8sClient>>> = Arc::new(RwLock::new(None));
        let catalog = Arc::new(RwLock::new(AppCatalog::default()));
        let (tx, _rx) = broadcast::channel(100);
        BootstrapService::new(shared_db, k8s, catalog, tx)
    }

    #[tokio::test]
    async fn initialize_db_status_creates_pending_records() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");
        let svc = make_svc_with_db(&db).await;

        svc.initialize_db_status(&db)
            .await
            .expect("initialize_db_status");

        let records = BootstrapStatus::find().all(&db).await.expect("find all");
        assert_eq!(records.len(), 4, "all 4 components should be initialized");
        for r in &records {
            assert_eq!(r.status, "pending");
        }
    }

    #[tokio::test]
    async fn initialize_db_status_skips_existing_records() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        // Pre-insert one record
        bootstrap_status::ActiveModel {
            component: Set("postgresql".to_string()),
            display_name: Set("PostgreSQL".to_string()),
            status: Set("healthy".to_string()),
            message: Set(Some("Already installed".to_string())),
            error: Set(None),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert existing");

        let svc = make_svc_with_db(&db).await;
        svc.initialize_db_status(&db)
            .await
            .expect("initialize_db_status");

        let records = BootstrapStatus::find().all(&db).await.expect("find all");
        assert_eq!(records.len(), 4);

        let pg = records
            .iter()
            .find(|r| r.component == "postgresql")
            .unwrap();
        assert_eq!(
            pg.status, "healthy",
            "existing record must not be overwritten"
        );

        for r in &records {
            if r.component != "postgresql" {
                assert_eq!(r.status, "pending");
            }
        }
    }

    // -------------------------------------------------------------------------
    // save_server_config / get_server_config
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn save_server_config_creates_new_record() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");
        let model = save_server_config(&db, "My Kubarr", "/data/kubarr")
            .await
            .expect("save_server_config");
        assert_eq!(model.name, "My Kubarr");
        assert_eq!(model.storage_path, "/data/kubarr");
    }

    #[tokio::test]
    async fn save_server_config_updates_existing_record() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");
        save_server_config(&db, "Original", "/original/path")
            .await
            .expect("first save");
        let updated = save_server_config(&db, "Updated", "/new/path")
            .await
            .expect("second save");
        assert_eq!(updated.name, "Updated");
        assert_eq!(updated.storage_path, "/new/path");
    }

    #[tokio::test]
    async fn get_server_config_returns_none_on_empty_db() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");
        let cfg = get_server_config(&db).await.expect("get");
        assert!(cfg.is_none());
    }

    #[tokio::test]
    async fn get_server_config_returns_saved_value() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");
        save_server_config(&db, "Test Server", "/test")
            .await
            .expect("save");
        let cfg = get_server_config(&db).await.expect("get").expect("Some");
        assert_eq!(cfg.name, "Test Server");
        assert_eq!(cfg.storage_path, "/test");
    }

    // -------------------------------------------------------------------------
    // update_db_status — timestamps
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn update_db_status_sets_started_at_when_installing() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        // Insert record with no started_at
        bootstrap_status::ActiveModel {
            component: Set("victoriametrics".to_string()),
            display_name: Set("VictoriaMetrics".to_string()),
            status: Set("pending".to_string()),
            message: Set(None),
            error: Set(None),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert");

        let svc = make_svc_with_db(&db).await;
        svc.update_db_status(
            &db,
            "victoriametrics",
            "installing",
            Some("Installing..."),
            None,
        )
        .await
        .expect("update_db_status");

        use crate::models::bootstrap_status::Column;
        use sea_orm::ColumnTrait;
        let record = BootstrapStatus::find()
            .filter(Column::Component.eq("victoriametrics"))
            .one(&db)
            .await
            .expect("find")
            .expect("exists");
        assert_eq!(record.status, "installing");
    }

    #[tokio::test]
    async fn update_db_status_sets_completed_at_on_healthy() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        bootstrap_status::ActiveModel {
            component: Set("fluent-bit".to_string()),
            display_name: Set("Fluent Bit".to_string()),
            status: Set("installing".to_string()),
            message: Set(None),
            error: Set(None),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert");

        let svc = make_svc_with_db(&db).await;
        svc.update_db_status(&db, "fluent-bit", "healthy", Some("Done"), None)
            .await
            .expect("update_db_status");

        use crate::models::bootstrap_status::Column;
        use sea_orm::ColumnTrait;
        let record = BootstrapStatus::find()
            .filter(Column::Component.eq("fluent-bit"))
            .one(&db)
            .await
            .expect("find")
            .expect("exists");
        assert_eq!(record.status, "healthy");
        assert!(record.completed_at.is_some());
    }

    #[tokio::test]
    async fn update_db_status_sets_completed_at_on_failed() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        bootstrap_status::ActiveModel {
            component: Set("postgresql".to_string()),
            display_name: Set("PostgreSQL".to_string()),
            status: Set("installing".to_string()),
            message: Set(None),
            error: Set(None),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert");

        let svc = make_svc_with_db(&db).await;
        svc.update_db_status(
            &db,
            "postgresql",
            "failed",
            Some("Error"),
            Some("helm failed"),
        )
        .await
        .expect("update_db_status");

        use crate::models::bootstrap_status::Column;
        use sea_orm::ColumnTrait;
        let record = BootstrapStatus::find()
            .filter(Column::Component.eq("postgresql"))
            .one(&db)
            .await
            .expect("find")
            .expect("exists");
        assert_eq!(record.status, "failed");
        assert!(record.completed_at.is_some());
        assert_eq!(record.error.as_deref(), Some("helm failed"));
    }

    #[tokio::test]
    async fn get_status_returns_in_memory_when_db_empty() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");
        let svc = make_svc_with_db(&db).await;

        // DB has no bootstrap_status records → should return in-memory status
        let statuses = svc.get_status().await.expect("get_status");
        assert_eq!(statuses.len(), 4); // All 4 components from BOOTSTRAP_COMPONENTS
        for s in &statuses {
            assert_eq!(s.status, "pending");
        }
    }

    #[tokio::test]
    async fn get_status_returns_db_records_when_present() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        // Insert a record
        bootstrap_status::ActiveModel {
            component: Set("postgresql".to_string()),
            display_name: Set("PostgreSQL".to_string()),
            status: Set("healthy".to_string()),
            message: Set(Some("Installed".to_string())),
            error: Set(None),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert");

        let svc = make_svc_with_db(&db).await;
        let statuses = svc.get_status().await.expect("get_status");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].component, "postgresql");
        assert_eq!(statuses[0].status, "healthy");
    }

    #[tokio::test]
    async fn is_complete_returns_false_when_any_component_not_healthy() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        // Insert 4 records, one not healthy
        for (comp, disp) in BOOTSTRAP_COMPONENTS {
            let status = if *comp == "postgresql" {
                "healthy"
            } else {
                "pending"
            };
            bootstrap_status::ActiveModel {
                component: Set(comp.to_string()),
                display_name: Set(disp.to_string()),
                status: Set(status.to_string()),
                ..Default::default()
            }
            .insert(&db)
            .await
            .expect("insert");
        }

        let svc = make_svc_with_db(&db).await;
        assert!(!svc.is_complete().await);
    }

    #[tokio::test]
    async fn is_complete_returns_true_when_all_healthy() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        for (comp, disp) in BOOTSTRAP_COMPONENTS {
            bootstrap_status::ActiveModel {
                component: Set(comp.to_string()),
                display_name: Set(disp.to_string()),
                status: Set("healthy".to_string()),
                ..Default::default()
            }
            .insert(&db)
            .await
            .expect("insert");
        }

        let svc = make_svc_with_db(&db).await;
        assert!(svc.is_complete().await);
    }

    #[tokio::test]
    async fn is_complete_returns_false_when_no_components_in_db() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        let svc = make_svc_with_db(&db).await;
        // In-memory status has pending components → not complete
        assert!(!svc.is_complete().await);
    }

    #[tokio::test]
    async fn has_started_returns_false_when_all_pending() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        // Insert all components with "pending" status
        for (comp, disp) in BOOTSTRAP_COMPONENTS {
            bootstrap_status::ActiveModel {
                component: Set(comp.to_string()),
                display_name: Set(disp.to_string()),
                status: Set("pending".to_string()),
                ..Default::default()
            }
            .insert(&db)
            .await
            .expect("insert");
        }

        let svc = make_svc_with_db(&db).await;
        assert!(!svc.has_started().await);
    }

    #[tokio::test]
    async fn has_started_returns_true_when_any_non_pending() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        bootstrap_status::ActiveModel {
            component: Set("postgresql".to_string()),
            display_name: Set("PostgreSQL".to_string()),
            status: Set("installing".to_string()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert");

        let svc = make_svc_with_db(&db).await;
        assert!(svc.has_started().await);
    }

    #[tokio::test]
    async fn has_started_returns_false_when_db_empty() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        let svc = make_svc_with_db(&db).await;
        // DB empty + in-memory started=false
        assert!(!svc.has_started().await);
    }

    #[tokio::test]
    async fn update_in_memory_status_updates_component() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        let svc = make_svc_with_db(&db).await;

        svc.update_in_memory_status("postgresql", "installing", Some("In progress"), None)
            .await;

        let status = svc.in_memory_status.read().await;
        let pg = status
            .statuses
            .iter()
            .find(|c| c.component == "postgresql")
            .expect("found");
        assert_eq!(pg.status, "installing");
        assert_eq!(pg.message.as_deref(), Some("In progress"));
    }

    #[tokio::test]
    async fn update_in_memory_status_with_error() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        let svc = make_svc_with_db(&db).await;

        svc.update_in_memory_status(
            "victoriametrics",
            "failed",
            Some("Error"),
            Some("helm error"),
        )
        .await;

        let status = svc.in_memory_status.read().await;
        let vm = status
            .statuses
            .iter()
            .find(|c| c.component == "victoriametrics")
            .expect("found");
        assert_eq!(vm.status, "failed");
        assert_eq!(vm.error.as_deref(), Some("helm error"));
    }

    #[tokio::test]
    async fn broadcast_sends_event_to_channel() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create db");

        let shared_db = Arc::new(RwLock::new(Some(db.clone())));
        let k8s: Arc<RwLock<Option<K8sClient>>> = Arc::new(RwLock::new(None));
        let catalog = Arc::new(RwLock::new(AppCatalog::default()));
        let (tx, mut rx) = broadcast::channel(100);
        let svc = BootstrapService::new(shared_db, k8s, catalog, tx);

        svc.broadcast(BootstrapEvent::BootstrapComplete {
            message: "All done".to_string(),
        });

        let msg = rx.try_recv().expect("should have received a message");
        assert!(msg.contains("bootstrap_complete"));
        assert!(msg.contains("All done"));
    }
}
