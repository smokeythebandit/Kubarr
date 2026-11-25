//! Per-event × per-channel routing for external notifications.
//!
//! A route says whether an event type is delivered over an external channel
//! (Telegram, email, SMS). A missing route row means "off". The in-app bell
//! never consults routes: it is on whenever the event itself is enabled.

use std::collections::{BTreeMap, HashMap, HashSet};

use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};

use super::{ChannelType, NotificationSeverity};
use crate::error::Result;
use crate::models::notification_route;

/// Channels routed on by default for an event of `severity`, matching the seed
/// in migration 000038.
pub fn default_channels(severity: NotificationSeverity) -> &'static [ChannelType] {
    match severity {
        NotificationSeverity::Critical => &[
            ChannelType::Telegram,
            ChannelType::Email,
            ChannelType::MessageBird,
        ],
        NotificationSeverity::Warning => &[ChannelType::Telegram],
        NotificationSeverity::Info => &[],
    }
}

/// External channels `event_type` is routed to.
pub async fn routed_channels<C: ConnectionTrait>(
    db: &C,
    event_type: &str,
) -> Result<HashSet<ChannelType>> {
    let rows = notification_route::Entity::find()
        .filter(notification_route::Column::EventType.eq(event_type))
        .filter(notification_route::Column::Enabled.eq(true))
        .all(db)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| ChannelType::parse(&row.channel_type))
        .collect())
}

/// Every stored route as `event_type → {channel → enabled}`.
pub async fn all_routes<C: ConnectionTrait>(
    db: &C,
) -> Result<BTreeMap<String, HashMap<ChannelType, bool>>> {
    let rows = notification_route::Entity::find().all(db).await?;
    let mut routes: BTreeMap<String, HashMap<ChannelType, bool>> = BTreeMap::new();
    for row in rows {
        if let Some(channel) = ChannelType::parse(&row.channel_type) {
            routes
                .entry(row.event_type)
                .or_default()
                .insert(channel, row.enabled);
        }
    }
    Ok(routes)
}

