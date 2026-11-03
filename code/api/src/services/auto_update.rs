//! Automatic app updates inside a maintenance window.
//!
//! The worker checks every few minutes. When auto-update is on and the clock
//! is inside the configured window, it queues an update for one app at a
//! time, and only while nothing else is in the operation queue, so a bad
//! release is easy to spot and never piles up behind manual work.
//!
//! Major version jumps are skipped unless explicitly allowed: they are the
//! releases most likely to need a human (migrations, changed config), and
//! security fixes ship as minor or patch releases.
//!
//! An update that failed for a given target version is not retried
//! automatically. A newer release, or a manual update, clears the way.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, Result};
use crate::models::system_setting;
use crate::services::app_manager::AppManager;

/// Row in `system_settings` holding the JSON-encoded [`AutoUpdateConfig`].
/// Deliberately not one of the generic settings keys: it is validated as a
/// whole and served through its own endpoint.
pub const AUTO_UPDATE_SETTING_KEY: &str = "auto_update";

/// How often the worker looks for something to update.
const CHECK_INTERVAL: Duration = Duration::from_secs(120);

const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AutoUpdateConfig {
    /// Master switch. Off by default: nothing updates until someone opts in.
    pub enabled: bool,
    /// Also take major version jumps (1.x to 2.x).
    pub allow_major: bool,
    pub window: UpdateWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateWindow {
    /// Days the window opens on: "mon" through "sun".
    pub days: Vec<String>,
    /// Local opening time, "HH:MM".
    pub start: String,
    /// Local closing time, "HH:MM". Earlier than `start` means the window
    /// runs past midnight into the next day.
    pub end: String,
    /// IANA timezone name, such as "Europe/Amsterdam".
    pub timezone: String,
}

impl Default for AutoUpdateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_major: false,
            window: UpdateWindow {
                days: DAY_NAMES.iter().map(|d| d.to_string()).collect(),
                start: "03:00".into(),
                end: "06:00".into(),
                timezone: "UTC".into(),
            },
        }
    }
}

/// A window with its parts parsed, so checks cannot fail halfway.
struct ParsedWindow {
    days: [bool; 7],
    start: NaiveTime,
    end: NaiveTime,
    tz: Tz,
}

fn parse_time(raw: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(raw, "%H:%M").ok()
}

fn weekday_index(day: Weekday) -> usize {
    day.num_days_from_monday() as usize
}

impl UpdateWindow {
    fn parse(&self) -> Result<ParsedWindow> {
        let mut days = [false; 7];
        for day in &self.days {
            let index = DAY_NAMES
                .iter()
                .position(|name| name == day)
                .ok_or_else(|| AppError::BadRequest(format!("Unknown day '{}'", day)))?;
            days[index] = true;
        }
        if !days.iter().any(|d| *d) {
            return Err(AppError::BadRequest(
                "The update window needs at least one day".into(),
            ));
        }
        let start = parse_time(&self.start)
            .ok_or_else(|| AppError::BadRequest("Window start must be HH:MM".into()))?;
        let end = parse_time(&self.end)
            .ok_or_else(|| AppError::BadRequest("Window end must be HH:MM".into()))?;
        if start == end {
            return Err(AppError::BadRequest(
                "Window start and end must differ".into(),
            ));
        }
        let tz: Tz = self
            .timezone
            .parse()
            .map_err(|_| AppError::BadRequest(format!("Unknown timezone '{}'", self.timezone)))?;
        Ok(ParsedWindow {
            days,
            start,
            end,
            tz,
        })
    }
}

impl ParsedWindow {
    fn contains(&self, now: DateTime<Utc>) -> bool {
        let local = now.with_timezone(&self.tz);
        let time = local.time();
        let today = weekday_index(local.weekday());
        if self.start < self.end {
            self.days[today] && time >= self.start && time < self.end
        } else {
            // Past midnight: the part after midnight belongs to the window
            // that opened the previous evening.
            let yesterday = weekday_index(local.weekday().pred());
            (self.days[today] && time >= self.start) || (self.days[yesterday] && time < self.end)
        }
    }

