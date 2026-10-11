//! Typed configuration (SPEC §10, §11.2).
//!
//! Settings whose default depends on the theme are `Option`s: `None` (`null` in JSON) means
//! "use the theme default" from [`ThemeDefaults`]. Every struct uses `#[serde(default)]`, so a
//! partial or older file still loads and missing keys take their defaults.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

/// Current config schema version (SPEC §11.2). Bump together with a step in `migrate.rs`.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Config {
    pub schema: u32,
    pub meta: Meta,
    pub settings: Settings,
    pub layout: Layout,
    /// Runtime state per sensor id, e.g. the uptime record (`set_sensor_state`).
    #[ts(type = "Record<string, Record<string, unknown>>")]
    pub state: BTreeMap<String, Map<String, Value>>,
    pub legacy: Legacy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            meta: Meta::default(),
            settings: Settings::default(),
            layout: Layout::default(),
            state: BTreeMap::new(),
            legacy: Legacy::default(),
        }
    }
}

impl Config {
    /// The config created on first run (SPEC §11.5): CPU Graph, Memory Meter and Network Graph on
    /// the left, the logo in the center, Date and Time on the right.
    ///
    /// Machine-derived props (Time `use24h`) are filled in by the sensor framework on first load.
    pub fn first_run(app_version: &str) -> Self {
        let mut ids = IdGen::default();
        let mut sensor = |kind: &str, props: Map<String, Value>| SensorInstance {
            id: ids.fresh(),
            kind: kind.to_owned(),
            props,
        };
        let mut image_props = Map::new();
        image_props.insert("popupText".into(), Value::String("Infodock %v".into()));
        let layout = Layout {
            left: vec![
                sensor("cpu-graph", Map::new()),
                sensor("memory-meter", Map::new()),
                sensor("network-graph", Map::new()),
            ],
            center: vec![sensor("image", image_props)],
            right: vec![sensor("date", Map::new()), sensor("time", Map::new())],
        };
        Self {
            meta: Meta {
                created_with: app_version.to_owned(),
                last_written_with: app_version.to_owned(),
                first_run_done: false,
            },
            layout,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Meta {
    pub created_with: String,
    pub last_written_with: String,
    pub first_run_done: bool,
}

// ---------------------------------------------------------------------------------------------
// Settings (SPEC §10)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    // §10.1 General
    /// `"auto"` or a catalog id.
    pub language: String,
    pub date_locale: DateLocale,
    pub autostart: bool,
    pub show_tray_icon: bool,
    /// Accelerator, e.g. `Ctrl+Alt+Shift+I`.
    pub hotkey: String,
    pub hotkey_action: HotkeyAction,
    pub locked: bool,
    pub popups_enabled: bool,
    pub clicks_when_locked: ClicksWhenLocked,
    /// Gap between sensors in logical px, 1–20.
    pub sensor_spacing: u32,
    pub new_sensor_menu: NewSensorMenu,
    // §10.2
    pub bar: BarSettings,
    // §10.3
    pub theme: Theme,
    pub background: BackgroundSettings,
    pub classic: ClassicSettings,
    // §10.4
    pub sensors: SensorDefaults,
    // §10.5
    pub popups: PopupSettings,
    // §10.6
    pub apps: AppSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "auto".into(),
            date_locale: DateLocale::System,
            autostart: false,
            show_tray_icon: true,
            hotkey: "Ctrl+Alt+Shift+I".into(),
            hotkey_action: HotkeyAction::BringToFront,
            locked: false,
            popups_enabled: true,
            clicks_when_locked: ClicksWhenLocked::Always,
            sensor_spacing: 10,
            new_sensor_menu: NewSensorMenu::Dialog,
            bar: BarSettings::default(),
            theme: Theme::Modern,
            background: BackgroundSettings::default(),
            classic: ClassicSettings::default(),
            sensors: SensorDefaults::default(),
            popups: PopupSettings::default(),
            apps: AppSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct BarSettings {
    pub edge: Edge,
    pub mode: BarMode,
    /// `"primary"` or a monitor device name such as `\\.\DISPLAY2`.
    pub monitor: String,
    pub reserve_space: bool,
    pub z_order: ZOrder,
    pub hide_on_fullscreen: bool,
    /// Logical px, 22–40; `None` = theme default.
    pub height: Option<u32>,
    pub floating: Option<FloatingRect>,
    pub highlight_on_hover: bool,
    pub shadow: bool,
    pub animations: Animations,
}

impl Default for BarSettings {
    fn default() -> Self {
        Self {
            edge: Edge::Bottom,
            mode: BarMode::Docked,
            monitor: PRIMARY_MONITOR.into(),
            reserve_space: true,
            z_order: ZOrder::Topmost,
            hide_on_fullscreen: true,
            height: None,
            floating: None,
            highlight_on_hover: false,
            shadow: false,
            animations: Animations::Auto,
        }
    }
}

/// Value of [`BarSettings::monitor`] that selects the primary monitor.
pub const PRIMARY_MONITOR: &str = "primary";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FloatingRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct BackgroundSettings {
    pub color_mode: BackgroundColorMode,
    /// `#RRGGBB`; `None` = theme default.
    pub color: Option<String>,
    /// Percent; `None` = theme default.
    pub opacity: Option<u32>,
    /// `None` = theme default.
    pub material: Option<Material>,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            color_mode: BackgroundColorMode::Custom,
            color: None,
            opacity: None,
            material: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ClassicSettings {
    pub skin: String,
    pub custom_skin: String,
}

impl Default for ClassicSettings {
    fn default() -> Self {
        Self {
            skin: "gloss".into(),
            custom_skin: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct SensorDefaults {
    pub text_color: String,
    /// `None` = theme default.
    pub font: Option<String>,
    /// `None` = theme default.
    pub font_style: Option<FontStyle>,
    pub inactive_mode: InactiveMode,
    pub inactive_color: String,
    /// Percent.
    pub inactive_opacity: u32,
}

impl Default for SensorDefaults {
    fn default() -> Self {
        Self {
            text_color: "#FFFFFF".into(),
            font: None,
            font_style: None,
            inactive_mode: InactiveMode::Opacity,
            inactive_color: "#555555".into(),
            inactive_opacity: 78,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct PopupSettings {
    /// `None` = theme default.
    pub background: Option<String>,
    /// Percent; `None` = theme default.
    pub opacity: Option<u32>,
    pub strong_text_color: String,
    pub text_color: String,
    /// `None` = theme default.
    pub font: Option<String>,
    /// `None` = theme default.
    pub font_style: Option<FontStyle>,
    /// 9–15.
    pub font_size: u32,
    pub shadow: bool,
    /// 0–2000.
    pub show_delay_ms: u32,
    /// 0–1000.
    pub hide_delay_ms: u32,
}

impl Default for PopupSettings {
    fn default() -> Self {
        Self {
            background: None,
            opacity: None,
            strong_text_color: "#93ECFF".into(),
            text_color: "#FFFFFF".into(),
            font: None,
            font_style: None,
            font_size: 12,
            shadow: true,
            show_delay_ms: 600,
            hide_delay_ms: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AppSettings {
    /// File path or URI; empty = `ms-settings:dateandtime`.
    pub calendar: String,
}

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[serde(rename_all = "camelCase")]
        #[ts(export)]
        pub enum $name { $($variant),+ }
    };
}

string_enum!(DateLocale { System, Ui });
string_enum!(HotkeyAction {
    BringToFront,
    ToggleVisibility
});
string_enum!(ClicksWhenLocked { Always, AltOnly });
string_enum!(NewSensorMenu { Dialog, Submenu });
string_enum!(Edge { Top, Bottom });
string_enum!(BarMode { Docked, Floating });
string_enum!(ZOrder {
    Topmost,
    Normal,
    Bottom
});
string_enum!(Animations { Auto, On, Off });
string_enum!(Theme { Modern, Classic });
string_enum!(BackgroundColorMode { Standard, Custom });
string_enum!(Material {
    None,
    Acrylic,
    Mica
});
string_enum!(FontStyle {
    Normal,
    Semibold,
    Bold,
    Italic,
    SemiboldItalic,
    BoldItalic
});
string_enum!(InactiveMode { Opacity, Colorize });
string_enum!(Zone {
    Left,
    Center,
    Right
});

// ---------------------------------------------------------------------------------------------
// Theme defaults (SPEC §10.2–10.5, §12.2–12.3)
// ---------------------------------------------------------------------------------------------

/// Values used when the matching setting is `None`. Exported to the UI as
/// `src/lib/bindings/themeDefaults.json` so both sides share one table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThemeDefaults {
    pub bar_height: u32,
    pub background_color: String,
    pub background_opacity: u32,
    pub background_material: Material,
    pub sensor_font: String,
    pub sensor_font_style: FontStyle,
    pub popup_background: String,
    pub popup_opacity: u32,
    pub popup_font: String,
    pub popup_font_style: FontStyle,
}

impl ThemeDefaults {
    pub fn of(theme: Theme) -> Self {
        match theme {
            Theme::Modern => Self {
                bar_height: 28,
                background_color: "#1C1C1F".into(),
                background_opacity: 80,
                background_material: Material::Acrylic,
                sensor_font: "Segoe UI Variable Text".into(),
                sensor_font_style: FontStyle::Semibold,
                popup_background: "#202024".into(),
                popup_opacity: 92,
                popup_font: "Segoe UI Variable Text".into(),
                popup_font_style: FontStyle::Normal,
            },
            Theme::Classic => Self {
                bar_height: 25,
                background_color: "#2A2A2E".into(),
                background_opacity: 90,
                background_material: Material::None,
                sensor_font: "Arial".into(),
                sensor_font_style: FontStyle::Bold,
                popup_background: "#000000".into(),
                popup_opacity: 78,
                popup_font: "Arial".into(),
                popup_font_style: FontStyle::Bold,
            },
        }
    }
}

impl Settings {
    /// Bar height in logical px with the theme default applied.
    pub fn bar_height(&self) -> u32 {
        self.bar
            .height
            .unwrap_or_else(|| ThemeDefaults::of(self.theme).bar_height)
    }

    /// Clamps numbers to their ranges and replaces malformed colours with defaults, so a
    /// hand-edited or patched config can never carry out-of-range values.
    pub fn sanitize(&mut self) {
        let d = Settings::default();
        self.sensor_spacing = self.sensor_spacing.clamp(1, 20);
        self.bar.height = self.bar.height.map(|h| h.clamp(22, 40));
        if self.bar.monitor.trim().is_empty() {
            self.bar.monitor = PRIMARY_MONITOR.into();
        }
        if let Some(f) = &mut self.bar.floating {
            f.width = f.width.max(100);
        }
        self.background.opacity = self.background.opacity.map(|o| o.min(100));
        self.background.color = self.background.color.take().and_then(normalize_color);
        self.sensors.text_color = normalize_color(std::mem::take(&mut self.sensors.text_color))
            .unwrap_or(d.sensors.text_color);
        self.sensors.inactive_color =
            normalize_color(std::mem::take(&mut self.sensors.inactive_color))
                .unwrap_or(d.sensors.inactive_color);
        self.sensors.inactive_opacity = self.sensors.inactive_opacity.min(100);
        self.popups.background = self.popups.background.take().and_then(normalize_color);
        self.popups.opacity = self.popups.opacity.map(|o| o.min(100));
        self.popups.strong_text_color =
            normalize_color(std::mem::take(&mut self.popups.strong_text_color))
                .unwrap_or(d.popups.strong_text_color);
        self.popups.text_color = normalize_color(std::mem::take(&mut self.popups.text_color))
            .unwrap_or(d.popups.text_color);
        self.popups.font_size = self.popups.font_size.clamp(9, 15);
        self.popups.show_delay_ms = self.popups.show_delay_ms.min(2000);
        self.popups.hide_delay_ms = self.popups.hide_delay_ms.min(1000);
        if self
            .sensors
            .font
            .as_deref()
            .is_some_and(|f| f.trim().is_empty())
        {
            self.sensors.font = None;
        }
        if self
            .popups
            .font
            .as_deref()
            .is_some_and(|f| f.trim().is_empty())
        {
            self.popups.font = None;
        }
    }
}

/// Returns `#RRGGBB` in upper case, or `None` if `s` is not a 6-digit hex colour.
pub fn normalize_color(s: String) -> Option<String> {
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_ascii_uppercase()))
}

// ---------------------------------------------------------------------------------------------
// Layout (SPEC §5.4, §11.2)
// ---------------------------------------------------------------------------------------------

/// Sensors per zone, each in visual left→right order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Layout {
    pub left: Vec<SensorInstance>,
    pub center: Vec<SensorInstance>,
    pub right: Vec<SensorInstance>,
}

impl Layout {
    pub fn zone(&self, zone: Zone) -> &Vec<SensorInstance> {
        match zone {
            Zone::Left => &self.left,
            Zone::Center => &self.center,
            Zone::Right => &self.right,
        }
    }

    pub fn all(&self) -> impl Iterator<Item = &SensorInstance> {
        self.left.iter().chain(&self.center).chain(&self.right)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SensorInstance {
    pub id: String,
    /// Sensor type id, e.g. `cpu-graph`. Unknown types are kept and shown as placeholders.
    #[serde(rename = "type")]
    pub kind: String,
    /// Only values that differ from the type's static defaults (SPEC §11.2).
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub props: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Legacy {
    /// Legacy sensors without an Infodock equivalent yet (SPEC §11.4), kept for later versions.
    pub unsupported: Vec<LegacyUnsupported>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LegacyUnsupported {
    #[serde(rename = "type")]
    pub kind: String,
    pub zone: Zone,
    #[serde(default)]
    pub props: BTreeMap<String, String>,
}

/// Generates sensor ids of the form `s_xxxxx` (lower-case base 36), unique within one generator.
#[derive(Default)]
pub struct IdGen {
    taken: std::collections::HashSet<String>,
}

impl IdGen {
    /// A generator that avoids every id already used in `layout`.
    pub fn avoiding(layout: &Layout) -> Self {
        Self {
            taken: layout.all().map(|s| s.id.clone()).collect(),
        }
    }

    pub fn fresh(&mut self) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        loop {
            let mut id = String::from("s_");
            for _ in 0..5 {
                id.push(ALPHABET[fastrand::usize(..ALPHABET.len())] as char);
            }
            if self.taken.insert(id.clone()) {
                return id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_layout_matches_spec() {
        let c = Config::first_run("1.2.3");
        let kinds = |z: Zone| {
            c.layout
                .zone(z)
                .iter()
                .map(|s| s.kind.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds(Zone::Left),
            ["cpu-graph", "memory-meter", "network-graph"]
        );
        assert_eq!(kinds(Zone::Center), ["image"]);
        assert_eq!(kinds(Zone::Right), ["date", "time"]);
        assert_eq!(c.layout.center[0].props["popupText"], "Infodock %v");
        assert_eq!(c.meta.created_with, "1.2.3");
        assert!(!c.meta.first_run_done);
        let ids: std::collections::HashSet<_> = c.layout.all().map(|s| &s.id).collect();
        assert_eq!(ids.len(), 6);
        assert!(ids.iter().all(|id| id.len() == 7 && id.starts_with("s_")));
    }

    #[test]
    fn empty_object_loads_as_defaults() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.settings.bar.edge, Edge::Bottom);
        assert_eq!(c.settings.bar_height(), 28);
    }

    #[test]
    fn serializes_camel_case_and_type_key() {
        let c = Config::first_run("1.0.0");
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["settings"]["bar"]["reserveSpace"], true);
        assert_eq!(v["settings"]["hotkeyAction"], "bringToFront");
        assert_eq!(v["settings"]["sensors"]["fontStyle"], Value::Null);
        assert_eq!(v["layout"]["left"][0]["type"], "cpu-graph");
        assert_eq!(v["meta"]["firstRunDone"], false);
    }

    #[test]
    fn theme_defaults_follow_theme() {
        let mut s = Settings::default();
        assert_eq!(s.bar_height(), 28);
        s.theme = Theme::Classic;
        assert_eq!(s.bar_height(), 25);
        s.bar.height = Some(33);
        assert_eq!(s.bar_height(), 33);
    }

    #[test]
    fn sanitize_clamps_and_fixes_colors() {
        let mut s = Settings {
            sensor_spacing: 0,
            ..Settings::default()
        };
        s.bar.height = Some(100);
        s.bar.monitor = " ".into();
        s.background.color = Some("#abcdef".into());
        s.popups.background = Some("red".into());
        s.sensors.text_color = "nope".into();
        s.popups.font_size = 30;
        s.popups.show_delay_ms = 9999;
        s.sensors.font = Some(String::new());
        s.sanitize();
        assert_eq!(s.sensor_spacing, 1);
        assert_eq!(s.bar.height, Some(40));
        assert_eq!(s.bar.monitor, "primary");
        assert_eq!(s.background.color.as_deref(), Some("#ABCDEF"));
        assert_eq!(s.popups.background, None);
        assert_eq!(s.sensors.text_color, "#FFFFFF");
        assert_eq!(s.popups.font_size, 15);
        assert_eq!(s.popups.show_delay_ms, 2000);
        assert_eq!(s.sensors.font, None);
    }

    #[test]
    fn id_gen_avoids_existing_ids() {
        let c = Config::first_run("1.0.0");
        let mut g = IdGen::avoiding(&c.layout);
        for _ in 0..200 {
            let id = g.fresh();
            assert!(c.layout.all().all(|s| s.id != id));
        }
    }

    /// Writes the theme defaults for the UI next to the ts-rs bindings (checked in CI).
    #[test]
    fn export_theme_defaults() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/bindings");
        std::fs::create_dir_all(&dir).unwrap();
        let table: BTreeMap<&str, ThemeDefaults> = [
            ("modern", ThemeDefaults::of(Theme::Modern)),
            ("classic", ThemeDefaults::of(Theme::Classic)),
        ]
        .into();
        let json = serde_json::to_string_pretty(&table).unwrap() + "\n";
        std::fs::write(dir.join("themeDefaults.json"), json).unwrap();
    }
}
