//! Migration: Create app_operations table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AppOperations::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppOperations::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AppOperations::AppName).string().not_null())
                    .col(ColumnDef::new(AppOperations::Operation).string().not_null())
                    .col(ColumnDef::new(AppOperations::Status).string().not_null())
                    .col(ColumnDef::new(AppOperations::Message).text().null())
                    .col(ColumnDef::new(AppOperations::Error).text().null())
                    .col(ColumnDef::new(AppOperations::CustomConfig).text().null())
                    .col(
                        ColumnDef::new(AppOperations::Attempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AppOperations::CreatedBy)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppOperations::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppOperations::StartedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppOperations::FinishedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppOperations::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_app_operations_status_created")
                    .table(AppOperations::Table)
                    .col(AppOperations::Status)
                    .col(AppOperations::CreatedAt)
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(AppOperations::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "app_operations"]
enum AppOperations {
    Table,
    Id,
    #[iden = "app_name"]
    AppName,
    Operation,
    Status,
    Message,
    Error,
    #[iden = "custom_config"]
    CustomConfig,
    Attempts,
    #[iden = "created_by"]
    CreatedBy,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "started_at"]
    StartedAt,
    #[iden = "finished_at"]
    FinishedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
