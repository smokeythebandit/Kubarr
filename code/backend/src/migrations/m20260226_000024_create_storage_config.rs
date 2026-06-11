//! Migration: Create storage_config table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(StorageConfig::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(StorageConfig::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(StorageConfig::Mode).string().not_null())
                    .col(
                        ColumnDef::new(StorageConfig::MountPath)
                            .string()
                            .not_null()
                            .default("/data"),
                    )
                    .col(
                        ColumnDef::new(StorageConfig::Uid)
                            .integer()
                            .not_null()
                            .default(1000),
                    )
                    .col(
                        ColumnDef::new(StorageConfig::Gid)
                            .integer()
                            .not_null()
                            .default(1000),
                    )
                    .col(
                        ColumnDef::new(StorageConfig::FsGroup)
                            .integer()
                            .not_null()
                            .default(1000),
                    )
                    .col(ColumnDef::new(StorageConfig::ConfigJson).text().not_null())
                    .col(ColumnDef::new(StorageConfig::ValidationJson).text().null())
                    .col(
                        ColumnDef::new(StorageConfig::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(StorageConfig::UpdatedAt)
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
                    .table(StorageConfig::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "storage_config"]
enum StorageConfig {
    Table,
    Id,
    Mode,
    #[iden = "mount_path"]
    MountPath,
    Uid,
    Gid,
    #[iden = "fs_group"]
    FsGroup,
    #[iden = "config_json"]
    ConfigJson,
    #[iden = "validation_json"]
    ValidationJson,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
