//! Migration: Create two_factor_recovery_codes table

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
                    .table(TwoFactorRecoveryCodes::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TwoFactorRecoveryCodes::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(TwoFactorRecoveryCodes::UserId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TwoFactorRecoveryCodes::CodeHash)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TwoFactorRecoveryCodes::UsedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TwoFactorRecoveryCodes::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(
                                TwoFactorRecoveryCodes::Table,
                                TwoFactorRecoveryCodes::UserId,
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
                    .name("idx_2fa_recovery_user_id")
                    .table(TwoFactorRecoveryCodes::Table)
                    .col(TwoFactorRecoveryCodes::UserId)
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
                    .table(TwoFactorRecoveryCodes::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "two_factor_recovery_codes"]
pub enum TwoFactorRecoveryCodes {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    #[iden = "code_hash"]
    CodeHash,
    #[iden = "used_at"]
    UsedAt,
    #[iden = "created_at"]
    CreatedAt,
}
