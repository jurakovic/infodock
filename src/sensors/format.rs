//! Formatting for sensor text (SPEC §8.6): sizes, rates, durations and percentages. Words and
//! units come from the catalog (with plural forms); the decimal separator from the locale.

use crate::i18n::{t, t_args, t_count};
use crate::platform::Locale;

/// How numbers are written: the locale's decimal separator (SPEC §8.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Numbers {
    pub decimal: char,
}

impl From<&Locale> for Numbers {
    /// How `locale` writes numbers.
    fn from(locale: &Locale) -> Self {
        Self {
            decimal: locale.decimal,
        }
    }
}

impl Default for Numbers {
    fn default() -> Self {
        Self { decimal: '.' }
    }
}

impl Numbers {
    /// `value` with `digits` decimals.
    pub fn fixed(&self, value: f64, digits: usize) -> String {
        let text = format!("{value:.digits$}");
        if self.decimal == '.' {
            text
        } else {
            text.replace('.', &self.decimal.to_string())
        }
    }
}

/// Units of 1024ⁿ bytes, from KB.
const SIZE_UNITS: [&str; 4] = ["format.kb", "format.mb", "format.gb", "format.tb"];
/// Units of 1024ⁿ bytes per second, from B/s.
const RATE_UNITS: [&str; 5] = [
    "format.rate.b",
    "format.rate.kb",
    "format.rate.mb",
    "format.rate.gb",
    "format.rate.tb",
];

fn rounded(value: f64, digits: usize) -> f64 {
    let factor = 10f64.powi(digits as i32);
    (value * factor).round() / factor
}

/// `n` divided by 1024 for each unit it reaches (0: plain bytes … 4: TB); a value that would
/// round to 1024 moves on, so it reads `1.00 MB`, not `1024 KB`.
fn scaled(n: f64) -> (f64, usize) {
    let (mut value, mut unit) = (n, 0);
    while value.round() >= 1024.0 && unit < 4 {
        value /= 1024.0;
        unit += 1;
    }
    (value, unit)
}

/// The automatic number of decimals (SPEC §8.6): two below 10, one below 100, else none, judged
/// after rounding (9.999 is `10.0`, not `10.00`).
fn auto_digits(value: f64) -> usize {
    if value == 0.0 {
        0
    } else if rounded(value, 2) < 10.0 {
        2
    } else if rounded(value, 1) < 100.0 {
        1
    } else {
        0
    }
}

/// A size in bytes, base 1024 (SPEC §8.6): `0 bytes`, `1 byte`, `512 bytes`, `1.50 KB`,
/// `12.3 MB`, `431 GB`. `digits` fixes the decimals of KB and above; plain bytes have none.
pub fn bytes(n: u64, digits: Option<usize>, numbers: Numbers) -> String {
    let (value, unit) = scaled(n as f64);
    if unit == 0 {
        return t_count("format.bytes", n, &[]);
    }
    let digits = digits.unwrap_or_else(|| auto_digits(value));
    t_args(
        SIZE_UNITS[unit - 1],
        &[("value", &numbers.fixed(value, digits))],
    )
}

/// A rate in bytes per second, scaled like [`bytes`]: `512 B/s`, `86.0 KB/s`, `1.24 MB/s`.
pub fn rate(bytes_per_second: f64, numbers: Numbers) -> String {
    let (value, unit) = scaled(bytes_per_second.max(0.0));
    let digits = if unit == 0 { 0 } else { auto_digits(value) };
    t_args(
        RATE_UNITS[unit],
        &[("value", &numbers.fixed(value, digits))],
    )
}

/// Days, hours and minutes of `seconds`.
fn parts(seconds: u64) -> (u64, u64, u64) {
    (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60)
}

/// A compact duration (Uptime, SPEC §9.6): `3d 04h 12m`, `4h 05m`, `12m`. The leading unit is
/// not padded, the ones after it are.
pub fn duration_compact(seconds: u64) -> String {
    let (days, hours, minutes) = parts(seconds);
    let mut out = Vec::new();
    if days > 0 {
        out.push(t_args("format.short.days", &[("value", &days.to_string())]));
    }
    if days > 0 || hours > 0 {
        let value = if out.is_empty() {
            hours.to_string()
        } else {
            format!("{hours:02}")
        };
        out.push(t_args("format.short.hours", &[("value", &value)]));
    }
    let value = if out.is_empty() {
        minutes.to_string()
    } else {
        format!("{minutes:02}")
    };
    out.push(t_args("format.short.minutes", &[("value", &value)]));
    out.join(" ")
}

