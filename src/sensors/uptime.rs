//! `uptime` — how long the system has been running (SPEC §9.6), with the longest uptime seen
//! so far kept in the sensor's `state`.

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use super::format::{duration_compact, duration_long};
use super::legacy::{self, LegacyCtx, LegacyKey};
use super::schema::{FieldKind, Lit, PropField};
use super::text::shadow_color;
use super::{
    Category, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx, SensorType, State, typed,
};
use crate::config::legacy::{self as encoding, LegacyProps};
use crate::config::model::FontStyle;
use crate::i18n::t_args;
use crate::platform::{Canvas, Font, PointF, TextLayout, TextStyle};
use crate::sampler::{Sample, Topic, TopicRequest};

/// The `uptime` topic is read this often; the sensor counts on its own in between.
const RESAMPLE: Duration = Duration::from_secs(60);
/// Wake-ups come this long after a minute of uptime is complete, so it has surely passed.
const PAST_BOUNDARY: Duration = Duration::from_millis(20);
/// A new record is stored once it has grown this much, so the config is not rewritten every
/// minute; at most this much of a record is lost if Infodock ends abruptly.
const RECORD_STEP: u64 = 10 * 60;
/// Legacy records longer than a century are not uptimes; they are refused on import.
const MAX_RECORD_SECS: u64 = 100 * 366 * 86_400;
/// The `state` key of the record (SPEC §11.2).
const LONGEST: &str = "longestSecs";
/// The record's XML key, `d:h:m:s` (SPEC §9.6).
const LEGACY_LONGEST: &str = "max-uptime";

const SCHEMA: &[PropField] = &[
    PropField::new(
        "text",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.uptime.field.text",
        Lit::Str("Uptime:"),
    )
    .describe("sensor.uptime.field.text.help"),
    PropField::new("font", FieldKind::Font, "sensor.field.font", Lit::Str("")),
    PropField::new(
        "fontStyle",
        FieldKind::FontStyle,
        "sensor.field.fontStyle",
        Lit::Str("bold"),
    ),
    PropField::new(
        "fontSize",
        FieldKind::Slider {
            min: 9.0,
            max: 25.0,
            step: 1.0,
            unit: Some("common.px"),
        },
        "sensor.field.fontSize",
        Lit::Int(12),
    ),
    PropField::new(
        "shadow",
        FieldKind::Bool,
        "sensor.field.shadow",
        Lit::Bool(false),
    )
    .describe("sensor.field.shadow.help"),
];

/// SPEC §9.6; the record goes into the state.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("text", "text"),
    LegacyKey::new("font", "font"),
    LegacyKey::new("fontstyle", "fontStyle"),
    LegacyKey::new("fontsize", "fontSize"),
    LegacyKey::new("shadow", "shadow"),
];

pub struct UptimeType;

