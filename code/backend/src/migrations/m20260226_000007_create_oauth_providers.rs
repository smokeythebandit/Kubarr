//! Migration: Create oauth_providers table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(OauthProviders::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(OauthProviders::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(OauthProviders::Name).string().not_null())
                    .col(
                        ColumnDef::new(OauthProviders::Enabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(ColumnDef::new(OauthProviders::ClientId).string().null())
                    .col(ColumnDef::new(OauthProviders::ClientSecret).string().null())
                    .col(
                        ColumnDef::new(OauthProviders::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthProviders::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(OauthProviders::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "oauth_providers"]
enum OauthProviders {
    Table,
    Id,
    Name,
    Enabled,
    #[iden = "client_id"]
    ClientId,
    #[iden = "client_secret"]
    ClientSecret,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
