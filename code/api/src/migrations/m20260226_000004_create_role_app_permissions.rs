//! Migration: Create role_app_permissions table

use sea_orm_migration::prelude::*;

use super::m20260226_000002_create_roles::Roles;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(RoleAppPermissions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RoleAppPermissions::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(RoleAppPermissions::RoleId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RoleAppPermissions::AppName)
                            .string()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(RoleAppPermissions::Table, RoleAppPermissions::RoleId)
                            .to(Roles::Table, Roles::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_role_app_permissions_role")
                    .table(RoleAppPermissions::Table)
                    .col(RoleAppPermissions::RoleId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_role_app_permissions_unique")
                    .table(RoleAppPermissions::Table)
                    .col(RoleAppPermissions::RoleId)
                    .col(RoleAppPermissions::AppName)
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
                    .table(RoleAppPermissions::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "role_app_permissions"]
enum RoleAppPermissions {
    Table,
    Id,
    #[iden = "role_id"]
    RoleId,
    #[iden = "app_name"]
    AppName,
}
