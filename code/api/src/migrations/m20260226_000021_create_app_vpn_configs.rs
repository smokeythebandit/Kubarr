//! Migration: Create app_vpn_configs table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AppVpnConfigs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppVpnConfigs::AppName)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AppVpnConfigs::VpnProviderId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppVpnConfigs::KillSwitchOverride)
                            .boolean()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppVpnConfigs::PortForwarding)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(AppVpnConfigs::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppVpnConfigs::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_app_vpn_configs_vpn_provider")
                            .from(AppVpnConfigs::Table, AppVpnConfigs::VpnProviderId)
                            .to(VpnProviders::Table, VpnProviders::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(AppVpnConfigs::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "app_vpn_configs"]
enum AppVpnConfigs {
    Table,
    #[iden = "app_name"]
    AppName,
    #[iden = "vpn_provider_id"]
    VpnProviderId,
    #[iden = "kill_switch_override"]
    KillSwitchOverride,
    #[iden = "port_forwarding"]
    PortForwarding,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}

#[derive(Iden)]
#[iden = "vpn_providers"]
enum VpnProviders {
    Table,
    Id,
}
