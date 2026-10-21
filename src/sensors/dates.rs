//! Calendar dates for the Date and Time sensors (SPEC §8.6, §9.4–9.5): weekdays, the day of the
//! year, ISO 8601 week numbers and the locale's date patterns. Pure.

use std::time::Duration;

use crate::platform::Locale;

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

/// A piece of a date pattern: text to copy, or a run of one field letter.
enum Piece {
    Text(String),
    Field(char, usize),
}

/// `pattern` in pieces: `'…'` literal text (`''` a quote), runs of `d`, `M`, `y` and `g`, and
/// anything else copied.
fn pieces(pattern: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                text.push('\'');
                continue;
            }
            for c in chars.by_ref() {
                if c == '\'' {
                    break;
                }
                text.push(c);
            }
            continue;
        }
        if !matches!(c, 'd' | 'M' | 'y' | 'g') {
            text.push(c);
            continue;
        }
        let mut run = 1;
        while chars.peek() == Some(&c) {
            chars.next();
            run += 1;
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut text)));
        }
        pieces.push(Piece::Field(c, run));
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    pieces
}

/// Whether the month name at `pieces[at]` is written in the genitive, as `GetDateFormatEx`
/// writes it: the nearest day before it is a day number (`d`, `dd`), or else the nearest day
/// after it is (a weekday name, `ddd` or `dddd`, does not count).
fn genitive(pieces: &[Piece], at: usize) -> bool {
    let day = |piece: &Piece| match piece {
        Piece::Field('d', run) => Some(*run <= 2),
        _ => None,
    };
    pieces[..at].iter().rev().find_map(day) == Some(true)
        || pieces[at + 1..].iter().find_map(day) == Some(true)
}

/// `date` written with `pattern`, a date pattern in the Windows picture format (SPEC §8.6):
/// `d`/`dd` the day, `ddd`/`dddd` the weekday's short or long name, `M`/`MM` the month,
/// `MMM`/`MMMM` its short or long name (declined next to a day number, see [`genitive`]),
/// `y`/`yy` the year's last two digits, `yyy`/`yyyy` the full year, `'…'` literal text (`''` a
/// quote); era marks (`g`) are left out and anything else is copied.
pub fn format_date(pattern: &str, date: Date, locale: &Locale) -> String {
    let pieces = pieces(pattern);
    let weekday = usize::from(date.weekday());
    let month = usize::from(date.month - 1);
    let mut out = String::new();
    for (i, piece) in pieces.iter().enumerate() {
        let (c, run) = match piece {
            Piece::Text(text) => {
                out.push_str(text);
                continue;
            }
            Piece::Field(c, run) => (*c, *run),
        };
        match (c, run) {
            ('d', 1) => out.push_str(&date.day.to_string()),
            ('d', 2) => out.push_str(&format!("{:02}", date.day)),
            ('d', 3) => out.push_str(&locale.weekdays_short[weekday]),
            ('d', _) => out.push_str(&locale.weekdays[weekday]),
            ('M', 1) => out.push_str(&date.month.to_string()),
            ('M', 2) => out.push_str(&format!("{:02}", date.month)),
            ('M', 3) if genitive(&pieces, i) => out.push_str(&locale.months_short_genitive[month]),
            ('M', 3) => out.push_str(&locale.months_short[month]),
            ('M', _) if genitive(&pieces, i) => out.push_str(&locale.months_genitive[month]),
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
        // German does not decline them: Windows gives the same names.
        de.months_genitive = de.months.clone();
        de.long_date = "dddd, d. MMMM yyyy".into();
        de.short_date = "dd.MM.yyyy".into();
        de
    }

    /// Month names as Windows gives them for `hr-HR` and `pl-PL`, with its long date and
    /// month-day patterns.
    fn declining(code: &str) -> Locale {
        let names = |s: &str| -> [String; 12] {
            let names: Vec<String> = s.split(' ').map(str::to_owned).collect();
            names.try_into().expect("12 names")
        };
        let mut locale = Locale::en_us();
        match code {
            "hr-HR" => {
                locale.months = names(
                    "siječanj veljača ožujak travanj svibanj lipanj srpanj kolovoz rujan \
                     listopad studeni prosinac",
                );
                locale.months_genitive = names(
                    "siječnja veljače ožujka travnja svibnja lipnja srpnja kolovoza rujna \
                     listopada studenoga prosinca",
                );
                locale.months_short = names("sij velj ožu tra svi lip srp kol ruj lis stu pro");
                locale.months_short_genitive = locale.months_short.clone();
                locale.weekdays[2] = "srijeda".into();
                locale.long_date = "d. MMMM yyyy.".into();
                locale.month_day = "d. MMMM".into();
            }
            "pl-PL" => {
                locale.months = names(
                    "styczeń luty marzec kwiecień maj czerwiec lipiec sierpień wrzesień \
                     październik listopad grudzień",
                );
                locale.months_genitive = names(
                    "stycznia lutego marca kwietnia maja czerwca lipca sierpnia września \
                     października listopada grudnia",
                );
                // Made up, to tell the short genitive names apart.
                locale.months_short = names("sty lut mar kwi maj cze lip sie wrz paź lis gru");
                locale.months_short_genitive =
                    names("sty. lut. mar. kwi. maj. cze. lip. sie. wrz. paź. lis. gru.");
                locale.weekdays[2] = "środa".into();
                locale.long_date = "dddd, d MMMM yyyy".into();
                locale.month_day = "d MMMM".into();
            }
            _ => unreachable!("no data for {code}"),
        }
        locale
    }

    #[test]
    fn month_names_decline_next_to_a_day_number() {
        let day = date(2026, 10, 21);
        let hr = declining("hr-HR");
        assert_eq!(format_date(&hr.long_date, day, &hr), "21. listopada 2026.");
        assert_eq!(format_date(&hr.month_day, day, &hr), "21. listopada");
        // Alone, or with only the year or a weekday name: as it is named.
        assert_eq!(format_date("MMMM yyyy.", day, &hr), "listopad 2026.");
        assert_eq!(format_date("MMMM", day, &hr), "listopad");
        assert_eq!(format_date("dddd, MMMM", day, &hr), "srijeda, listopad");
        // The day after the month counts too, as for GetDateFormatEx; a weekday name between
        // them does not stop it.
        assert_eq!(format_date("MMMM d.", day, &hr), "listopada 21.");
        assert_eq!(
            format_date("dddd, d. MMMM", day, &hr),
            "srijeda, 21. listopada"
        );
        // Quoted letters are text, not a day.
        assert_eq!(format_date("MMMM 'd'", day, &hr), "listopad d");

        let pl = declining("pl-PL");
        assert_eq!(
            format_date(&pl.long_date, day, &pl),
            "środa, 21 października 2026"
        );
        assert_eq!(format_date("MMMM yyyy", day, &pl), "październik 2026");
        // Short names decline as well.
        assert_eq!(format_date("d MMM", day, &pl), "21 paź.");
        assert_eq!(format_date("MMM yyyy", day, &pl), "paź 2026");

        // English names are the same either way.
        let en = Locale::en_us();
        assert_eq!(format_date("d MMMM", day, &en), "21 October");
        assert_eq!(format_date("MMMM yyyy", day, &en), "October 2026");
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