/// Insert or update the routes of `event_type` for the given channels.
pub async fn set_routes<C: ConnectionTrait>(
    db: &C,
    event_type: &str,
    channels: &HashMap<ChannelType, bool>,
) -> Result<()> {
    for (channel, enabled) in channels {
        let row = notification_route::ActiveModel {
            event_type: Set(event_type.to_string()),
            channel_type: Set(channel.as_str().to_string()),
            enabled: Set(*enabled),
        };
        notification_route::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([
                    notification_route::Column::EventType,
                    notification_route::Column::ChannelType,
                ])
                .update_column(notification_route::Column::Enabled)
                .to_owned(),
            )
            .exec_without_returning(db)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::Migrator;
    use crate::models::audit_log::AuditAction;
    use crate::models::{
        notification_event, notification_log, user, user_notification, user_notification_pref,
    };
    use crate::services::notification::NotificationService;
    use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, PaginatorTrait};
    use sea_orm_migration::MigratorTrait;

    async fn migrated_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("db");
        Migrator::up(&db, None).await.expect("migrate");
        db
    }

    async fn insert_event(db: &DatabaseConnection, event_type: &str, severity: &str) {
        notification_event::ActiveModel {
            event_type: Set(event_type.to_string()),
            enabled: Set(true),
            severity: Set(severity.to_string()),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert event");
    }

    /// A user with an enabled, verified destination on every external channel.
    async fn user_with_all_channels(db: &DatabaseConnection) -> i64 {
        let now = chrono::Utc::now();
        let u = user::ActiveModel {
            username: Set("routed".to_string()),
            email: Set("routed@example.com".to_string()),
            hashed_password: Set("hashed".to_string()),
            is_active: Set(true),
            is_approved: Set(true),
            totp_secret: Set(None),
            totp_enabled: Set(false),
            totp_verified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert user");
        for (channel, destination) in [
            ("email", "to@example.com"),
            ("telegram", "123456789"),
            ("messagebird", "+15550001234"),
        ] {
            user_notification_pref::ActiveModel {
                user_id: Set(u.id),
                channel_type: Set(channel.to_string()),
                enabled: Set(true),
                destination: Set(Some(destination.to_string())),
                verified: Set(true),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(db)
            .await
            .expect("insert pref");
        }
        u.id
    }

    /// Channels a delivery was attempted on (no providers are configured, so each
    /// attempt is logged as failed — the log row proves the send was routed).
    async fn attempted_channels(db: &DatabaseConnection) -> Vec<String> {
        let mut channels: Vec<String> = notification_log::Entity::find()
            .all(db)
            .await
            .expect("logs")
            .into_iter()
            .map(|log| log.channel_type)
            .collect();
        channels.sort();
        channels
    }

    async fn bell_count(db: &DatabaseConnection, user_id: i64) -> u64 {
        user_notification::Entity::find()
            .filter(user_notification::Column::UserId.eq(user_id))
            .count(db)
            .await
            .expect("count")
    }

    #[test]
    fn default_channels_follow_severity() {
        assert_eq!(default_channels(NotificationSeverity::Critical).len(), 3);
        assert_eq!(
            default_channels(NotificationSeverity::Warning),
            &[ChannelType::Telegram]
        );
        assert!(default_channels(NotificationSeverity::Info).is_empty());
    }

    #[tokio::test]
    async fn migration_seeds_existing_events_by_severity() {
        let db = Database::connect("sqlite::memory:").await.expect("db");
        let before = Migrator::migrations()
            .iter()
            .position(|m| m.name().contains("notification_routes"))
            .expect("routes migration registered") as u32;
        Migrator::up(&db, Some(before)).await.expect("migrate");
        insert_event(&db, "login_failed", "Critical").await;
        insert_event(&db, "user_deleted", "warning").await;
        insert_event(&db, "login", "info").await;
        Migrator::up(&db, None).await.expect("migrate rest");

        let routes = all_routes(&db).await.expect("routes");
        for (event_type, severity) in [
            ("login_failed", NotificationSeverity::Critical),
            ("user_deleted", NotificationSeverity::Warning),
            ("login", NotificationSeverity::Info),
            ("app_update_rollback_failed", NotificationSeverity::Critical),
            ("app_update_failed", NotificationSeverity::Warning),
            ("app_updated", NotificationSeverity::Info),
        ] {
            let expected: HashSet<ChannelType> =
                default_channels(severity).iter().copied().collect();
            let actual: HashSet<ChannelType> = routes
                .get(event_type)
                .map(|m| m.iter().filter(|(_, on)| **on).map(|(c, _)| *c).collect())
                .unwrap_or_default();
            assert_eq!(actual, expected, "{event_type}");
            assert_eq!(
                routed_channels(&db, event_type).await.expect("routed"),
                expected,
                "{event_type}"
            );
        }
    }

    #[tokio::test]
    async fn set_routes_upserts() {
        let db = migrated_db().await;
        set_routes(&db, "login", &HashMap::from([(ChannelType::Email, true)]))
            .await
            .expect("insert");
        set_routes(
            &db,
            "login",
            &HashMap::from([(ChannelType::Email, false), (ChannelType::Telegram, true)]),
        )
        .await
        .expect("update");
        assert_eq!(
            routed_channels(&db, "login").await.expect("routed"),
            HashSet::from([ChannelType::Telegram])
        );
        assert_eq!(all_routes(&db).await.expect("all")["login"].len(), 2);
    }

    #[tokio::test]
    async fn routes_filter_external_sends_but_not_the_bell() {
        let db = migrated_db().await;
        insert_event(&db, "login", "info").await;
        let uid = user_with_all_channels(&db).await;
        let svc = NotificationService::new();
        svc.set_db(db.clone()).await;

        // No route rows: bell only.
        svc.notify_event(&AuditAction::Login, Some(uid), Some("routed"), None)
            .await
            .expect("notify");
        assert!(attempted_channels(&db).await.is_empty());
        assert_eq!(bell_count(&db, uid).await, 1);

        // Telegram on, email explicitly off, SMS absent.
        set_routes(
            &db,
            "login",
            &HashMap::from([(ChannelType::Telegram, true), (ChannelType::Email, false)]),
        )
        .await
        .expect("routes");
        svc.notify_event(&AuditAction::Login, Some(uid), Some("routed"), None)
            .await
            .expect("notify");
        assert_eq!(attempted_channels(&db).await, vec!["telegram".to_string()]);
        assert_eq!(bell_count(&db, uid).await, 2);
    }

    #[tokio::test]
    async fn routes_of_another_event_do_not_leak() {
        let db = migrated_db().await;
        insert_event(&db, "login", "info").await;
        let uid = user_with_all_channels(&db).await;
        set_routes(&db, "logout", &HashMap::from([(ChannelType::Email, true)]))
            .await
            .expect("routes");
        let svc = NotificationService::new();
        svc.set_db(db.clone()).await;
        svc.notify_event(&AuditAction::Login, Some(uid), Some("routed"), None)
            .await
            .expect("notify");
        assert!(attempted_channels(&db).await.is_empty());
        assert_eq!(bell_count(&db, uid).await, 1);
    }
}
