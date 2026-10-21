//! The sensor framework (SPEC §8). A sensor *type* is a static description: its id, category,
//! names, required capabilities and property schema; it creates *sensors*, the placed instances
//! that live on the bar thread and are re-created when their props change. Adding a sensor
//! type means adding one module and listing it in `registry.rs`.
//!
//! The [`Sensor`] trait grows with the slices that need it: topics and samples arrive with the
//! sampler (M1 S10), clicks and menus with the interaction slices (M1 S5–S8, M3 S2), drops with
//! M3, and `convert_legacy` with the legacy import (M4 S9).

pub mod battery;
pub mod color;
pub mod common;
pub mod cpu;
pub mod date;
pub mod dates;
pub mod disk;
pub mod disk_launcher;
pub mod drop;
pub mod eject_usb;
pub mod folder_menu;
pub mod format;
pub mod image;
pub mod import;
pub mod legacy;
pub mod memory;
pub mod network;
pub mod ops;
pub mod placeholder;
pub mod popup;
pub mod power;
pub mod recycle_bin;
pub mod registry;
pub mod schema;
pub mod shortcut;
pub mod spacer;
pub mod start_menu;
pub mod text;
pub mod time;
pub mod uptime;
pub mod wifi;

use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::bar::theme::Theme;
use crate::config::legacy::LegacyProps;
use crate::config::model::Settings;
use crate::platform::{Bitmap, Canvas, Capability, Platform, PointF, RectF, SizeF, TextEngine};
use crate::sampler::{ChangeSink, Sample, Topic, TopicRequest};
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
    /// The XML keys of its legacy types and the props they set (SPEC §9 "XML key").
    fn legacy_keys(&self) -> &'static [legacy::LegacyKey] {
        &[]
    }
    /// The props of a legacy sensor of one of [`Self::legacy_types`] (`cx.legacy_type`): by
    /// default each of [`Self::legacy_keys`] read by its field's encoding, with warnings for
    /// values that cannot be read and properties it does not name (SPEC §11.4).
    fn convert_legacy(&self, raw: &LegacyProps, cx: &mut legacy::LegacyCtx) -> Props {
        legacy::convert(self.schema(), self.legacy_keys(), raw, cx)
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
    /// A note the picker adds to the description on this machine (SPEC §6.6), e.g. the CPU
    /// dual graph's on a single-processor computer.
    fn picker_note(&self, _ctx: &DefaultsCtx) -> Option<MsgKey> {
        None
    }
}

/// What measuring may use (bar thread).
pub struct LayoutCtx<'a> {
    pub theme: &'a Theme,
    pub text: &'a dyn TextEngine,
    /// Physical px per logical px, e.g. for decoding images at their drawn size.
    pub scale: f32,
    /// How the locale writes numbers (SPEC §8.6), kept by the bar.
    pub numbers: format::Numbers,
    /// Shell icons of files, folders and drives (SPEC §9.17–9.18).
    pub icons: &'a dyn IconSource,
}

/// A shell icon as an [`IconSource`] has it.
#[derive(Debug, Clone, PartialEq)]
pub enum ShellIcon {
    /// Being read; the sensors measure again once it is there.
    Loading,
    /// The shell has none, e.g. for a path that does not exist.
    None,
    Ready(Bitmap),
}

/// Shell icons for measuring sensors (bar thread). The bar reads them on a worker thread,
/// since the shell may wait for a network server (SPEC §4.1).
pub trait IconSource {
    /// The icon of `path`, `px` physical px square.
    fn shell_icon(&self, path: &str, px: u32) -> ShellIcon;
    /// Forgets the icons of `path`, so they are read again, e.g. once a missing target is back.
    fn forget(&self, path: &str);
}

/// No shell icons: tests, and platforms without a shell.
pub struct NoIcons;

impl IconSource for NoIcons {
    fn shell_icon(&self, _path: &str, _px: u32) -> ShellIcon {
        ShellIcon::None
    }

    fn forget(&self, _path: &str) {}
}

/// Shell icons read from the platform at once, on the calling thread: for screenshots.
pub struct DirectIcons<'a>(pub &'a dyn Platform);

impl IconSource for DirectIcons<'_> {
    fn shell_icon(&self, path: &str, px: u32) -> ShellIcon {
        self.0
            .shell_icon(path, px)
            .map_or(ShellIcon::None, ShellIcon::Ready)
    }

    fn forget(&self, _path: &str) {}
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
    /// Tells the sampler what a sensor's action changed, e.g. that the Recycle Bin was emptied,
    /// so its topics are read again at once (SPEC §4.4); clone it for actions that finish on
    /// another thread.
    pub changes: &'a ChangeSink,
}

impl SensorCtx<'_> {
    /// Whether a click would reach the sensor now (SPEC §6.1): hints end with their
    /// "Click to …" line only then. Hints are built without Alt held, so with Lock and
    /// *Alt only* they have none.
    pub fn clicks(&self) -> bool {
        crate::bar::clicks_allowed(self.locked, self.settings.clicks_when_locked, self.alt)
    }

    /// `text` with the "Click to …" line `key` when a click would reach the sensor.
    pub fn with_click_line(&self, mut text: String, key: MsgKey) -> String {
        if self.clicks() {
            text.push('\n');
            text.push_str(&crate::i18n::t(key));
        }
        text
    }
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

