//! Calendar dates and the local wall-clock time (SPEC §8.6, §9.4–9.5) as plain data, as the
//! platform reads them (`Platform::local_time`): days, weekdays, the day of the year and ISO
//! 8601 week numbers. Pure; formatting with the locale's patterns is `sensors::dates`.

use std::time::Duration;

/// Wake-ups aim this far past a second or minute boundary, so a timer that fires a little
/// early still finds the new second.
const BOUNDARY_MARGIN: Duration = Duration::from_millis(20);

/// A day in the proleptic Gregorian calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

fn is_leap(year: i32) -> bool {
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

    fn add_days(self, days: i64) -> Self {
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

/// A local wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub date: Date,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millis: u16,
}

impl LocalTime {
    /// The UTC time `seconds` and `millis` after 1970-01-01, for platforms that do not know
    /// the time zone.
    pub fn from_unix(seconds: i64, millis: u16) -> Self {
        let in_day = seconds.rem_euclid(86_400);
        Self {
            date: Date::from_days(seconds.div_euclid(86_400)),
            hour: (in_day / 3600) as u8,
            minute: (in_day / 60 % 60) as u8,
            second: (in_day % 60) as u8,
            millis: millis.min(999),
        }
    }

    /// How long until just after the next second starts (SPEC §9.5: aligned, no polling).
    pub fn until_next_second(&self) -> Duration {
        Duration::from_millis(u64::from(1000 - self.millis.min(999))) + BOUNDARY_MARGIN
    }

    /// How long until just after the next minute starts.
    pub fn until_next_minute(&self) -> Duration {
        let elapsed = u64::from(self.second.min(59)) * 1000 + u64::from(self.millis.min(999));
        Duration::from_millis(60_000 - elapsed) + BOUNDARY_MARGIN
    }
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
    fn local_times_and_boundaries() {
        let t = LocalTime::from_unix(date(2026, 10, 13).days() * 86_400 + 3723, 250);
        assert_eq!(t.date, date(2026, 10, 13));
        assert_eq!((t.hour, t.minute, t.second, t.millis), (1, 2, 3, 250));
        assert_eq!(t.until_next_second(), Duration::from_millis(770));
        assert_eq!(t.until_next_minute(), Duration::from_millis(56_770));
        let before_1970 = LocalTime::from_unix(-1, 0);
        assert_eq!(before_1970.date, date(1969, 12, 31));
        assert_eq!(before_1970.second, 59);
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
}
