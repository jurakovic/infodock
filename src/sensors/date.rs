//! `date` — today's date (SPEC §9.4): the day large with month and year small beside it, or
//! one line from the locale's patterns; a month calendar as its hint.

use std::time::Instant;

use serde::Deserialize;

use super::common::two_line;
use super::dates::{Date, days_in_month, format_date};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField, SelectOption};
use super::text::shadow_color;
use super::{
    Category, ClickResult, DrawCtx, HintContent, LayoutCtx, Props, RichHint, Sensor, SensorCtx,
    SensorType, State, typed,
};
use crate::bar::theme::Theme;
use crate::i18n::t_args;
use crate::platform::{
    Brush, Canvas, Font, Locale, PointF, RectF, Shape, SizeF, TextEngine, TextLayout, TextStyle,
};

/// Between the day and the month and year, logical px.
const GAP: f32 = 3.0;
/// The smallest calendar column (SPEC §9.4).
const MIN_COLUMN: f32 = 29.0;

macro_rules! options {
    ($($value:literal => $label:literal),* $(,)?) => {
        &[$(SelectOption { value: Lit::Str($value), label: $label }),*]
    };
}

const SCHEMA: &[PropField] = &[
    PropField::new(
        "weekday",
        FieldKind::Select {
            options: options!(
                "none" => "sensor.date.weekday.none",
                "short" => "sensor.date.weekday.short",
                "long" => "sensor.date.weekday.long",
            ),
        },
        "sensor.date.field.weekday",
        Lit::Str("long"),
    ),
    PropField::new(
        "month",
        FieldKind::Select {
            options: options!(
                "none" => "sensor.date.month.none",
                "short" => "sensor.date.month.short",
                "long" => "sensor.date.month.long",
                "number" => "sensor.date.month.number",
            ),
        },
        "sensor.date.field.month",
        Lit::Str("long"),
    ),
    PropField::new(
        "year",
        FieldKind::Select {
            options: options!(
                "none" => "sensor.date.year.none",
                "top" => "sensor.date.year.top",
                "bottom" => "sensor.date.year.bottom",
            ),
        },
        "sensor.date.field.year",
        Lit::Str("top"),
    ),
    PropField::new(
        "shadow",
        FieldKind::Bool,
        "sensor.field.shadow",
        Lit::Bool(false),
    )
    .describe("sensor.field.shadow.help"),
    PropField::new(
        "calendar",
        FieldKind::Select {
            options: options!(
                "none" => "sensor.date.calendar.none",
                "monday" => "sensor.date.calendar.monday",
                "sunday" => "sensor.date.calendar.sunday",
            ),
        },
        "sensor.date.field.calendar",
        Lit::Str("monday"),
    )
    .describe("sensor.date.field.calendar.help"),
];

/// SPEC §9.4.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::options(
        "dayname",
        "weekday",
        &[
            ("0", Lit::Str("none")),
            ("1", Lit::Str("short")),
            ("2", Lit::Str("long")),
        ],
    ),
    LegacyKey::options(
        "month-format",
        "month",
        &[
            ("0", Lit::Str("none")),
            ("1", Lit::Str("short")),
            ("2", Lit::Str("long")),
            ("3", Lit::Str("number")),
        ],
    ),
    LegacyKey::options(
        "year-format",
        "year",
        &[
            ("0", Lit::Str("none")),
            ("4", Lit::Str("top")),
            ("5", Lit::Str("bottom")),
        ],
    ),
    LegacyKey::new("shadow", "shadow"),
    LegacyKey::options(
        "show-calendar",
        "calendar",
        &[
            ("0", Lit::Str("none")),
            ("1", Lit::Str("monday")),
            ("2", Lit::Str("sunday")),
        ],
    ),
];

pub struct DateType;

