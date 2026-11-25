//! Pure policy for automatic app updates: classify a pending chart update and
//! decide whether "now" falls inside the owner's weekly update schedule.

use chrono::{DateTime, Duration, NaiveTime, Timelike, Utc};

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

/// An IANA time zone backed by jiff's bundled tz database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tz(jiff::tz::TimeZone);

impl Tz {
    /// Local `(weekday, hour, seconds into the hour)` at `t`, Monday = 0.
    fn local_slot(&self, t: DateTime<Utc>) -> Option<(usize, usize, i64)> {
        let z = jiff::Timestamp::new(t.timestamp(), 0)
            .ok()?
            .to_zoned(self.0.clone());
        let day = usize::try_from(z.weekday().to_monday_zero_offset()).ok()?;
        let hour = usize::try_from(z.hour()).ok()?;
        let into = i64::from(z.minute()) * 60 + i64::from(z.second());
        Some((day, hour, into))
    }

    /// Start (UTC, whole seconds) of the local hour containing `t`.
    fn hour_start(&self, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let (_, _, into) = self.local_slot(t)?;
        DateTime::from_timestamp(t.timestamp() - into, 0)
    }
}

impl std::str::FromStr for Tz {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        jiff::tz::TimeZone::get(s)
            .map(Tz)
            .map_err(|e| format!("unknown time zone '{s}': {e}"))
    }
}

/// Days per week and hours per day of a [`Schedule`].
pub const DAYS: usize = 7;
pub const HOURS: usize = 24;

/// Weekly hour grid: `0[day][hour]` (Monday = day 0) is true when an
/// automatic update may start during that local hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule(pub [[bool; HOURS]; DAYS]);

