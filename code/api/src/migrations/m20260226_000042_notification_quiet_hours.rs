//! Migration: per-user quiet hours and the outbox of held messages.
//!
//! - `user_quiet_hours`: one optional row per user. Times are minutes after
//!   local midnight in the user's IANA `timezone`; the window may cross
//!   midnight. A missing row means quiet hours are off.
//! - `notification_outbox`: external messages held during quiet hours, one
//!   row per (user, channel, event). The leader flushes rows whose
//!   `deliver_after` has passed as one summary per (user, channel).
//!   `claimed_until` marks rows a flush is sending (a lease, separate from
//!   `deliver_after` so a settings change never moves claimed rows);
//!   `attempts` counts failed sends for the retry backoff.
//!
//! Follows 000041 (assistant actions).

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
                    .table(UserQuietHours::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserQuietHours::UserId)
                            .big_integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UserQuietHours::Enabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(UserQuietHours::FromMinute)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserQuietHours::ToMinute)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UserQuietHours::Timezone).text().not_null())
                    .col(
                        ColumnDef::new(UserQuietHours::CriticalBypass)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(UserQuietHours::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_quiet_hours_user")
                            .from(UserQuietHours::Table, UserQuietHours::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(NotificationOutbox::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(NotificationOutbox::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(NotificationOutbox::UserId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(NotificationOutbox::ChannelType)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(NotificationOutbox::EventType)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(NotificationOutbox::Title).text().not_null())
                    .col(ColumnDef::new(NotificationOutbox::Body).text().not_null())
                    .col(
                        ColumnDef::new(NotificationOutbox::Severity)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(NotificationOutbox::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(NotificationOutbox::DeliverAfter)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(NotificationOutbox::ClaimedUntil)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(NotificationOutbox::Attempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_notification_outbox_user")
                            .from(NotificationOutbox::Table, NotificationOutbox::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_notification_outbox_deliver_after")
                    .table(NotificationOutbox::Table)
                    .col(NotificationOutbox::DeliverAfter)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_notification_outbox_user")
                    .table(NotificationOutbox::Table)
                    .col(NotificationOutbox::UserId)
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
                    .table(NotificationOutbox::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(UserQuietHours::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "user_quiet_hours"]
enum UserQuietHours {
    Table,
    #[iden = "user_id"]
    UserId,
    Enabled,
    #[iden = "from_minute"]
    FromMinute,
    #[iden = "to_minute"]
    ToMinute,
    Timezone,
    #[iden = "critical_bypass"]
    CriticalBypass,
    #[iden = "updated_at"]
    UpdatedAt,
}

#[derive(Iden)]
#[iden = "notification_outbox"]
enum NotificationOutbox {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    #[iden = "channel_type"]
    ChannelType,
    #[iden = "event_type"]
    EventType,
    Title,
    Body,
    Severity,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "deliver_after"]
    DeliverAfter,
    #[iden = "claimed_until"]
    ClaimedUntil,
    Attempts,
}
