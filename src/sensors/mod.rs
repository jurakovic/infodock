//! The sensor framework (SPEC §8). A sensor *type* is a static description: its id, category,
//! names, required capabilities and property schema; it creates *sensors*, the placed instances
//! that live on the bar thread and are re-created when their props change. Adding a sensor
//! type means adding one module and listing it in `registry.rs`.
//!
//! The [`Sensor`] trait grows with the slices that need it: topics and samples arrive with the
//! sampler (M1 S10), clicks, menus and drops with the interaction slices (S5–S8, M3), and
//! `from_legacy` with the legacy import (M4).

pub mod color;
pub mod common;
pub mod cpu;
pub mod date;
pub mod dates;
pub mod format;
pub mod image;
pub mod ops;
pub mod placeholder;
pub mod registry;
pub mod schema;
pub mod spacer;
pub mod text;
pub mod time;
pub mod uptime;

use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::bar::theme::Theme;
use crate::config::model::Settings;
use crate::platform::{Canvas, Capability, Platform, PointF, RectF, SizeF, TextEngine};
use crate::sampler::{Sample, Topic, TopicRequest};
use schema::PropField;

/// An `i18n` message key.
pub type MsgKey = &'static str;

/// A sensor's props: as stored, only the values that differ from the defaults (SPEC §11.2);
/// resolved, every schema field.
pub type Props = Map<String, Value>;

/// Runtime state kept per sensor id in the config, e.g. the uptime record.
pub type State = Map<String, Value>;

/// Whether what a sensor shows changed; the bar then re-measures and redraws it (SPEC §4.4).
pub type Dirty = bool;

/// Resolved props as a type's typed props struct (`camelCase` fields, as in the schema).
pub fn typed<T: DeserializeOwned>(props: &Props) -> serde_json::Result<T> {
    serde_json::from_value(Value::Object(props.clone()))
}

/// A `colorMode` prop (SPEC §8.2, §8.4).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ColorChoice {
    pub mode: ColorModeKind,
    /// `#RRGGBB`, used in custom mode.
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorModeKind {
    /// The sensor text colour.
    Standard,
    Custom,
    /// Green → yellow → red by value.
    Dynamic,
}

/// Groups of the sensor picker (SPEC §6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    General,
    Info,
    System,
    Network,
    Launchers,
}

impl Category {
    pub fn name(self) -> MsgKey {
        match self {
            Category::General => "sensor.category.general",
            Category::Info => "sensor.category.info",
            Category::System => "sensor.category.system",
            Category::Network => "sensor.category.network",
            Category::Launchers => "sensor.category.launchers",
        }
    }
}

/// What a sensor type may consult for its machine-derived defaults (SPEC §8.2, §11.2).
pub struct DefaultsCtx<'a> {
    pub platform: &'a dyn Platform,
}

/// One per sensor module; registered in `registry.rs`. Static and thread-safe (SPEC §8.1).
pub trait SensorType: Send + Sync {
    /// Stable id, e.g. `cpu-graph`.
    fn id(&self) -> &'static str;
    fn category(&self) -> Category;
    /// Localised display name.
    fn name(&self) -> MsgKey;
    fn description(&self) -> MsgKey;
    /// Legacy XML type ids this type imports (SPEC §11.4).
    fn legacy_types(&self) -> &'static [&'static str] {
        &[]
    }
    fn requires(&self) -> &'static [Capability] {
        &[]
    }
    /// Ordered fields: the Settings form, validation and static defaults (SPEC §8.2).
    fn schema(&self) -> &'static [PropField] {
        &[]
    }
    /// Values for the machine-derived fields on this machine.
    fn machine_defaults(&self, _ctx: &DefaultsCtx) -> Props {
        Props::new()
    }
    /// A sensor for resolved `props` (every schema field present and valid). Fails only if the
    /// type's typed props do not match its schema, which its tests rule out.
    fn create(&self, props: &Props, state: &State) -> serde_json::Result<Box<dyn Sensor>>;
    /// What tells this instance apart in lists such as the Layout manager, e.g. a Text
    /// sensor's text; `props` are resolved. `None` when the type's name says it all.
    fn summary(&self, _props: &Props) -> Option<String> {
        None
    }
}

/// What measuring may use (bar thread).
pub struct LayoutCtx<'a> {
    pub theme: &'a Theme,
    pub text: &'a dyn TextEngine,
    /// Physical px per logical px, e.g. for decoding images at their drawn size.
    pub scale: f32,
}

