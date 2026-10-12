//! The sensor framework (SPEC §8). A sensor *type* is a static description: its id, category,
//! names, required capabilities and property schema; it creates *sensors*, the placed instances
//! that live on the bar thread and are re-created when their props change. Adding a sensor
//! type means adding one module and listing it in `registry.rs`.
//!
//! The [`Sensor`] trait grows with the slices that need it: topics and samples arrive with the
//! sampler (M1 S10), clicks, menus and drops with the interaction slices (S5–S8, M3), and
//! `from_legacy` with the legacy import (M4).

pub mod image;
pub mod ops;
pub mod placeholder;
pub mod registry;
pub mod schema;
pub mod spacer;
pub mod text;

use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::bar::theme::Theme;
use crate::config::model::Settings;
use crate::platform::{Canvas, Capability, Platform, RectF, TextEngine};
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
    pub platform: &'a dyn Platform,
}

/// The content of a hover hint (SPEC §7.1).
#[derive(Debug, Clone, PartialEq)]
pub enum HintContent {
    /// Plain text; `\n` breaks lines, tabs align "label: value" columns.
    Text(String),
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
    use super::{DrawCtx, LayoutCtx, Sensor};
    use crate::bar::theme::Theme;
    use crate::config::model::Settings;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::platform::{OsAppearance, RectF};

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
