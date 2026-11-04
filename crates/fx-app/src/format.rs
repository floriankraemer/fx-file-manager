//! Pure display helpers: sizes, timestamps, relative ages, extensions.

use crate::i18n::{Language, current, find_in};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const KIB: f64 = 1024.0;
const SIZE_UNITS: [&str; 5] = ["unit.kb", "unit.mb", "unit.gb", "unit.tb", "unit.pb"];

/// How `format_time` writes dates. Stored by [`set_date_format`] so call sites stay unchanged.
static ISO_DATES: AtomicBool = AtomicBool::new(false);

pub fn set_date_format(format: crate::settings::DateFormat) {
    ISO_DATES.store(
        format == crate::settings::DateFormat::Iso,
        Ordering::Relaxed,
    );
}

/// Decimal separator and digit group separator of `lang`.
// ponytail: table for the six supported languages; swap for ICU4X if many more are added.
fn separators(lang: Language) -> (char, char) {
    match lang {
        Language::De | Language::Es => (',', '.'),
        Language::Fr | Language::Uk => (',', '\u{a0}'),
        _ => ('.', ','),
    }
}

/// `n` with digit groups, e.g. `12,345` or `12.345`.
pub fn format_number(n: u64) -> String {
    format_number_in(current(), n)
}

pub(crate) fn format_number_in(lang: Language, n: u64) -> String {
    let (_, group) = separators(lang);
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(group);
        }
        out.push(c);
    }
    out
}

/// `value` with one decimal and the language's decimal separator.
fn decimal(lang: Language, value: f64) -> String {
    let text = format!("{value:.1}");
    text.replace('.', &separators(lang).0.to_string())
}

const SECS_PER_MINUTE: u64 = 60;
const SECS_PER_HOUR: u64 = 60 * SECS_PER_MINUTE;
const SECS_PER_DAY: u64 = 24 * SECS_PER_HOUR;
const SECS_PER_WEEK: u64 = 7 * SECS_PER_DAY;
const SECS_PER_MONTH: u64 = 30 * SECS_PER_DAY;
const SECS_PER_YEAR: u64 = 365 * SECS_PER_DAY;

/// Binary units, one decimal from KB up. Rounding never shows "1024.0 KB": it promotes to the next unit.
pub fn format_size(bytes: u64) -> String {
    size_in(current(), bytes)
}

fn unit(lang: Language, key: &'static str) -> &'static str {
    find_in(lang, key).unwrap_or(key)
}

fn size_in(lang: Language, bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} {}", unit(lang, "unit.b"));
    }
    let mut value = bytes as f64 / KIB;
    let mut unit = 0;
    while (value * 10.0).round() >= 1024.0 * 10.0 && unit + 1 < SIZE_UNITS.len() {
        value /= KIB;
        unit += 1;
    }
    format!(
        "{} {}",
        decimal(lang, value),
        self::unit(lang, SIZE_UNITS[unit])
    )
}

/// Milliseconds with one decimal below 10 ms (`"0.4 ms"`), whole milliseconds from there up.
pub fn format_millis(d: std::time::Duration) -> String {
    millis_in(current(), d)
}

fn millis_in(lang: Language, d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    let value = if ms < 9.95 {
        decimal(lang, ms)
    } else {
        format!("{ms:.0}")
    };
    format!("{value} {}", unit(lang, "unit.ms"))
}

/// Date in the language's order plus `HH:MM`, or `YYYY-MM-DD HH:MM` when the date format is ISO.
// ponytail: UTC; local tz when a time crate is added
pub fn format_time(time: SystemTime) -> String {
    let lang = if ISO_DATES.load(Ordering::Relaxed) {
        Language::En
    } else {
        current()
    };
    time_in(lang, time)
}

fn time_in(lang: Language, time: SystemTime) -> String {
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
    let date = match lang {
        Language::De | Language::Uk => format!("{day:02}.{month:02}.{year:04}"),
        Language::Es | Language::Fr => format!("{day:02}/{month:02}/{year:04}"),
        Language::Zh => format!("{year}年{month}月{day}日"),
        _ => format!("{year:04}-{month:02}-{day:02}"),
    };
    format!("{date} {hour:02}:{minute:02}")
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

/// Compact age such as `"5m"`, `"3h"`, `"4d"`, `"2w"`, `"6mo"`, `"2y"` (English; other languages use their own units).
/// Future times (clock skew) read as `"0m"`. A missing time yields `(Years, "")`; callers should show nothing for an empty string.
pub fn age(modified: Option<SystemTime>, now: SystemTime) -> (AgeBucket, String) {
    let Some(modified) = modified else {
        return (AgeBucket::Years, String::new());
    };
    let secs = now.duration_since(modified).map_or(0, |d| d.as_secs());
    let (bucket, unit, key) = match secs {
        s if s < SECS_PER_HOUR => (AgeBucket::Minutes, SECS_PER_MINUTE, "age.minutes"),
        s if s < SECS_PER_DAY => (AgeBucket::Hours, SECS_PER_HOUR, "age.hours"),
        s if s < SECS_PER_WEEK => (AgeBucket::Days, SECS_PER_DAY, "age.days"),
        s if s < SECS_PER_MONTH => (AgeBucket::Weeks, SECS_PER_WEEK, "age.weeks"),
        s if s < SECS_PER_YEAR => (AgeBucket::Months, SECS_PER_MONTH, "age.months"),
        _ => (AgeBucket::Years, SECS_PER_YEAR, "age.years"),
    };
    (bucket, crate::tr(key, &[("n", &(secs / unit).to_string())]))
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

    #[test]
    fn sizes_numbers_dates_per_locale() {
        let t = UNIX_EPOCH + Duration::from_secs(1_793_802_600); // 2026-11-04 14:30 UTC
        // (language, 1536 bytes, 512 bytes, 12345, date, 0.4 ms)
        let table = [
            (
                Language::En,
                "1.5 KB",
                "512 B",
                "12,345",
                "2026-11-04 14:30",
                "0.4 ms",
            ),
            (
                Language::De,
                "1,5 KB",
                "512 B",
                "12.345",
                "04.11.2026 14:30",
                "0,4 ms",
            ),
            (
                Language::Es,
                "1,5 KB",
                "512 B",
                "12.345",
                "04/11/2026 14:30",
                "0,4 ms",
            ),
            (
                Language::Fr,
                "1,5 Ko",
                "512 o",
                "12\u{a0}345",
                "04/11/2026 14:30",
                "0,4 ms",
            ),
            (
                Language::Uk,
                "1,5 КБ",
                "512 Б",
                "12\u{a0}345",
                "04.11.2026 14:30",
                "0,4 мс",
            ),
            (
                Language::Zh,
                "1.5 KB",
                "512 字节",
                "12,345",
                "2026年11月4日 14:30",
                "0.4 毫秒",
            ),
        ];
        for (lang, kb, b, n, date, ms) in table {
            assert_eq!(size_in(lang, 1536), kb, "{lang:?}");
            assert_eq!(size_in(lang, 512), b, "{lang:?}");
            assert_eq!(format_number_in(lang, 12_345), n, "{lang:?}");
            assert_eq!(time_in(lang, t), date, "{lang:?}");
            assert_eq!(millis_in(lang, Duration::from_micros(400)), ms, "{lang:?}");
        }
        assert_eq!(format_number_in(Language::En, 999), "999");
        assert_eq!(format_number_in(Language::En, 1_000_000), "1,000,000");
    }
}
