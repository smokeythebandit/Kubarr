use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Seed default roles
        seed_roles(db).await?;

        // Seed OAuth providers
        seed_oauth_providers(db).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Seeding is not reversible - data may have been modified
        Ok(())
    }
}

async fn seed_roles(db: &SchemaManagerConnection<'_>) -> Result<(), DbErr> {
    use crate::models::prelude::*;
    use crate::models::{role, role_app_permission, role_permission};

    let role_count = Role::find().count(db).await?;
    if role_count > 0 {
        return Ok(());
    }

    let now = chrono::Utc::now();

    // Create default roles
    let default_roles = [
        ("admin", "Full administrator access", true),
        ("viewer", "View-only access to media apps", true),
        ("downloader", "Access to download clients", true),
    ];

    for (name, description, is_system) in default_roles {
        let new_role = role::ActiveModel {
            name: Set(name.to_string()),
            description: Set(Some(description.to_string())),
            is_system: Set(is_system),
            created_at: Set(now),
            ..Default::default()
        };
        new_role.insert(db).await?;
    }

    // Get roles
    let admin_role = Role::find()
        .filter(role::Column::Name.eq("admin"))
        .one(db)
        .await?
        .ok_or(DbErr::Custom("Admin role not found".to_string()))?;

    let viewer_role = Role::find()
        .filter(role::Column::Name.eq("viewer"))
        .one(db)
        .await?
        .ok_or(DbErr::Custom("Viewer role not found".to_string()))?;

    let downloader_role = Role::find()
        .filter(role::Column::Name.eq("downloader"))
        .one(db)
        .await?
        .ok_or(DbErr::Custom("Downloader role not found".to_string()))?;

    // Admin permissions (all)
    let admin_permissions = [
        "app.*",
        "apps.view",
        "apps.install",
        "apps.delete",
        "apps.restart",
        "storage.view",
        "storage.write",
        "storage.delete",
        "storage.download",
        "logs.view",
        "monitoring.view",
        "networking.view",
        "networking.manage",
        "users.view",
        "users.manage",
        "users.reset_password",
        "roles.view",
        "roles.manage",
        "settings.view",
        "settings.manage",
        "vpn.view",
        "vpn.manage",
        "audit.view",
        "audit.manage",
    ];
    for perm in admin_permissions {
        let permission = role_permission::ActiveModel {
            role_id: Set(admin_role.id),
            permission: Set(perm.to_string()),
            ..Default::default()
        };
        permission.insert(db).await?;
    }

    // Viewer permissions
    let viewer_permissions = [
        "apps.view",
        "logs.view",
        "monitoring.view",
        "storage.view",
        "storage.download",
    ];
    for perm in viewer_permissions {
        let permission = role_permission::ActiveModel {
            role_id: Set(viewer_role.id),
            permission: Set(perm.to_string()),
            ..Default::default()
        };
        permission.insert(db).await?;
    }

    // Viewer app permissions
    let viewer_apps = ["jellyfin", "jellyseerr"];
    for app in viewer_apps {
        let permission = role_app_permission::ActiveModel {
            role_id: Set(viewer_role.id),
            app_name: Set(app.to_string()),
            ..Default::default()
        };
        permission.insert(db).await?;
    }

    // Downloader permissions
    let downloader_permissions = [
        "apps.view",
        "apps.restart",
        "storage.view",
        "storage.download",
    ];
    for perm in downloader_permissions {
        let permission = role_permission::ActiveModel {
            role_id: Set(downloader_role.id),
            permission: Set(perm.to_string()),
            ..Default::default()
        };
        permission.insert(db).await?;
    }

    // Downloader app permissions
    let downloader_apps = [
        "qbittorrent",
        "transmission",
        "deluge",
        "rutorrent",
        "sabnzbd",
        "jackett",
        "prowlarr",
    ];
    for app in downloader_apps {
        let permission = role_app_permission::ActiveModel {
            role_id: Set(downloader_role.id),
            app_name: Set(app.to_string()),
            ..Default::default()
        };
        permission.insert(db).await?;
    }

    Ok(())
}

async fn seed_oauth_providers(db: &SchemaManagerConnection<'_>) -> Result<(), DbErr> {
    use crate::models::oauth_provider;
    use crate::models::prelude::*;

    let oauth_count = OauthProvider::find().count(db).await?;
    if oauth_count > 0 {
        return Ok(());
    }

    let now = chrono::Utc::now();

    let default_providers = [("google", "Google"), ("microsoft", "Microsoft")];

    for (id, name) in default_providers {
        let provider = oauth_provider::ActiveModel {
            id: Set(id.to_string()),
            name: Set(name.to_string()),
            enabled: Set(false),
            client_id: Set(None),
            client_secret: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        provider.insert(db).await?;
    }

    Ok(())
}
