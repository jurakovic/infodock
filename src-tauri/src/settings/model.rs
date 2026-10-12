//! Pure helpers between the settings model and the Settings window: pages, choice lists,
//! settings patches and the monitor list.

use serde_json::{Map, Value};

use crate::config::model::PRIMARY_MONITOR;
use crate::i18n::t_args;
use crate::platform::MonitorInfo;

/// Settings pages in navigation order (SPEC §10.7).
pub const PAGES: [&str; 2] = ["general", "bar"];

/// The page index for a route such as `bar` or `bar/height`; unknown routes open General.
pub fn page_index(route: &str) -> i32 {
    let page = route.split('/').next().unwrap_or_default();
    PAGES
        .iter()
        .position(|p| *p == page)
        .map_or(0, |i| i as i32)
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
        path: "bar.edge",
        values: &["top", "bottom"],
    },
    Choice {
        path: "bar.zOrder",
        values: &["topmost", "normal", "bottom"],
    },
];

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
    use crate::config::model::Settings;
    use crate::platform::Rect;

    #[test]
    fn routes_map_to_pages() {
        assert_eq!(page_index("general"), 0);
        assert_eq!(page_index("bar"), 1);
        assert_eq!(page_index("bar/height"), 1);
        assert_eq!(page_index("nope"), 0);
        assert_eq!(page_index(""), 0);
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
