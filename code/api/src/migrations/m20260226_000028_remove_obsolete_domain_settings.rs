//! Migration: Remove obsolete domain settings rows from the settings-based prototype

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DELETE FROM system_settings
                WHERE key IN (
                    'domain_configs',
                    'app_domain_assignments',
                    'domain_primary',
                    'domain_routing_mode',
                    'ddns_enabled',
                    'ddns_provider',
                    'ddns_hostname',
                    'ddns_username',
                    'ddns_token',
                    'certificates_enabled',
                    'certificates_email',
                    'certificates_staging'
                )
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
