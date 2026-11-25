//! Owner-facing settings for automatic app updates, stored in `system_settings`
//! under the `auto_update.` prefix. Absent keys fall back to the defaults.

use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set,
    TransactionSession, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::models::audit_log::{AuditAction, ResourceType};
use crate::models::system_setting;
use crate::services::audit::log_on_transaction;
use crate::services::update_policy::{Schedule, Tz};

const KEY_ENABLED: &str = "auto_update.enabled";
const KEY_SCHEDULE: &str = "auto_update.schedule";
/// Legacy daily window, superseded by [`KEY_SCHEDULE`]; read only when no
/// schedule was saved and deleted on the next save.
const KEY_WINDOW_START: &str = "auto_update.window_start";
const KEY_WINDOW_MINUTES: &str = "auto_update.window_minutes";
const LEGACY_WINDOW_START: &str = "04:00";
const LEGACY_WINDOW_MINUTES: u32 = 120;
const KEY_TIMEZONE: &str = "auto_update.timezone";
const KEY_HEALTH_TIMEOUT: &str = "auto_update.health_timeout_minutes";

/// At least 5 minutes: verification needs time to become healthy plus the
/// worker's 3-minute settle period, or every update would fail.
const HEALTH_TIMEOUT_RANGE: std::ops::RangeInclusive<u32> = 5..=60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AutoUpdateSettings {
    pub enabled: bool,
    /// Weekly schedule: exactly 7 days (Monday first) of 24 local hours each;
    /// an automatic update may start while the current local hour is true.
    pub schedule: Vec<Vec<bool>>,
    /// IANA timezone name the schedule is evaluated in.
    pub timezone: String,
    pub health_timeout_minutes: u32,
}

impl Default for AutoUpdateSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            schedule: default_schedule().to_rows(),
            timezone: "UTC".into(),
            health_timeout_minutes: 10,
        }
    }
}

