//! Pure policy for automatic app updates: classify a pending chart update and
//! decide whether "now" falls inside the owner's update window.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum UpdateKind {
    /// Same semver core, higher build number.
    Revision,
    Patch,
    Minor,
    Major,
    Downgrade,
    Unknown,
}

/// Parsed `MAJOR.MINOR.PATCH[+BUILD]`.
type Parsed = ((u64, u64, u64), u64);

fn parse_version(v: &str) -> Option<Parsed> {
    let (core, build) = match v.split_once('+') {
        Some((c, b)) => (c, b.parse::<u64>().ok()?),
        None => (v, 0),
    };
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some(((major, minor, patch), build))
}

/// Classify the update from `installed` to `available` chart versions.
pub fn update_kind(installed: &str, available: &str) -> UpdateKind {
    let (Some((ic, ib)), Some((ac, ab))) = (parse_version(installed), parse_version(available))
    else {
        return UpdateKind::Unknown;
    };
    if ac < ic || (ac == ic && ab < ib) {
        return UpdateKind::Downgrade;
    }
    if ac == ic {
        // Equal versions are not an update; treat as unknown so they never auto-apply.
        return if ab > ib {
            UpdateKind::Revision
        } else {
            UpdateKind::Unknown
        };
    }
    if ac.0 != ic.0 {
        UpdateKind::Major
    } else if ac.1 != ic.1 {
        UpdateKind::Minor
    } else {
        UpdateKind::Patch
    }
}

/// Only revisions, patches and minor updates may be applied automatically.
pub fn auto_eligible(kind: UpdateKind) -> bool {
    matches!(
        kind,
        UpdateKind::Revision | UpdateKind::Patch | UpdateKind::Minor
    )
}

#[derive(Debug, Clone, Copy)]
pub struct UpdateWindow {
    pub start: NaiveTime,
    pub minutes: u32,
    pub tz: Tz,
}

/// Resolve a local wall-clock time to UTC. A time skipped by a DST jump
/// resolves to one hour later (the first valid instant after the gap).
pub(crate) fn resolve_local(tz: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let naive = NaiveDateTime::new(date, time);
    tz.from_local_datetime(&naive)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(naive + Duration::hours(1)))
                .earliest()
        })
        .map(|d| d.with_timezone(&Utc))
}

/// The window containing `now` as `(start, end)` in UTC, or `None` when outside.
/// A window that began on the previous local day and wraps past midnight counts.
pub fn window_bounds(
    w: &UpdateWindow,
    now: DateTime<Utc>,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let today = now.with_timezone(&w.tz).date_naive();
    [today.pred_opt()?, today].into_iter().find_map(|date| {
        let start = resolve_local(w.tz, date, w.start)?;
        let end = start + Duration::minutes(i64::from(w.minutes));
        (start <= now && now < end).then_some((start, end))
    })
}

/// Parse an `HH:MM` window start.
pub fn parse_window_start(s: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|_| format!("invalid window start '{s}', expected HH:MM"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn kinds() {
        assert_eq!(update_kind("1.29.2+5", "1.30.1+6"), UpdateKind::Minor);
        assert_eq!(update_kind("0.18.2+15", "0.18.3+16"), UpdateKind::Patch);
        assert_eq!(update_kind("6.4.4+25", "7.0.3+26"), UpdateKind::Major);
        assert_eq!(update_kind("1.30.1+3", "1.30.1+4"), UpdateKind::Revision);
        assert_eq!(update_kind("1.30.1+4", "1.30.1+3"), UpdateKind::Downgrade);
        assert_eq!(update_kind("2.0.0", "1.9.9"), UpdateKind::Downgrade);
        assert_eq!(update_kind("v0.0.37", "garbage"), UpdateKind::Unknown);
        assert_eq!(update_kind("1.0.0+3", "1.0.0+3"), UpdateKind::Unknown);
        assert!(
            auto_eligible(UpdateKind::Revision)
                && !auto_eligible(UpdateKind::Major)
                && !auto_eligible(UpdateKind::Unknown)
        );
    }

    #[test]
    fn test_window_in_and_out() {
        let w = UpdateWindow {
            start: parse_window_start("04:00").unwrap(),
            minutes: 120,
            tz: chrono_tz::Europe::Amsterdam,
        };
        // 2026-11-24 is CET (UTC+1): 04:30 local = 03:30Z, 06:01 local = 05:01Z.
        let (s, e) = window_bounds(&w, utc("2026-11-24T03:30:00Z")).unwrap();
        assert_eq!(s, utc("2026-11-24T03:00:00Z"));
        assert_eq!(e, utc("2026-11-24T05:00:00Z"));
        assert!(window_bounds(&w, utc("2026-11-24T05:01:00Z")).is_none());
        assert!(window_bounds(&w, utc("2026-11-24T02:59:00Z")).is_none());
    }

    #[test]
    fn test_window_wraps_midnight() {
        let w = UpdateWindow {
            start: parse_window_start("23:00").unwrap(),
            minutes: 180,
            tz: chrono_tz::UTC,
        };
        let (s, e) = window_bounds(&w, utc("2026-11-25T01:30:00Z")).unwrap();
        assert_eq!(s, utc("2026-11-24T23:00:00Z"));
        assert_eq!(e, utc("2026-11-25T02:00:00Z"));
        assert!(window_bounds(&w, utc("2026-11-25T02:00:00Z")).is_none());
    }

    #[test]
    fn test_window_dst_forward() {
        // 2027-03-28: Amsterdam jumps 02:00 -> 03:00, so 02:30 does not exist.
        let w = UpdateWindow {
            start: parse_window_start("02:30").unwrap(),
            minutes: 60,
            tz: chrono_tz::Europe::Amsterdam,
        };
        // Resolves to 03:30 CEST = 01:30Z.
        let (s, e) = window_bounds(&w, utc("2027-03-28T01:45:00Z")).unwrap();
        assert_eq!(s, utc("2027-03-28T01:30:00Z"));
        assert_eq!(e, utc("2027-03-28T02:30:00Z"));
        assert!(window_bounds(&w, utc("2027-03-28T01:00:00Z")).is_none());
    }

    #[test]
    fn parse_start() {
        assert!(parse_window_start("04:00").is_ok());
        assert!(parse_window_start("25:00").is_err());
    }
}
