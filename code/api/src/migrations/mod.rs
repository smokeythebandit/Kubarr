pub use sea_orm_migration::prelude::*;

mod m20260226_000001_create_users;
mod m20260226_000002_create_roles;
mod m20260226_000003_create_user_roles;
mod m20260226_000004_create_role_app_permissions;
mod m20260226_000005_create_role_permissions;
mod m20260226_000006_create_oauth_accounts;
mod m20260226_000007_create_oauth_providers;
mod m20260226_000008_create_system_settings;
mod m20260226_000009_create_user_preferences;
mod m20260226_000010_create_invites;
mod m20260226_000011_create_audit_logs;
mod m20260226_000012_create_notification_channels;
mod m20260226_000013_create_notification_events;
mod m20260226_000014_create_user_notification_prefs;
mod m20260226_000015_create_notification_logs;
mod m20260226_000016_create_user_notifications;
mod m20260226_000017_create_sessions;
mod m20260226_000018_seed_defaults;
mod m20260226_000020_create_vpn_providers;
mod m20260226_000021_create_app_vpn_configs;
mod m20260226_000022_create_two_factor_recovery_codes;
mod m20260226_000024_create_storage_config;
mod m20260226_000025_create_app_operations;
mod m20260226_000026_create_app_states;
mod m20260226_000027_create_domain_management;
mod m20260226_000028_remove_obsolete_domain_settings;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260226_000001_create_users::Migration),
            Box::new(m20260226_000002_create_roles::Migration),
            Box::new(m20260226_000003_create_user_roles::Migration),
            Box::new(m20260226_000004_create_role_app_permissions::Migration),
            Box::new(m20260226_000005_create_role_permissions::Migration),
            Box::new(m20260226_000006_create_oauth_accounts::Migration),
            Box::new(m20260226_000007_create_oauth_providers::Migration),
            Box::new(m20260226_000008_create_system_settings::Migration),
            Box::new(m20260226_000009_create_user_preferences::Migration),
            Box::new(m20260226_000010_create_invites::Migration),
            Box::new(m20260226_000011_create_audit_logs::Migration),
            Box::new(m20260226_000012_create_notification_channels::Migration),
            Box::new(m20260226_000013_create_notification_events::Migration),
            Box::new(m20260226_000014_create_user_notification_prefs::Migration),
            Box::new(m20260226_000015_create_notification_logs::Migration),
            Box::new(m20260226_000016_create_user_notifications::Migration),
            Box::new(m20260226_000017_create_sessions::Migration),
            Box::new(m20260226_000018_seed_defaults::Migration),
            Box::new(m20260226_000020_create_vpn_providers::Migration),
            Box::new(m20260226_000021_create_app_vpn_configs::Migration),
            Box::new(m20260226_000022_create_two_factor_recovery_codes::Migration),
            Box::new(m20260226_000024_create_storage_config::Migration),
            Box::new(m20260226_000025_create_app_operations::Migration),
            Box::new(m20260226_000026_create_app_states::Migration),
            Box::new(m20260226_000027_create_domain_management::Migration),
            Box::new(m20260226_000028_remove_obsolete_domain_settings::Migration),
        ]
    }
}
