//! Owner-facing settings for automatic app updates, stored in `system_settings`
//! under the `auto_update.` prefix. Absent keys fall back to the defaults.

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use chrono_tz::Tz;
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, EntityTrait, Set, TransactionSession, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::models::system_setting;
use crate::services::audit::log_on_transaction;
use crate::services::update_policy::{
    parse_window_start, resolve_local, window_bounds, UpdateWindow,
};

const KEY_ENABLED: &str = "auto_update.enabled";
const KEY_WINDOW_START: &str = "auto_update.window_start";
const KEY_WINDOW_MINUTES: &str = "auto_update.window_minutes";
const KEY_TIMEZONE: &str = "auto_update.timezone";
const KEY_HEALTH_TIMEOUT: &str = "auto_update.health_timeout_minutes";

const WINDOW_MINUTES_RANGE: std::ops::RangeInclusive<u32> = 30..=720;
/// At least 5 minutes: verification needs time to become healthy plus the
/// worker's 3-minute settle period, or every update would fail.
const HEALTH_TIMEOUT_RANGE: std::ops::RangeInclusive<u32> = 5..=60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AutoUpdateSettings {
    pub enabled: bool,
    /// Local window start, `HH:MM`.
    pub window_start: String,
    pub window_minutes: u32,
    /// IANA timezone name the window is evaluated in.
    pub timezone: String,
    pub health_timeout_minutes: u32,
}

impl Default for AutoUpdateSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            window_start: "04:00".into(),
            window_minutes: 120,
            timezone: "UTC".into(),
            health_timeout_minutes: 10,
        }
    }
}

impl AutoUpdateSettings {
    /// Check every field, naming the offending one in the error.
    pub fn validate(&self) -> Result<()> {
        parse_window_start(&self.window_start)
            .map_err(|e| AppError::BadRequest(format!("window_start: {e}")))?;
        if !WINDOW_MINUTES_RANGE.contains(&self.window_minutes) {
            return Err(AppError::BadRequest(format!(
                "window_minutes: must be between {} and {}",
                WINDOW_MINUTES_RANGE.start(),
                WINDOW_MINUTES_RANGE.end()
            )));
        }
        self.timezone.parse::<Tz>().map_err(|_| {
            AppError::BadRequest(format!(
                "timezone: '{}' is not an IANA timezone name",
                self.timezone
            ))
        })?;
        if !HEALTH_TIMEOUT_RANGE.contains(&self.health_timeout_minutes) {
            return Err(AppError::BadRequest(format!(
                "health_timeout_minutes: must be between {} and {}",
                HEALTH_TIMEOUT_RANGE.start(),
                HEALTH_TIMEOUT_RANGE.end()
            )));
        }
        Ok(())
    }

    pub fn health_timeout(&self) -> Duration {
        Duration::from_secs(u64::from(self.health_timeout_minutes) * 60)
    }

    /// The window as a policy value; `None` when the stored values are unusable.
    pub fn window(&self) -> Option<UpdateWindow> {
        Some(UpdateWindow {
            start: parse_window_start(&self.window_start).ok()?,
            minutes: self.window_minutes,
            tz: self.timezone.parse().ok()?,
        })
    }

    /// Start of the window that is running at `now`, otherwise of the next one.
    pub fn next_window_start(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let window = self.window()?;
        if let Some((start, _)) = window_bounds(&window, now) {
            return Some(start);
        }
        let today = now.with_timezone(&window.tz).date_naive();
        (0..=2)
            .filter_map(|days| {
                resolve_local(window.tz, today + ChronoDuration::days(days), window.start)
            })
            .find(|start| *start >= now)
    }
}

fn parse<T: std::str::FromStr>(key: &str, value: &str) -> Result<T> {
    value
        .parse()
        .map_err(|_| AppError::Internal(format!("Stored setting '{key}' is invalid: '{value}'")))
}

/// Load the settings; keys that were never saved use their defaults.
pub async fn load<C: ConnectionTrait>(db: &C) -> Result<AutoUpdateSettings> {
    let mut settings = AutoUpdateSettings::default();
    for row in system_setting::Entity::find().all(db).await? {
        match row.key.as_str() {
            KEY_ENABLED => settings.enabled = parse(KEY_ENABLED, &row.value)?,
            KEY_WINDOW_START => settings.window_start = row.value,
            KEY_WINDOW_MINUTES => settings.window_minutes = parse(KEY_WINDOW_MINUTES, &row.value)?,
            KEY_TIMEZONE => settings.timezone = row.value,
            KEY_HEALTH_TIMEOUT => {
                settings.health_timeout_minutes = parse(KEY_HEALTH_TIMEOUT, &row.value)?
            }
            _ => {}
        }
    }
    Ok(settings)
}

/// Whether an owner has ever saved a timezone (the stored default is only a
/// placeholder until then).
pub async fn timezone_saved<C: ConnectionTrait>(db: &C) -> Result<bool> {
    Ok(system_setting::Entity::find_by_id(KEY_TIMEZONE)
        .one(db)
        .await?
        .is_some())
}