/// What drawing may use (bar thread).
pub struct DrawCtx<'a> {
    pub theme: &'a Theme,
    /// The sensor's place on the bar, in logical px; its height is the bar's.
    pub rect: RectF,
}

/// What a sensor may read when asked for its hint or accessible value (SPEC §8.1). Actions
/// (state, props, notifications, opening Settings) arrive with the slices that need them.
pub struct SensorCtx<'a> {
    pub id: &'a str,
    pub settings: &'a Settings,
    pub theme: &'a Theme,
    pub locked: bool,
    /// Alt is held.
    pub alt: bool,
    /// Transitions animate (SPEC §5.5); otherwise they jump.
    pub animate: bool,
    pub platform: &'a dyn Platform,
}

/// The content of a hover hint (SPEC §7.1).
pub enum HintContent {
    /// Plain text; `\n` breaks lines, tabs align "label: value" columns.
    Text(String),
    /// Content that lays itself out and draws itself, e.g. Date's calendar.
    Rich(Box<dyn RichHint>),
}

impl PartialEq for HintContent {
    /// Text compares by its text; rich content is never equal.
    fn eq(&self, other: &Self) -> bool {
        matches!((self, other), (Self::Text(a), Self::Text(b)) if a == b)
    }
}

impl std::fmt::Debug for HintContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::Rich(_) => f.write_str("Rich(..)"),
        }
    }
}

/// A hint that lays itself out and draws itself (SPEC §7.1), in the popup font and colours.
pub trait RichHint {
    /// Lays the content out; returns its size, logical px.
    fn layout(&mut self, text: &dyn TextEngine, theme: &Theme) -> SizeF;
    /// Draws it with its top-left corner at `origin`, after [`RichHint::layout`].
    fn draw(&self, canvas: &mut dyn Canvas, origin: PointF, theme: &Theme);
}

/// What a click did (SPEC §8.1). Popups arrive with the sensors that open them (M3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickResult {
    /// The sensor has no click action.
    None,
    Handled,
}

/// A placed sensor. Lives on the bar thread; re-created when its props change (SPEC §8.1).
pub trait Sensor {
    /// Width in logical px; the height is the bar's. Called after creation and whenever the
    /// sensor or the theme changed, so text layouts and bitmaps are built (and kept) here.
    fn measure(&mut self, cx: &LayoutCtx) -> f32;
    /// Draws into `cx.rect`.
    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx);
    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        None
    }
    /// The click action (SPEC §6.1); called only when the Lock rules allow clicks.
    fn click(&mut self, _cx: &SensorCtx) -> ClickResult {
        ClickResult::None
    }
    /// Drives inactive rendering (SPEC §8.5).
    fn inactive(&self) -> bool {
        false
    }
    /// The value read by screen readers (SPEC §14.4).
    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        String::new()
    }
    /// The pointer entered (`true`) or left the sensor, e.g. to fade something in. A sensor
    /// that animates asks for frames through [`Sensor::wake_at`].
    fn on_hover(&mut self, _hovered: bool, _now: Instant, _cx: &SensorCtx) -> Dirty {
        false
    }
    /// The system data this sensor needs (SPEC §4.4); asked once per instance.
    fn topics(&self) -> Vec<TopicRequest> {
        Vec::new()
    }
    /// A new sample of one of its topics; returns whether what it shows changed.
    fn on_sample(&mut self, _topic: &Topic, _sample: &Sample, _cx: &SensorCtx) -> Dirty {
        false
    }
    /// State to store for this sensor in the config (SPEC §11.2 `state`), e.g. a new uptime
    /// record; the bar asks after samples and wake-ups, and each change is taken once.
    fn take_state(&mut self) -> Option<State> {
        None
    }
    /// The wall clock jumped: the time or time zone changed, or the system resumed. Clocks
    /// read the time again and move their wake-ups (SPEC §9.4–9.5).
    fn on_clock_change(&mut self, _now: Instant, _cx: &SensorCtx) -> Dirty {
        false
    }
    /// When the sensor next needs [`Sensor::on_wake`], for timer-driven sensors (clock,
    /// uptime). The bar keeps one timer for the earliest wake-up of all sensors.
    fn wake_at(&self, _now: Instant) -> Option<Instant> {
        None
    }
    /// Called at or after [`Sensor::wake_at`].
    fn on_wake(&mut self, _now: Instant, _cx: &SensorCtx) -> Dirty {
        false
    }
}

