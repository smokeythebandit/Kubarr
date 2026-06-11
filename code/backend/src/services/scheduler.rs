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

    /// Verify start_scheduler spawns tasks without panicking
    #[tokio::test]
    async fn start_scheduler_spawns_tasks_without_panic() {
        use crate::services::catalog::AppCatalog;
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory SQLite");
        let db_arc = Arc::new(db);
        let catalog = Arc::new(RwLock::new(AppCatalog::new()));
        let chart_sync = Arc::new(crate::services::chart_sync::ChartSyncService::new(catalog));

        // Just verify start_scheduler doesn't panic; tasks are spawned in background
        start_scheduler(db_arc, chart_sync);
        // Yield so spawned tasks get a chance to start (but not to tick)
        tokio::task::yield_now().await;
    }

    /// Cover run_task ok and err branches using a short-interval task and a short sleep.
    #[tokio::test]
    async fn run_task_ok_and_err_branches() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc as StdArc;

        let call_count = StdArc::new(std::sync::atomic::AtomicU32::new(0));

        struct ShortTask {
            count: StdArc<AtomicU32>,
        }
        #[async_trait::async_trait]
        impl PeriodicTask for ShortTask {
            fn name(&self) -> &'static str {
                "short_task"
            }
            fn interval(&self) -> Duration {
                Duration::from_millis(5)
            }
            async fn run(&self, _db: &DatabaseConnection) -> anyhow::Result<()> {
                let n = self.count.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Ok(())
                } else {
                    anyhow::bail!("simulated failure")
                }
            }
        }

        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("in-memory SQLite");
        let db_arc = StdArc::new(db);

        let task_handle = tokio::spawn(run_task(
            Box::new(ShortTask {
                count: call_count.clone(),
            }),
            db_arc,
        ));

        // Sleep long enough for the task to fire at least twice (ok + err branches)
        tokio::time::sleep(Duration::from_millis(30)).await;

        task_handle.abort();
        let _ = task_handle.await;
    }

    #[test]
    fn session_cleanup_task_interval_is_one_hour() {
        let d = SessionCleanupTask.interval();
        assert_eq!(d, Duration::from_secs(60 * 60));
    }

    /// Verify that run() executes without error against an empty in-memory DB.
    /// This covers the DB delete paths in SessionCleanupTask::run.
    #[tokio::test]
    async fn session_cleanup_task_run_with_empty_db() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create in-memory SQLite DB");
        SessionCleanupTask
            .run(&db)
            .await
            .expect("run must succeed on empty DB");
    }

    /// Verify run() reports success even when the DB has no expired/revoked sessions,
    /// exercising the rows_affected == 0 branch of the log guard.
    #[tokio::test]
    async fn session_cleanup_task_run_no_op_when_nothing_to_clean() {
        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create in-memory SQLite DB");
        // Run twice — both times with an empty table, covering idempotency.
        SessionCleanupTask.run(&db).await.expect("first run");
        SessionCleanupTask.run(&db).await.expect("second run");
    }

    /// Verify the logging branch (rows_affected > 0) is exercised.
    /// Creates an expired session in the DB, then runs cleanup to confirm deletion.
    #[tokio::test]
    async fn session_cleanup_task_run_deletes_expired_sessions() {
        use crate::models::{session, user};
        use sea_orm::{ActiveModelTrait, Set};

        let db = crate::application::database::connect_with_url("sqlite::memory:")
            .await
            .expect("create in-memory SQLite DB");

        let now = Utc::now();

        // Insert a user (sessions reference users via FK)
        user::ActiveModel {
            username: Set("testuser".to_string()),
            email: Set("test@example.com".to_string()),
            hashed_password: Set("hash".to_string()),
            is_active: Set(true),
            is_approved: Set(true),
            totp_secret: Set(None),
            totp_enabled: Set(false),
            totp_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert user");

        // Insert a session that expired in the past
        let past = now - chrono::Duration::days(2);
        session::ActiveModel {
            id: Set("sess-expired-123".to_string()),
            user_id: Set(1),
            user_agent: Set(None),
            ip_address: Set(None),
            created_at: Set(past),
            expires_at: Set(past), // already expired
            last_accessed_at: Set(past),
            is_revoked: Set(false),
        }
        .insert(&db)
        .await
        .expect("insert session");

        // Run cleanup — should delete the expired session and log info (line 150)
        SessionCleanupTask.run(&db).await.expect("cleanup run");

        // Verify session was deleted
        let remaining = Session::find().all(&db).await.expect("find sessions");
        assert!(remaining.is_empty(), "expired session should be deleted");
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
