//! Calendar dates for the Date and Time sensors (SPEC §8.6, §9.4–9.5): weekdays, the day of the
//! year, ISO 8601 week numbers and the locale's date patterns. Pure.

use crate::platform::Locale;

/// A day in the proleptic Gregorian calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The number of days in `month` (1–12) of `year`.
pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl Date {
    /// The date, if it exists.
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        ((1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month))
            .then_some(Self { year, month, day })
    }

    /// Days since 1970-01-01 (H. Hinnant's `days_from_civil`).
    pub fn days(self) -> i64 {
        let (m, d) = (i64::from(self.month), i64::from(self.day));
        let y = i64::from(self.year) - i64::from(m <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// The date `days` after 1970-01-01 (`civil_from_days`).
    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Self { year, month, day }
    }

    pub fn add_days(self, days: i64) -> Self {
        Self::from_days(self.days() + days)
    }

    /// 0 = Monday … 6 = Sunday.
    pub fn weekday(self) -> u8 {
        // 1970-01-01 was a Thursday.
        (self.days() + 3).rem_euclid(7) as u8
    }

    /// 1 = January 1st.
    pub fn day_of_year(self) -> u16 {
        (self.days() - Self::first_of_year(self.year) + 1) as u16
    }

    fn first_of_year(year: i32) -> i64 {
        Self {
            year,
            month: 1,
            day: 1,
        }
        .days()
    }

    /// The ISO 8601 week-numbering year and week (1–53): weeks start on Monday, and week 1 is
    /// the one with the year's first Thursday.
    pub fn iso_week(self) -> (i32, u8) {
        let thursday = self.add_days(3 - i64::from(self.weekday()));
        let week = (thursday.day_of_year() - 1) / 7 + 1;
        (thursday.year, week as u8)
    }
}

/// `date` written with `pattern`, a date pattern in the Windows picture format (SPEC §8.6):
/// `d`/`dd` the day, `ddd`/`dddd` the weekday's short or long name, `M`/`MM` the month,
/// `MMM`/`MMMM` its short or long name, `y`/`yy` the year's last two digits, `yyy`/`yyyy` the
/// full year, `'…'` literal text (`''` a quote); era marks (`g`) are left out and anything else
/// is copied.
pub fn format_date(pattern: &str, date: Date, locale: &Locale) -> String {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                out.push('\'');
                continue;
            }
            for c in chars.by_ref() {
                if c == '\'' {
                    break;
                }
                out.push(c);
            }
            continue;
        }
        if !matches!(c, 'd' | 'M' | 'y' | 'g') {
            out.push(c);
            continue;
        }
        let mut run = 1;
        while chars.peek() == Some(&c) {
            chars.next();
            run += 1;
        }
        let weekday = usize::from(date.weekday());
        let month = usize::from(date.month - 1);
        match (c, run) {
            ('d', 1) => out.push_str(&date.day.to_string()),
            ('d', 2) => out.push_str(&format!("{:02}", date.day)),
            ('d', 3) => out.push_str(&locale.weekdays_short[weekday]),
            ('d', _) => out.push_str(&locale.weekdays[weekday]),
            ('M', 1) => out.push_str(&date.month.to_string()),
            ('M', 2) => out.push_str(&format!("{:02}", date.month)),
            ('M', 3) => out.push_str(&locale.months_short[month]),
            ('M', _) => out.push_str(&locale.months[month]),
            ('y', 1) => out.push_str(&date.year.rem_euclid(100).to_string()),
            ('y', 2) => out.push_str(&format!("{:02}", date.year.rem_euclid(100))),
            ('y', _) => out.push_str(&date.year.to_string()),
            _ => {}
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u8, day: u8) -> Date {
        Date::new(year, month, day).expect("a real date")
    }

    #[test]
    fn days_round_trip() {
        assert_eq!(date(1970, 1, 1).days(), 0);
        assert_eq!(date(2000, 3, 1).days(), 11_017);
        assert_eq!(date(1969, 12, 31).days(), -1);
        for days in [-800_000, -1, 0, 59, 10_957, 20_000, 2_932_896] {
            assert_eq!(Date::from_days(days).days(), days);
        }
        assert_eq!(date(2026, 12, 31).add_days(1), date(2027, 1, 1));
        assert_eq!(date(2024, 2, 28).add_days(1), date(2024, 2, 29));
        assert!(Date::new(2100, 2, 29).is_none());
        assert!(Date::new(2000, 2, 29).is_some());
        assert!(Date::new(2026, 13, 1).is_none());
    }

    #[test]
    fn weekdays_and_days_of_the_year() {
        assert_eq!(date(1970, 1, 1).weekday(), 3, "Thursday");
        assert_eq!(date(2026, 10, 11).weekday(), 6, "Sunday");
        assert_eq!(date(2026, 10, 13).weekday(), 1, "Tuesday");
        assert_eq!(date(2026, 1, 1).day_of_year(), 1);
        assert_eq!(date(2026, 12, 31).day_of_year(), 365);
        assert_eq!(date(2024, 12, 31).day_of_year(), 366);
    }

    #[test]
    fn iso_weeks() {
        assert_eq!(date(2021, 1, 3).iso_week(), (2020, 53));
        assert_eq!(date(2021, 1, 4).iso_week(), (2021, 1));
        assert_eq!(date(2024, 12, 30).iso_week(), (2025, 1));
        // 2026 starts on a Thursday, so it has 53 weeks.
        assert_eq!(date(2026, 1, 1).iso_week(), (2026, 1));
        assert_eq!(date(2026, 12, 31).iso_week(), (2026, 53));
        assert_eq!(date(2026, 10, 13).iso_week(), (2026, 42));
    }

    fn german() -> Locale {
        let names =
            |items: &[&str]| -> Vec<String> { items.iter().map(|s| (*s).to_owned()).collect() };
        let mut de = Locale::en_us();
        de.weekdays = names(&[
            "Montag",
            "Dienstag",
            "Mittwoch",
            "Donnerstag",
            "Freitag",
            "Samstag",
            "Sonntag",
        ])
        .try_into()
        .expect("7 names");
        de.months[9] = "Oktober".into();
        de.long_date = "dddd, d. MMMM yyyy".into();
        de.short_date = "dd.MM.yyyy".into();
        de
    }

    #[test]
    fn formats_with_the_locales_patterns() {
        let en = Locale::en_us();
        let day = date(2026, 10, 11);
        assert_eq!(
            format_date(&en.long_date, day, &en),
            "Sunday, October 11, 2026"
        );
        assert_eq!(format_date(&en.short_date, day, &en), "10/11/2026");
        assert_eq!(format_date(&en.month_day, day, &en), "October 11");
        assert_eq!(format_date("ddd, MMM d yy", day, &en), "Sun, Oct 11 26");
        assert_eq!(format_date("y", date(2005, 1, 2), &en), "5");

        let de = german();
        assert_eq!(
            format_date(&de.long_date, day, &de),
            "Sonntag, 11. Oktober 2026"
        );
        assert_eq!(
            format_date(&de.short_date, date(2026, 3, 4), &de),
            "04.03.2026"
        );

        // Spanish-style literals, a doubled quote, and an era mark that is left out.
        assert_eq!(
            format_date("d 'de' MMMM 'de' yyyy", day, &en),
            "11 de October de 2026"
        );
        assert_eq!(format_date("d''M", day, &en), "11'10");
        assert_eq!(format_date("gg yyyy", day, &en), "2026");
    }
}
