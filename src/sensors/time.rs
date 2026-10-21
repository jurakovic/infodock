//! `time` — the clock (SPEC §9.5): the hour large, the minutes (or seconds) small and raised,
//! am/pm under them on a 12-hour clock, and an optional chime.

use std::time::{Duration, Instant};

use serde::Deserialize;

use super::common::two_line;
use super::dates::{LocalTime, format_date};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField, SelectOption};
use super::text::shadow_color;
use super::{
    Category, ClickResult, DefaultsCtx, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx,
    SensorType, State, typed,
};
use crate::bar::theme::Theme;
use crate::config::model::DateLocale;
use crate::platform::{Canvas, Font, PointF, TextEngine, TextLayout, TextStyle};

/// The bundled chime (a placeholder until the artwork, SPEC Appendix C).
static CHIME: &[u8] = include_bytes!("../../assets/sounds/chime.wav");
/// No chime this soon after the start, a resume or a time change (SPEC §9.5).
const QUIET: Duration = Duration::from_secs(2);
/// Between the hour and the small column, logical px.
const GAP: f32 = 2.0;
/// The size of am/pm, logical px.
const AM_PM_SIZE: f32 = 9.0;

const CHIMES: &[SelectOption] = &[
    SelectOption {
        value: Lit::Int(0),
        label: "sensor.time.chime.never",
    },
    SelectOption {
        value: Lit::Int(60),
        label: "sensor.time.chime.hour",
    },
    SelectOption {
        value: Lit::Int(30),
        label: "sensor.time.chime.halfHour",
    },
    SelectOption {
        value: Lit::Int(15),
        label: "sensor.time.chime.quarter",
    },
];

const SCHEMA: &[PropField] = &[
    PropField::new(
        "use24h",
        FieldKind::Bool,
        "sensor.time.field.use24h",
        Lit::Bool(true),
    )
    .machine(),
    PropField::new(
        "showSeconds",
        FieldKind::Bool,
        "sensor.time.field.showSeconds",
        Lit::Bool(false),
    ),
    PropField::new(
        "shadow",
        FieldKind::Bool,
        "sensor.field.shadow",
        Lit::Bool(false),
    )
    .describe("sensor.field.shadow.help"),
    PropField::new(
        "chime",
        FieldKind::Select { options: CHIMES },
        "sensor.time.field.chime",
        Lit::Int(0),
    ),
];

/// SPEC §9.5: the chime's XML values are its minutes, as stored.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("24hours", "use24h"),
    LegacyKey::new("seconds", "showSeconds"),
    LegacyKey::new("shadow", "shadow"),
    LegacyKey::new("chime", "chime"),
];

pub struct TimeType;

