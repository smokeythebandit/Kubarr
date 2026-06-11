//! Migration: Create notification_logs table

use sea_orm_migration::prelude::*;

use super::m20260226_000001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(NotificationLogs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(NotificationLogs::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(NotificationLogs::UserId)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(NotificationLogs::ChannelType)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(NotificationLogs::EventType)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(NotificationLogs::Recipient).string().null())
                    .col(
                        ColumnDef::new(NotificationLogs::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(
                        ColumnDef::new(NotificationLogs::ErrorMessage)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(NotificationLogs::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(NotificationLogs::Table, NotificationLogs::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_notification_logs_user")
                    .table(NotificationLogs::Table)
                    .col(NotificationLogs::UserId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_notification_logs_status")
                    .table(NotificationLogs::Table)
                    .col(NotificationLogs::Status)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_notification_logs_created")
                    .table(NotificationLogs::Table)
                    .col(NotificationLogs::CreatedAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(NotificationLogs::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "notification_logs"]
enum NotificationLogs {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    #[iden = "channel_type"]
    ChannelType,
    #[iden = "event_type"]
    EventType,
    Recipient,
    Status,
    #[iden = "error_message"]
    ErrorMessage,
    #[iden = "created_at"]
    CreatedAt,
}
