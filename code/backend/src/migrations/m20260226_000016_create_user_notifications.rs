//! Migration: Create user_notifications table

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
                    .table(UserNotifications::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserNotifications::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(UserNotifications::UserId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UserNotifications::Title).string().not_null())
                    .col(
                        ColumnDef::new(UserNotifications::Message)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UserNotifications::EventType).string().null())
                    .col(
                        ColumnDef::new(UserNotifications::Severity)
                            .string()
                            .not_null()
                            .default("info"),
                    )
                    .col(
                        ColumnDef::new(UserNotifications::Read)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(UserNotifications::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(UserNotifications::Table, UserNotifications::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_notifications_user")
                    .table(UserNotifications::Table)
                    .col(UserNotifications::UserId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_notifications_read")
                    .table(UserNotifications::Table)
                    .col(UserNotifications::UserId)
                    .col(UserNotifications::Read)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_notifications_created")
                    .table(UserNotifications::Table)
                    .col(UserNotifications::CreatedAt)
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
                    .table(UserNotifications::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "user_notifications"]
enum UserNotifications {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    Title,
    Message,
    #[iden = "event_type"]
    EventType,
    Severity,
    Read,
    #[iden = "created_at"]
    CreatedAt,
}
