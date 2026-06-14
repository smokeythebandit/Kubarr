//! Migration: Create cloudflare_tunnels table

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(CloudflareTunnels::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(CloudflareTunnels::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(CloudflareTunnels::Name).text().not_null())
                    .col(
                        ColumnDef::new(CloudflareTunnels::TunnelToken)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CloudflareTunnels::Status)
                            .string()
                            .not_null()
                            .default("not_deployed"),
                    )
                    .col(ColumnDef::new(CloudflareTunnels::Error).text().null())
                    .col(ColumnDef::new(CloudflareTunnels::ApiToken).text().null())
                    .col(ColumnDef::new(CloudflareTunnels::AccountId).text().null())
                    .col(ColumnDef::new(CloudflareTunnels::TunnelId).text().null())
                    .col(ColumnDef::new(CloudflareTunnels::ZoneId).text().null())
                    .col(ColumnDef::new(CloudflareTunnels::ZoneName).text().null())
                    .col(ColumnDef::new(CloudflareTunnels::Subdomain).text().null())
                    .col(ColumnDef::new(CloudflareTunnels::DnsRecordId).text().null())
                    .col(ColumnDef::new(CloudflareTunnels::Hostname).text().null())
                    .col(
                        ColumnDef::new(CloudflareTunnels::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CloudflareTunnels::UpdatedAt)
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
                    .table(CloudflareTunnels::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "cloudflare_tunnels"]
enum CloudflareTunnels {
    Table,
    Id,
    Name,
    #[iden = "tunnel_token"]
    TunnelToken,
    Status,
    Error,
    #[iden = "api_token"]
    ApiToken,
    #[iden = "account_id"]
    AccountId,
    #[iden = "tunnel_id"]
    TunnelId,
    #[iden = "zone_id"]
    ZoneId,
    #[iden = "zone_name"]
    ZoneName,
    Subdomain,
    #[iden = "dns_record_id"]
    DnsRecordId,
    Hostname,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
