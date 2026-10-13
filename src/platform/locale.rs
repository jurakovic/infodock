//! The locale for dates and numbers (SPEC §8.6, §13): names, date patterns, the hour cycle and
//! the decimal separator, read from the OS for `settings.dateLocale` and never from catalogs.
//! Plain data, so formatting (`sensors/dates.rs`) is tested against fixed locales on every OS.

/// Locale data for one locale.
#[derive(Debug, Clone, PartialEq)]
pub struct Locale {
    /// Weekday names, Monday first.
    pub weekdays: [String; 7],
    pub weekdays_short: [String; 7],
    /// Month names, January first.
    pub months: [String; 12],
    pub months_short: [String; 12],
    /// Date patterns in the Windows picture format (`d`, `dd`, `ddd`, `dddd`, `M` … `MMMM`,
    /// `y`, `yy`, `yyyy`, `'literal'`): the long date (`dddd, MMMM d, yyyy`), the short date
    /// (`M/d/yyyy`) and month with day (`MMMM d`).
    pub long_date: String,
    pub short_date: String,
    pub month_day: String,
    /// The clock is 12-hour (`h11`/`h12`): Time's machine-derived `use24h` is its opposite.
    pub hour12: bool,
    pub am: String,
    pub pm: String,
    /// The first day of the week: 0 = Monday … 6 = Sunday.
    pub first_day: u8,
    pub decimal: char,
}

fn strings<const N: usize>(items: [&str; N]) -> [String; N] {
    items.map(str::to_owned)
}

impl Locale {
    /// English (United States), the stub platform's locale and the fallback when the OS cannot
    /// be read.
    pub fn en_us() -> Self {
        Self {
            weekdays: strings([
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ]),
            weekdays_short: strings(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]),
            months: strings([
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ]),
            months_short: strings([
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ]),
            long_date: "dddd, MMMM d, yyyy".into(),
            short_date: "M/d/yyyy".into(),
            month_day: "MMMM d".into(),
            hour12: true,
            am: "AM".into(),
            pm: "PM".into(),
            first_day: 6,
            decimal: '.',
        }
    }

    /// How numbers are written in this locale.
    pub fn numbers(&self) -> crate::sensors::format::Numbers {
        crate::sensors::format::Numbers {
            decimal: self.decimal,
        }
    }
}

/// Whether a time pattern in the Windows picture format (`h:mm:ss tt`, `HH:mm`) uses a 12-hour
/// clock: its first hour letter outside quotes is `h`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn hour12_from_pattern(pattern: &str) -> bool {
    let mut quoted = false;
    for c in pattern.chars() {
        match c {
            '\'' => quoted = !quoted,
            'h' if !quoted => return true,
            'H' if !quoted => return false,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_cycle_from_the_time_pattern() {
        assert!(hour12_from_pattern("h:mm:ss tt"));
        assert!(!hour12_from_pattern("HH:mm:ss"));
        assert!(
            !hour12_from_pattern("'h'H.mm"),
            "quoted letters do not count"
        );
        assert!(!hour12_from_pattern("mm:ss"));
    }
}
