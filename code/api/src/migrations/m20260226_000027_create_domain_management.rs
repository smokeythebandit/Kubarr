//! Migration: Create domain management tables

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(DynamicDnsProfiles::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(DynamicDnsProfiles::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(DynamicDnsProfiles::Name).string().not_null())
                    .col(
                        ColumnDef::new(DynamicDnsProfiles::Provider)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DynamicDnsProfiles::CapabilitiesJson)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DynamicDnsProfiles::ConfigJson)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DynamicDnsProfiles::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(DynamicDnsProfiles::Status)
                            .string()
                            .not_null()
                            .default("unknown"),
                    )
                    .col(ColumnDef::new(DynamicDnsProfiles::LastError).text().null())
                    .col(
                        ColumnDef::new(DynamicDnsProfiles::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DynamicDnsProfiles::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(LetsEncryptProfiles::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::Name)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::Email)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::Environment)
                            .string()
                            .not_null()
                            .default("staging"),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::ChallengeType)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::DnsProfileId)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::RenewalEnabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::Status)
                            .string()
                            .not_null()
                            .default("unknown"),
                    )
                    .col(ColumnDef::new(LetsEncryptProfiles::LastError).text().null())
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LetsEncryptProfiles::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_letsencrypt_profiles_dns_profile")
                            .from(
                                LetsEncryptProfiles::Table,
                                LetsEncryptProfiles::DnsProfileId,
                            )
                            .to(DynamicDnsProfiles::Table, DynamicDnsProfiles::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Domains::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Domains::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Domains::Domain)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Domains::Kind).string().not_null())
                    .col(
                        ColumnDef::new(Domains::Scope)
                            .string()
                            .not_null()
                            .default("public"),
                    )
                    .col(
                        ColumnDef::new(Domains::Primary)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Domains::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(Domains::DnsMode)
                            .string()
                            .not_null()
                            .default("manual"),
                    )
                    .col(ColumnDef::new(Domains::DdnsProfileId).big_integer().null())
                    .col(
                        ColumnDef::new(Domains::DnsStatus)
                            .string()
                            .not_null()
                            .default("unknown"),
                    )
                    .col(
                        ColumnDef::new(Domains::TlsMode)
                            .string()
                            .not_null()
                            .default("none"),
                    )
                    .col(
                        ColumnDef::new(Domains::LetsEncryptProfileId)
                            .big_integer()
                            .null(),
                    )
                    .col(ColumnDef::new(Domains::TlsSecretName).string().null())
                    .col(
                        ColumnDef::new(Domains::CertificateStatus)
                            .string()
                            .not_null()
                            .default("unknown"),
                    )
                    .col(
                        ColumnDef::new(Domains::CertificateExpiresAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Domains::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Domains::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_domains_ddns_profile")
                            .from(Domains::Table, Domains::DdnsProfileId)
                            .to(DynamicDnsProfiles::Table, DynamicDnsProfiles::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_domains_letsencrypt_profile")
                            .from(Domains::Table, Domains::LetsEncryptProfileId)
                            .to(LetsEncryptProfiles::Table, LetsEncryptProfiles::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AppDomainAssignments::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppDomainAssignments::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::AppName)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::DomainId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::RouteMode)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::Hostname)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::PathPrefix)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::Primary)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AppDomainAssignments::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_app_domain_assignments_domain")
                            .from(AppDomainAssignments::Table, AppDomainAssignments::DomainId)
                            .to(Domains::Table, Domains::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_app_domain_assignments_app")
                    .table(AppDomainAssignments::Table)
                    .col(AppDomainAssignments::AppName)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(AppDomainAssignments::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Domains::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(LetsEncryptProfiles::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(DynamicDnsProfiles::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(Iden)]
#[iden = "dynamic_dns_profiles"]
enum DynamicDnsProfiles {
    Table,
    Id,
    Name,
    Provider,
    #[iden = "capabilities_json"]
    CapabilitiesJson,
    #[iden = "config_json"]
    ConfigJson,
    Enabled,
    Status,
    #[iden = "last_error"]
    LastError,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}

#[derive(Iden)]
#[iden = "letsencrypt_profiles"]
enum LetsEncryptProfiles {
    Table,
    Id,
    Name,
    Email,
    Environment,
    #[iden = "challenge_type"]
    ChallengeType,
    #[iden = "dns_profile_id"]
    DnsProfileId,
    #[iden = "renewal_enabled"]
    RenewalEnabled,
    Enabled,
    Status,
    #[iden = "last_error"]
    LastError,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}

#[derive(Iden)]
#[iden = "domains"]
enum Domains {
    Table,
    Id,
    Domain,
    Kind,
    Scope,
    Primary,
    Enabled,
    #[iden = "dns_mode"]
    DnsMode,
    #[iden = "ddns_profile_id"]
    DdnsProfileId,
    #[iden = "dns_status"]
    DnsStatus,
    #[iden = "tls_mode"]
    TlsMode,
    #[iden = "letsencrypt_profile_id"]
    LetsEncryptProfileId,
    #[iden = "tls_secret_name"]
    TlsSecretName,
    #[iden = "certificate_status"]
    CertificateStatus,
    #[iden = "certificate_expires_at"]
    CertificateExpiresAt,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}

#[derive(Iden)]
#[iden = "app_domain_assignments"]
enum AppDomainAssignments {
    Table,
    Id,
    #[iden = "app_name"]
    AppName,
    #[iden = "domain_id"]
    DomainId,
    #[iden = "route_mode"]
    RouteMode,
    Hostname,
    #[iden = "path_prefix"]
    PathPrefix,
    Primary,
    Enabled,
    #[iden = "created_at"]
    CreatedAt,
    #[iden = "updated_at"]
    UpdatedAt,
}
