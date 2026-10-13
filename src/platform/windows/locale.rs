//! The locale from Windows NLS (SPEC §8.6): `GetLocaleInfoEx` for names, date patterns, the hour
//! cycle, the first day of the week and the decimal separator.

use windows::Win32::Globalization::{
    GetLocaleInfoEx, LOCALE_IFIRSTDAYOFWEEK, LOCALE_S1159, LOCALE_S2359, LOCALE_SABBREVDAYNAME1,
    LOCALE_SABBREVMONTHNAME1, LOCALE_SDAYNAME1, LOCALE_SDECIMAL, LOCALE_SLONGDATE,
    LOCALE_SMONTHDAY, LOCALE_SMONTHNAME1, LOCALE_SSHORTDATE, LOCALE_STIMEFORMAT,
};
use windows::core::{PCWSTR, w};

use crate::config::model::DateLocale;
use crate::platform::Locale;
use crate::platform::locale::hour12_from_pattern;

/// Reads `which` from Windows: the user's regional format for `System`, English (United
/// States) for `Ui` (the only UI language in v1). Values it cannot read stay as in en-US.
pub fn read(which: DateLocale) -> Locale {
    let name = match which {
        // LOCALE_NAME_USER_DEFAULT
        DateLocale::System => PCWSTR::null(),
        DateLocale::Ui => w!("en-US"),
    };
    let info = |lctype: u32| -> Option<String> {
        let mut buffer = [0u16; 128];
        // SAFETY: the buffer is writable for its whole length, which the call is told; the
        // name is null (the user default) or a static NUL-terminated string.
        let len = unsafe { GetLocaleInfoEx(name, lctype, Some(&mut buffer)) };
        // The length counts the terminating NUL.
        let len = usize::try_from(len).ok().filter(|n| *n > 1)?;
        Some(String::from_utf16_lossy(&buffer[..len - 1]))
    };
    let fill = |slots: &mut [String], first: u32| {
        for (i, slot) in (first..).zip(slots.iter_mut()) {
            if let Some(value) = info(i) {
                *slot = value;
            }
        }
    };
    let mut locale = Locale::en_us();
    fill(&mut locale.weekdays, LOCALE_SDAYNAME1);
    fill(&mut locale.weekdays_short, LOCALE_SABBREVDAYNAME1);
    fill(&mut locale.months, LOCALE_SMONTHNAME1);
    fill(&mut locale.months_short, LOCALE_SABBREVMONTHNAME1);
    for (slot, lctype) in [
        (&mut locale.long_date, LOCALE_SLONGDATE),
        (&mut locale.short_date, LOCALE_SSHORTDATE),
        (&mut locale.month_day, LOCALE_SMONTHDAY),
        (&mut locale.am, LOCALE_S1159),
        (&mut locale.pm, LOCALE_S2359),
    ] {
        if let Some(value) = info(lctype) {
            *slot = value;
        }
    }
    if let Some(pattern) = info(LOCALE_STIMEFORMAT) {
        locale.hour12 = hour12_from_pattern(&pattern);
    }
    if let Some(day) = info(LOCALE_IFIRSTDAYOFWEEK).and_then(|d| d.parse::<u8>().ok())
        && day < 7
    {
        locale.first_day = day;
    }
    if let Some(decimal) = info(LOCALE_SDECIMAL).and_then(|d| d.chars().next()) {
        locale.decimal = decimal;
    }
    locale
}