impl Schedule {
    /// Parse the stored form: 168 chars of `0`/`1`, index `day * 24 + hour`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let bytes = s.as_bytes();
        if bytes.len() != DAYS * HOURS {
            return Err(format!(
                "expected {} characters, got {}",
                DAYS * HOURS,
                bytes.len()
            ));
        }
        let mut grid = [[false; HOURS]; DAYS];
        for (i, b) in bytes.iter().enumerate() {
            grid[i / HOURS][i % HOURS] = match b {
                b'0' => false,
                b'1' => true,
                _ => return Err(format!("invalid character at index {i}, expected 0 or 1")),
            };
        }
        Ok(Self(grid))
    }

    /// The stored form; inverse of [`Schedule::parse`].
    pub fn format(&self) -> String {
        self.0
            .iter()
            .flatten()
            .map(|&on| if on { '1' } else { '0' })
            .collect()
    }

    /// Build from API rows; exactly 7 rows of 24 hours each.
    pub fn from_rows(rows: &[Vec<bool>]) -> Result<Self, String> {
        if rows.len() != DAYS || rows.iter().any(|r| r.len() != HOURS) {
            return Err(format!("expected {DAYS} days of {HOURS} hours each"));
        }
        let mut grid = [[false; HOURS]; DAYS];
        for (day, row) in rows.iter().enumerate() {
            grid[day].copy_from_slice(row);
        }
        Ok(Self(grid))
    }

    /// The API form: 7 rows (Monday first) of 24 hours.
    pub fn to_rows(&self) -> Vec<Vec<bool>> {
        self.0.iter().map(|r| r.to_vec()).collect()
    }

    /// Convert a legacy daily window (`HH:MM` start, length in minutes) to the
    /// hours it touched on every day. A window past midnight spills into the
    /// next day (Sunday wraps to Monday).
    pub fn from_legacy_window(start: &str, minutes: u32) -> Result<Self, String> {
        let start = NaiveTime::parse_from_str(start, "%H:%M")
            .map_err(|_| format!("invalid window start '{start}', expected HH:MM"))?;
        let begin = start.hour() as usize * 60 + start.minute() as usize;
        let end = begin + minutes as usize;
        let mut grid = [[false; HOURS]; DAYS];
        // Hours from the one holding `begin` up to the one holding `end - 1`.
        for slot in (begin / 60)..end.div_ceil(60) {
            for day in 0..DAYS {
                let index = (day * HOURS + slot) % (DAYS * HOURS);
                grid[index / HOURS][index % HOURS] = true;
            }
        }
        Ok(Self(grid))
    }

    pub fn is_empty(&self) -> bool {
        !self.0.iter().flatten().any(|&on| on)
    }

    /// Whether the local weekday and hour of `now` in `tz` is enabled. A local
    /// hour skipped by DST never occurs; a repeated one is open both times.
    pub fn is_open(&self, tz: &Tz, now: DateTime<Utc>) -> bool {
        tz.local_slot(now)
            .is_some_and(|(day, hour, _)| self.0[day][hour])
    }

    /// When open, the start of the current contiguous open run (looking back
    /// at most 168 hours); otherwise the start of the next enabled local hour
    /// within 8 days. `None` for an empty schedule.
    pub fn next_open(&self, tz: &Tz, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.is_empty() {
            return None;
        }
        let mut cur = tz.hour_start(now)?;
        if self.is_open(tz, now) {
            // Stop after a full week so an all-open schedule still terminates.
            for _ in 1..DAYS * HOURS {
                let prev = tz.hour_start(cur - Duration::seconds(1))?;
                if !self.is_open(tz, prev) {
                    break;
                }
                cur = prev;
            }
            return Some(cur);
        }
        for _ in 0..8 * HOURS {
            let next = tz
                .hour_start(cur + Duration::hours(1))
                .filter(|n| *n > cur)
                .unwrap_or(cur + Duration::hours(1));
            if self.is_open(tz, next) {
                return Some(next);
            }
            cur = next;
        }
        None
    }
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

    fn amsterdam() -> Tz {
        "Europe/Amsterdam".parse().unwrap()
    }

    fn hours(list: &[(usize, usize)]) -> Schedule {
        let mut grid = [[false; HOURS]; DAYS];
        for &(d, h) in list {
            grid[d][h] = true;
        }
        Schedule(grid)
    }

    #[test]
    fn test_parse_format_round_trip() {
        let s = hours(&[(0, 0), (2, 13), (6, 23)]);
        let text = s.format();
        assert_eq!(text.len(), 168);
        assert_eq!(&text[0..1], "1");
        assert_eq!(&text[2 * 24 + 13..2 * 24 + 14], "1");
        assert_eq!(&text[167..], "1");
        assert_eq!(Schedule::parse(&text).unwrap(), s);
        assert!(Schedule::parse("01").is_err());
        assert!(Schedule::parse(&"2".repeat(168)).is_err());
        assert_eq!(Schedule::from_rows(&s.to_rows()).unwrap(), s);
        assert!(Schedule::from_rows(&vec![vec![false; 24]; 6]).is_err());
        assert!(Schedule::from_rows(&vec![vec![false; 23]; 7]).is_err());
    }

    #[test]
    fn test_legacy_conversion() {
        let daily = |hs: &[usize]| {
            let list: Vec<_> = (0..DAYS)
                .flat_map(|d| hs.iter().map(move |&h| (d, h)))
                .collect();
            hours(&list)
        };
        assert_eq!(
            Schedule::from_legacy_window("04:00", 120).unwrap(),
            daily(&[4, 5])
        );
        assert_eq!(
            Schedule::from_legacy_window("04:30", 60).unwrap(),
            daily(&[4, 5])
        );
        // 23:00 for 3 h: hour 23 on day d, hours 0-1 on d+1 (Sunday wraps).
        assert_eq!(
            Schedule::from_legacy_window("23:00", 180).unwrap(),
            daily(&[23, 0, 1])
        );
        let s = Schedule::from_legacy_window("23:00", 180).unwrap();
        assert!(s.0[0][0] && s.0[0][1], "Monday 00-01 from Sunday's window");
        assert!(Schedule::from_legacy_window("4am", 60).is_err());
    }

    #[test]
    fn test_open_and_next_basic() {
        let tz = amsterdam();
        // Wednesday 2026-11-25 is CET (UTC+1). Hours 4-5 on Wednesday (day 2).
        let s = hours(&[(2, 4), (2, 5)]);
        assert!(s.is_open(&tz, utc("2026-11-25T03:30:00Z")));
        assert!(s.is_open(&tz, utc("2026-11-25T04:59:59Z")));
        assert!(!s.is_open(&tz, utc("2026-11-25T05:00:00Z")));
        assert!(!s.is_open(&tz, utc("2026-11-25T02:59:59Z")));
        // Inside: start of the run, even from its second hour.
        assert_eq!(
            s.next_open(&tz, utc("2026-11-25T04:30:00Z")),
            Some(utc("2026-11-25T03:00:00Z"))
        );
        assert_eq!(
            s.next_open(&tz, utc("2026-11-25T01:10:00Z")),
            Some(utc("2026-11-25T03:00:00Z"))
        );
        // After the run: next Wednesday.
        assert_eq!(
            s.next_open(&tz, utc("2026-11-25T05:00:00Z")),
            Some(utc("2026-12-02T03:00:00Z"))
        );
    }

    #[test]
    fn test_midnight_crossing_run() {
        let tz: Tz = "UTC".parse().unwrap();
        // Tuesday 23:00 through Wednesday 01:59.
        let s = hours(&[(1, 23), (2, 0), (2, 1)]);
        let now = utc("2026-11-25T01:30:00Z"); // Wednesday
        assert!(s.is_open(&tz, now));
        assert_eq!(s.next_open(&tz, now), Some(utc("2026-11-24T23:00:00Z")));
        assert!(!s.is_open(&tz, utc("2026-11-25T02:00:00Z")));
    }

    #[test]
    fn test_sunday_to_monday_wrap() {
        let tz: Tz = "UTC".parse().unwrap();
        let s = hours(&[(6, 23), (0, 0)]);
        // Monday 2026-11-30 00:15 belongs to the run started Sunday 23:00.
        let now = utc("2026-11-30T00:15:00Z");
        assert!(s.is_open(&tz, now));
        assert_eq!(s.next_open(&tz, now), Some(utc("2026-11-29T23:00:00Z")));
        // From Monday afternoon the next start is Sunday 23:00.
        assert_eq!(
            s.next_open(&tz, utc("2026-11-30T12:00:00Z")),
            Some(utc("2026-12-06T23:00:00Z"))
        );
    }

    #[test]
    fn test_empty_and_full_schedule() {
        let tz = amsterdam();
        let now = utc("2026-11-25T12:34:56Z");
        let empty = Schedule([[false; HOURS]; DAYS]);
        assert!(empty.is_empty());
        assert!(!empty.is_open(&tz, now));
        assert_eq!(empty.next_open(&tz, now), None);
        let full = Schedule([[true; HOURS]; DAYS]);
        assert!(full.is_open(&tz, now));
        let start = full.next_open(&tz, now).unwrap();
        assert!(start <= now && now - start <= Duration::hours(168));
        assert_eq!(start.timestamp() % 3600, 0);
    }

    #[test]
    fn test_dst_spring_forward() {
        // Sunday 2027-03-28: Amsterdam jumps 02:00 CET -> 03:00 CEST at 01:00Z.
        let tz = amsterdam();
        let skipped = hours(&[(6, 2)]);
        // Local 02:xx never occurs that day: closed across the jump.
        assert!(!skipped.is_open(&tz, utc("2027-03-28T00:59:59Z")));
        assert!(!skipped.is_open(&tz, utc("2027-03-28T01:00:00Z")));
        // The next 02:00 is a week later, CEST: 00:00Z.
        assert_eq!(
            skipped.next_open(&tz, utc("2027-03-27T12:00:00Z")),
            Some(utc("2027-04-04T00:00:00Z"))
        );
        // A run 01-03 local spans 00:00Z..02:00Z (two real hours).
        let run = hours(&[(6, 1), (6, 2), (6, 3)]);
        assert!(run.is_open(&tz, utc("2027-03-28T01:30:00Z")));
        assert_eq!(
            run.next_open(&tz, utc("2027-03-28T01:30:00Z")),
            Some(utc("2027-03-28T00:00:00Z"))
        );
        assert!(!run.is_open(&tz, utc("2027-03-28T02:00:00Z")));
        let three = hours(&[(6, 3)]);
        assert_eq!(
            three.next_open(&tz, utc("2027-03-27T23:00:00Z")),
            Some(utc("2027-03-28T01:00:00Z"))
        );
    }

    #[test]
    fn test_dst_fall_back() {
        // Sunday 2026-10-25: 03:00 CEST -> 02:00 CET at 01:00Z; 02:xx repeats.
        let tz = amsterdam();
        let s = hours(&[(6, 2)]);
        assert!(s.is_open(&tz, utc("2026-10-25T00:30:00Z")), "first 02:30");
        assert!(s.is_open(&tz, utc("2026-10-25T01:30:00Z")), "second 02:30");
        assert!(!s.is_open(&tz, utc("2026-10-25T02:00:00Z")));
        // Both repeats form one run starting at the first.
        assert_eq!(
            s.next_open(&tz, utc("2026-10-25T01:30:00Z")),
            Some(utc("2026-10-25T00:00:00Z"))
        );
        assert_eq!(
            s.next_open(&tz, utc("2026-10-24T12:00:00Z")),
            Some(utc("2026-10-25T00:00:00Z"))
        );
    }
}
