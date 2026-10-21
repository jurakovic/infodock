//! Pure helpers between the settings model and the Settings window: pages, choice lists,
//! settings patches and the monitor list.

use serde_json::{Map, Value};

use crate::config::model::{
    FloatingRect, LIGHT_BAR_BACKGROUND, PRIMARY_MONITOR, Settings, ThemeDefaults, Zone,
};
use crate::config::patch::apply_settings_patch;
use crate::i18n::t_args;
use crate::platform::MonitorInfo;
use crate::platform::geometry::{floating_rect, to_physical};

/// Settings pages in navigation order (SPEC §10.7); the window's `Pages` global numbers the
/// last ones and those the navigation does not list.
pub const PAGES: [&str; 8] = [
    "general",
    "bar",
    "appearance",
    "sensors",
    "popups",
    "applications",
    "help",
    "about",
];
/// The page with the Layout manager.
pub const SENSORS_PAGE: usize = 3;

/// Where Settings opens (SPEC §4.1, §6.6, §6.7).
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// A page of the navigation, by index into [`PAGES`].
    Page(usize),
    /// The sensor picker: insert into `zone` at `index`, or next to `target`.
    New {
        zone: Option<Zone>,
        index: Option<usize>,
        target: Option<String>,
    },
    /// A sensor's properties.
    Sensor(String),
    /// The welcome page (SPEC §11.5).
    Welcome,
}

/// Parses a route such as `bar`, `layout`, `sensor/s_1a2b3` or `new?zone=left&index=2`;
/// anything unknown opens General.
pub fn route(route: &str) -> Route {
    let (path, query) = route.split_once('?').unwrap_or((route, ""));
    let mut parts = path.split('/');
    let first = parts.next().unwrap_or_default();
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_owned())
    };
    match first {
        // The layout manager is on the Sensors page.
        "layout" => Route::Page(SENSORS_PAGE),
        "welcome" => Route::Welcome,
        "sensor" => match parts.next() {
            Some(id) if !id.is_empty() => Route::Sensor(id.to_owned()),
            _ => Route::Page(SENSORS_PAGE),
        },
        "new" => Route::New {
            zone: param("zone").and_then(|z| serde_json::from_value(Value::String(z)).ok()),
            index: param("index").and_then(|i| i.parse().ok()),
            target: param("target").filter(|t| !t.is_empty()),
        },
        page => Route::Page(PAGES.iter().position(|p| *p == page).unwrap_or(0)),
    }
}

/// A setting shown as a list of choices, in the order the window lists them.
pub struct Choice {
    /// Dotted settings key, e.g. `bar.zOrder`.
    pub path: &'static str,
    /// The stored value of each choice.
    pub values: &'static [&'static str],
}

pub const CHOICES: &[Choice] = &[
    Choice {
        path: "hotkeyAction",
        values: &["bringToFront", "toggleVisibility"],
    },
    Choice {
        path: "clicksWhenLocked",
        values: &["always", "altOnly"],
    },
    Choice {
        path: "newSensorMenu",
        values: &["dialog", "submenu"],
    },
    Choice {
        path: "bar.mode",
        values: &["docked", "floating"],
    },
    Choice {
        path: "bar.edge",
        values: &["top", "bottom"],
    },
    Choice {
        path: "bar.zOrder",
        values: &["topmost", "normal", "bottom"],
    },
    Choice {
        path: "bar.animations",
        values: &["auto", "on", "off"],
    },
    Choice {
        path: "theme",
        values: &["modern", "classic"],
    },
    Choice {
        path: "colorScheme",
        values: &["system", "light", "dark"],
    },
    Choice {
        path: "background.colorMode",
        values: &["standard", "custom"],
    },
    Choice {
        path: "sensors.inactiveMode",
        values: &["opacity", "colorize"],
    },
];

/// The font styles in the order Settings lists them, after "Theme default" (SPEC §8.2).
pub const FONT_STYLES: [&str; 6] = [
    "normal",
    "semibold",
    "bold",
    "italic",
    "semiboldItalic",
    "boldItalic",
];

/// The materials in the order Settings lists them, after "Theme default" (SPEC §10.3).
pub const MATERIALS: [&str; 3] = ["none", "acrylic", "mica"];