impl SensorType for TimeType {
    fn id(&self) -> &'static str {
        "time"
    }

    fn category(&self) -> Category {
        Category::Info
    }

    fn name(&self) -> &'static str {
        "sensor.time.name"
    }

    fn description(&self) -> &'static str {
        "sensor.time.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockTime"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    /// A 24-hour clock unless the regional format uses a 12-hour one.
    fn machine_defaults(&self, ctx: &DefaultsCtx) -> Props {
        let mut props = Props::new();
        let hour12 = ctx.platform.locale(DateLocale::System).hour12;
        props.insert("use24h".into(), (!hour12).into());
        props
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Time {
            props: typed(props)?,
            now: None,
            next: None,
            quiet_until: None,
            am_pm: None,
            layouts: None,
        }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeProps {
    use24h: bool,
    show_seconds: bool,
    shadow: bool,
    /// Every this many minutes; 0 for never.
    chime: u32,
}

struct Layouts {
    /// The hour, or `H:MM` with seconds.
    big: TextLayout,
    /// The minutes, or the seconds.
    small: TextLayout,
    am_pm: Option<TextLayout>,
    /// The small column, as wide as `00`, am and pm, so the bar does not shift.
    column: f32,
}

struct Time {
    props: TimeProps,
    /// The time shown; `None` until the first wake-up reads it (at once).
    now: Option<LocalTime>,
    /// When to read the time again: just after the next minute or second.
    next: Option<Instant>,
    quiet_until: Option<Instant>,
    /// The locale's am and pm, in lower case (SPEC §9.5), read with the time.
    am_pm: Option<(String, String)>,
    layouts: Option<Layouts>,
}

impl Time {
    /// What the clock shows of `t`: hour, minute and (with seconds) second.
    fn shown(&self, t: LocalTime) -> (u8, u8, u8) {
        let second = if self.props.show_seconds { t.second } else { 0 };
        (t.hour, t.minute, second)
    }

    /// Reads the time at `at`; chimes when a chiming minute begins. Returns whether what the
    /// clock shows changed.
    fn read(&mut self, at: Instant, cx: &SensorCtx) -> bool {
        let t = cx.platform.local_time();
        if self.am_pm.is_none() {
            let locale = cx.platform.locale(cx.settings.date_locale);
            self.am_pm = Some((locale.am.to_lowercase(), locale.pm.to_lowercase()));
        }
        let quiet_until = *self.quiet_until.get_or_insert(at + QUIET);
        let new_minute = self
            .now
            .is_some_and(|old| (old.hour, old.minute) != (t.hour, t.minute));
        let chime = self.props.chime;
        if new_minute && chime > 0 && u32::from(t.minute) % chime == 0 && at >= quiet_until {
            cx.platform.play_sound(CHIME);
        }
        let changed = self.now.is_none_or(|old| self.shown(old) != self.shown(t));
        self.now = Some(t);
        let wait = if self.props.show_seconds {
            t.until_next_second()
        } else {
            t.until_next_minute()
        };
        self.next = Some(at + wait);
        changed
    }

    fn texts(&self, t: LocalTime) -> (String, String, Option<String>) {
        let hour = if self.props.use24h {
            t.hour
        } else {
            (t.hour + 11) % 12 + 1
        };
        let (big, small) = if self.props.show_seconds {
            (
                format!("{hour}:{:02}", t.minute),
                format!("{:02}", t.second),
            )
        } else {
            (hour.to_string(), format!("{:02}", t.minute))
        };
        let am_pm = (!self.props.use24h).then(|| {
            let (am, pm) = self.am_pm.clone().unwrap_or_default();
            if t.hour < 12 { am } else { pm }
        });
        (big, small, am_pm)
    }
}

fn styles(theme: &Theme) -> (TextStyle, TextStyle, TextStyle) {
    let font = &theme.sensor_font;
    let bold = Font {
        weight: 700,
        ..font.clone()
    };
    (
        TextStyle::new(&bold, theme.large_size).tabular(),
        TextStyle::new(font, theme.medium_size).tabular(),
        TextStyle::new(font, AM_PM_SIZE),
    )
}

fn column_width(engine: &dyn TextEngine, theme: &Theme, am_pm: Option<&(String, String)>) -> f32 {
    let (_, small, tiny) = styles(theme);
    let mut width = engine.layout("00", &small).width();
    if let Some((am, pm)) = am_pm {
        for text in [am, pm] {
            width = width.max(engine.layout(text, &tiny).width());
        }
    }
    width
}

impl Sensor for Time {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let Some(t) = self.now else {
            self.layouts = None;
            return 0.0;
        };
        let (big, small, am_pm) = self.texts(t);
        let (big_style, small_style, tiny) = styles(cx.theme);
        let am_pms = (!self.props.use24h).then_some(()).and(self.am_pm.as_ref());
        let layouts = Layouts {
            big: cx.text.layout(&big, &big_style),
            small: cx.text.layout(&small, &small_style),
            am_pm: am_pm.map(|text| cx.text.layout(&text, &tiny)),
            column: column_width(cx.text, cx.theme, am_pms),
        };
        let shadow = if self.props.shadow { 1.0 } else { 0.0 };
        let width = layouts.big.width() + GAP + layouts.column + shadow;
        self.layouts = Some(layouts);
        width.ceil()
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(l) = &self.layouts else {
            return;
        };
        let r = cx.rect;
        let color = cx.theme.sensor_text;
        // The small column uses the two-line block's rows: minutes raised above the hour's
        // middle, am/pm below.
        let [upper, lower] = two_line::baselines(cx.theme, r.height());
        let x = r.left + l.big.width() + GAP;
        let texts = [
            (&l.big, r.left, r.top + (r.height() - l.big.height()) / 2.0),
            (&l.small, x, r.top + upper - l.small.baseline()),
        ];
        let am_pm = l
            .am_pm
            .as_ref()
            .map(|layout| (layout, x, r.top + lower - layout.baseline()));
        for (layout, x, y) in texts.into_iter().chain(am_pm) {
            let at = PointF::new(canvas.snap(x), canvas.snap(y));
            if self.props.shadow {
                let shadow = shadow_color(color).with_alpha(color.a);
                canvas.draw_text(layout, PointF::new(at.x + 1.0, at.y + 1.0), shadow);
            }
            canvas.draw_text(layout, at, color);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        let date = self.now?.date;
        let locale = cx.platform.locale(cx.settings.date_locale);
        Some(HintContent::Text(format_date(
            &locale.long_date,
            date,
            &locale,
        )))
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open("ms-settings:dateandtime");
        ClickResult::Handled
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        let Some(t) = self.now else {
            return String::new();
        };
        let (big, small, am_pm) = self.texts(t);
        let time = format!("{big}:{small}");
        match am_pm {
            Some(am_pm) => format!("{time} {am_pm}"),
            None => time,
        }
    }

    fn wake_at(&self, now: Instant) -> Option<Instant> {
        Some(self.next.unwrap_or(now))
    }

    fn on_wake(&mut self, now: Instant, cx: &SensorCtx) -> bool {
        self.read(now, cx)
    }

    fn on_clock_change(&mut self, now: Instant, cx: &SensorCtx) -> bool {
        self.quiet_until = Some(now + QUIET);
        self.am_pm = None;
        self.read(now, cx)
    }
}

/// The chime's sound, for tests that check it is a playable WAV.
#[cfg(test)]
pub(crate) fn chime() -> &'static [u8] {
    CHIME
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::model::Settings;
    use crate::platform::RectF;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, theme};

    fn time(stored: serde_json::Value) -> Box<dyn Sensor> {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty());
        TimeType.create(&props, &State::new()).expect("valid props")
    }

    struct Harness {
        clock: FakeClock,
        settings: Settings,
        theme: Theme,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                clock: FakeClock::default(),
                settings: Settings::default(),
                theme: theme(),
            }
        }

        fn ctx(&self) -> SensorCtx<'_> {
            SensorCtx {
                id: "s_time",
                settings: &self.settings,
                theme: &self.theme,
                locked: false,
                alt: false,
                animate: true,
                changes: &crate::sampler::ChangeSink::NONE,
                platform: &self.clock,
            }
        }

        /// Measures and draws `sensor`; returns its width and draw calls.
        fn drawn(&self, sensor: &mut dyn Sensor) -> (f32, String) {
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
            (width, canvas.dump())
        }
    }

    #[test]
    fn reads_the_time_at_once_then_at_each_minute() {
        let h = Harness::new();
        h.clock.set(9, 5, 30);
        let mut sensor = time(json!({}));
        let start = Instant::now();
        assert_eq!(sensor.wake_at(start), Some(start), "due at once");
        assert_eq!(
            h.drawn(sensor.as_mut()).0,
            0.0,
            "nothing before the first reading"
        );
        assert!(sensor.on_wake(start, &h.ctx()));
        // 30 s to the next minute, plus the margin.
        let next = sensor.wake_at(start).expect("a wake-up");
        assert_eq!(next - start, Duration::from_millis(30_020));
        // 24 h by default: "9" at 15 px bold (7.5 px each), then "05" at 11 px.
        let (width, dump) = h.drawn(sensor.as_mut());
        assert_eq!(width, (7.5 + GAP + 11.0f32).ceil());
        let lines: Vec<&str> = dump.lines().collect();
        assert!(
            lines[0].starts_with("text \"9\"") && lines[0].contains(" 700 15px "),
            "{dump}"
        );
        assert!(
            lines[1].starts_with("text \"05\"") && lines[1].contains(" 11px at 10,2 "),
            "{dump}"
        );
        // Reading again within the minute changes nothing.
        h.clock.set(9, 5, 40);
        assert!(!sensor.on_wake(next, &h.ctx()));
        h.clock.set(9, 6, 0);
        assert!(sensor.on_wake(next, &h.ctx()));
    }

    #[test]
    fn twelve_hour_clock_with_seconds() {
        let h = Harness::new();
        h.clock.set(21, 5, 7);
        let mut sensor = time(json!({ "use24h": false, "showSeconds": true }));
        let start = Instant::now();
        sensor.on_wake(start, &h.ctx());
        assert_eq!(
            sensor.wake_at(start).map(|at| at - start),
            Some(Duration::from_millis(1020))
        );
        let (_, dump) = h.drawn(sensor.as_mut());
        let texts: Vec<&str> = dump.lines().filter_map(|l| l.split('"').nth(1)).collect();
        assert_eq!(texts, ["9:05", "07", "pm"]);
        assert_eq!(sensor.accessible_value(&h.ctx()), "9:05:07 pm");
        h.clock.set(0, 0, 0);
        sensor.on_wake(start, &h.ctx());
        assert_eq!(sensor.accessible_value(&h.ctx()), "12:00:00 am");
    }

    #[test]
    fn chimes_when_a_chiming_minute_begins_but_not_right_after_start() {
        let h = Harness::new();
        let mut sensor = time(json!({ "chime": 15 }));
        let start = Instant::now();
        h.clock.set(10, 14, 59);
        sensor.on_wake(start, &h.ctx());
        h.clock.set(10, 15, 0);
        // Within 2 s of the start: quiet.
        sensor.on_wake(start + Duration::from_secs(1), &h.ctx());
        assert!(h.clock.log().is_empty());
        h.clock.set(10, 16, 0);
        sensor.on_wake(start + Duration::from_secs(61), &h.ctx());
        assert!(h.clock.log().is_empty(), "16 is no quarter");
        h.clock.set(10, 30, 0);
        sensor.on_wake(start + Duration::from_secs(900), &h.ctx());
        assert_eq!(h.clock.log(), [format!("play {}", chime().len())]);
        // A clock change is quiet too.
        h.clock.set(10, 45, 0);
        sensor.on_clock_change(start + Duration::from_secs(1000), &h.ctx());
        assert_eq!(h.clock.log().len(), 1);
    }

    #[test]
    fn hint_click_and_machine_default() {
        let h = Harness::new();
        h.clock.set(9, 5, 0);
        let mut sensor = time(json!({}));
        assert!(sensor.hint(&h.ctx()).is_none(), "no time read yet");
        sensor.on_wake(Instant::now(), &h.ctx());
        let hint = sensor.hint(&h.ctx());
        assert!(
            matches!(&hint, Some(HintContent::Text(t)) if t == "Tuesday, October 13, 2026"),
            "{:?}",
            hint.is_some()
        );
        assert_eq!(sensor.click(&h.ctx()), ClickResult::Handled);
        assert_eq!(h.clock.log(), ["open ms-settings:dateandtime"]);
        // en-US uses a 12-hour clock.
        let defaults = TimeType.machine_defaults(&DefaultsCtx { platform: &h.clock });
        assert_eq!(defaults["use24h"], json!(false));
    }

    #[test]
    fn the_chime_is_a_wave_file() {
        let wav = chime();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }
}