impl SensorType for DateType {
    fn id(&self) -> &'static str {
        "date"
    }

    fn category(&self) -> Category {
        Category::Info
    }

    fn name(&self) -> &'static str {
        "sensor.date.name"
    }

    fn description(&self) -> &'static str {
        "sensor.date.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockDate"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(DateSensor {
            props: typed(props)?,
            today: None,
            locale: None,
            next: None,
            layouts: None,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Weekday {
    None,
    Short,
    Long,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Month {
    None,
    Short,
    Long,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Year {
    None,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Calendar {
    None,
    Monday,
    Sunday,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DateProps {
    weekday: Weekday,
    month: Month,
    year: Year,
    shadow: bool,
    calendar: Calendar,
}

/// What the sensor shows, laid out: one large line, and up to two small ones beside it.
struct Layouts {
    big: TextLayout,
    /// The upper and lower small line (month and year in the order of `year`).
    small: [Option<TextLayout>; 2],
}

struct DateSensor {
    props: DateProps,
    /// `None` until the first wake-up reads the clock (at once).
    today: Option<Date>,
    locale: Option<Locale>,
    next: Option<Instant>,
    layouts: Option<Layouts>,
}

/// `pattern` without its year and the separator next to it: `M/d/yyyy` → `M/d`,
/// `dd.MM.yyyy` → `dd.MM`, `yyyy-MM-dd` → `MM-dd`.
fn without_year(pattern: &str) -> String {
    let Some(start) = pattern.find('y') else {
        return pattern.to_owned();
    };
    let end = pattern[start..]
        .find(|c| c != 'y')
        .map_or(pattern.len(), |n| start + n);
    let is_separator = |c: char| !c.is_alphanumeric() && c != '\'';
    let before = pattern[..start].trim_end_matches(is_separator);
    let after = &pattern[end..];
    if before.is_empty() {
        after.trim_start_matches(is_separator).to_owned()
    } else {
        format!("{before}{}", after.trim_start_matches(is_separator))
    }
}

impl DateSensor {
    /// The single line shown without a year (SPEC §9.4), built from the locale's patterns so
    /// the order follows the locale: `Saturday, October 11`, `Sat, Oct 11`, `Saturday, 10/11`.
    fn one_line(&self, date: Date, locale: &Locale) -> String {
        let month_day = match self.props.month {
            Month::Long => locale.month_day.clone(),
            Month::Short => locale.month_day.replace("MMMM", "MMM"),
            Month::Number => without_year(&locale.short_date),
            Month::None => "d".into(),
        };
        let pattern = match self.props.weekday {
            Weekday::Long => format!("dddd, {month_day}"),
            Weekday::Short => format!("ddd, {month_day}"),
            Weekday::None => month_day,
        };
        format_date(&pattern, date, locale)
    }

    /// The large day line with the year shown: `Saturday, 11`, `Sat, 11` or `11`.
    fn day_line(&self, date: Date, locale: &Locale) -> String {
        let weekday = usize::from(date.weekday());
        match self.props.weekday {
            Weekday::Long => format!("{}, {}", locale.weekdays[weekday], date.day),
            Weekday::Short => format!("{}, {}", locale.weekdays_short[weekday], date.day),
            Weekday::None => date.day.to_string(),
        }
    }

    /// The month line in lower case (`october`, `oct`, `10`), if shown.
    fn month_line(&self, date: Date, locale: &Locale) -> Option<String> {
        let month = usize::from(date.month - 1);
        match self.props.month {
            Month::Long => Some(locale.months[month].to_lowercase()),
            Month::Short => Some(locale.months_short[month].to_lowercase()),
            Month::Number => Some(date.month.to_string()),
            Month::None => None,
        }
    }

    fn read(&mut self, at: Instant, cx: &SensorCtx) -> bool {
        let now = cx.platform.local_time();
        if self.locale.is_none() {
            self.locale = Some(cx.platform.locale(cx.settings.date_locale));
        }
        // Every minute: midnight is noticed within a moment, and a missed timer at most a
        // minute late (SPEC §9.4: midnight timer plus a 60 s safety tick).
        self.next = Some(at + now.until_next_minute());
        let changed = self.today != Some(now.date);
        self.today = Some(now.date);
        changed
    }

    fn footer(today: Date) -> String {
        let (_, week) = today.iso_week();
        t_args(
            "sensor.date.hint.week",
            &[
                ("week", &week.to_string()),
                ("day", &today.day_of_year().to_string()),
            ],
        )
    }
}

fn big_style(theme: &Theme) -> TextStyle {
    let bold = Font {
        weight: 700,
        ..theme.sensor_font.clone()
    };
    TextStyle::new(&bold, theme.large_size)
}

impl Sensor for DateSensor {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let (Some(today), Some(locale)) = (self.today, self.locale.as_ref()) else {
            self.layouts = None;
            return 0.0;
        };
        let small_style = TextStyle::new(&cx.theme.sensor_font, cx.theme.medium_size);
        let layouts = if self.props.year == Year::None {
            Layouts {
                big: cx
                    .text
                    .layout(&self.one_line(today, locale), &big_style(cx.theme)),
                small: [None, None],
            }
        } else {
            let month = self.month_line(today, locale);
            let year = Some(today.year.to_string());
            let [upper, lower] = if self.props.year == Year::Top {
                [year, month]
            } else {
                [month, year]
            };
            let layout = |text: Option<String>| text.map(|t| cx.text.layout(&t, &small_style));
            Layouts {
                big: cx
                    .text
                    .layout(&self.day_line(today, locale), &big_style(cx.theme)),
                small: [layout(upper), layout(lower)],
            }
        };
        let column = layouts
            .small
            .iter()
            .flatten()
            .map(TextLayout::width)
            .fold(0.0, f32::max);
        let beside = if column > 0.0 { GAP + column } else { 0.0 };
        let shadow = if self.props.shadow { 1.0 } else { 0.0 };
        let width = layouts.big.width() + beside + shadow;
        self.layouts = Some(layouts);
        width.ceil()
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(l) = &self.layouts else {
            return;
        };
        let r = cx.rect;
        let color = cx.theme.sensor_text;
        let baselines = two_line::baselines_for(cx.theme.medium_size, r.height());
        let x = r.left + l.big.width() + GAP;
        let big = (&l.big, r.left, r.top + (r.height() - l.big.height()) / 2.0);
        let small = l
            .small
            .iter()
            .zip(baselines)
            .filter_map(|(layout, baseline)| {
                layout
                    .as_ref()
                    .map(|layout| (layout, x, r.top + baseline - layout.baseline()))
            });
        for (layout, x, y) in std::iter::once(big).chain(small) {
            let at = PointF::new(canvas.snap(x), canvas.snap(y));
            if self.props.shadow {
                let shadow = shadow_color(color).with_alpha(color.a);
                canvas.draw_text(layout, PointF::new(at.x + 1.0, at.y + 1.0), shadow);
            }
            canvas.draw_text(layout, at, color);
        }
    }

    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        let today = self.today?;
        let first_day = match self.props.calendar {
            Calendar::None => return Some(HintContent::Text(Self::footer(today))),
            Calendar::Monday => 0,
            Calendar::Sunday => 6,
        };
        let names = self.locale.as_ref()?.weekdays_short.clone();
        Some(HintContent::Rich(Box::new(MonthCalendar::new(
            today, first_day, names,
        ))))
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        let app = cx.settings.apps.calendar.trim();
        let target = if app.is_empty() {
            "ms-settings:dateandtime"
        } else {
            app
        };
        cx.platform.open(target);
        ClickResult::Handled
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        match (self.today, self.locale.as_ref()) {
            (Some(today), Some(locale)) => format_date(&locale.long_date, today, locale),
            _ => String::new(),
        }
    }

    fn wake_at(&self, now: Instant) -> Option<Instant> {
        Some(self.next.unwrap_or(now))
    }

    fn on_wake(&mut self, now: Instant, cx: &SensorCtx) -> bool {
        self.read(now, cx)
    }

    fn on_clock_change(&mut self, now: Instant, cx: &SensorCtx) -> bool {
        self.locale = None;
        self.read(now, cx)
    }
}

/// The month calendar (SPEC §9.4): short weekday names from the chosen first day, the weeks
/// with right-aligned day numbers, weekends in the strong colour, today highlighted, and the
/// week and day of the year below.
pub struct MonthCalendar {
    today: Date,
    /// 0 = Monday … 6 = Sunday.
    first_day: u8,
    /// Short weekday names, Monday first.
    names: [String; 7],
    laid: Option<Laid>,
}

struct Laid {
    /// The header, in display order.
    header: Vec<TextLayout>,
    /// (row from 0, column, number).
    days: Vec<(usize, usize, TextLayout)>,
    footer: TextLayout,
    column: f32,
    row: f32,
    weeks: usize,
}

/// Space around today's number in its highlight, and right of every number.
const CELL_PAD: f32 = 4.0;

impl MonthCalendar {
    pub fn new(today: Date, first_day: u8, names: [String; 7]) -> Self {
        Self {
            today,
            first_day: first_day % 7,
            names,
            laid: None,
        }
    }

    /// Whether display column `column` is Saturday or Sunday.
    fn weekend(&self, column: usize) -> bool {
        (usize::from(self.first_day) + column) % 7 >= 5
    }
}

impl RichHint for MonthCalendar {
    fn layout(&mut self, text: &dyn TextEngine, theme: &Theme) -> SizeF {
        let style = TextStyle::new(&theme.popup_font, theme.popup_font_size).tabular();
        let header: Vec<TextLayout> = (0..7)
            .map(|c| {
                let weekday = (usize::from(self.first_day) + c) % 7;
                text.layout(&self.names[weekday], &style)
            })
            .collect();
        let widest = header
            .iter()
            .map(TextLayout::width)
            .fold(text.layout("00", &style).width(), f32::max);
        let column = (widest + 2.0 * CELL_PAD).max(MIN_COLUMN).ceil();
        let row = (text.layout("Ag", &style).height() + 2.0).ceil();
        let first = Date {
            day: 1,
            ..self.today
        };
        let offset = usize::from((first.weekday() + 7 - self.first_day) % 7);
        let count = usize::from(days_in_month(self.today.year, self.today.month));
        let days = (1..=count)
            .map(|day| {
                let cell = offset + day - 1;
                (cell / 7, cell % 7, text.layout(&day.to_string(), &style))
            })
            .collect();
        let weeks = (offset + count).div_ceil(7);
        let footer = text.layout(&DateSensor::footer(self.today), &style);
        let size = SizeF::new(
            (7.0 * column).max(footer.width()),
            row * (1 + weeks) as f32 + row / 2.0 + footer.height(),
        );
        self.laid = Some(Laid {
            header,
            days,
            footer,
            column,
            row,
            weeks,
        });
        size
    }

    fn draw(&self, canvas: &mut dyn Canvas, origin: PointF, theme: &Theme) {
        let Some(l) = &self.laid else {
            return;
        };
        // Whole physical px, so the digits stay crisp.
        let scale = canvas.scale();
        let snap = |v: f32| (v * scale).round() / scale;
        let color = |column: usize| {
            if self.weekend(column) {
                theme.popup_strong_text
            } else {
                theme.popup_text
            }
        };
        // Right-aligned in its column, `CELL_PAD` from the column's right edge.
        let at = |row: usize, column: usize, layout: &TextLayout| {
            let right = origin.x + (column + 1) as f32 * l.column - CELL_PAD;
            let y = origin.y + row as f32 * l.row + (l.row - layout.height()) / 2.0;
            PointF::new(snap(right - layout.width()), snap(y))
        };
        for (column, layout) in l.header.iter().enumerate() {
            canvas.draw_text(layout, at(0, column, layout), color(column));
        }
        for (i, (week, column, layout)) in l.days.iter().enumerate() {
            let point = at(week + 1, *column, layout);
            let mut ink = color(*column);
            if i + 1 == usize::from(self.today.day) {
                let right = origin.x + (*column + 1) as f32 * l.column;
                let top = origin.y + (week + 1) as f32 * l.row;
                let highlight = RectF::new(right - l.column + 1.0, top, right - 1.0, top + l.row);
                canvas.fill(
                    &Shape::RoundedRect {
                        rect: highlight,
                        radius: 4.0,
                    },
                    &Brush::Solid(ink),
                );
                ink = theme.popup_background.with_alpha(1.0);
            }
            canvas.draw_text(layout, point, ink);
        }
        let footer_y = origin.y + (1 + l.weeks) as f32 * l.row + l.row / 2.0;
        canvas.draw_text(
            &l.footer,
            PointF::new(origin.x, snap(footer_y)),
            theme.popup_text,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::config::model::Settings;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, theme};

    fn sensor(stored: serde_json::Value) -> Box<dyn Sensor> {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        DateType.create(&props, &State::new()).expect("valid props")
    }

    struct Harness {
        clock: FakeClock,
        settings: Settings,
        theme: Theme,
    }

    impl Harness {
        fn new() -> Self {
            let clock = FakeClock::default();
            clock.set(9, 5, 0);
            Self {
                clock,
                settings: Settings::default(),
                theme: theme(),
            }
        }

        fn ctx(&self) -> SensorCtx<'_> {
            SensorCtx {
                id: "s_date",
                settings: &self.settings,
                theme: &self.theme,
                locked: false,
                alt: false,
                animate: true,
                changes: &crate::sampler::ChangeSink::NONE,
                platform: &self.clock,
            }
        }

        /// The texts the sensor draws after reading the (fake) clock.
        fn texts(&self, stored: serde_json::Value) -> Vec<String> {
            let mut sensor = sensor(stored);
            sensor.on_wake(Instant::now(), &self.ctx());
            let width = sensor.measure(&LayoutCtx {
                theme: &self.theme,
                text: &FixedMetrics,
                scale: 1.0,
                numbers: Default::default(),
                icons: &crate::sensors::NoIcons,
            });
            let mut canvas = RecordingCanvas::new(1.0);
            let rect = RectF::new(0.0, 0.0, width, 28.0);
            sensor.draw(
                &mut canvas,
                &DrawCtx {
                    theme: &self.theme,
                    rect,
                },
            );
            canvas
                .dump()
                .lines()
                .filter_map(|l| l.split('"').nth(1).map(str::to_owned))
                .collect()
        }
    }

    #[test]
    fn day_with_month_and_year_beside_it() {
        let h = Harness::new();
        // 2026-10-13 is a Tuesday.
        assert_eq!(h.texts(json!({})), ["Tuesday, 13", "2026", "october"]);
        assert_eq!(
            h.texts(json!({ "weekday": "short", "month": "short", "year": "bottom" })),
            ["Tue, 13", "oct", "2026"]
        );
        assert_eq!(
            h.texts(json!({ "weekday": "none", "month": "none" })),
            ["13", "2026"]
        );
        assert_eq!(
            h.texts(json!({ "month": "number" })),
            ["Tuesday, 13", "2026", "10"]
        );
    }

    #[test]
    fn one_line_without_a_year_follows_the_locale() {
        let h = Harness::new();
        let line = |weekday: &str, month: &str| {
            h.texts(json!({ "weekday": weekday, "month": month, "year": "none" }))
        };
        assert_eq!(line("long", "long"), ["Tuesday, October 13"]);
        assert_eq!(line("short", "short"), ["Tue, Oct 13"]);
        assert_eq!(line("long", "number"), ["Tuesday, 10/13"]);
        assert_eq!(line("none", "none"), ["13"]);
        assert_eq!(without_year("dd.MM.yyyy"), "dd.MM");
        assert_eq!(without_year("yyyy-MM-dd"), "MM-dd");
        assert_eq!(without_year("M/d/yyyy"), "M/d");
    }

    #[test]
    fn reads_the_date_at_once_and_every_minute() {
        let h = Harness::new();
        let mut date = sensor(json!({}));
        let start = Instant::now();
        assert_eq!(date.wake_at(start), Some(start));
        assert!(date.on_wake(start, &h.ctx()));
        assert_eq!(
            date.wake_at(start).map(|at| at - start),
            Some(Duration::from_millis(60_020))
        );
        assert!(!date.on_wake(start, &h.ctx()), "the same day");
    }

    #[test]
    fn calendar_hint() {
        let h = Harness::new();
        let mut date = sensor(json!({}));
        date.on_wake(Instant::now(), &h.ctx());
        let Some(HintContent::Rich(mut calendar)) = date.hint(&h.ctx()) else {
            panic!("a calendar");
        };
        let size = calendar.layout(&FixedMetrics, &h.theme);
        // October 2026 starts on a Thursday: 5 weeks from Monday; columns of at least 29 px.
        assert!(size.width >= 7.0 * 29.0, "{size:?}");
        let mut canvas = RecordingCanvas::new(1.0);
        calendar.draw(&mut canvas, PointF::new(0.0, 0.0), &h.theme);
        let dump = canvas.dump();
        let texts: Vec<&str> = dump.lines().filter_map(|l| l.split('"').nth(1)).collect();
        assert_eq!(
            &texts[..7],
            ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        );
        assert_eq!(texts[7], "1");
        assert_eq!(texts.last(), Some(&"Week 42, Day 286"));
        // Today (the 13th) on a highlight, in the background colour.
        let today = dump.lines().position(|l| l.contains("\"13\"")).expect("13");
        assert!(
            dump.lines()
                .nth(today - 1)
                .is_some_and(|l| l.starts_with("fill rounded"))
        );
        // Weekends in the strong colour (#93ECFF by default): Saturday's header, not Monday's.
        let lines: Vec<&str> = dump.lines().collect();
        assert!(
            lines[5].ends_with("#93ECFF") && lines[0].ends_with("#FFFFFF"),
            "{dump}"
        );

        // From Sunday; without a calendar only the week line.
        let mut date = sensor(json!({ "calendar": "sunday" }));
        date.on_wake(Instant::now(), &h.ctx());
        let Some(HintContent::Rich(mut calendar)) = date.hint(&h.ctx()) else {
            panic!("a calendar");
        };
        calendar.layout(&FixedMetrics, &h.theme);
        let mut canvas = RecordingCanvas::new(1.0);
        calendar.draw(&mut canvas, PointF::new(0.0, 0.0), &h.theme);
        assert!(canvas.dump().starts_with("text \"Sun\""));
        let mut date = sensor(json!({ "calendar": "none" }));
        date.on_wake(Instant::now(), &h.ctx());
        assert_eq!(
            date.hint(&h.ctx()),
            Some(HintContent::Text("Week 42, Day 286".into()))
        );
    }

    #[test]
    fn click_opens_the_calendar_app_or_the_date_settings() {
        let mut h = Harness::new();
        let mut date = sensor(json!({}));
        date.click(&h.ctx());
        h.settings.apps.calendar = "outlookcal:".into();
        date.click(&h.ctx());
        assert_eq!(
            h.clock.log(),
            ["open ms-settings:dateandtime", "open outlookcal:"]
        );
    }
}
