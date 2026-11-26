//! Per-user quiet hours and the summary sent when they end.
//!
//! While a user is in quiet hours, external messages (email, Telegram, SMS)
//! are written to `notification_outbox` instead of being sent; the in-app bell
//! is never held. Critical messages go out straight away unless the user turned
//! `critical_bypass` off. A leader-only task ([`QuietHoursFlushTask`]) sends
//! the held rows of each (user, channel) as one summary once the window ends.
//!
//! Times are minutes after local midnight in the user's IANA time zone, so the
//! window follows daylight saving changes: 23:00–07:00 stays 23:00–07:00 on the
//! wall clock. A window may cross midnight; `from == to` is rejected.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use jiff::tz::{AmbiguousOffset, TimeZone};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, Set,
};

use super::{NotificationMessage, NotificationService, NotificationSeverity};
use crate::error::{AppError, Result};
use crate::models::{notification_outbox, user_notification_pref, user_quiet_hours};
use crate::services::scheduler::PeriodicTask;

/// Minutes in a day; valid minute values are `0..MINUTES_PER_DAY`.
pub const MINUTES_PER_DAY: i32 = 24 * 60;
/// Held rows older than this are dropped instead of delivered.
pub const MAX_HOLD_DAYS: i64 = 7;
/// Most recent titles listed in a summary.
pub const SUMMARY_RECENT: usize = 10;
/// Title of the summary message.
pub const SUMMARY_TITLE: &str = "While you were away";
/// `event_type` recorded in the delivery log for a summary.
pub const SUMMARY_EVENT_TYPE: &str = "quiet_hours_summary";
/// How long a flush claims rows before another attempt may pick them up.
const CLAIM_LEASE_MINUTES: i64 = 5;

/// A validated quiet-hours window.
#[derive(Debug, Clone)]
pub struct QuietWindow {
    pub from_minute: i32,
    pub to_minute: i32,
    timezone: TimeZone,
    timezone_name: String,
}

impl QuietWindow {
    /// Validate raw settings: minutes in `0..1440`, `from != to`, a known IANA zone.
    pub fn new(from_minute: i32, to_minute: i32, timezone: &str) -> Result<Self> {
        if !(0..MINUTES_PER_DAY).contains(&from_minute)
            || !(0..MINUTES_PER_DAY).contains(&to_minute)
        {
            return Err(AppError::BadRequest(
                "Quiet hours times must be between 00:00 and 23:59".to_string(),
            ));
        }
        if from_minute == to_minute {
            return Err(AppError::BadRequest(
                "Quiet hours must start and end at different times".to_string(),
            ));
        }
        let (timezone, timezone_name) = parse_timezone(timezone)?;
        Ok(Self {
            from_minute,
            to_minute,
            timezone,
            timezone_name,
        })
    }

    /// The canonical IANA name of the window's time zone.
    pub fn timezone_name(&self) -> &str {
        &self.timezone_name
    }

    /// Whether the local minute-of-day `minute` falls inside the window.
    fn contains_minute(&self, minute: i32) -> bool {
        if self.from_minute < self.to_minute {
            (self.from_minute..self.to_minute).contains(&minute)
        } else {
            minute >= self.from_minute || minute < self.to_minute
        }
    }

    /// If `now` is inside the window, the UTC instant the window ends.
    pub fn end_if_inside(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let local = jiff::Timestamp::from_microsecond(now.timestamp_micros())
            .ok()?
            .to_zoned(self.timezone.clone())
            .datetime();
        let minute = i32::from(local.hour()) * 60 + i32::from(local.minute());
        if !self.contains_minute(minute) {
            return None;
        }
        // The end is today, unless the window crosses midnight and we are in
        // its evening part.
        let mut date = local.date();
        if self.from_minute > self.to_minute && minute >= self.from_minute {
            date = date.tomorrow().ok()?;
        }
        let hour = i8::try_from(self.to_minute / 60).ok()?;
        let min = i8::try_from(self.to_minute % 60).ok()?;
        resolve_local(&self.timezone, date.at(hour, min, 0, 0), now)
    }
}

/// Parse an IANA time zone name into the zone and its canonical name.
pub fn parse_timezone(name: &str) -> Result<(TimeZone, String)> {
    let tz = TimeZone::get(name)
        .map_err(|_| AppError::BadRequest(format!("Unknown time zone: {name}")))?;
    let canonical = tz.iana_name().unwrap_or(name).to_string();
    Ok((tz, canonical))
}