/// "Theme default (…)" naming the theme's choice, then every choice in `ids`, each named by the
/// message `<prefix>.<id>`.
fn after_theme_default(prefix: &str, ids: &[&str], theme_default: &str) -> Vec<String> {
    let name = |id: &str| crate::i18n::t(&format!("{prefix}.{id}"));
    std::iter::once(t_args(
        "common.themeDefault",
        &[("value", &name(theme_default))],
    ))
    .chain(ids.iter().map(|id| name(id)))
    .collect()
}

/// The index of `value` (one of `ids`, or `None` for the theme default) in such a list.
fn index_after_theme_default(ids: &[&str], value: Option<&str>) -> i32 {
    value
        .and_then(|v| ids.iter().position(|id| *id == v))
        .map_or(0, |i| i as i32 + 1)
}

/// The font style list: "Theme default (…)" naming the theme's style, then every style.
pub fn font_style_labels(theme_default: &str) -> Vec<String> {
    after_theme_default("fontStyle", &FONT_STYLES, theme_default)
}

/// The index of `style` (a font style id, or `None` for the theme default) in that list.
pub fn font_style_index(style: Option<&str>) -> i32 {
    index_after_theme_default(&FONT_STYLES, style)
}

/// The material list: "Theme default (…)" naming the theme's material, then every material.
pub fn material_labels(theme_default: &str) -> Vec<String> {
    after_theme_default("settings.appearance.material", &MATERIALS, theme_default)
}

/// The index of `material` (a material id, or `None` for the theme default) in that list.
pub fn material_index(material: Option<&str>) -> i32 {
    index_after_theme_default(&MATERIALS, material)
}

/// The font list: "Theme default (…)" naming the theme's family, then the installed families
/// and a configured family that is not installed.
pub fn font_labels(theme_default: &str, families: &[String], current: Option<&str>) -> Vec<String> {
    let mut labels = vec![t_args("common.themeDefault", &[("value", theme_default)])];
    labels.extend(families.iter().cloned());
    if let Some(current) = current
        && !families.iter().any(|f| f == current)
    {
        labels.push(current.to_owned());
    }
    labels
}

/// The index of `font` (`None` for the theme default) in [`font_labels`].
pub fn font_index(labels: &[String], font: Option<&str>) -> i32 {
    font.and_then(|f| labels.iter().skip(1).position(|l| l == f))
        .map_or(0, |i| i as i32 + 1)
}

pub fn choice(path: &str) -> Option<&'static Choice> {
    CHOICES.iter().find(|c| c.path == path)
}

/// The value at a dotted path in serialized settings.
pub fn value_at<'a>(settings: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(settings, |v, key| v.get(key))
}

/// The index of the current value of `choice` in serialized settings (0 if unknown).
pub fn choice_index(settings: &Value, choice: &Choice) -> i32 {
    let current = value_at(settings, choice.path).and_then(Value::as_str);
    choice
        .values
        .iter()
        .position(|v| Some(*v) == current)
        .map_or(0, |i| i as i32)
}

/// The value in effect at a dotted path: the stored one, or the theme's default for a
/// theme-dependent setting stored as `null`; `None` for an unknown path. `os_dark`: the OS apps
/// are dark, for the modern bar's light look with `colorScheme: system` (SPEC §12.2).
pub fn in_effect(settings: &Settings, os_dark: bool, path: &str) -> Option<Value> {
    let json = serde_json::to_value(settings).ok()?;
    let stored = value_at(&json, path)?;
    if !stored.is_null() {
        return Some(stored.clone());
    }
    let d = ThemeDefaults::of(settings.theme);
    let default = match path {
        "bar.height" => Value::from(d.bar_height),
        "background.color" if settings.light_bar(os_dark) => Value::from(LIGHT_BAR_BACKGROUND),
        "background.color" => Value::from(d.background_color),
        "background.opacity" => Value::from(d.background_opacity),
        "background.material" => serde_json::to_value(d.background_material).ok()?,
        "sensors.font" => Value::from(d.sensor_font),
        "sensors.fontStyle" => serde_json::to_value(d.sensor_font_style).ok()?,
        "popups.background" => Value::from(d.popup_background),
        "popups.opacity" => Value::from(d.popup_opacity),
        "popups.font" => Value::from(d.popup_font),
        "popups.fontStyle" => serde_json::to_value(d.popup_font_style).ok()?,
        _ => Value::Null,
    };
    Some(default)
}

