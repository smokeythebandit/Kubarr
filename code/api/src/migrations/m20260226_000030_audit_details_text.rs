//! Allow audit details longer than PostgreSQL's default VARCHAR(255).

use sea_orm_migration::{prelude::*, sea_orm::DbBackend};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() == DbBackend::Postgres {
            manager
                .alter_table(
                    Table::alter()
                        .table(AuditLogs::Table)
                        .modify_column(ColumnDef::new(AuditLogs::Details).text().null())
                        .to_owned(),
                )
                .await?;
        }

        // SQLite's existing STRING column already has unbounded TEXT affinity.
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Do not shrink back to VARCHAR(255): that could reject or truncate existing details.
        Ok(())
    }
}

#[derive(Iden)]
#[iden = "audit_logs"]
enum AuditLogs {
    Table,
    Details,
}
