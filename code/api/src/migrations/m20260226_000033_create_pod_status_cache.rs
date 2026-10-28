//! Pod status observed by the worker's Kubernetes watch.
//!
//! These rows are observations, not records: they can be rebuilt from the
//! cluster at any time, so on PostgreSQL the tables are UNLOGGED and skip the
//! write-ahead log. PostgreSQL truncates unlogged tables after an unclean
//! shutdown, which is why every reader treats an empty table as "ask
//! Kubernetes" rather than "there are no pods".
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let unlogged = match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => "UNLOGGED ",
            _ => "",
        };
        let connection = manager.get_connection();

        connection
            .execute_unprepared(&format!(
                "CREATE {unlogged}TABLE IF NOT EXISTS pod_status_cache (
                     namespace   TEXT PRIMARY KEY,
                     payload       TEXT NOT NULL,
                     observed_unix BIGINT NOT NULL
                 )"
            ))
            .await?;

        // A namespace with no pods never updates its own row, so stream health
        // is tracked separately: one row, rewritten on every watch event.
        connection
            .execute_unprepared(&format!(
                "CREATE {unlogged}TABLE IF NOT EXISTS pod_watch_heartbeat (
                     id          INTEGER PRIMARY KEY,
                     synced_unix BIGINT NOT NULL
                 )"
            ))
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let connection = manager.get_connection();
        connection
            .execute_unprepared("DROP TABLE IF EXISTS pod_status_cache")
            .await?;
        connection
            .execute_unprepared("DROP TABLE IF EXISTS pod_watch_heartbeat")
            .await?;
        Ok(())
    }
}
