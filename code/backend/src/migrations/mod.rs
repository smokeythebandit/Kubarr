pub use sea_orm_migration::prelude::*;

mod m20260127_000001_create_users;
mod m20260127_000002_create_roles;
mod m20260127_000003_create_user_roles;
mod m20260127_000004_create_role_app_permissions;
mod m20260127_000005_create_role_permissions;
mod m20260127_000006_create_oauth_accounts;
mod m20260127_000007_create_oauth_providers;
mod m20260127_000009_create_system_settings;
mod m20260127_000010_create_user_preferences;
mod m20260127_000011_create_invites;
mod m20260127_000012_create_audit_logs;
mod m20260127_000013_create_notification_channels;
mod m20260127_000014_create_notification_events;
mod m20260127_000015_create_user_notification_prefs;
mod m20260127_000016_create_notification_logs;
mod m20260127_000017_create_user_notifications;
mod m20260128_000001_create_sessions;
mod m20260128_000002_seed_defaults;
mod m20260129_000001_create_bootstrap_status;
mod m20260130_000001_create_vpn_providers;
mod m20260130_000002_create_app_vpn_configs;
mod m20260131_000001_add_port_forwarding;
mod m20260219_000001_create_two_factor_recovery_codes;
mod m20260221_000001_create_cloudflare_tunnels;
mod m20260221_000002_add_cloudflare_api_fields;
mod m20260225_000001_add_nfs_server_to_server_config;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260127_000001_create_users::Migration),
            Box::new(m20260127_000002_create_roles::Migration),
            Box::new(m20260127_000003_create_user_roles::Migration),
            Box::new(m20260127_000004_create_role_app_permissions::Migration),
            Box::new(m20260127_000005_create_role_permissions::Migration),
            Box::new(m20260127_000006_create_oauth_accounts::Migration),
            Box::new(m20260127_000007_create_oauth_providers::Migration),
            Box::new(m20260127_000009_create_system_settings::Migration),
            Box::new(m20260127_000010_create_user_preferences::Migration),
            Box::new(m20260127_000011_create_invites::Migration),
            Box::new(m20260127_000012_create_audit_logs::Migration),
            Box::new(m20260127_000013_create_notification_channels::Migration),
            Box::new(m20260127_000014_create_notification_events::Migration),
            Box::new(m20260127_000015_create_user_notification_prefs::Migration),
            Box::new(m20260127_000016_create_notification_logs::Migration),
            Box::new(m20260127_000017_create_user_notifications::Migration),
            Box::new(m20260128_000001_create_sessions::Migration),
            Box::new(m20260128_000002_seed_defaults::Migration),
            Box::new(m20260129_000001_create_bootstrap_status::Migration),
            Box::new(m20260130_000001_create_vpn_providers::Migration),
            Box::new(m20260130_000002_create_app_vpn_configs::Migration),
            Box::new(m20260131_000001_add_port_forwarding::Migration),
            Box::new(m20260219_000001_create_two_factor_recovery_codes::Migration),
            Box::new(m20260221_000001_create_cloudflare_tunnels::Migration),
            Box::new(m20260221_000002_add_cloudflare_api_fields::Migration),
            Box::new(m20260225_000001_add_nfs_server_to_server_config::Migration),
        ]
    }
}
