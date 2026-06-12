use crate::config::CONFIG;
use crate::error::{AppError, Result};
use crate::migrations::Migrator;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;

pub type DbConn = DatabaseConnection;

/// Create a new database connection and run migrations using config
pub async fn connect() -> Result<DbConn> {
    connect_with_url(&CONFIG.database.database_url).await
}

/// Create a new database connection with a specific URL and run migrations
pub async fn connect_with_url(database_url: &str) -> Result<DbConn> {
    tracing::info!("Connecting to database...");

    let mut opts = ConnectOptions::new(database_url);
    opts.max_connections(10)
        .min_connections(1)
        .connect_timeout(std::time::Duration::from_secs(30))
        .idle_timeout(std::time::Duration::from_secs(600))
        .sqlx_logging(false);

    let db = Database::connect(opts)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to connect to database: {}", e)))?;

    tracing::info!("Running database migrations...");
    Migrator::up(&db, None)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to run migrations: {}", e)))?;
    tracing::info!("Database migrations completed");

    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_with_url_succeeds_on_sqlite_memory() {
        let db = connect_with_url("sqlite::memory:").await.expect("connect");
        // Migrations must have run; just verify the connection works
        drop(db);
    }

    #[tokio::test]
    async fn connect_with_url_fails_on_bad_url() {
        let result = connect_with_url("invalid://notadb").await;
        assert!(result.is_err(), "bad URL must fail");
    }
}