impl AutoUpdateSettings {
    /// Check every field, naming the offending one in the error.
    pub fn validate(&self) -> Result<()> {
        Schedule::from_rows(&self.schedule)
            .map_err(|e| AppError::BadRequest(format!("schedule: {e}")))?;
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

    /// The schedule and timezone as policy values; `None` when unusable.
    pub fn policy(&self) -> Option<(Schedule, Tz)> {
        Some((
            Schedule::from_rows(&self.schedule).ok()?,
            self.timezone.parse().ok()?,
        ))
    }

    /// Whether an automatic update may start at `now`.
    pub fn is_open(&self, now: DateTime<Utc>) -> bool {
        self.policy()
            .is_some_and(|(schedule, tz)| schedule.is_open(&tz, now))
    }

    /// Start of the open run containing `now`, otherwise of the next one.
    pub fn next_window_start(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let (schedule, tz) = self.policy()?;
        schedule.next_open(&tz, now)
    }
}

fn default_schedule() -> Schedule {
    // The legacy defaults always convert; an empty grid is only a fallback.
    Schedule::from_legacy_window(LEGACY_WINDOW_START, LEGACY_WINDOW_MINUTES)
        .unwrap_or(Schedule([[false; 24]; 7]))
}

fn parse<T: std::str::FromStr>(key: &str, value: &str) -> Result<T> {
    value
        .parse()
        .map_err(|_| AppError::Internal(format!("Stored setting '{key}' is invalid: '{value}'")))
}

/// Load the settings; keys that were never saved use their defaults.
pub async fn load<C: ConnectionTrait>(db: &C) -> Result<AutoUpdateSettings> {
    let mut settings = AutoUpdateSettings::default();
    let mut schedule = None;
    let mut window_start = LEGACY_WINDOW_START.to_string();
    let mut window_minutes = LEGACY_WINDOW_MINUTES;
    for row in system_setting::Entity::find().all(db).await? {
        match row.key.as_str() {
            KEY_ENABLED => settings.enabled = parse(KEY_ENABLED, &row.value)?,
            KEY_SCHEDULE => {
                schedule = Some(Schedule::parse(&row.value).map_err(|e| {
                    AppError::Internal(format!("Stored setting '{KEY_SCHEDULE}' is invalid: {e}"))
                })?)
            }
            KEY_WINDOW_START => window_start = row.value,
            KEY_WINDOW_MINUTES => window_minutes = parse(KEY_WINDOW_MINUTES, &row.value)?,
            KEY_TIMEZONE => settings.timezone = row.value,
            KEY_HEALTH_TIMEOUT => {
                settings.health_timeout_minutes = parse(KEY_HEALTH_TIMEOUT, &row.value)?
            }
            _ => {}
        }
    }
    let schedule = match schedule {
        Some(schedule) => schedule,
        None => Schedule::from_legacy_window(&window_start, window_minutes).map_err(|e| {
            AppError::Internal(format!(
                "Stored setting '{KEY_WINDOW_START}' is invalid: {e}"
            ))
        })?,
    };
    settings.schedule = schedule.to_rows();
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
    let schedule = Schedule::from_rows(&settings.schedule)
        .map_err(|e| AppError::BadRequest(format!("schedule: {e}")))?;
    let now = Utc::now();
    let txn = db.begin().await?;
    let rows = [
        (KEY_ENABLED, settings.enabled.to_string()),
        (KEY_SCHEDULE, schedule.format()),
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
    system_setting::Entity::delete_many()
        .filter(system_setting::Column::Key.is_in([KEY_WINDOW_START, KEY_WINDOW_MINUTES]))
        .exec(&txn)
        .await?;
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
        for day in &settings.schedule {
            let open: Vec<usize> = (0..24).filter(|&h| day[h]).collect();
            assert_eq!(open, [4, 5]);
        }
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
            schedule: {
                let mut rows = vec![vec![false; 24]; 7];
                rows[0][23] = true;
                rows[6][2] = true;
                rows
            },
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
                    schedule: vec![vec![true; 24]; 6],
                    ..base.clone()
                },
                "schedule",
            ),
            (
                AutoUpdateSettings {
                    schedule: vec![vec![true; 25]; 7],
                    ..base.clone()
                },
                "schedule",
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

    async fn put_raw(db: &sea_orm::DatabaseConnection, key: &str, value: &str) {
        system_setting::ActiveModel {
            key: Set(key.into()),
            value: Set(value.into()),
            description: Set(None),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_legacy_window_converts_and_save_drops_it() {
        let db = db().await;
        put_raw(&db, KEY_WINDOW_START, "23:00").await;
        put_raw(&db, KEY_WINDOW_MINUTES, "180").await;
        let settings = load(&db).await.unwrap();
        let expected = Schedule::from_legacy_window("23:00", 180).unwrap();
        assert_eq!(settings.schedule, expected.to_rows());
        save(&db, &settings, None).await.unwrap();
        for key in [KEY_WINDOW_START, KEY_WINDOW_MINUTES] {
            assert!(system_setting::Entity::find_by_id(key)
                .one(&db)
                .await
                .unwrap()
                .is_none());
        }
        let stored = system_setting::Entity::find_by_id(KEY_SCHEDULE)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.value, expected.format());
        assert_eq!(load(&db).await.unwrap(), settings);
    }

    #[tokio::test]
    async fn test_empty_schedule_allowed() {
        let db = db().await;
        let empty = AutoUpdateSettings {
            schedule: vec![vec![false; 24]; 7],
            ..Default::default()
        };
        save(&db, &empty, None).await.unwrap();
        let loaded = load(&db).await.unwrap();
        assert_eq!(loaded, empty);
        assert!(!loaded.is_open(utc("2026-11-25T04:30:00Z")));
        assert_eq!(loaded.next_window_start(utc("2026-11-25T04:30:00Z")), None);
    }

    #[test]
    fn test_next_window_start() {
        let s = AutoUpdateSettings::default(); // hours 04-05 UTC daily
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
