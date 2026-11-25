//! Migration: AI assistant chat.
//!
//! - `assistant_conversations`: one row per conversation, owned by a user,
//!   with the running summary of turns no longer sent in full.
//! - `assistant_messages`: the user's questions and the assistant's answers
//!   (text and sources only; tool results are never stored).
//! - `assistant_requests`: one row per question asked, for the per-user rate
//!   limit. Kept apart from the messages so deleting a conversation does not
//!   reset the limit.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Conversations::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Conversations::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Conversations::UserId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Conversations::Title).string().not_null())
                    .col(ColumnDef::new(Conversations::ContextJson).text().null())
                    .col(ColumnDef::new(Conversations::Summary).text().null())
                    .col(
                        ColumnDef::new(Conversations::SummaryThrough)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Conversations::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Conversations::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_assistant_conversations_user")
                            .from(Conversations::Table, Conversations::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_assistant_conversations_user")
                    .table(Conversations::Table)
                    .col(Conversations::UserId)
                    .col(Conversations::UpdatedAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_assistant_conversations_updated_at")
                    .table(Conversations::Table)
                    .col(Conversations::UpdatedAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Messages::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Messages::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Messages::ConversationId).string().not_null())
                    .col(ColumnDef::new(Messages::Role).string().not_null())
                    .col(ColumnDef::new(Messages::ContentJson).text().not_null())
                    .col(
                        ColumnDef::new(Messages::Status)
                            .string()
                            .not_null()
                            .default("complete"),
                    )
                    .col(
                        ColumnDef::new(Messages::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Messages::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_assistant_messages_conversation")
                            .from(Messages::Table, Messages::ConversationId)
                            .to(Conversations::Table, Conversations::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_assistant_messages_conversation")
                    .table(Messages::Table)
                    .col(Messages::ConversationId)
                    .col(Messages::Id)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Requests::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Requests::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Requests::UserId).big_integer().not_null())
                    .col(
                        ColumnDef::new(Requests::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_assistant_requests_user_created")
                    .table(Requests::Table)
                    .col(Requests::UserId)
                    .col(Requests::CreatedAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Requests::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Messages::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Conversations::Table)
                    .if_exists()
                    .to_owned(),
            )
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
    #[iden = "user_id"]
    UserId,
    Title,
    #[iden = "context_json"]
    ContextJson,
    Summary,
    #[iden = "summary_through"]
    SummaryThrough,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}

#[derive(Iden)]
#[iden = "assistant_messages"]
enum Messages {
    Table,
    Id,
    #[iden = "conversation_id"]
    ConversationId,
    Role,
    #[iden = "content_json"]
    ContentJson,
    Status,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}

#[derive(Iden)]
#[iden = "assistant_requests"]
enum Requests {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    #[iden = "created_at"]
    CreatedAt,
}
