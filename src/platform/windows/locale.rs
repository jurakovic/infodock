//! The locale from Windows NLS (SPEC §8.6): `GetLocaleInfoEx` for names, date patterns, the hour
//! cycle, the first day of the week and the decimal separator; and text in ANSI code pages for
//! the legacy import (§11.4).

use windows::Win32::Globalization::{
    CP_ACP, GetLocaleInfoEx, LOCALE_IFIRSTDAYOFWEEK, LOCALE_RETURN_GENITIVE_NAMES, LOCALE_S1159,
    LOCALE_S2359, LOCALE_SABBREVDAYNAME1, LOCALE_SABBREVMONTHNAME1, LOCALE_SDAYNAME1,
    LOCALE_SDECIMAL, LOCALE_SLONGDATE, LOCALE_SMONTHDAY, LOCALE_SMONTHNAME1, LOCALE_SSHORTDATE,
    LOCALE_STIMEFORMAT, MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar,
};
use windows::core::{PCWSTR, w};

use crate::config::model::DateLocale;
use crate::platform::Locale;
use crate::platform::locale::hour12_from_pattern;

/// `bytes` in the Windows code page `code_page`, or the system's ANSI code page with `None`;
/// `None` if Windows cannot convert them (e.g. a code page it does not have).
pub fn decode(bytes: &[u8], code_page: Option<u32>) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    let code_page = code_page.unwrap_or(CP_ACP);
    let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
    // SAFETY: the first call only measures; the second writes into a buffer of the measured
    // length, which the call is told. Imported files are at most 16 MB, within an i32.
    unsafe {
        let len = usize::try_from(MultiByteToWideChar(code_page, flags, bytes, None)).ok()?;
        let mut wide = vec![0u16; len];
        let written = MultiByteToWideChar(code_page, flags, bytes, Some(&mut wide));
        let written = usize::try_from(written).ok().filter(|n| *n > 0)?;
        Some(String::from_utf16_lossy(&wide[..written]))
    }
}

/// Reads `which` from Windows: the user's regional format for `System`, English (United
/// States) for `Ui` (the only UI language in v1). Values it cannot read stay as in en-US.
pub fn read(which: DateLocale) -> Locale {
    match which {
        // LOCALE_NAME_USER_DEFAULT
        DateLocale::System => read_named(PCWSTR::null()),
        DateLocale::Ui => read_named(w!("en-US")),
    }
}

/// Reads the locale `name` (null: the user's).
fn read_named(name: PCWSTR) -> Locale {
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
    // Languages without declined names answer with the plain ones (SPEC §8.6).
    locale.months_genitive = locale.months.clone();
    locale.months_short_genitive = locale.months_short.clone();
    fill(
        &mut locale.months_genitive,
        LOCALE_SMONTHNAME1 | LOCALE_RETURN_GENITIVE_NAMES,
    );
    fill(
        &mut locale.months_short_genitive,
        LOCALE_SABBREVMONTHNAME1 | LOCALE_RETURN_GENITIVE_NAMES,
    );
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

/// Compares two names as Explorer sorts them (SPEC §9.19): in the user's locale, ignoring case,
/// digits by their value. Falls back to the platform-neutral order if Windows cannot compare.
pub fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    use windows::Win32::Globalization::{
        CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringEx, LINGUISTIC_IGNORECASE,
        SORT_DIGITSASNUMBERS,
    };
    let (wide_a, wide_b): (Vec<u16>, Vec<u16>) =
        (a.encode_utf16().collect(), b.encode_utf16().collect());
    // SAFETY: two UTF-16 slices with their lengths; no locale name means the user's locale.
    let result = unsafe {
        CompareStringEx(
            PCWSTR::null(),
            LINGUISTIC_IGNORECASE | SORT_DIGITSASNUMBERS,
            &wide_a,
            &wide_b,
            None,
            None,
            None,
        )
    };
    match result {
        CSTR_LESS_THAN => Ordering::Less,
        CSTR_GREATER_THAN => Ordering::Greater,
        r if r.0 == 0 => crate::platform::natural_cmp(a, b),
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::time::Date;
    use crate::sensors::dates::format_date;

    /// `date` written with `pattern` by Windows for the locale `name` (NUL-terminated).
    fn windows_date(name: &[u16], date: Date, pattern: &str) -> String {
        use windows::Win32::Foundation::SYSTEMTIME;
        use windows::Win32::Globalization::{ENUM_DATE_FORMATS_FLAGS, GetDateFormatEx};
        let time = SYSTEMTIME {
            wYear: date.year as u16,
            wMonth: u16::from(date.month),
            wDay: u16::from(date.day),
            // Sunday is 0.
            wDayOfWeek: u16::from((date.weekday() + 1) % 7),
            ..Default::default()
        };
        let format: Vec<u16> = pattern.encode_utf16().chain([0]).collect();
        let mut buffer = [0u16; 256];
        // SAFETY: NUL-terminated name and pattern; the buffer is writable for its length,
        // which the call is told.
        let len = unsafe {
            GetDateFormatEx(
                PCWSTR(name.as_ptr()),
                ENUM_DATE_FORMATS_FLAGS(0),
                Some(std::ptr::from_ref(&time)),
                PCWSTR(format.as_ptr()),
                Some(&mut buffer[..]),
                PCWSTR::null(),
            )
        };
        let len = usize::try_from(len).expect("formatted");
        assert!(len > 0, "GetDateFormatEx failed for {pattern:?}");
        String::from_utf16_lossy(&buffer[..len - 1])
    }

    /// Month names decline where `GetDateFormatEx` declines them (SPEC §8.6): next to a day
    /// number, in long and short form, and stay as they are named elsewhere.
    #[test]
    fn dates_are_written_as_windows_writes_them() {
        let patterns = [
            "d. MMMM yyyy.",
            "dddd, d MMMM",
            "dd MMMM",
            "MMMM d",
            "MMMM yyyy",
            "dddd, MMMM",
            "MMMM",
            "d MMM yyyy",
            "MMM yyyy",
            "ddd d. M. yy",
        ];
        for code in [
            "hr-HR", "pl-PL", "cs-CZ", "ru-RU", "uk-UA", "en-US", "de-DE",
        ] {
            let name: Vec<u16> = code.encode_utf16().chain([0]).collect();
            let locale = read_named(PCWSTR(name.as_ptr()));
            for (month, day) in [(1, 5), (3, 1), (10, 21), (11, 30)] {
                let date = Date::new(2026, month, day).expect("a date");
                for pattern in patterns {
                    assert_eq!(
                        format_date(pattern, date, &locale),
                        windows_date(&name, date, pattern),
                        "{code} {pattern:?} on {date:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn decodes_ansi_code_pages() {
        // A Croatian title in Windows-1250 (SPEC §11.4).
        assert_eq!(
            decode(b"Ra\xE8unalo \x9A\xF0", Some(1250)).as_deref(),
            Some("Računalo šđ")
        );
        assert_eq!(decode(b"Caf\xE9", Some(1252)).as_deref(), Some("Café"));
        assert_eq!(decode(b"", None).as_deref(), Some(""));
        // The system's code page reads ASCII as it is.
        assert_eq!(decode(b"plain", None).as_deref(), Some("plain"));
        // A code page Windows does not have.
        assert_eq!(decode(b"x", Some(12_345)), None);
    }
}