/// Whether setting `path` to `value` changes what is in effect. A theme-dependent setting left
/// at the theme's value (a colour box losing the focus, a slider clicked without moving) stays
/// `null` and keeps following the theme (SPEC §11.2). An invalid value counts as a change, so
/// saving it reports why.
pub fn changes(settings: &Settings, os_dark: bool, path: &str, value: &Value) -> bool {
    apply_settings_patch(settings, &patch(path, value.clone())).map_or(true, |next| {
        in_effect(&next, os_dark, path) != in_effect(settings, os_dark, path)
    })
}

/// A settings patch that sets one dotted path: `bar.edge` → `{"bar": {"edge": value}}`.
pub fn patch(path: &str, value: Value) -> Value {
    path.rsplit('.').fold(value, |inner, key| {
        let mut object = Map::new();
        object.insert(key.to_owned(), inner);
        Value::Object(object)
    })
}

/// The monitor list shown in Settings ▸ Bar.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorChoices {
    pub labels: Vec<String>,
    /// The stored value for each label: `primary` or a monitor key (SPEC §10.2).
    pub keys: Vec<String>,
    pub current: usize,
}

/// "Primary monitor", then every monitor, then the saved monitor if it is not connected.
/// A floating bar's place with a new `width` (logical px): where it is, or, before it was ever
/// placed, centred where it starts (SPEC §5.1).
pub fn floating_with_width(
    settings: &Settings,
    monitors: &[MonitorInfo],
    width: u32,
) -> FloatingRect {
    if let Some(f) = settings.bar.floating {
        return FloatingRect { width, ..f };
    }
    let bar = &settings.bar;
    match floating_rect(
        monitors,
        &bar.monitor,
        None,
        bar.edge,
        settings.bar_height(),
    ) {
        Some((rect, monitor)) => {
            let w = to_physical(width, monitor.scale).min(monitor.work_area.width());
            FloatingRect {
                x: rect.left + (rect.width() - w) / 2,
                y: rect.top,
                width,
            }
        }
        None => FloatingRect { x: 0, y: 0, width },
    }
}

