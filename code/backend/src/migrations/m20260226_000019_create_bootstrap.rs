use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(BootstrapStatus::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(BootstrapStatus::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(BootstrapStatus::Component)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(BootstrapStatus::DisplayName)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(BootstrapStatus::Status)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(ColumnDef::new(BootstrapStatus::Message).string())
                    .col(ColumnDef::new(BootstrapStatus::StartedAt).timestamp_with_time_zone())
                    .col(ColumnDef::new(BootstrapStatus::CompletedAt).timestamp_with_time_zone())
                    .col(ColumnDef::new(BootstrapStatus::Error).text())
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ServerConfig::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ServerConfig::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ServerConfig::Name).string().not_null())
                    .col(
                        ColumnDef::new(ServerConfig::StoragePath)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ServerConfig::NfsServer).text().null())
                    .col(
                        ColumnDef::new(ServerConfig::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(BootstrapStatus::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(ServerConfig::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(Iden)]
enum BootstrapStatus {
    Table,
    Id,
    Component,
    DisplayName,
    Status,
    Message,
    StartedAt,
    CompletedAt,
    Error,
}

#[derive(Iden)]
enum ServerConfig {
    Table,
    Id,
    Name,
    StoragePath,
    NfsServer,
    CreatedAt,
}