/// A duration in words (SPEC §8.6): `3 days, 4 hours, 12 minutes`; parts that are zero are left
/// out, and less than a minute is `0 minutes`.
pub fn duration_long(seconds: u64) -> String {
    let (days, hours, minutes) = parts(seconds);
    let words: Vec<String> = [
        (days, "format.days"),
        (hours, "format.hours"),
        (minutes, "format.minutes"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, key)| t_count(key, n, &[]))
    .collect();
    if words.is_empty() {
        return t_count("format.minutes", 0, &[]);
    }
    words.join(&t("format.listSeparator"))
}

/// Hours and minutes as `h:mm` (battery times, SPEC §9.14).
pub fn hours_minutes(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 3600, seconds / 60 % 60)
}

/// A whole percentage: `37%`.
pub fn percent(value: f64) -> String {
    // `+ 0.0` turns -0 into 0, which would print as `-0`.
    let value = if value.is_finite() {
        value.round() + 0.0
    } else {
        0.0
    };
    t_args("format.percent", &[("value", &format!("{value:.0}"))])
}

/// A percentage with `digits` decimals: `37.0%`.
pub fn percent_digits(value: f64, digits: usize, numbers: Numbers) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    t_args(
        "format.percent",
        &[("value", &numbers.fixed(value, digits))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    fn b(n: u64) -> String {
        bytes(n, None, Numbers::default())
    }

    #[test]
    fn sizes_use_base_1024_and_auto_digits() {
        // The spec's examples.
        assert_eq!(b(0), "0 bytes");
        assert_eq!(b(512), "512 bytes");
        assert_eq!(b(1536), "1.50 KB");
        assert_eq!(b(12 * MB + 3 * MB / 10), "12.3 MB");
        assert_eq!(b(431 * GB), "431 GB");
        assert_eq!(b(1), "1 byte");
        assert_eq!(b(1023), "1023 bytes");
        assert_eq!(b(KB), "1.00 KB");
        // Rounding up to the next unit, or to the next number of decimals.
        assert_eq!(b(MB - 1), "1.00 MB");
        assert_eq!(b(10 * KB - 1), "10.0 KB");
        assert_eq!(b(5 * 1024 * GB), "5.00 TB");
        assert_eq!(b(5000 * 1024 * GB), "5000 TB");
        assert_eq!(bytes(1536, Some(1), Numbers::default()), "1.5 KB");
        assert_eq!(bytes(1536, None, Numbers { decimal: ',' }), "1,50 KB");
    }

    #[test]
    fn rates_scale_like_sizes() {
        let r = |n: f64| rate(n, Numbers::default());
        assert_eq!(r(0.0), "0 B/s");
        assert_eq!(r(512.4), "512 B/s");
        assert_eq!(r(86.0 * KB as f64), "86.0 KB/s");
        assert_eq!(r(1.24 * MB as f64), "1.24 MB/s");
        assert_eq!(r(1023.0 * MB as f64), "1023 MB/s");
        assert_eq!(r(-5.0), "0 B/s");
    }

    #[test]
    fn durations() {
        let secs = |d: u64, h: u64, m: u64| d * 86_400 + h * 3600 + m * 60;
        assert_eq!(duration_compact(secs(3, 4, 12)), "3d 04h 12m");
        assert_eq!(duration_compact(secs(0, 4, 5)), "4h 05m");
        assert_eq!(duration_compact(secs(0, 0, 12) + 59), "12m");
        assert_eq!(duration_compact(secs(2, 0, 0)), "2d 00h 00m");

        assert_eq!(duration_long(secs(3, 4, 12)), "3 days, 4 hours, 12 minutes");
        assert_eq!(duration_long(secs(12, 1, 3)), "12 days, 1 hour, 3 minutes");
        assert_eq!(duration_long(secs(1, 0, 0)), "1 day");
        assert_eq!(duration_long(secs(0, 0, 1)), "1 minute");
        assert_eq!(duration_long(30), "0 minutes");

        assert_eq!(hours_minutes(secs(0, 1, 5)), "1:05");
    }

    #[test]
    fn percentages() {
        assert_eq!(percent(37.4), "37%");
        assert_eq!(percent(99.5), "100%");
        assert_eq!(percent(f64::NAN), "0%");
        assert_eq!(percent(-0.3), "0%", "not -0%");
        assert_eq!(percent_digits(37.04, 1, Numbers::default()), "37.0%");
        assert_eq!(percent_digits(37.04, 1, Numbers { decimal: ',' }), "37,0%");
    }
}
