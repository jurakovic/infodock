//! The sensor framework (SPEC §8). A sensor *type* is a static description: its id, category,
//! names, required capabilities and property schema; it creates *sensors*, the placed instances
//! that live on the bar thread and are re-created when their props change. Adding a sensor
//! type means adding one module and listing it in `registry.rs`.
//!
//! The [`Sensor`] trait grows with the slices that need it: topics and samples arrive with the
//! sampler (M1 S10), wake-ups with the first timed sensors, clicks, menus and drops with the
//! interaction slices (S5–S8, M3), and `from_legacy` with the legacy import (M4).

pub mod placeholder;
pub mod registry;
pub mod schema;

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
    /// A sensor for resolved `props` (every schema field present and valid).
    fn create(&self, props: &Props, state: &State) -> Box<dyn Sensor>;
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
    /// Drives inactive rendering (SPEC §8.5).
    fn inactive(&self) -> bool {
        false
    }
    /// The value read by screen readers (SPEC §14.4).
    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        String::new()
    }
}