pub fn monitor_choices(saved: &str, monitors: &[MonitorInfo]) -> MonitorChoices {
    let mut labels = vec![crate::i18n::t("settings.bar.monitor.primary")];
    let mut keys = vec![PRIMARY_MONITOR.to_owned()];
    for m in monitors {
        let (width, height) = (m.bounds.width().to_string(), m.bounds.height().to_string());
        let key = if m.primary {
            "settings.bar.monitor.itemPrimary"
        } else {
            "settings.bar.monitor.item"
        };
        labels.push(t_args(
            key,
            &[("name", &m.name), ("width", &width), ("height", &height)],
        ));
        keys.push(m.key.clone());
    }
    if !keys.iter().any(|k| k == saved) {
        // Monitor keys are `<device>|<name>|<w>x<h>` (SPEC §10.2).
        let name = saved.split('|').nth(1).unwrap_or(saved);
        labels.push(t_args("settings.bar.monitor.missing", &[("name", name)]));
        keys.push(saved.to_owned());
    }
    let current = keys.iter().position(|k| k == saved).unwrap_or(0);
    MonitorChoices {
        labels,
        keys,
        current,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::platform::Rect;

    #[test]
    fn floating_width_keeps_the_place_or_centres() {
        let monitors = [MonitorInfo::new(
            r"\\.\DISPLAY1".into(),
            "Test".into(),
            Rect::new(0, 0, 1920, 1080),
            Rect::new(0, 0, 1920, 1040),
            1.0,
            true,
        )];
        let mut s = Settings::default();
        // Never placed: centred where a new floating bar starts (800 px at x = 560).
        assert_eq!(
            floating_with_width(&s, &monitors, 1000),
            FloatingRect {
                x: 460,
                y: 1000,
                width: 1000
            }
        );
        s.bar.floating = Some(FloatingRect {
            x: 10,
            y: 20,
            width: 300,
        });
        assert_eq!(
            floating_with_width(&s, &monitors, 600),
            FloatingRect {
                x: 10,
                y: 20,
                width: 600
            }
        );
    }

    #[test]
    fn routes() {
        assert_eq!(route("general"), Route::Page(0));
        assert_eq!(route("bar"), Route::Page(1));
        assert_eq!(route("bar/height"), Route::Page(1));
        assert_eq!(route("appearance"), Route::Page(2));
        assert_eq!(route("popups"), Route::Page(4));
        assert_eq!(route("applications"), Route::Page(5));
        assert_eq!(route("help"), Route::Page(6));
        assert_eq!(route("about"), Route::Page(7));
        assert_eq!(route("welcome"), Route::Welcome);
        assert_eq!(route("layout"), Route::Page(SENSORS_PAGE));
        assert_eq!(route("nope"), Route::Page(0));
        assert_eq!(route(""), Route::Page(0));
        assert_eq!(route("sensor/s_1a2b3"), Route::Sensor("s_1a2b3".into()));
        assert_eq!(route("sensor/"), Route::Page(SENSORS_PAGE));
        assert_eq!(
            route("new?zone=right&index=0"),
            Route::New {
                zone: Some(Zone::Right),
                index: Some(0),
                target: None
            }
        );
        assert_eq!(
            route("new?target=s_1&zone=middle"),
            Route::New {
                zone: None,
                index: None,
                target: Some("s_1".into())
            }
        );
    }

    #[test]
    fn patches_nest_dotted_paths() {
        assert_eq!(patch("locked", json!(true)), json!({ "locked": true }));
        assert_eq!(
            patch("bar.height", Value::Null),
            json!({ "bar": { "height": null } })
        );
    }

    #[test]
    fn theme_defaults_are_in_effect_until_set() {
        let modern = Settings::default();
        assert_eq!(
            in_effect(&modern, true, "popups.background"),
            Some(json!("#202024"))
        );
        assert_eq!(in_effect(&modern, true, "bar.height"), Some(json!(28)));
        assert_eq!(
            in_effect(&modern, true, "sensors.fontStyle"),
            Some(json!("semibold"))
        );
        assert_eq!(
            in_effect(&modern, true, "sensors.textColor"),
            Some(json!("#FFFFFF"))
        );
        assert_eq!(in_effect(&modern, true, "bar.floating"), Some(Value::Null));
        assert_eq!(in_effect(&modern, true, "bar.nope"), None);
        let defaults = serde_json::to_value(&modern).expect("serializable");
        for path in [
            "bar.height",
            "background.color",
            "background.opacity",
            "background.material",
            "sensors.font",
            "sensors.fontStyle",
            "popups.background",
            "popups.opacity",
            "popups.font",
            "popups.fontStyle",
        ] {
            assert_eq!(value_at(&defaults, path), Some(&Value::Null), "{path}");
            let value = in_effect(&modern, true, path).unwrap_or_default();
            assert!(!value.is_null(), "{path} has no theme default");
            assert!(!changes(&modern, true, path, &value), "{path}");
        }

        let mut classic = Settings {
            theme: crate::config::model::Theme::Classic,
            ..Settings::default()
        };
        assert_eq!(
            in_effect(&classic, true, "popups.background"),
            Some(json!("#000000"))
        );
        classic.popups.background = Some("#202024".into());
        assert_eq!(
            in_effect(&classic, true, "popups.background"),
            Some(json!("#202024"))
        );
    }

    #[test]
    fn the_light_look_is_the_modern_themes_colour_in_light_mode() {
        let settings = Settings::default();
        // OS in light mode: the light look's background is the theme's colour (SPEC §12.2), so
        // leaving the colour box keeps following it.
        assert_eq!(
            in_effect(&settings, false, "background.color"),
            Some(json!("#F3F3F3"))
        );
        assert!(!changes(
            &settings,
            false,
            "background.color",
            &json!("#F3F3F3")
        ));
        assert!(changes(
            &settings,
            false,
            "background.color",
            &json!("#1C1C1F")
        ));
        // Dark forced, or a text colour of the user's: the dark colour.
        let dark = Settings {
            color_scheme: crate::config::model::ColorScheme::Dark,
            ..Settings::default()
        };
        assert_eq!(
            in_effect(&dark, false, "background.color"),
            Some(json!("#1C1C1F"))
        );
        let mut yellow = Settings::default();
        yellow.sensors.text_color = "#FFCC00".into();
        assert_eq!(
            in_effect(&yellow, false, "background.color"),
            Some(json!("#1C1C1F"))
        );
    }

    #[test]
    fn only_a_different_value_is_a_change() {
        let settings = Settings::default();
        // The theme's values: nothing to save, the settings stay `null`.
        assert!(!changes(
            &settings,
            true,
            "popups.background",
            &json!("#202024")
        ));
        assert!(!changes(&settings, true, "popups.opacity", &json!(92)));
        assert!(!changes(&settings, true, "bar.height", &json!(28)));
        assert!(!changes(
            &settings,
            true,
            "sensors.textColor",
            &json!("#FFFFFF")
        ));
        // Anything else is saved.
        assert!(changes(
            &settings,
            true,
            "popups.background",
            &json!("#202025")
        ));
        assert!(changes(&settings, true, "popups.opacity", &json!(91)));
        assert!(changes(&settings, true, "bar.height", &json!(30)));
        // A value set explicitly is kept as it is.
        let pinned = Settings {
            popups: crate::config::model::PopupSettings {
                background: Some("#202024".into()),
                ..Default::default()
            },
            ..Settings::default()
        };
        assert!(!changes(
            &pinned,
            true,
            "popups.background",
            &json!("#202024")
        ));
        // Invalid: saving it reports the error.
        assert!(changes(&settings, true, "popups.opacity", &json!("x")));
        assert!(changes(&settings, true, "nope", &json!(1)));
    }

    #[test]
    fn every_choice_matches_the_settings_model() {
        let defaults = serde_json::to_value(Settings::default()).expect("serializable");
        for c in CHOICES {
            let current = value_at(&defaults, c.path).and_then(Value::as_str);
            assert!(
                current.is_some_and(|v| c.values.contains(&v)),
                "{}: default {current:?} not in {:?}",
                c.path,
                c.values
            );
            for v in c.values {
                let next = crate::config::patch::apply_settings_patch(
                    &Settings::default(),
                    &patch(c.path, json!(v)),
                );
                assert!(next.is_ok(), "{}={v}: {next:?}", c.path);
            }
        }
    }

    #[test]
    fn font_lists_start_with_the_theme_default() {
        let families = vec!["Arial".to_owned(), "Consolas".to_owned()];
        let labels = font_labels("Segoe UI Variable Text", &families, Some("Gone"));
        assert_eq!(
            labels,
            [
                "Theme default (Segoe UI Variable Text)",
                "Arial",
                "Consolas",
                "Gone"
            ]
        );
        assert_eq!(font_index(&labels, None), 0);
        assert_eq!(font_index(&labels, Some("Consolas")), 2);
        assert_eq!(font_index(&labels, Some("Gone")), 3);

        let styles = font_style_labels("semibold");
        assert_eq!(styles[0], "Theme default (Semibold)");
        assert_eq!(styles[6], "Bold italic");
        assert_eq!(font_style_index(None), 0);
        assert_eq!(font_style_index(Some("bold")), 3);

        let materials = material_labels("acrylic");
        assert_eq!(
            materials,
            ["Theme default (Acrylic)", "None", "Acrylic", "Mica"]
        );
        assert_eq!(material_index(None), 0);
        assert_eq!(material_index(Some("mica")), 3);
    }

    #[test]
    fn choice_index_reads_current_values() {
        let settings = json!({ "bar": { "zOrder": "bottom" }, "hotkeyAction": "nope" });
        let z = choice("bar.zOrder").expect("known");
        assert_eq!(choice_index(&settings, z), 2);
        let action = choice("hotkeyAction").expect("known");
        assert_eq!(choice_index(&settings, action), 0);
    }

    fn monitor(key: &str, name: &str, primary: bool) -> MonitorInfo {
        let bounds = Rect::new(0, 0, 2560, 1440);
        let mut m = MonitorInfo::new(key.into(), name.into(), bounds, bounds, 1.0, primary);
        m.key = key.into();
        m
    }

    #[test]
    fn monitor_list_marks_primary_and_keeps_a_missing_saved_monitor() {
        let monitors = [
            monitor("a", "Dell U2720Q", true),
            monitor("b", "LG 27GL850", false),
        ];
        let list = monitor_choices("b", &monitors);
        assert_eq!(list.keys, ["primary", "a", "b"]);
        assert_eq!(list.labels[1], "Dell U2720Q (2560 × 1440, primary)");
        assert_eq!(list.labels[2], "LG 27GL850 (2560 × 1440)");
        assert_eq!(list.current, 2);

        let gone = monitor_choices(r"\\.\DISPLAY3|Old TV|1920x1080", &monitors);
        assert_eq!(gone.keys.len(), 4);
        assert_eq!(gone.current, 3);
        assert_eq!(
            gone.labels[3],
            "Old TV (not connected; the primary monitor is used)"
        );

        assert_eq!(monitor_choices("primary", &[]).current, 0);
    }
}
