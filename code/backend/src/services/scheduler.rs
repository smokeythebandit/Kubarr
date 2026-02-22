//! Periodic task scheduler
//!
//! A simple scheduler for running background tasks at regular intervals.
//! Add new tasks by implementing the `PeriodicTask` trait.

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

use super::chart_sync::{ChartSyncService, ChartSyncTask};

/// Trait for periodic background tasks
#[async_trait]
pub trait PeriodicTask: Send + Sync {
    /// Task name for logging
    fn name(&self) -> &'static str;

    /// How often to run (e.g., every 1 hour)
    fn interval(&self) -> Duration;

    /// Execute the task
    async fn run(&self, db: &DatabaseConnection) -> anyhow::Result<()>;
}

/// Start all periodic tasks
pub fn start_scheduler(db: Arc<DatabaseConnection>, chart_sync: Arc<ChartSyncService>) {
    let tasks: Vec<Box<dyn PeriodicTask>> = vec![
        Box::new(SessionCleanupTask),
        Box::new(ChartSyncTask {
            service: chart_sync,
        }),
    ];

    for task in tasks {
        let db = db.clone();
        tokio::spawn(async move {
            run_task(task, db).await;
        });
    }

    tracing::info!("Periodic task scheduler started");
}

/// Run a single task on its interval
async fn run_task(task: Box<dyn PeriodicTask>, db: Arc<DatabaseConnection>) {
    let mut ticker = interval(task.interval());

    // Skip the first immediate tick
    ticker.tick().await;

    loop {
        ticker.tick().await;

        tracing::debug!(task = task.name(), "Running periodic task");

        match task.run(&db).await {
            Ok(()) => {
                tracing::debug!(task = task.name(), "Periodic task completed");
            }
            Err(e) => {
                tracing::error!(task = task.name(), error = %e, "Periodic task failed");
            }
        }
    }
}

// ============================================================================
// Session Cleanup Task
// ============================================================================

use crate::models::prelude::*;
use crate::models::session;
use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

/// Cleans up expired and revoked sessions
struct SessionCleanupTask;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cleanup_task_name() {
        assert_eq!(SessionCleanupTask.name(), "session_cleanup");
    }

    #[test]
    fn session_cleanup_task_interval_is_one_hour() {
        let d = SessionCleanupTask.interval();
        assert_eq!(d, Duration::from_secs(60 * 60));
    }
}

#[async_trait]
impl PeriodicTask for SessionCleanupTask {
    fn name(&self) -> &'static str {
        "session_cleanup"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(60 * 60) // Every hour
    }

    async fn run(&self, db: &DatabaseConnection) -> anyhow::Result<()> {
        let now = Utc::now();

        // Delete expired sessions
        let expired = Session::delete_many()
            .filter(session::Column::ExpiresAt.lt(now))
            .exec(db)
            .await?;

        // Delete revoked sessions older than 1 day (keep recent ones for audit)
        let day_ago = now - chrono::Duration::days(1);
        let revoked = Session::delete_many()
            .filter(session::Column::IsRevoked.eq(true))
            .filter(session::Column::CreatedAt.lt(day_ago))
            .exec(db)
            .await?;

        if expired.rows_affected > 0 || revoked.rows_affected > 0 {
            tracing::info!(
                expired = expired.rows_affected,
                revoked = revoked.rows_affected,
                "Cleaned up sessions"
            );
        }

        Ok(())
    }
}