/// The first UTC instant after `now` showing wall-clock time `local` in `tz`.
/// An ambiguous time (clocks going back) picks the first occurrence still
/// ahead; a skipped time (clocks going forward) moves to the first minute
/// after the gap.
fn resolve_local(
    tz: &TimeZone,
    local: jiff::civil::DateTime,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    // Gaps are at most a couple of hours; three covers every real zone.
    for step in 0..=180i64 {
        let candidate = local.checked_add(jiff::Span::new().minutes(step)).ok()?;
        let offsets = match tz.to_ambiguous_timestamp(candidate).offset() {
            AmbiguousOffset::Unambiguous { offset } => vec![offset],
            AmbiguousOffset::Fold { before, after } => vec![before, after],
            AmbiguousOffset::Gap { .. } => continue,
        };
        return offsets
            .into_iter()
            .filter_map(|offset| offset.to_timestamp(candidate).ok())
            .filter_map(|ts| DateTime::from_timestamp_micros(ts.as_microsecond()))
            .find(|t| *t > now);
    }
    None
}

/// The user's window if quiet hours are enabled and valid.
async fn user_window<C: ConnectionTrait>(
    db: &C,
    user_id: i64,
) -> Result<Option<(QuietWindow, bool)>> {
    let Some(row) = user_quiet_hours::Entity::find_by_id(user_id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    if !row.enabled {
        return Ok(None);
    }
    match QuietWindow::new(row.from_minute, row.to_minute, &row.timezone) {
        Ok(window) => Ok(Some((window, row.critical_bypass))),
        Err(e) => {
            tracing::warn!(user_id, "Ignoring invalid quiet hours: {}", e);
            Ok(None)
        }
    }
}

/// When a message of `severity` for `user_id` should be held until, or `None`
/// to send it now.
pub async fn hold_until<C: ConnectionTrait>(
    db: &C,
    user_id: i64,
    severity: NotificationSeverity,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>> {
    let Some((window, critical_bypass)) = user_window(db, user_id).await? else {
        return Ok(None);
    };
    if critical_bypass && severity == NotificationSeverity::Critical {
        return Ok(None);
    }
    Ok(window.end_if_inside(now))
}

/// A message to hold in the outbox.
pub struct HeldMessage<'a> {
    pub user_id: i64,
    pub channel_type: &'a str,
    pub event_type: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub severity: NotificationSeverity,
}

/// Hold `message` until `deliver_after`.
pub async fn hold<C: ConnectionTrait>(
    db: &C,
    message: &HeldMessage<'_>,
    now: DateTime<Utc>,
    deliver_after: DateTime<Utc>,
) -> Result<()> {
    notification_outbox::ActiveModel {
        user_id: Set(message.user_id),
        channel_type: Set(message.channel_type.to_string()),
        event_type: Set(message.event_type.to_string()),
        title: Set(message.title.to_string()),
        body: Set(message.body.to_string()),
        severity: Set(message.severity.as_str().to_string()),
        created_at: Set(now),
        deliver_after: Set(deliver_after),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(())
}

/// After a user changed their quiet hours, move the delivery time of their
/// still-held rows to the end of the new window (or now, if they are no
/// longer in quiet hours).
pub async fn reschedule_held<C: ConnectionTrait>(
    db: &C,
    user_id: i64,
    window: Option<QuietWindow>,
    now: DateTime<Utc>,
) -> Result<()> {
    let deliver_after = window.and_then(|w| w.end_if_inside(now)).unwrap_or(now);
    notification_outbox::Entity::update_many()
        .col_expr(
            notification_outbox::Column::DeliverAfter,
            Expr::value(deliver_after),
        )
        .filter(notification_outbox::Column::UserId.eq(user_id))
        .filter(notification_outbox::Column::DeliverAfter.gt(now))
        .exec(db)
        .await?;
    Ok(())
}

/// Title and body of the summary for `rows` (newest first).
pub fn summary_message(
    rows: &[notification_outbox::Model],
) -> (String, NotificationSeverity, String) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut severity = NotificationSeverity::Info;
    for row in rows {
        *counts.entry(row.title.as_str()).or_default() += 1;
        severity = max_severity(severity, NotificationSeverity::parse(&row.severity));
    }
    let mut counted: Vec<(&str, usize)> = counts.into_iter().collect();
    // Most frequent first; ties alphabetical (BTreeMap order, stable sort).
    counted.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    let tally = counted
        .iter()
        .map(|(title, n)| format!("{n}× {title}"))
        .collect::<Vec<_>>()
        .join(", ");
    let held = rows.len();
    let mut body = format!(
        "{held} notification{} held during quiet hours: {tally}.",
        if held == 1 { " was" } else { "s were" }
    );
    body.push_str("\n\nMost recent:");
    for row in rows.iter().take(SUMMARY_RECENT) {
        body.push_str(&format!(
            "\n- {} ({})",
            row.title,
            row.created_at.format("%Y-%m-%d %H:%M UTC")
        ));
    }
    if held > SUMMARY_RECENT {
        body.push_str(&format!("\n…and {} more in Kubarr.", held - SUMMARY_RECENT));
    }
    (SUMMARY_TITLE.to_string(), severity, body)
}

fn max_severity(a: NotificationSeverity, b: NotificationSeverity) -> NotificationSeverity {
    let rank = |s| match s {
        NotificationSeverity::Info => 0,
        NotificationSeverity::Warning => 1,
        NotificationSeverity::Critical => 2,
    };
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

/// What one flush did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FlushStats {
    /// Summaries delivered.
    pub sent: usize,
    /// Summaries that failed to send (rows kept for the next run).
    pub failed: usize,
    /// Rows dropped: too old, or no destination to deliver to any more.
    pub dropped: u64,
}

/// Send one summary per (user, channel) for every held row due at `now`.
///
/// Rows are claimed by pushing `deliver_after` forward with a conditional
/// update, so a second flusher (a replica that briefly believes it is leader)
/// skips them. They are deleted after a successful send and made due again
/// after a failed one.
pub async fn flush_due(
    service: &NotificationService,
    db: &DatabaseConnection,
    now: DateTime<Utc>,
) -> Result<FlushStats> {
    let mut stats = FlushStats::default();

    let cutoff = now - ChronoDuration::days(MAX_HOLD_DAYS);
    let expired = notification_outbox::Entity::delete_many()
        .filter(notification_outbox::Column::CreatedAt.lt(cutoff))
        .exec(db)
        .await?;
    if expired.rows_affected > 0 {
        tracing::warn!(
            rows = expired.rows_affected,
            "Dropped held notifications older than {} days",
            MAX_HOLD_DAYS
        );
        stats.dropped += expired.rows_affected;
    }

    let due = notification_outbox::Entity::find()
        .filter(notification_outbox::Column::DeliverAfter.lte(now))
        .order_by_desc(notification_outbox::Column::CreatedAt)
        .order_by_desc(notification_outbox::Column::Id)
        .all(db)
        .await?;
    if due.is_empty() {
        return Ok(stats);
    }
    service.refresh_or_warn().await;

    let mut groups: BTreeMap<(i64, String), Vec<notification_outbox::Model>> = BTreeMap::new();
    for row in due {
        groups
            .entry((row.user_id, row.channel_type.clone()))
            .or_default()
            .push(row);
    }

    for ((user_id, channel_type), rows) in groups {
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();

        let pref = user_notification_pref::Entity::find()
            .filter(user_notification_pref::Column::UserId.eq(user_id))
            .filter(user_notification_pref::Column::ChannelType.eq(&channel_type))
            .filter(user_notification_pref::Column::Enabled.eq(true))
            .filter(user_notification_pref::Column::Verified.eq(true))
            .one(db)
            .await?;
        let Some(destination) = pref.and_then(|p| p.destination) else {
            let deleted = notification_outbox::Entity::delete_many()
                .filter(notification_outbox::Column::Id.is_in(ids))
                .exec(db)
                .await?;
            tracing::info!(
                user_id,
                channel = %channel_type,
                rows = deleted.rows_affected,
                "Dropped held notifications: destination no longer enabled and verified"
            );
            stats.dropped += deleted.rows_affected;
            continue;
        };

        let lease = now + ChronoDuration::minutes(CLAIM_LEASE_MINUTES);
        let claimed = notification_outbox::Entity::update_many()
            .col_expr(
                notification_outbox::Column::DeliverAfter,
                Expr::value(lease),
            )
            .filter(notification_outbox::Column::Id.is_in(ids.clone()))
            .filter(notification_outbox::Column::DeliverAfter.lte(now))
            .exec(db)
            .await?;
        if claimed.rows_affected != ids.len() as u64 {
            // Someone else claimed part of the group; whatever we claimed comes
            // due again after the lease.
            tracing::warn!(user_id, channel = %channel_type, "Held notifications claimed elsewhere; skipping");
            continue;
        }

        let (title, severity, body) = summary_message(&rows);
        let message = NotificationMessage {
            recipient: destination.clone(),
            title,
            body,
            severity,
        };
        let result = service.send_to_channel(&channel_type, &message).await;
        if let Err(e) = service
            .log_notification(
                db,
                Some(user_id),
                &channel_type,
                SUMMARY_EVENT_TYPE,
                &destination,
                &result,
            )
            .await
        {
            tracing::warn!(user_id, "Failed to log quiet hours summary: {}", e);
        }

        if result.success {
            notification_outbox::Entity::delete_many()
                .filter(notification_outbox::Column::Id.is_in(ids))
                .exec(db)
                .await?;
            tracing::info!(user_id, channel = %channel_type, held = rows.len(), "Sent quiet hours summary");
            stats.sent += 1;
        } else {
            notification_outbox::Entity::update_many()
                .col_expr(notification_outbox::Column::DeliverAfter, Expr::value(now))
                .filter(notification_outbox::Column::Id.is_in(ids))
                .exec(db)
                .await?;
            tracing::warn!(
                user_id,
                channel = %channel_type,
                error = result.error.as_deref().unwrap_or("unknown"),
                "Quiet hours summary failed; will retry"
            );
            stats.failed += 1;
        }
    }
    Ok(stats)
}

/// Leader-only: delivers held notifications every minute.
pub struct QuietHoursFlushTask {
    notifications: NotificationService,
}

impl QuietHoursFlushTask {
    pub fn new(notifications: NotificationService) -> Self {
        Self { notifications }
    }
}

#[async_trait]
impl PeriodicTask for QuietHoursFlushTask {
    fn name(&self) -> &'static str {
        "notification_quiet_hours_flush"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    async fn run(&self, db: &DatabaseConnection) -> anyhow::Result<()> {
        let stats = flush_due(&self.notifications, db, Utc::now()).await?;
        if stats != FlushStats::default() {
            tracing::debug!(?stats, "Quiet hours flush");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::Migrator;
    use crate::models::audit_log::AuditAction;
    use crate::models::{notification_event, notification_log, user, user_notification};
    use crate::services::notification::{routing, ChannelType, NotificationProvider, SendResult};
    use chrono::NaiveDate;
    use sea_orm::{Database, PaginatorTrait};
    use sea_orm_migration::MigratorTrait;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(y, mo, d)
            .and_then(|date| date.and_hms_opt(h, mi, 0))
            .map(|naive| naive.and_utc())
            .expect("valid date")
    }

    fn window(from: &str, to: &str, tz: &str) -> QuietWindow {
        let minutes = |s: &str| {
            let (h, m) = s.split_once(':').expect("hh:mm");
            h.parse::<i32>().expect("h") * 60 + m.parse::<i32>().expect("m")
        };
        QuietWindow::new(minutes(from), minutes(to), tz).expect("valid window")
    }

    // ---------------------------------------------------------------------
    // Window math
    // ---------------------------------------------------------------------

    #[test]
    fn validation_rejects_bad_minutes_equal_ends_and_unknown_zones() {
        assert!(QuietWindow::new(-1, 60, "UTC").is_err());
        assert!(QuietWindow::new(0, 1440, "UTC").is_err());
        assert!(QuietWindow::new(420, 420, "UTC").is_err());
        assert!(QuietWindow::new(1380, 420, "Mars/Olympus_Mons").is_err());
        assert!(QuietWindow::new(0, 1439, "Europe/Amsterdam").is_ok());
    }

    #[test]
    fn same_day_window() {
        let w = window("09:00", "17:00", "UTC");
        assert_eq!(w.end_if_inside(utc(2026, 6, 1, 8, 59)), None);
        assert_eq!(
            w.end_if_inside(utc(2026, 6, 1, 9, 0)),
            Some(utc(2026, 6, 1, 17, 0))
        );
        assert_eq!(
            w.end_if_inside(utc(2026, 6, 1, 16, 59)),
            Some(utc(2026, 6, 1, 17, 0))
        );
        assert_eq!(w.end_if_inside(utc(2026, 6, 1, 17, 0)), None);
    }

    #[test]
    fn window_crossing_midnight() {
        let w = window("23:00", "07:00", "UTC");
        // Evening part ends tomorrow morning.
        assert_eq!(
            w.end_if_inside(utc(2026, 6, 1, 23, 30)),
            Some(utc(2026, 6, 2, 7, 0))
        );
        // Morning part ends this morning.
        assert_eq!(
            w.end_if_inside(utc(2026, 6, 2, 6, 59)),
            Some(utc(2026, 6, 2, 7, 0))
        );
        assert_eq!(w.end_if_inside(utc(2026, 6, 2, 7, 0)), None);
        assert_eq!(w.end_if_inside(utc(2026, 6, 2, 12, 0)), None);
        assert_eq!(w.end_if_inside(utc(2026, 6, 2, 22, 59)), None);
    }

    #[test]
    fn local_time_zone_is_used() {
        // 23:30 in Amsterdam (summer, UTC+2) is 21:30 UTC.
        let w = window("23:00", "07:00", "Europe/Amsterdam");
        assert_eq!(
            w.end_if_inside(utc(2026, 6, 1, 21, 30)),
            Some(utc(2026, 6, 2, 5, 0))
        );
        assert_eq!(w.end_if_inside(utc(2026, 6, 1, 20, 30)), None);
    }

    #[test]
    fn spring_forward_keeps_the_wall_clock_end() {
        // Amsterdam: 2026-03-29 02:00 CET → 03:00 CEST.
        let w = window("23:00", "07:00", "Europe/Amsterdam");
        // 23:30 CET on the 28th; ends 07:00 CEST = 05:00 UTC.
        assert_eq!(
            w.end_if_inside(utc(2026, 3, 28, 22, 30)),
            Some(utc(2026, 3, 29, 5, 0))
        );
        // After the change: 03:30 CEST is 01:30 UTC.
        assert_eq!(
            w.end_if_inside(utc(2026, 3, 29, 1, 30)),
            Some(utc(2026, 3, 29, 5, 0))
        );
    }

    #[test]
    fn end_inside_the_skipped_hour_moves_past_the_gap() {
        // 02:30 does not exist on 2026-03-29 in Amsterdam; 03:00 CEST does.
        let w = window("01:00", "02:30", "Europe/Amsterdam");
        assert_eq!(
            w.end_if_inside(utc(2026, 3, 29, 0, 30)),
            Some(utc(2026, 3, 29, 1, 0))
        );
        // 03:05 CEST: past the end.
        assert_eq!(w.end_if_inside(utc(2026, 3, 29, 1, 5)), None);
    }

    #[test]
    fn fall_back_keeps_the_wall_clock_end() {
        // Amsterdam: 2026-10-25 03:00 CEST → 02:00 CET.
        let w = window("23:00", "07:00", "Europe/Amsterdam");
        // 23:30 CEST on the 24th; ends 07:00 CET = 06:00 UTC.
        assert_eq!(
            w.end_if_inside(utc(2026, 10, 24, 21, 30)),
            Some(utc(2026, 10, 25, 6, 0))
        );
    }

    #[test]
    fn end_inside_the_repeated_hour_picks_the_next_occurrence() {
        let w = window("01:00", "02:30", "Europe/Amsterdam");
        // 01:30 CEST: ends at the first 02:30 (CEST) = 00:30 UTC.
        assert_eq!(
            w.end_if_inside(utc(2026, 10, 24, 23, 30)),
            Some(utc(2026, 10, 25, 0, 30))
        );
        // 02:10 CET (second pass): the first 02:30 is gone, so the second.
        assert_eq!(
            w.end_if_inside(utc(2026, 10, 25, 1, 10)),
            Some(utc(2026, 10, 25, 1, 30))
        );
    }

    // ---------------------------------------------------------------------
    // Summary text
    // ---------------------------------------------------------------------

    fn outbox_row(
        id: i64,
        title: &str,
        severity: &str,
        minutes_ago: i64,
    ) -> notification_outbox::Model {
        notification_outbox::Model {
            id,
            user_id: 1,
            channel_type: "telegram".to_string(),
            event_type: "app_updated".to_string(),
            title: title.to_string(),
            body: String::new(),
            severity: severity.to_string(),
            created_at: Utc::now() - ChronoDuration::minutes(minutes_ago),
            deliver_after: Utc::now(),
        }
    }

    #[test]
    fn summary_counts_titles_and_lists_recent_ones() {
        let rows = vec![
            outbox_row(3, "App update failed", "warning", 1),
            outbox_row(2, "App updated", "info", 2),
            outbox_row(1, "App updated", "info", 3),
        ];
        let (title, severity, body) = summary_message(&rows);
        assert_eq!(title, SUMMARY_TITLE);
        assert_eq!(severity, NotificationSeverity::Warning);
        assert!(body.contains("3 notifications were held"), "{body}");
        assert!(
            body.contains("2× App updated, 1× App update failed"),
            "{body}"
        );
        assert!(body.contains("- App update failed"), "{body}");
    }

    #[test]
    fn summary_caps_the_recent_list() {
        let rows: Vec<_> = (0..13)
            .map(|i| outbox_row(i, &format!("Event {i}"), "info", i))
            .collect();
        let (_, _, body) = summary_message(&rows);
        assert_eq!(body.matches("\n- ").count(), SUMMARY_RECENT);
        assert!(body.contains("…and 3 more"), "{body}");
    }

    // ---------------------------------------------------------------------
    // Holding during delivery
    // ---------------------------------------------------------------------

    async fn migrated_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("db");
        Migrator::up(&db, None).await.expect("migrate");
        db
    }

    async fn insert_user(db: &DatabaseConnection, name: &str) -> i64 {
        let now = Utc::now();
        user::ActiveModel {
            username: Set(name.to_string()),
            email: Set(format!("{name}@example.com")),
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
        .expect("insert user")
        .id
    }

    async fn insert_pref(db: &DatabaseConnection, user_id: i64, channel: &str, destination: &str) {
        let now = Utc::now();
        user_notification_pref::ActiveModel {
            user_id: Set(user_id),
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

    /// A user with verified Telegram and email destinations; `login` is
    /// enabled with `severity` and routed to both.
    async fn setup(severity: &str) -> (DatabaseConnection, NotificationService, i64) {
        let db = migrated_db().await;
        notification_event::ActiveModel {
            event_type: Set("login".to_string()),
            enabled: Set(true),
            severity: Set(severity.to_string()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("insert event");
        routing::set_routes(
            &db,
            "login",
            &HashMap::from([(ChannelType::Telegram, true), (ChannelType::Email, true)]),
        )
        .await
        .expect("routes");
        let uid = insert_user(&db, "quiet").await;
        insert_pref(&db, uid, "telegram", "123456789").await;
        insert_pref(&db, uid, "email", "quiet@example.com").await;
        let svc = NotificationService::new();
        svc.set_db(db.clone()).await;
        (db, svc, uid)
    }

    /// Quiet hours (UTC) covering every minute except one a few minutes from
    /// now, so "now" is inside for the length of the test.
    async fn set_quiet(
        db: &DatabaseConnection,
        user_id: i64,
        enabled: bool,
        critical_bypass: bool,
    ) {
        let now = Utc::now();
        let minute = (chrono::Timelike::hour(&now) * 60 + chrono::Timelike::minute(&now)) as i32;
        user_quiet_hours::ActiveModel {
            user_id: Set(user_id),
            enabled: Set(enabled),
            from_minute: Set((minute + 5) % MINUTES_PER_DAY),
            to_minute: Set((minute + 4) % MINUTES_PER_DAY),
            timezone: Set("UTC".to_string()),
            critical_bypass: Set(critical_bypass),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert quiet hours");
    }

    async fn counts(db: &DatabaseConnection, user_id: i64) -> (u64, u64, u64) {
        let bell = user_notification::Entity::find()
            .filter(user_notification::Column::UserId.eq(user_id))
            .count(db)
            .await
            .expect("bell");
        let attempts = notification_log::Entity::find()
            .count(db)
            .await
            .expect("logs");
        let held = notification_outbox::Entity::find()
            .count(db)
            .await
            .expect("outbox");
        (bell, attempts, held)
    }

    #[tokio::test]
    async fn non_critical_messages_are_held_and_the_bell_is_not() {
        let (db, svc, uid) = setup("warning").await;
        set_quiet(&db, uid, true, true).await;
        svc.notify_event(&AuditAction::Login, Some(uid), Some("quiet"), None)
            .await
            .expect("notify");
        // Bell written, nothing sent, one held row per routed channel.
        assert_eq!(counts(&db, uid).await, (1, 0, 2));
        let held = notification_outbox::Entity::find()
            .all(&db)
            .await
            .expect("rows");
        assert!(held.iter().all(|r| r.deliver_after > Utc::now()));
        assert!(held
            .iter()
            .all(|r| r.event_type == "login" && r.severity == "warning"));
    }

    #[tokio::test]
    async fn only_routed_channels_are_held() {
        let (db, svc, uid) = setup("info").await;
        routing::set_routes(&db, "login", &HashMap::from([(ChannelType::Email, false)]))
            .await
            .expect("routes");
        set_quiet(&db, uid, true, true).await;
        svc.notify_event(&AuditAction::Login, Some(uid), Some("quiet"), None)
            .await
            .expect("notify");
        let held = notification_outbox::Entity::find()
            .all(&db)
            .await
            .expect("rows");
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].channel_type, "telegram");
    }

    #[tokio::test]
    async fn critical_messages_bypass_quiet_hours_when_allowed() {
        let (db, svc, uid) = setup("critical").await;
        set_quiet(&db, uid, true, true).await;
        svc.notify_event(&AuditAction::Login, Some(uid), Some("quiet"), None)
            .await
            .expect("notify");
        // Sent (attempted: no provider is configured, so logged as failed).
        assert_eq!(counts(&db, uid).await, (1, 2, 0));
    }

    #[tokio::test]
    async fn critical_messages_wait_when_bypass_is_off() {
        let (db, svc, uid) = setup("critical").await;
        set_quiet(&db, uid, true, false).await;
        svc.notify_event(&AuditAction::Login, Some(uid), Some("quiet"), None)
            .await
            .expect("notify");
        assert_eq!(counts(&db, uid).await, (1, 0, 2));
    }

    #[tokio::test]
    async fn disabled_quiet_hours_send_straight_away() {
        let (db, svc, uid) = setup("info").await;
        set_quiet(&db, uid, false, true).await;
        svc.notify_event(&AuditAction::Login, Some(uid), Some("quiet"), None)
            .await
            .expect("notify");
        assert_eq!(counts(&db, uid).await, (1, 2, 0));
    }

    // ---------------------------------------------------------------------
    // Flush
    // ---------------------------------------------------------------------

    /// Records every message; fails while `fail` is set.
    #[derive(Clone)]
    struct Recorder {
        channel: ChannelType,
        sent: Arc<Mutex<Vec<NotificationMessage>>>,
        fail: Arc<Mutex<bool>>,
    }

    impl Recorder {
        fn new(channel: ChannelType) -> Self {
            Self {
                channel,
                sent: Arc::new(Mutex::new(Vec::new())),
                fail: Arc::new(Mutex::new(false)),
            }
        }

        fn sent(&self) -> Vec<NotificationMessage> {
            self.sent.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl NotificationProvider for Recorder {
        fn channel_type(&self) -> ChannelType {
            self.channel
        }

        async fn send(&self, message: &NotificationMessage) -> SendResult {
            if *self.fail.lock().expect("lock") {
                return SendResult {
                    success: false,
                    error: Some("down".to_string()),
                };
            }
            self.sent.lock().expect("lock").push(message.clone());
            SendResult {
                success: true,
                error: None,
            }
        }

        async fn test(&self, _destination: &str) -> SendResult {
            SendResult {
                success: true,
                error: None,
            }
        }
    }

    async fn held_row(
        db: &DatabaseConnection,
        user_id: i64,
        channel: &str,
        title: &str,
        created_at: DateTime<Utc>,
        deliver_after: DateTime<Utc>,
    ) {
        notification_outbox::ActiveModel {
            user_id: Set(user_id),
            channel_type: Set(channel.to_string()),
            event_type: Set("app_updated".to_string()),
            title: Set(title.to_string()),
            body: Set("body".to_string()),
            severity: Set("info".to_string()),
            created_at: Set(created_at),
            deliver_after: Set(deliver_after),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("insert held row");
    }

    #[tokio::test]
    async fn flush_sends_one_summary_per_user_and_channel_and_deletes_rows() {
        let (db, svc, uid) = setup("info").await;
        let other = insert_user(&db, "other").await;
        insert_pref(&db, other, "telegram", "987654321").await;
        let telegram = Recorder::new(ChannelType::Telegram);
        let email = Recorder::new(ChannelType::Email);
        svc.set_provider_override(Arc::new(telegram.clone())).await;
        svc.set_provider_override(Arc::new(email.clone())).await;

        let now = Utc::now();
        let past = now - ChronoDuration::minutes(1);
        for title in ["App updated", "App updated", "App update failed"] {
            held_row(
                &db,
                uid,
                "telegram",
                title,
                now - ChronoDuration::hours(2),
                past,
            )
            .await;
        }
        held_row(
            &db,
            uid,
            "email",
            "App updated",
            now - ChronoDuration::hours(2),
            past,
        )
        .await;
        held_row(
            &db,
            other,
            "telegram",
            "App updated",
            now - ChronoDuration::hours(1),
            past,
        )
        .await;
        // Not yet due: stays.
        held_row(
            &db,
            uid,
            "telegram",
            "Later",
            now,
            now + ChronoDuration::hours(1),
        )
        .await;

        let stats = flush_due(&svc, &db, now).await.expect("flush");
        assert_eq!(
            stats,
            FlushStats {
                sent: 3,
                failed: 0,
                dropped: 0
            }
        );

        let sent = telegram.sent();
        assert_eq!(sent.len(), 2);
        let mine = sent
            .iter()
            .find(|m| m.recipient == "123456789")
            .expect("summary");
        assert_eq!(mine.title, SUMMARY_TITLE);
        assert!(
            mine.body.contains("2× App updated, 1× App update failed"),
            "{}",
            mine.body
        );
        assert!(!mine.body.contains("Later"));
        assert_eq!(email.sent().len(), 1);

        let left = notification_outbox::Entity::find()
            .all(&db)
            .await
            .expect("rows");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].title, "Later");
        let logged = notification_log::Entity::find()
            .filter(notification_log::Column::EventType.eq(SUMMARY_EVENT_TYPE))
            .count(&db)
            .await
            .expect("logs");
        assert_eq!(logged, 3);

        // Nothing due any more: a second run sends nothing.
        let stats = flush_due(&svc, &db, now).await.expect("flush");
        assert_eq!(stats, FlushStats::default());
    }

    #[tokio::test]
    async fn failed_summary_keeps_rows_for_the_next_minute() {
        let (db, svc, uid) = setup("info").await;
        let telegram = Recorder::new(ChannelType::Telegram);
        *telegram.fail.lock().expect("lock") = true;
        svc.set_provider_override(Arc::new(telegram.clone())).await;
        let now = Utc::now();
        held_row(
            &db,
            uid,
            "telegram",
            "App updated",
            now - ChronoDuration::hours(1),
            now,
        )
        .await;

        let stats = flush_due(&svc, &db, now).await.expect("flush");
        assert_eq!(stats.failed, 1);
        let left = notification_outbox::Entity::find()
            .all(&db)
            .await
            .expect("rows");
        assert_eq!(left.len(), 1);
        assert!(left[0].deliver_after <= now, "due again on the next run");

        *telegram.fail.lock().expect("lock") = false;
        let next = now + ChronoDuration::minutes(1);
        let stats = flush_due(&svc, &db, next).await.expect("flush");
        assert_eq!(stats.sent, 1);
        assert_eq!(telegram.sent().len(), 1);
        assert_eq!(
            notification_outbox::Entity::find()
                .count(&db)
                .await
                .expect("n"),
            0
        );
    }

    #[tokio::test]
    async fn rows_older_than_seven_days_are_dropped() {
        let (db, svc, uid) = setup("info").await;
        let telegram = Recorder::new(ChannelType::Telegram);
        svc.set_provider_override(Arc::new(telegram.clone())).await;
        let now = Utc::now();
        let old = now - ChronoDuration::days(MAX_HOLD_DAYS) - ChronoDuration::hours(1);
        held_row(&db, uid, "telegram", "Ancient", old, old).await;

        let stats = flush_due(&svc, &db, now).await.expect("flush");
        assert_eq!(
            stats,
            FlushStats {
                sent: 0,
                failed: 0,
                dropped: 1
            }
        );
        assert!(telegram.sent().is_empty());
        assert_eq!(
            notification_outbox::Entity::find()
                .count(&db)
                .await
                .expect("n"),
            0
        );
    }

    #[tokio::test]
    async fn rows_without_a_verified_destination_are_dropped() {
        let (db, svc, _) = setup("info").await;
        let stranger = insert_user(&db, "nodest").await;
        let now = Utc::now();
        held_row(&db, stranger, "telegram", "App updated", now, now).await;
        let stats = flush_due(&svc, &db, now).await.expect("flush");
        assert_eq!(stats.dropped, 1);
        assert_eq!(
            notification_outbox::Entity::find()
                .count(&db)
                .await
                .expect("n"),
            0
        );
    }

    #[tokio::test]
    async fn rows_claimed_by_another_flush_are_not_sent_twice() {
        let (db, svc, uid) = setup("info").await;
        let telegram = Recorder::new(ChannelType::Telegram);
        svc.set_provider_override(Arc::new(telegram.clone())).await;
        let now = Utc::now();
        held_row(
            &db,
            uid,
            "telegram",
            "App updated",
            now - ChronoDuration::hours(1),
            now,
        )
        .await;

        // Two flushes racing on the same rows: the first claims, the second
        // sees nothing due.
        let (a, b) = tokio::join!(flush_due(&svc, &db, now), flush_due(&svc, &db, now));
        let sent = a.expect("a").sent + b.expect("b").sent;
        assert_eq!(sent, 1);
        assert_eq!(telegram.sent().len(), 1);
    }

    #[tokio::test]
    async fn rescheduling_follows_new_settings() {
        let (db, _, uid) = setup("info").await;
        let now = utc(2026, 6, 1, 23, 30);
        let later = utc(2026, 6, 2, 7, 0);
        held_row(&db, uid, "telegram", "App updated", now, later).await;
        // Turned off: due now.
        reschedule_held(&db, uid, None, now)
            .await
            .expect("reschedule");
        let row = notification_outbox::Entity::find()
            .one(&db)
            .await
            .expect("q")
            .expect("row");
        assert_eq!(row.deliver_after, now);
        // Already-due rows are left alone; future rows follow a new window.
        let mut active: notification_outbox::ActiveModel = row.into();
        active.deliver_after = Set(later);
        active.update(&db).await.expect("update");
        reschedule_held(&db, uid, Some(window("23:00", "08:00", "UTC")), now)
            .await
            .expect("reschedule");
        let row = notification_outbox::Entity::find()
            .one(&db)
            .await
            .expect("q")
            .expect("row");
        assert_eq!(row.deliver_after, utc(2026, 6, 2, 8, 0));
    }
}
