//! Migration: per-event × per-channel notification routing.
//!
//! Creates `notification_routes`, which decides which external channels an
//! event is sent to. A missing row means "off"; the in-app bell ignores routes.
//! Existing events are seeded by severity: critical → Telegram, email and SMS;
//! warning → Telegram; info → bell only.
//!
//! Follows 000037 (notification verifications).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// (severity, channel types routed on by default)
const DEFAULTS: [(&str, &[&str]); 2] = [
    ("critical", &["telegram", "email", "messagebird"]),
    ("warning", &["telegram"]),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let connection = manager.get_connection();
        connection
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS notification_routes (
                     event_type   TEXT NOT NULL,
                     channel_type TEXT NOT NULL,
                     enabled      BOOLEAN NOT NULL,
                     PRIMARY KEY (event_type, channel_type)
                 )",
            )
            .await?;

        for (severity, channels) in DEFAULTS {
            for channel in channels {
                let events = Query::select()
                    .column(Alias::new("event_type"))
                    .expr(Expr::val(*channel))
                    .expr(Expr::val(true))
                    .from(Alias::new("notification_events"))
                    .and_where(Func::lower(Expr::col(Alias::new("severity"))).eq(severity))
                    .to_owned();
                let insert = Query::insert()
                    .into_table(Alias::new("notification_routes"))
                    .columns([
                        Alias::new("event_type"),
                        Alias::new("channel_type"),
                        Alias::new("enabled"),
                    ])
                    .select_from(events)
                    .map_err(|e| DbErr::Migration(e.to_string()))?
                    .on_conflict(OnConflict::new().do_nothing().to_owned())
                    .to_owned();
                manager.exec_stmt(insert).await?;
            }
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS notification_routes")
            .await?;
        Ok(())
    }
}