impl SensorType for UptimeType {
    fn id(&self) -> &'static str {
        "uptime"
    }

    fn category(&self) -> Category {
        Category::Info
    }

    fn name(&self) -> &'static str {
        "sensor.uptime.name"
    }

    fn description(&self) -> &'static str {
        "sensor.uptime.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockUpTime"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    /// The record `max-uptime` as the state's `longestSecs`; empty means none.
    fn convert_legacy(&self, raw: &LegacyProps, cx: &mut LegacyCtx) -> Props {
        let mut raw = raw.clone();
        if let Some(record) = raw.remove(LEGACY_LONGEST)
            && !record.trim().is_empty()
        {
            match encoding::duration(&record) {
                Some(0) => {}
                Some(secs) if secs <= MAX_RECORD_SECS => {
                    cx.state.insert(LONGEST.into(), secs.into());
                }
                _ => cx.invalid(LEGACY_LONGEST, &record),
            }
        }
        legacy::convert(SCHEMA, LEGACY_KEYS, &raw, cx)
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        let longest = state.get(LONGEST).and_then(Value::as_u64).unwrap_or(0);
        Ok(Box::new(Uptime {
            props: typed(props)?,
            reading: None,
            shown: 0,
            longest,
            stored: longest,
            layout: None,
        }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UptimeProps {
    /// Shown before the uptime; empty for none.
    text: String,
    /// `""` follows the global sensor font.
    font: String,
    font_style: FontStyle,
    font_size: f32,
    shadow: bool,
}

struct Uptime {
    props: UptimeProps,
    /// The last sample's uptime and when it was read.
    reading: Option<(u64, Instant)>,
    /// The whole minutes of uptime shown.
    shown: u64,
    /// The longest uptime seen, this one included.
    longest: u64,
    /// The record as stored in the config.
    stored: u64,
    layout: Option<TextLayout>,
}

impl Uptime {
    /// The uptime at `at`, in seconds.
    fn uptime(&self, at: Instant) -> Option<u64> {
        let (secs, received) = self.reading?;
        Some(secs + at.saturating_duration_since(received).as_secs())
    }

    /// The text for the shown minute; nothing before the first sample.
    fn text(&self) -> Option<String> {
        self.reading?;
        let uptime = duration_compact(self.shown * 60);
        Some(if self.props.text.is_empty() {
            uptime
        } else {
            format!("{} {uptime}", self.props.text)
        })
    }

    /// Notes the uptime at `at` as a record if it is one, and shows its minute; returns
    /// whether the minute shown changed.
    fn note(&mut self, at: Instant) -> bool {
        let Some(uptime) = self.uptime(at) else {
            return false;
        };
        self.longest = self.longest.max(uptime);
        let minute = uptime / 60;
        let changed = minute != self.shown;
        self.shown = minute;
        changed
    }
}

impl Sensor for Uptime {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let Some(text) = self.text() else {
            self.layout = None;
            return 0.0;
        };
        let family = if self.props.font.is_empty() {
            &cx.theme.sensor_font.family
        } else {
            &self.props.font
        };
        let style = TextStyle::new(
            &Font::new(family, self.props.font_style),
            self.props.font_size,
        )
        .tabular();
        let layout = cx.text.layout(&text, &style);
        let shadow = if self.props.shadow { 1.0 } else { 0.0 };
        let width = (layout.width() + shadow).ceil();
        self.layout = Some(layout);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(layout) = &self.layout else {
            return;
        };
        let r = cx.rect;
        let color = cx.theme.sensor_text;
        let y = canvas.snap(r.top + (r.height() - layout.height()) / 2.0);
        if self.props.shadow {
            let shadow = shadow_color(color).with_alpha(color.a);
            canvas.draw_text(layout, PointF::new(r.left + 1.0, y + 1.0), shadow);
        }
        canvas.draw_text(layout, PointF::new(r.left, y), color);
    }

    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        let uptime = self.uptime(Instant::now())?;
        let mut text = duration_long(uptime);
        if self.longest > uptime {
            let longest = duration_long(self.longest);
            text.push('\n');
            text.push_str(&t_args(
                "sensor.uptime.hint.longest",
                &[("duration", &longest)],
            ));
        }
        Some(HintContent::Text(text))
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.uptime(Instant::now())
            .map(duration_long)
            .unwrap_or_default()
    }

    fn topics(&self) -> Vec<TopicRequest> {
        vec![TopicRequest {
            topic: Topic::Uptime,
            interval: RESAMPLE,
        }]
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Sample::Uptime(sample) = sample else {
            return false;
        };
        let first = self.reading.is_none();
        self.reading = Some((sample.uptime_secs, sample.read_at));
        self.note(Instant::now()) || first
    }

    /// When the minute after the one shown is complete (in the past if that has happened).
    fn wake_at(&self, _now: Instant) -> Option<Instant> {
        let (secs, read_at) = self.reading?;
        let next = (self.shown + 1) * 60;
        Some(read_at + Duration::from_secs(next.saturating_sub(secs)) + PAST_BOUNDARY)
    }

    fn on_wake(&mut self, now: Instant, _cx: &SensorCtx) -> bool {
        self.note(now)
    }

    fn take_state(&mut self) -> Option<State> {
        // Saturating: a record from a file may be as large as a u64 holds.
        if self.longest.saturating_sub(self.stored) < RECORD_STEP {
            return None;
        }
        self.stored = self.longest;
        let mut state = State::new();
        state.insert(LONGEST.into(), json!(self.longest));
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::Settings;
    use crate::platform::RectF;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sampler::UptimeSample;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{NoPlatform, theme};

    fn uptime(stored: Value, state: Value) -> Box<dyn Sensor> {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty());
        let state = state.as_object().cloned().unwrap_or_default();
        UptimeType.create(&props, &state).expect("valid props")
    }

    fn sample(secs: u64) -> Sample {
        read(secs, Instant::now())
    }

    #[test]
    fn imports_the_legacy_record_into_the_state() {
        let convert = |pairs: &[(&str, &str)]| {
            crate::sensors::test_support::convert_legacy(
                &UptimeType,
                &NoPlatform,
                "DockUpTime",
                pairs,
            )
        };
        let (props, state, warnings) = convert(&[
            ("text", ""),
            ("fontstyle", "italic"),
            ("max-uptime", "12:1:3:0"),
        ]);
        assert_eq!(
            Value::Object(props),
            json!({ "text": "", "fontStyle": "italic" })
        );
        assert_eq!(
            Value::Object(state),
            json!({ "longestSecs": 12 * 86_400 + 3_600 + 180 })
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        for empty in ["", " ", "0:0:0:0"] {
            let (_, state, warnings) = convert(&[("max-uptime", empty)]);
            assert!(state.is_empty() && warnings.is_empty(), "{empty:?}");
        }
        let (_, state, warnings) = convert(&[("max-uptime", "12 days")]);
        assert!(state.is_empty());
        assert_eq!(
            warnings,
            [
                "DockUpTime: “12 days” is not a valid value of “max-uptime”, so its default is used \
              instead."
            ]
        );
        // Not an uptime: over a century (review M4).
        let (_, state, warnings) = convert(&[("max-uptime", "0:0:0:18446744073709551615")]);
        assert!(state.is_empty());
        assert_eq!(warnings.len(), 1);
    }

    fn read(secs: u64, read_at: Instant) -> Sample {
        Sample::Uptime(UptimeSample {
            boot_unix_ms: 0,
            uptime_secs: secs,
            read_at,
        })
    }

    fn with_ctx<R>(f: impl FnOnce(&SensorCtx) -> R) -> R {
        let (settings, theme) = (Settings::default(), theme());
        f(&SensorCtx {
            id: "s_up",
            settings: &settings,
            theme: &theme,
            locked: false,
            alt: false,
            animate: true,
            changes: &crate::sampler::ChangeSink::NONE,
            platform: &NoPlatform,
        })
    }

    const DAY: u64 = 86_400;

    fn drawn(sensor: &mut dyn Sensor) -> String {
        let theme = theme();
        let width = sensor.measure(&LayoutCtx {
            theme: &theme,
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
                theme: &theme,
                rect,
            },
        );
        canvas.dump()
    }

    #[test]
    fn shows_the_uptime_after_its_prefix() {
        let mut sensor = uptime(json!({}), json!({}));
        assert_eq!(sensor.topics()[0].interval, RESAMPLE);
        assert!(
            drawn(sensor.as_mut()).is_empty(),
            "nothing before the first sample"
        );
        let up = 3 * DAY + 4 * 3600 + 12 * 60 + 30;
        assert!(with_ctx(|cx| sensor.on_sample(
            &Topic::Uptime,
            &sample(up),
            cx
        )));
        let dump = drawn(sensor.as_mut());
        assert!(dump.starts_with("text \"Uptime: 3d 04h 12m\""), "{dump}");
        assert!(dump.contains(" 700 12px "), "bold 12 px: {dump}");
        // The next whole minute of uptime is 30 s away.
        let woken = sensor.wake_at(Instant::now()).expect("a wake-up");
        assert!(woken - Instant::now() <= Duration::from_secs(30) + PAST_BOUNDARY);

        let mut bare = uptime(json!({ "text": "" }), json!({}));
        with_ctx(|cx| bare.on_sample(&Topic::Uptime, &sample(125), cx));
        assert!(drawn(bare.as_mut()).starts_with("text \"2m\""));
    }

    #[test]
    fn wakes_at_each_minute_of_uptime() {
        let mut sensor = uptime(json!({}), json!({}));
        // Read 90 s ago at 12m 30s (a catch-up keeps its reading time): 14m now.
        let read_at = Instant::now() - Duration::from_secs(90);
        with_ctx(|cx| sensor.on_sample(&Topic::Uptime, &read(750, read_at), cx));
        assert!(drawn(sensor.as_mut()).contains("\"Uptime: 14m\""));
        // The 15th minute is complete 150 s after the reading.
        let boundary = read_at + Duration::from_secs(900 - 750) + PAST_BOUNDARY;
        assert_eq!(sensor.wake_at(Instant::now()), Some(boundary));
        assert!(with_ctx(|cx| sensor.on_wake(boundary, cx)), "15m");
        assert!(drawn(sensor.as_mut()).contains("\"Uptime: 15m\""));
        // A late timer: the wake-up stays due, and the text catches up.
        let next = boundary + Duration::from_secs(60);
        assert_eq!(sensor.wake_at(boundary), Some(next));
        let late = next + Duration::from_secs(100);
        assert!(sensor.wake_at(late).is_some_and(|at| at <= late));
        assert!(with_ctx(|cx| sensor.on_wake(late, cx)));
        assert!(drawn(sensor.as_mut()).contains("\"Uptime: 17m\""));
        assert!(!with_ctx(|cx| sensor.on_wake(late, cx)), "nothing new");
    }

    #[test]
    fn hint_shows_the_longer_record() {
        let mut sensor = uptime(json!({}), json!({ "longestSecs": 12 * DAY + 3600 + 180 }));
        with_ctx(|cx| sensor.on_sample(&Topic::Uptime, &sample(4 * 3600 + 12 * 60), cx));
        let hint = with_ctx(|cx| sensor.hint(cx));
        assert_eq!(
            hint,
            Some(HintContent::Text(
                "4 hours, 12 minutes\nLongest uptime: 12 days, 1 hour, 3 minutes".into()
            ))
        );
        assert_eq!(sensor.take_state(), None, "no new record");
    }

    #[test]
    fn a_new_record_is_stored_in_steps() {
        let mut sensor = uptime(json!({}), json!({ "longestSecs": DAY }));
        with_ctx(|cx| sensor.on_sample(&Topic::Uptime, &sample(DAY + 60), cx));
        assert_eq!(sensor.take_state(), None, "only a minute longer");
        let hint = with_ctx(|cx| sensor.hint(cx));
        assert_eq!(
            hint,
            Some(HintContent::Text("1 day, 1 minute".into())),
            "no record line"
        );
        with_ctx(|cx| sensor.on_sample(&Topic::Uptime, &sample(DAY + 600), cx));
        let state = sensor.take_state().expect("ten minutes longer");
        assert_eq!(state[LONGEST], json!(DAY + 600));
        assert_eq!(sensor.take_state(), None, "taken once");

        // A record as large as a u64 holds (from a file) neither overflows nor is stored again.
        let mut sensor = uptime(json!({}), json!({ "longestSecs": u64::MAX }));
        with_ctx(|cx| sensor.on_sample(&Topic::Uptime, &sample(DAY), cx));
        assert_eq!(sensor.take_state(), None);
    }
}