/// Helpers for the sensor types' tests: the default modern theme, fixed text metrics, scale 1.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    use serde_json::json;

    use super::dates::{Date, LocalTime};
    use super::schema::{FieldKind, Lit, PropField};
    use super::{Category, DefaultsCtx, DrawCtx, LayoutCtx, Props, Sensor, SensorType, State};
    use crate::bar::theme::Theme;
    use crate::config::model::DateLocale;
    use crate::config::model::Settings;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::platform::{
        Canvas, Capabilities, Locale, MonitorInfo, OsAppearance, Platform, RectF, TextEngine,
        TextStyle,
    };

    /// Draws its text as the label, so tests can see what it was created with.
    pub struct Echo(pub String);

    impl Sensor for Echo {
        fn measure(&mut self, _cx: &LayoutCtx) -> f32 {
            10.0
        }

        fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
            let layout = FixedMetrics.layout(&self.0, &TextStyle::new(&cx.theme.sensor_font, 10.0));
            canvas.draw_text(&layout, Default::default(), cx.theme.sensor_text);
        }
    }

    const CLOCK_SCHEMA: &[PropField] = &[
        PropField::new("use24h", FieldKind::Bool, "x", Lit::Bool(true)).machine(),
        PropField::new("showSeconds", FieldKind::Bool, "x", Lit::Bool(false)),
    ];

    /// A type with a machine-derived field: `use24h` defaults to `true`, the machine says
    /// `false`. It draws its props.
    pub struct Clock;

    impl SensorType for Clock {
        fn id(&self) -> &'static str {
            "clock"
        }
        fn category(&self) -> Category {
            Category::Info
        }
        fn name(&self) -> &'static str {
            "test.clock"
        }
        fn description(&self) -> &'static str {
            "test.clock.description"
        }
        fn schema(&self) -> &'static [PropField] {
            CLOCK_SCHEMA
        }
        fn machine_defaults(&self, _ctx: &DefaultsCtx) -> Props {
            let mut p = Props::new();
            p.insert("use24h".into(), json!(false));
            p
        }
        fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
            Ok(Box::new(Echo(format!(
                "use24h={} showSeconds={}",
                props["use24h"], props["showSeconds"]
            ))))
        }
    }

    /// A platform with a settable clock (2026-10-13, a Tuesday) and the en-US locale that
    /// records what it is asked to play and open.
    #[derive(Default)]
    pub struct FakeClock {
        time: Mutex<Option<LocalTime>>,
        log: Mutex<Vec<String>>,
    }

    impl FakeClock {
        pub fn set(&self, hour: u8, minute: u8, second: u8) {
            let date = Date::new(2026, 10, 13).expect("a date");
            *self.time.lock().expect("lock") = Some(LocalTime {
                date,
                hour,
                minute,
                second,
                millis: 0,
            });
        }

        pub fn log(&self) -> Vec<String> {
            self.log.lock().expect("lock").clone()
        }
    }

    impl Platform for FakeClock {
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn monitors(&self) -> Vec<MonitorInfo> {
            Vec::new()
        }
        fn appearance(&self) -> OsAppearance {
            OsAppearance::default()
        }
        fn uptime(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }
        fn local_time(&self) -> LocalTime {
            self.time.lock().expect("lock").expect("set first")
        }
        fn locale(&self, _which: DateLocale) -> Locale {
            Locale::en_us()
        }
        fn play_sound(&self, wav: &'static [u8]) {
            self.log
                .lock()
                .expect("lock")
                .push(format!("play {}", wav.len()));
        }
        fn open(&self, target: &str) {
            self.log
                .lock()
                .expect("lock")
                .push(format!("open {target}"));
        }
    }

    /// A platform without capabilities or monitors.
    pub struct NoPlatform;

    impl Platform for NoPlatform {
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn monitors(&self) -> Vec<MonitorInfo> {
            Vec::new()
        }
        fn appearance(&self) -> OsAppearance {
            OsAppearance::default()
        }
        fn uptime(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }
    }

    pub fn theme() -> Theme {
        Theme::resolve(&Settings::default(), OsAppearance::default())
    }

    pub fn measure(sensor: &mut dyn Sensor) -> f32 {
        sensor.measure(&LayoutCtx {
            theme: &theme(),
            text: &FixedMetrics,
            scale: 1.0,
        })
    }

    /// What `sensor` draws at x = 7 with `width`, on a 28 px bar.
    pub fn draw(sensor: &dyn Sensor, width: f32) -> String {
        let mut canvas = RecordingCanvas::new(1.0);
        sensor.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme(),
                rect: RectF::new(7.0, 0.0, 7.0 + width, 28.0),
            },
        );
        canvas.dump()
    }
}
