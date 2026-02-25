//! Migration: Create vpn_providers table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(VpnProviders::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(VpnProviders::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(VpnProviders::Name).string().not_null())
                    .col(
                        ColumnDef::new(VpnProviders::VpnType)
                            .string_len(16)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(VpnProviders::ServiceProvider)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(VpnProviders::CredentialsJson)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(VpnProviders::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(VpnProviders::KillSwitch)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(VpnProviders::FirewallOutboundSubnets)
                            .string()
                            .not_null()
                            .default("10.0.0.0/8,172.16.0.0/12,192.168.0.0/16"),
                    )
                    .col(
                        ColumnDef::new(VpnProviders::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(VpnProviders::UpdatedAt)
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
                    .table(VpnProviders::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "vpn_providers"]
enum VpnProviders {
    Table,
    Id,
    Name,
    #[iden = "vpn_type"]
    VpnType,
    #[iden = "service_provider"]
    ServiceProvider,
    #[iden = "credentials_json"]
    CredentialsJson,
    Enabled,
    #[iden = "kill_switch"]
    KillSwitch,
    #[iden = "firewall_outbound_subnets"]
    FirewallOutboundSubnets,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
