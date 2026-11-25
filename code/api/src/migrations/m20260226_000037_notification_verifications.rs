//! Migration: notification destination verification.
//!
//! One row per (user, channel) holds the pending code (salted hash only), its
//! expiry and attempt counter, plus the send-rate window. Rows outlive a
//! confirmed code so the rate limit keeps applying.

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
                    .table(NotificationVerifications::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(NotificationVerifications::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::UserId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::ChannelType)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::CodeHash)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::ExpiresAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::Attempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::LastSentAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::WindowStartedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::SendsInWindow)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(NotificationVerifications::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(
                                NotificationVerifications::Table,
                                NotificationVerifications::UserId,
                            )
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_notification_verifications_unique")
                    .table(NotificationVerifications::Table)
                    .col(NotificationVerifications::UserId)
                    .col(NotificationVerifications::ChannelType)
                    .unique()
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
                    .table(NotificationVerifications::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "notification_verifications"]
enum NotificationVerifications {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    #[iden = "channel_type"]
    ChannelType,
    #[iden = "code_hash"]
    CodeHash,
    #[iden = "expires_at"]
    ExpiresAt,
    Attempts,
    #[iden = "last_sent_at"]
    LastSentAt,
    #[iden = "window_started_at"]
    WindowStartedAt,
    #[iden = "sends_in_window"]
    SendsInWindow,
    #[iden = "updated_at"]
    UpdatedAt,
}