/// A sensor's own menu item id; unique within its type.
pub type MenuId = u16;

/// One of a sensor's own items in the sensor menu (SPEC §6.4), e.g. *Eject*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: MenuId,
    pub label: String,
    /// Greyed out unless set.
    pub enabled: bool,
}

impl MenuItem {
    pub fn new(id: MenuId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            enabled: true,
        }
    }

    /// The same item, greyed out unless `enabled`.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// A question the bar asks in a native dialog before a sensor acts (SPEC §6.4, §9.21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The main text, e.g. "Do you really want to shut down this computer?".
    pub message: String,
    /// The confirming button, e.g. "Shut down".
    pub action: String,
}

/// What a click did (SPEC §8.1).
pub enum ClickResult {
    /// The sensor has no click action.
    None,
    Handled,
    /// The bar asks this first and calls [`Sensor::confirmed`] if the user agrees.
    Confirm(Question),
    /// The bar shows this popup next to the sensor (SPEC §7.2).
    Popup(Box<dyn popup::Popup>),
}

impl PartialEq for ClickResult {
    /// Popups are never equal.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) | (Self::Handled, Self::Handled) => true,
            (Self::Confirm(a), Self::Confirm(b)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Debug for ClickResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Handled => f.write_str("Handled"),
            Self::Confirm(q) => f.debug_tuple("Confirm").field(q).finish(),
            Self::Popup(p) => write!(f, "Popup({})", p.kind()),
        }
    }
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
    /// The user agreed to the [`ClickResult::Confirm`] question of its last click; returns
    /// whether what it shows changed.
    fn confirmed(&mut self, _cx: &SensorCtx) -> Dirty {
        false
    }
    /// Its own items for the sensor menu (SPEC §6.4), shown between *Remove sensor* and
    /// *Tools*; the menu is built each time it opens.
    fn menu(&self, _cx: &SensorCtx) -> Vec<MenuItem> {
        Vec::new()
    }
    /// One of its menu items was chosen; returns whether what it shows changed.
    fn on_menu(&mut self, _item: MenuId, _cx: &SensorCtx) -> Dirty {
        false
    }
    /// What dropping `items` here would do (SPEC §6.3): the effect and the floating hint;
    /// `None` (the default) has the bar insert new sensors next to this one instead. Asked
    /// when the drag reaches the sensor and when its keys change.
    fn drop_action(
        &self,
        _items: &[drop::Dropped],
        _drag: drop::DragInfo,
        _cx: &SensorCtx,
    ) -> Option<drop::DropAction> {
        None
    }
    /// `items` were dropped here, as [`Sensor::drop_action`] said.
    fn drop_items(&mut self, _items: &[drop::Dropped], _drag: drop::DragInfo, _cx: &SensorCtx) {}
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
    /// Props it changed itself, to be saved (SPEC §9.14: a click toggles what the Battery
    /// shows), as the changed keys; the bar asks after clicks, and each change is taken once.
    fn take_props(&mut self) -> Option<Props> {
        None
    }
    /// Takes changed props in place, keeping what it has gathered (e.g. the Battery's charging
    /// history), and returns `true`; with `false` (the default) the bar creates it anew.
    /// `props` are resolved.
    fn reconfigure(&mut self, _props: &Props) -> bool {
        false
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
            MACHINE_READS.with(|n| n.set(n.get() + 1));
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
        fn reveal(&self, path: &str) {
            self.log
                .lock()
                .expect("lock")
                .push(format!("reveal {path}"));
        }
        fn known_folder(&self, folder: crate::platform::KnownFolder) -> Option<String> {
            Some(match folder {
                crate::platform::KnownFolder::Programs => "C:\\Users\\me\\Programs".into(),
                crate::platform::KnownFolder::CommonPrograms => "C:\\ProgramData\\Programs".into(),
            })
        }
        fn eject(&self, root: &str, report: crate::platform::EjectReport) {
            let (title, body) = report(&Ok(()));
            self.log
                .lock()
                .expect("lock")
                .push(format!("eject {root}: {title}: {body}"));
        }
        fn file_operation(
            &self,
            op: crate::platform::FileOp,
            paths: Vec<String>,
            folder: String,
            done: Box<dyn FnOnce() + Send>,
        ) {
            self.log
                .lock()
                .expect("lock")
                .push(format!("{op:?} {} into {folder}", paths.join(", ")));
            done();
        }
        fn recycle(&self, paths: Vec<String>, done: Box<dyn FnOnce() + Send>) {
            self.log
                .lock()
                .expect("lock")
                .push(format!("recycle {}", paths.join(", ")));
            done();
        }
        fn create_url_file(&self, folder: &str, stem: &str, url: &str) {
            self.log
                .lock()
                .expect("lock")
                .push(format!("url file {folder}\\{stem}.url {url}"));
        }
        fn launch(&self, launch: &crate::platform::Launch) {
            self.log.lock().expect("lock").push(format!(
                "launch {} args={} dir={} admin={}",
                launch.target,
                launch.args,
                launch.dir.unwrap_or_default(),
                launch.admin
            ));
        }
    }

    thread_local! {
        /// How often this thread asked [`Clock`] for its machine defaults.
        static MACHINE_READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    pub fn machine_reads() -> u32 {
        MACHINE_READS.with(std::cell::Cell::get)
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
            numbers: Default::default(),
            icons: &crate::sensors::NoIcons,
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
