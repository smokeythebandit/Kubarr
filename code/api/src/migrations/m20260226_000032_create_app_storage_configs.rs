//! Desired, validated per-app storage choices. No PVCs are created by this migration.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AppStorageConfigs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppStorageConfigs::AppName)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AppStorageConfigs::SchemaVersion)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppStorageConfigs::ChartVersion)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppStorageConfigs::ConfigJson)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppStorageConfigs::FreshPrivateConfig)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(AppStorageConfigs::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppStorageConfigs::UpdatedAt)
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
                    .table(AppStorageConfigs::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "app_storage_configs"]
enum AppStorageConfigs {
    Table,
    #[iden = "app_name"]
    AppName,
    #[iden = "schema_version"]
    SchemaVersion,
    #[iden = "chart_version"]
    ChartVersion,
    #[iden = "config_json"]
    ConfigJson,
    #[iden = "fresh_private_config"]
    FreshPrivateConfig,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