    /// The next time the window opens, strictly after `now`.
    fn next_open(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let local_today = now.with_timezone(&self.tz).date_naive();
        (0..=7)
            .filter_map(|offset| local_today.checked_add_days(chrono::Days::new(offset)))
            .filter(|date| self.days[weekday_index(date.weekday())])
            .filter_map(|date| {
                // A start inside a DST gap does not exist that day; take the
                // earliest valid reading of an ambiguous one.
                self.tz
                    .from_local_datetime(&date.and_time(self.start))
                    .earliest()
            })
            .map(|open| open.with_timezone(&Utc))
            .find(|open| *open > now)
    }
}

impl AutoUpdateConfig {
    pub fn validate(&self) -> Result<()> {
        self.window.parse().map(|_| ())
    }

    /// Whether automatic updates may start right now.
    pub fn may_run(&self, now: DateTime<Utc>) -> bool {
        self.enabled
            && self
                .window
                .parse()
                .map(|window| window.contains(now))
                .unwrap_or(false)
    }

    pub fn in_window(&self, now: DateTime<Utc>) -> bool {
        self.window
            .parse()
            .map(|window| window.contains(now))
            .unwrap_or(false)
    }

    pub fn next_window_start(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.window.parse().ok()?.next_open(now)
    }
}

/// The leading number of a chart version: "v2.3.1" and "2.3.1+build" are 2.
fn major_of(version: &str) -> Option<u64> {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .next()?
        .parse()
        .ok()
}

/// Whether going from `installed` to `available` crosses a major version.
/// A version that cannot be read counts as a major jump, so an odd version
/// string never slips through as a "safe" update.
pub fn is_major_jump(installed: &str, available: &str) -> bool {
    match (major_of(installed), major_of(available)) {
        (Some(from), Some(to)) => to != from,
        _ => true,
    }
}

/// Stored configuration, or the default when none has been saved. A stored
/// value that no longer parses also falls back to the default, which is off:
/// a corrupt row must never switch updates on.
pub async fn load<C: ConnectionTrait>(db: &C) -> Result<AutoUpdateConfig> {
    let row = system_setting::Entity::find_by_id(AUTO_UPDATE_SETTING_KEY.to_string())
        .one(db)
        .await?;
    Ok(row
        .and_then(|row| serde_json::from_str::<AutoUpdateConfig>(&row.value).ok())
        .filter(|config| config.validate().is_ok())
        .unwrap_or_default())
}

pub async fn save<C: ConnectionTrait>(db: &C, config: &AutoUpdateConfig) -> Result<()> {
    config.validate()?;
    let value = serde_json::to_string(config)?;
    let now = Utc::now();
    let existing = system_setting::Entity::find_by_id(AUTO_UPDATE_SETTING_KEY.to_string())
        .one(db)
        .await?;
    match existing {
        Some(row) => {
            let mut active: system_setting::ActiveModel = row.into();
            active.value = Set(value);
            active.updated_at = Set(now);
            active.update(db).await?;
        }
        None => {
            system_setting::ActiveModel {
                key: Set(AUTO_UPDATE_SETTING_KEY.to_string()),
                value: Set(value),
                description: Set(Some(
                    "Automatic app updates and their maintenance window".into(),
                )),
                updated_at: Set(now),
            }
            .insert(db)
            .await?;
        }
    }
    Ok(())
}

/// One pass: queue at most one automatic update if the window allows it.
pub async fn run_once(manager: &AppManager, now: DateTime<Utc>) -> Result<Option<String>> {
    let config = load(manager.db()).await?;
    if !config.may_run(now) {
        return Ok(None);
    }
    let Some((app_name, target)) = manager.auto_update_candidate(config.allow_major).await? else {
        return Ok(None);
    };
    manager.enqueue_auto_update(&app_name, &target).await?;
    tracing::info!(app = %app_name, version = %target, "Queued automatic update");
    Ok(Some(app_name))
}

