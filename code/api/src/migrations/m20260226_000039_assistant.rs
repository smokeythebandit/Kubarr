//! Migration: AI assistant foundation.
//!
//! Creates the `assistant_usage` ledger (one row per Messages API call, used
//! for the monthly budget), seeds the `assistant_budget_warning` notification
//! event and grants the new `assistant.use` permission to the admin role.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const EVENT_TYPE: &str = "assistant_budget_warning";
const PERMISSION: &str = "assistant.use";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AssistantUsage::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AssistantUsage::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AssistantUsage::UserId).big_integer().null())
                    .col(
                        ColumnDef::new(AssistantUsage::ConversationId)
                            .string()
                            .null(),
                    )
                    .col(ColumnDef::new(AssistantUsage::Model).string().not_null())
                    .col(
                        ColumnDef::new(AssistantUsage::InputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AssistantUsage::OutputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AssistantUsage::CacheReadTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AssistantUsage::CacheWriteTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AssistantUsage::CostUsdMicros)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AssistantUsage::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_assistant_usage_created_at")
                    .table(AssistantUsage::Table)
                    .col(AssistantUsage::CreatedAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        let db = manager.get_connection();

        // Notification event: enabled, warning. Keep an existing row.
        let event_exists = db
            .query_one(
                &Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("notification_events"))
                    .and_where(Expr::col(Alias::new("event_type")).eq(EVENT_TYPE))
                    .to_owned(),
            )
            .await?
            .is_some();
        if !event_exists {
            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(Alias::new("notification_events"))
                        .columns([
                            Alias::new("event_type"),
                            Alias::new("enabled"),
                            Alias::new("severity"),
                        ])
                        .values_panic([EVENT_TYPE.into(), true.into(), "warning".into()])
                        .to_owned(),
                )
                .await?;
        }

        // Permission: granted to the admin role (if it exists yet; a fresh
        // install seeds roles in an earlier migration).
        let admin = db
            .query_one(
                &Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("roles"))
                    .and_where(Expr::col(Alias::new("name")).eq("admin"))
                    .to_owned(),
            )
            .await?;
        if let Some(row) = admin {
            let role_id: i64 = row.try_get("", "id")?;
            let granted = db
                .query_one(
                    &Query::select()
                        .column(Alias::new("id"))
                        .from(Alias::new("role_permissions"))
                        .and_where(Expr::col(Alias::new("role_id")).eq(role_id))
                        .and_where(Expr::col(Alias::new("permission")).eq(PERMISSION))
                        .to_owned(),
                )
                .await?
                .is_some();
            if !granted {
                manager
                    .exec_stmt(
                        Query::insert()
                            .into_table(Alias::new("role_permissions"))
                            .columns([Alias::new("role_id"), Alias::new("permission")])
                            .values_panic([role_id.into(), PERMISSION.into()])
                            .to_owned(),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("role_permissions"))
                    .and_where(Expr::col(Alias::new("permission")).eq(PERMISSION))
                    .to_owned(),
            )
            .await?;
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("notification_events"))
                    .and_where(Expr::col(Alias::new("event_type")).eq(EVENT_TYPE))
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AssistantUsage::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "assistant_usage"]
enum AssistantUsage {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    #[iden = "conversation_id"]
    ConversationId,
    Model,
    #[iden = "input_tokens"]
    InputTokens,
    #[iden = "output_tokens"]
    OutputTokens,
    #[iden = "cache_read_tokens"]
    CacheReadTokens,
    #[iden = "cache_write_tokens"]
    CacheWriteTokens,
    #[iden = "cost_usd_micros"]
    CostUsdMicros,
    #[iden = "created_at"]
    CreatedAt,
}
