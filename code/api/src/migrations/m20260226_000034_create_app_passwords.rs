//! Migration: Create app_passwords table
//!
//! Native media clients (TV and phone apps) sign in with a username and
//! password and cannot answer a TOTP prompt. A user who has two-factor
//! authentication gives such a client a generated app password instead of
//! their account password. Only a SHA-256 digest is stored: the password is
//! 128 random bits, so a slow hash adds nothing and lookup stays a single
//! indexed query.

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
                    .table(AppPasswords::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppPasswords::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AppPasswords::UserId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AppPasswords::Name).string().not_null())
                    .col(
                        ColumnDef::new(AppPasswords::TokenHash)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(AppPasswords::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppPasswords::LastUsedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(AppPasswords::Table, AppPasswords::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_app_passwords_user_id")
                    .table(AppPasswords::Table)
                    .col(AppPasswords::UserId)
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
                    .table(AppPasswords::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "app_passwords"]
pub enum AppPasswords {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    Name,
    #[iden = "token_hash"]
    TokenHash,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "last_used_at"]
    LastUsedAt,
}
