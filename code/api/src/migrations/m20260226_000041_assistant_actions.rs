//! Migration: actions the AI assistant proposed, waiting for the person's
//! Run or Cancel click.
//!
//! - `assistant_pending_actions`: one row per proposed action, owned by the
//!   user of the conversation. `status` moves `pending` → `running` (a Run
//!   click claimed it) → `done`/`failed`, or `pending` → `cancelled`; a
//!   pending row past `expires_at` reads as expired. `operation_id` links the
//!   queued Kubarr operation so its outcome can be shown; `reason` is the
//!   model's one-line why (shown to the owner only, never in audit details).
//! - Seeds the notification event for a requested rollback that failed,
//!   `app_rollback_failed` (warning, on), routed to Telegram like other
//!   warnings when routing exists. A successful rollback is audited only:
//!   the person asked for it and sees the outcome on the card.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// (event_type, enabled, severity, channels routed on).
const EVENTS: [(&str, bool, &str, &[&str]); 1] =
    [("app_rollback_failed", true, "warning", &["telegram"])];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Actions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Actions::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Actions::ConversationId).string().not_null())
                    .col(ColumnDef::new(Actions::MessageId).big_integer().null())
                    .col(ColumnDef::new(Actions::UserId).big_integer().not_null())
                    .col(ColumnDef::new(Actions::Kind).string().not_null())
                    .col(ColumnDef::new(Actions::Target).string().not_null())
                    .col(ColumnDef::new(Actions::ParamsJson).text().not_null())
                    .col(ColumnDef::new(Actions::Summary).text().not_null())
                    .col(ColumnDef::new(Actions::Reason).text().null())
                    .col(ColumnDef::new(Actions::Status).string().not_null())
                    .col(ColumnDef::new(Actions::OperationId).string().null())
                    .col(ColumnDef::new(Actions::Error).text().null())
                    .col(
                        ColumnDef::new(Actions::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Actions::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Actions::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_assistant_pending_actions_conversation")
                            .from(Actions::Table, Actions::ConversationId)
                            .to(Conversations::Table, Conversations::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_assistant_pending_actions_user")
                            .from(Actions::Table, Actions::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_assistant_pending_actions_conversation")
                    .table(Actions::Table)
                    .col(Actions::ConversationId)
                    .col(Actions::CreatedAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        let routes = manager.has_table("notification_routes").await?;
        for (event_type, enabled, severity, channels) in EVENTS {
            // Idempotent: keep an existing row (and any owner-chosen setting).
            let lookup = Query::select()
                .column(Alias::new("id"))
                .from(Alias::new("notification_events"))
                .and_where(Expr::col(Alias::new("event_type")).eq(event_type))
                .to_owned();
            if manager.get_connection().query_one(&lookup).await?.is_some() {
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
            if !routes {
                continue;
            }
            for channel in channels {
                manager
                    .exec_stmt(
                        Query::insert()
                            .into_table(Alias::new("notification_routes"))
                            .columns([
                                Alias::new("event_type"),
                                Alias::new("channel_type"),
                                Alias::new("enabled"),
                            ])
                            .values_panic([event_type.into(), (*channel).into(), true.into()])
                            .on_conflict(
                                OnConflict::columns([
                                    Alias::new("event_type"),
                                    Alias::new("channel_type"),
                                ])
                                .do_nothing()
                                .to_owned(),
                            )
                            .to_owned(),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let names = EVENTS.map(|(name, _, _, _)| name);
        if manager.has_table("notification_routes").await? {
            manager
                .exec_stmt(
                    Query::delete()
                        .from_table(Alias::new("notification_routes"))
                        .and_where(Expr::col(Alias::new("event_type")).is_in(names))
                        .to_owned(),
                )
                .await?;
        }
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("notification_events"))
                    .and_where(Expr::col(Alias::new("event_type")).is_in(names))
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Actions::Table).if_exists().to_owned())
            .await
    }
}

#[derive(Iden)]
enum Users {
    Table,
    Id,
}

#[derive(Iden)]
#[iden = "assistant_conversations"]
enum Conversations {
    Table,
    Id,
}

#[derive(Iden)]
#[iden = "assistant_pending_actions"]
enum Actions {
    Table,
    Id,
    #[iden = "conversation_id"]
    ConversationId,
    #[iden = "message_id"]
    MessageId,
    #[iden = "user_id"]
    UserId,
    Kind,
    Target,
    #[iden = "params_json"]
    ParamsJson,
    Summary,
    Reason,
    Status,
    #[iden = "operation_id"]
    OperationId,
    Error,
    #[iden = "expires_at"]
    ExpiresAt,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
