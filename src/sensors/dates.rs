//! Dates written with the locale's date patterns, for the Date and Time sensors (SPEC §8.6,
//! §9.4–9.5). Pure; the dates themselves are the platform's (`platform::time`).

use crate::platform::Locale;
use crate::platform::time::Date;

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
