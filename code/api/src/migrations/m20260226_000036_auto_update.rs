//! Migration: automatic app updates.
//!
//! Adds the per-app `auto_update` flag (default on), the operation `trigger`
//! and failure `diagnostics`, and seeds the three update notification events.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// (event_type, enabled, severity)
const EVENTS: [(&str, bool, &str); 3] = [
    ("app_updated", false, "info"),
    ("app_update_failed", true, "warning"),
    ("app_update_rollback_failed", true, "critical"),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("app_states"))
                    .add_column(
                        ColumnDef::new(Alias::new("auto_update"))
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("app_operations"))
                    .add_column(ColumnDef::new(Alias::new("trigger")).text().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("app_operations"))
                    .add_column(ColumnDef::new(Alias::new("diagnostics")).text().null())
                    .to_owned(),
            )
            .await?;

        for (event_type, enabled, severity) in EVENTS {
            // Idempotent: keep an existing row (and any owner-chosen setting).
            let lookup = Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("notification_events"))
                .and_where(Expr::col(Alias::new("event_type")).eq(event_type))
                .to_owned();
            let exists = manager.get_connection().query_one(&lookup).await?.is_some();
            if exists {
                continue;
            }
            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(Alias::new("notification_events"))
                        .columns([
                            Alias::new("event_type"),
                            Alias::new("enabled"),
                            Alias::new("severity"),
                        ])
                        .values_panic([event_type.into(), enabled.into(), severity.into()])
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("notification_events"))
                    .and_where(
                        Expr::col(Alias::new("event_type")).is_in(EVENTS.map(|(name, _, _)| name)),
                    )
                    .to_owned(),
            )
            .await?;
        for (table, column) in [
            ("app_operations", "diagnostics"),
            ("app_operations", "trigger"),
            ("app_states", "auto_update"),
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(table))
                        .drop_column(Alias::new(column))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
