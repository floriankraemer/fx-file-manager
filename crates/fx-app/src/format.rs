//! Pure display helpers: sizes, timestamps, relative ages, extensions.

use std::time::{SystemTime, UNIX_EPOCH};

const KIB: f64 = 1024.0;
const SIZE_UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];

const SECS_PER_MINUTE: u64 = 60;
const SECS_PER_HOUR: u64 = 60 * SECS_PER_MINUTE;
const SECS_PER_DAY: u64 = 24 * SECS_PER_HOUR;
const SECS_PER_WEEK: u64 = 7 * SECS_PER_DAY;
const SECS_PER_MONTH: u64 = 30 * SECS_PER_DAY;
const SECS_PER_YEAR: u64 = 365 * SECS_PER_DAY;

/// Binary units, one decimal from KB up. Rounding never shows "1024.0 KB": it promotes to the next unit.
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / KIB;
    let mut unit = 0;
    while (value * 10.0).round() >= 1024.0 * 10.0 && unit + 1 < SIZE_UNITS.len() {
        value /= KIB;
        unit += 1;
    }
    format!("{value:.1} {}", SIZE_UNITS[unit])
}

/// Milliseconds with one decimal below 10 ms (`"0.4 ms"`), whole milliseconds from there up.
pub fn format_millis(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 9.95 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.0} ms")
    }
}

/// `YYYY-MM-DD HH:MM`.
// ponytail: UTC; local tz when a time crate is added
pub fn format_time(time: SystemTime) -> String {
    let secs = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        // Floor for pre-epoch times, so the minute never rounds toward zero.
        Err(e) => {
            let d = e.duration();
            -(d.as_secs() as i64) - i64::from(d.subsec_nanos() > 0)
        }
    };
    let (year, month, day) = civil_from_days(secs.div_euclid(SECS_PER_DAY as i64));
    let secs_of_day = secs.rem_euclid(SECS_PER_DAY as i64) as u64;
    let (hour, minute) = (
        secs_of_day / SECS_PER_HOUR,
        secs_of_day % SECS_PER_HOUR / SECS_PER_MINUTE,
    );
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar.
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgeBucket {
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
}

/// Compact age such as `"5m"`, `"3h"`, `"4d"`, `"2w"`, `"6mo"`, `"2y"`.
/// Future times (clock skew) read as `"0m"`. A missing time yields `(Years, "")`; callers should show nothing for an empty string.
pub fn age(modified: Option<SystemTime>, now: SystemTime) -> (AgeBucket, String) {
    let Some(modified) = modified else {
        return (AgeBucket::Years, String::new());
    };
    let secs = now.duration_since(modified).map_or(0, |d| d.as_secs());
    let (bucket, unit, suffix) = match secs {
        s if s < SECS_PER_HOUR => (AgeBucket::Minutes, SECS_PER_MINUTE, "m"),
        s if s < SECS_PER_DAY => (AgeBucket::Hours, SECS_PER_HOUR, "h"),
        s if s < SECS_PER_WEEK => (AgeBucket::Days, SECS_PER_DAY, "d"),
        s if s < SECS_PER_MONTH => (AgeBucket::Weeks, SECS_PER_WEEK, "w"),
        s if s < SECS_PER_YEAR => (AgeBucket::Months, SECS_PER_MONTH, "mo"),
        _ => (AgeBucket::Years, SECS_PER_YEAR, "y"),
    };
    (bucket, format!("{}{suffix}", secs / unit))
}

/// Text after the last dot, or `""` when there is none. Leading-dot names such as `.bashrc` have no extension.
pub fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[dot + 1..],
        _ => "",
    }
}

/// Number of characters before the extension dot (the whole name if there is none); what a rename dialog pre-selects.
pub fn stem_chars(name: &str) -> usize {
    let stem = &name[..name.len() - extension(name).len()];
    let stem = stem.strip_suffix('.').unwrap_or(stem);
    stem.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stem_excludes_the_extension() {
        assert_eq!(stem_chars("report.final.txt"), 12);
        assert_eq!(stem_chars("äb.rs"), 2);
        assert_eq!(stem_chars(".bashrc"), 7);
        assert_eq!(stem_chars("Makefile"), 8);
    }

    #[test]
    fn millis_one_decimal_below_ten() {
        assert_eq!(format_millis(Duration::from_micros(400)), "0.4 ms");
        assert_eq!(format_millis(Duration::from_micros(9_940)), "9.9 ms");
        assert_eq!(format_millis(Duration::from_micros(9_960)), "10 ms");
        assert_eq!(format_millis(Duration::from_millis(123)), "123 ms");
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(24_534_000), "23.4 MB");
        assert_eq!(format_size(1_048_575), "1.0 MB");
    }

    #[test]
    fn times_match_known_epoch_values() {
        assert_eq!(format_time(UNIX_EPOCH), "1970-01-01 00:00");
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(format_time(t), "2023-11-14 22:13");
        let before = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(format_time(before), "1969-12-31 23:59");
    }

    #[test]
    fn age_buckets() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let ago = |secs: u64| Some(now - Duration::from_secs(secs));
        let day = 86_400;
        assert_eq!(age(ago(5 * 60), now), (AgeBucket::Minutes, "5m".into()));
        assert_eq!(age(ago(3 * 3600), now), (AgeBucket::Hours, "3h".into()));
        assert_eq!(age(ago(4 * day), now), (AgeBucket::Days, "4d".into()));
        assert_eq!(age(ago(14 * day), now), (AgeBucket::Weeks, "2w".into()));
        assert_eq!(age(ago(180 * day), now), (AgeBucket::Months, "6mo".into()));
        assert_eq!(age(ago(730 * day), now), (AgeBucket::Years, "2y".into()));
        assert_eq!(age(Some(now + Duration::from_secs(60)), now).1, "0m");
        assert_eq!(age(None, now).1, "");
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("archive.tar.gz"), "gz");
        assert_eq!(extension(".bashrc"), "");
        assert_eq!(extension("README"), "");
    }
}