/// Validate and persist all settings in one transaction, recording who changed them.
pub async fn save(
    db: &impl TransactionTrait,
    settings: &AutoUpdateSettings,
    actor: Option<(i64, String)>,
) -> Result<()> {
    settings.validate()?;
    let now = Utc::now();
    let txn = db.begin().await?;
    let rows = [
        (KEY_ENABLED, settings.enabled.to_string()),
        (KEY_WINDOW_START, settings.window_start.clone()),
        (KEY_WINDOW_MINUTES, settings.window_minutes.to_string()),
        (KEY_TIMEZONE, settings.timezone.clone()),
        (
            KEY_HEALTH_TIMEOUT,
            settings.health_timeout_minutes.to_string(),
        ),
    ];
    for (key, value) in rows {
        match system_setting::Entity::find_by_id(key).one(&txn).await? {
            Some(existing) => {
                let mut row: system_setting::ActiveModel = existing.into();
                row.value = Set(value);
                row.updated_at = Set(now);
                row.update(&txn).await?;
            }
            None => {
                system_setting::ActiveModel {
                    key: Set(key.into()),
                    value: Set(value),
                    description: Set(Some("Automatic app updates".into())),
                    updated_at: Set(now),
                }
                .insert(&txn)
                .await?;
            }
        }
    }
    let (user_id, username) = actor.unzip();
    log_on_transaction(
        &txn,
        AuditAction::SystemSettingChanged,
        ResourceType::System,
        Some("auto_update".into()),
        user_id,
        username,
        None,
        None,
        None,
        true,
        None,
    )
    .await?;
    txn.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    async fn db() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        crate::migrations::Migrator::up(&db, None).await.unwrap();
        db
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[tokio::test]
    async fn test_settings_defaults() {
        let settings = load(&db().await).await.unwrap();
        assert_eq!(settings, AutoUpdateSettings::default());
        assert!(settings.enabled);
        assert_eq!(settings.window_start, "04:00");
        assert_eq!(settings.window_minutes, 120);
        assert_eq!(settings.timezone, "UTC");
        assert_eq!(settings.health_timeout_minutes, 10);
    }

    #[tokio::test]
    async fn test_timezone_saved_false_until_saved() {
        let db = db().await;
        assert!(!timezone_saved(&db).await.unwrap());
        save(&db, &AutoUpdateSettings::default(), None)
            .await
            .unwrap();
        assert!(timezone_saved(&db).await.unwrap());
    }

    #[tokio::test]
    async fn test_settings_save_round_trips_and_audits() {
        let db = db().await;
        let changed = AutoUpdateSettings {
            enabled: false,
            window_start: "23:30".into(),
            window_minutes: 90,
            timezone: "Europe/Amsterdam".into(),
            health_timeout_minutes: 15,
        };
        save(&db, &changed, Some((1, "owner".into())))
            .await
            .unwrap();
        assert_eq!(load(&db).await.unwrap(), changed);
        let audits = crate::models::audit_log::Entity::find()
            .all(&db)
            .await
            .unwrap();
        assert!(audits.iter().any(
            |a| a.action == AuditAction::SystemSettingChanged.to_string()
                && a.resource_id.as_deref() == Some("auto_update")
        ));
    }

    #[tokio::test]
    async fn test_settings_rejects_bad_window_and_tz() {
        let db = db().await;
        let base = AutoUpdateSettings::default();
        let cases = [
            (
                AutoUpdateSettings {
                    window_start: "25:00".into(),
                    ..base.clone()
                },
                "window_start",
            ),
            (
                AutoUpdateSettings {
                    window_start: "4am".into(),
                    ..base.clone()
                },
                "window_start",
            ),
            (
                AutoUpdateSettings {
                    window_minutes: 29,
                    ..base.clone()
                },
                "window_minutes",
            ),
            (
                AutoUpdateSettings {
                    window_minutes: 721,
                    ..base.clone()
                },
                "window_minutes",
            ),
            (
                AutoUpdateSettings {
                    timezone: "Mars/Olympus".into(),
                    ..base.clone()
                },
                "timezone",
            ),
            (
                AutoUpdateSettings {
                    health_timeout_minutes: 4,
                    ..base.clone()
                },
                "health_timeout_minutes",
            ),
            (
                AutoUpdateSettings {
                    health_timeout_minutes: 61,
                    ..base.clone()
                },
                "health_timeout_minutes",
            ),
        ];
        for (bad, field) in cases {
            match save(&db, &bad, None).await {
                Err(AppError::BadRequest(message)) => {
                    assert!(message.starts_with(field), "{message}")
                }
                other => panic!("expected BadRequest for {field}, got {other:?}"),
            }
        }
        // Nothing was persisted by the rejected saves.
        assert_eq!(load(&db).await.unwrap(), base);
    }

    #[test]
    fn test_next_window_start() {
        let s = AutoUpdateSettings::default(); // 04:00 UTC, 120 min
        assert_eq!(
            s.next_window_start(utc("2026-11-25T10:00:00Z")),
            Some(utc("2026-11-26T04:00:00Z"))
        );
        assert_eq!(
            s.next_window_start(utc("2026-11-25T02:00:00Z")),
            Some(utc("2026-11-25T04:00:00Z"))
        );
        // Inside a window: its own start.
        assert_eq!(
            s.next_window_start(utc("2026-11-25T05:00:00Z")),
            Some(utc("2026-11-25T04:00:00Z"))
        );
    }

    #[test]
    fn test_health_timeout_duration() {
        assert_eq!(
            AutoUpdateSettings::default().health_timeout(),
            Duration::from_secs(600)
        );
    }
}
