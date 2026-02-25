//! Migration: Create oauth_accounts table

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
                    .table(OauthAccounts::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(OauthAccounts::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(OauthAccounts::UserId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(OauthAccounts::Provider).string().not_null())
                    .col(
                        ColumnDef::new(OauthAccounts::ProviderUserId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(OauthAccounts::Email).string().null())
                    .col(ColumnDef::new(OauthAccounts::DisplayName).string().null())
                    .col(ColumnDef::new(OauthAccounts::AccessToken).string().null())
                    .col(ColumnDef::new(OauthAccounts::RefreshToken).string().null())
                    .col(
                        ColumnDef::new(OauthAccounts::TokenExpiresAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(OauthAccounts::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAccounts::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(OauthAccounts::Table, OauthAccounts::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_oauth_accounts_user")
                    .table(OauthAccounts::Table)
                    .col(OauthAccounts::UserId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_oauth_accounts_provider")
                    .table(OauthAccounts::Table)
                    .col(OauthAccounts::Provider)
                    .col(OauthAccounts::ProviderUserId)
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
                    .table(OauthAccounts::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "oauth_accounts"]
enum OauthAccounts {
    Table,
    Id,
    #[iden = "user_id"]
    UserId,
    Provider,
    #[iden = "provider_user_id"]
    ProviderUserId,
    Email,
    #[iden = "display_name"]
    DisplayName,
    #[iden = "access_token"]
    AccessToken,
    #[iden = "refresh_token"]
    RefreshToken,
    #[iden = "token_expires_at"]
    TokenExpiresAt,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