/// Run the auto-update check until cancelled. Only the elected worker calls
/// this, so two workers never queue competing updates.
pub fn start(manager: Arc<AppManager>, cancel: CancellationToken) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CHECK_INTERVAL);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {}
            }
            if let Err(error) = run_once(&manager, Utc::now()).await {
                tracing::warn!(%error, "Automatic update check failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(days: &[&str], start: &str, end: &str, tz: &str) -> AutoUpdateConfig {
        AutoUpdateConfig {
            enabled: true,
            allow_major: false,
            window: UpdateWindow {
                days: days.iter().map(|d| d.to_string()).collect(),
                start: start.into(),
                end: end.into(),
                timezone: tz.into(),
            },
        }
    }

    fn utc(raw: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(raw)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn the_default_is_off_and_valid() {
        let config = AutoUpdateConfig::default();
        assert!(!config.enabled);
        assert!(config.validate().is_ok());
        assert!(!config.may_run(utc("2026-11-04T04:00:00Z")));
    }

    #[test]
    fn a_same_day_window_opens_and_closes() {
        // 2026-11-04 is a Wednesday.
        let c = config(&["wed"], "03:00", "06:00", "UTC");
        assert!(!c.may_run(utc("2026-11-04T02:59:00Z")));
        assert!(c.may_run(utc("2026-11-04T03:00:00Z")));
        assert!(c.may_run(utc("2026-11-04T05:59:00Z")));
        assert!(!c.may_run(utc("2026-11-04T06:00:00Z")));
        assert!(
            !c.may_run(utc("2026-11-05T04:00:00Z")),
            "thursday is not listed"
        );
    }

    #[test]
    fn a_window_past_midnight_belongs_to_the_day_it_opened() {
        let c = config(&["sat"], "23:00", "02:00", "UTC");
        // 2026-11-07 is a Saturday.
        assert!(c.may_run(utc("2026-11-07T23:30:00Z")));
        assert!(
            c.may_run(utc("2026-11-08T01:30:00Z")),
            "sunday morning, opened saturday"
        );
        assert!(
            !c.may_run(utc("2026-11-08T23:30:00Z")),
            "sunday evening is not listed"
        );
        assert!(
            !c.may_run(utc("2026-11-07T01:30:00Z")),
            "friday's window does not exist"
        );
    }

    #[test]
    fn the_window_is_read_in_its_own_timezone() {
        // 03:00 in Amsterdam is 02:00 UTC in winter.
        let c = config(&["wed"], "03:00", "04:00", "Europe/Amsterdam");
        assert!(c.may_run(utc("2026-11-04T02:30:00Z")));
        assert!(!c.may_run(utc("2026-11-04T03:30:00Z")));
    }

    #[test]
    fn next_open_skips_unlisted_days_and_past_openings() {
        let c = config(&["fri"], "03:00", "06:00", "UTC");
        // Wednesday noon: next opening is Friday 03:00.
        assert_eq!(
            c.next_window_start(utc("2026-11-04T12:00:00Z")),
            Some(utc("2026-11-06T03:00:00Z"))
        );
        // Inside Friday's window: the next one is a week later.
        assert_eq!(
            c.next_window_start(utc("2026-11-06T04:00:00Z")),
            Some(utc("2026-11-13T03:00:00Z"))
        );
    }

    #[test]
    fn bad_windows_are_rejected() {
        assert!(config(&[], "03:00", "06:00", "UTC").validate().is_err());
        assert!(config(&["someday"], "03:00", "06:00", "UTC")
            .validate()
            .is_err());
        assert!(config(&["mon"], "3am", "06:00", "UTC").validate().is_err());
        assert!(config(&["mon"], "03:00", "03:00", "UTC")
            .validate()
            .is_err());
        assert!(config(&["mon"], "03:00", "06:00", "Mars/Olympus")
            .validate()
            .is_err());
    }

    #[test]
    fn major_jumps_are_detected_and_unreadable_versions_count_as_major() {
        assert!(!is_major_jump("1.2.3", "1.4.0"));
        assert!(!is_major_jump("v1.2.3", "1.2.4+build.7"));
        assert!(is_major_jump("1.9.0", "2.0.0"));
        assert!(is_major_jump("latest", "1.0.0"));
    }

    #[test]
    fn config_rejects_unknown_fields() {
        let raw = r#"{"enabled":true,"allow_major":false,"extra":1,
            "window":{"days":["mon"],"start":"03:00","end":"06:00","timezone":"UTC"}}"#;
        assert!(serde_json::from_str::<AutoUpdateConfig>(raw).is_err());
    }
}
