//! `uptime` — how long the system has been running (SPEC §9.6), with the longest uptime seen
//! so far kept in the sensor's `state`.

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use super::format::{duration_compact, duration_long};
use super::schema::{FieldKind, Lit, PropField};
use super::text::shadow_color;
use super::{
    Category, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx, SensorType, State, typed,
};
use crate::config::model::FontStyle;
use crate::i18n::t_args;
use crate::platform::{Canvas, Font, PointF, TextLayout, TextStyle};
use crate::sampler::{Sample, Topic, TopicRequest};

/// The `uptime` topic is read this often; the sensor counts on its own in between.
const RESAMPLE: Duration = Duration::from_secs(60);
/// A new record is stored once it has grown this much, so the config is not rewritten every
/// minute; at most this much of a record is lost if Infodock ends abruptly.
const RECORD_STEP: u64 = 10 * 60;
/// The `state` key of the record (SPEC §11.2; XML `max-uptime`).
const LONGEST: &str = "longestSecs";

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

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        let longest = state.get(LONGEST).and_then(Value::as_u64).unwrap_or(0);
        Ok(Box::new(Uptime {
            props: typed(props)?,
            reading: None,
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
    /// The last sample's uptime and when it arrived.
    reading: Option<(u64, Instant)>,
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

    fn text(&self, at: Instant) -> Option<String> {
        let uptime = duration_compact(self.uptime(at)?);
        Some(if self.props.text.is_empty() {
            uptime
        } else {
            format!("{} {uptime}", self.props.text)
        })
    }

    /// Notes the uptime at `at` as a record if it is one.
    fn note(&mut self, at: Instant) {
        if let Some(uptime) = self.uptime(at) {
            self.longest = self.longest.max(uptime);
        }
    }
}

impl Sensor for Uptime {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let Some(text) = self.text(Instant::now()) else {
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
        let now = Instant::now();
        let before = self.text(now);
        self.reading = Some((sample.uptime_secs, now));
        self.note(now);
        self.text(now) != before
    }

    /// At the next whole minute of uptime.
    fn wake_at(&self, _now: Instant) -> Option<Instant> {
        let (secs, received) = self.reading?;
        let elapsed = received.elapsed().as_secs();
        let next = 60 - (secs + elapsed) % 60;
        Some(received + Duration::from_secs(elapsed + next))
    }

    fn on_wake(&mut self, now: Instant, _cx: &SensorCtx) -> bool {
        self.note(now);
        true
    }

    fn take_state(&mut self) -> Option<State> {
        if self.longest < self.stored + RECORD_STEP {
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
        Sample::Uptime(UptimeSample {
            boot_unix_ms: 0,
            uptime_secs: secs,
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
        assert!(woken - Instant::now() <= Duration::from_secs(30));

        let mut bare = uptime(json!({ "text": "" }), json!({}));
        with_ctx(|cx| bare.on_sample(&Topic::Uptime, &sample(125), cx));
        assert!(drawn(bare.as_mut()).starts_with("text \"2m\""));
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
    }
}
