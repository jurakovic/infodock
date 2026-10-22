//! What the Settings window and Rust must agree on, where the compiler cannot check it: the
//! numbers both give the pages, the form's rows and the Layout manager's lines, the settings
//! paths the window hands to Rust, and the catalog keys Settings builds from ids. The tests
//! read `ui/settings.slint`, as the catalog's test reads the keys the UI uses.

use serde_json::Value;

use super::form::RowKind;
use super::layout::LineKind;
use super::{GENERAL_PAGE, appearance, model, picker, properties, welcome};
use crate::config::model::Settings;
use crate::config::patch::apply_settings_patch;

fn slint_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/settings.slint");
    std::fs::read_to_string(path).expect("ui/settings.slint")
}

/// The `out property <int> name: value;` lines of `export global {global}` in `source`.
fn global_ints(source: &str, global: &str) -> Vec<(String, i32)> {
    let start = source
        .find(&format!("export global {global} {{"))
        .unwrap_or_else(|| panic!("no global {global}"));
    let body = &source[start..];
    let body = &body[..body.find("\n}").expect("the global ends")];
    body.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("out property <int> ")?;
            let (name, value) = rest.trim_end_matches(';').split_once(": ")?;
            Some((name.to_owned(), value.parse().ok()?))
        })
        .collect()
}

/// Where `pattern` is followed by a digit in `source`: a number where a name belongs.
fn numbered<'a>(source: &'a str, pattern: &str) -> Vec<&'a str> {
    source
        .match_indices(pattern)
        .map(|(i, _)| &source[i..])
        .filter(|rest| rest[pattern.len()..].starts_with(|c: char| c.is_ascii_digit()))
        .map(|rest| rest.lines().next().unwrap_or_default())
        .collect()
}

#[test]
fn the_window_numbers_its_pages_as_rust_does() {
    let source = slint_source();
    let mut expected: Vec<(String, i32)> = (model::PAGES.iter().enumerate())
        .map(|(i, page)| ((*page).to_owned(), i as i32))
        .collect();
    expected.extend(
        [
            ("properties", properties::PAGE),
            ("picker", picker::PAGE),
            ("welcome", welcome::PAGE),
        ]
        .map(|(name, page)| (name.to_owned(), page)),
    );
    assert_eq!(global_ints(&source, "Pages"), expected);
    // The other constants that name a page of the navigation.
    let page = |name| model::PAGES.iter().position(|p| *p == name);
    assert_eq!(page("general"), Some(GENERAL_PAGE as usize));
    assert_eq!(page("appearance"), Some(appearance::PAGE as usize));
    assert_eq!(page("sensors"), Some(model::SENSORS_PAGE));
    // The navigation, then the page titles, list the pages in that order.
    let listed: Vec<&str> = source
        .match_indices("\"settings.nav.")
        .filter_map(|(i, m)| source[i + m.len()..].split('"').next())
        .filter(|key| *key != "label")
        .collect();
    let twice: Vec<&str> = model::PAGES.iter().chain(&model::PAGES).copied().collect();
    assert_eq!(listed, twice);
    // The window names its pages; a number would not follow a change.
    for pattern in [
        "SettingsState.page == ",
        "SettingsState.page != ",
        "SettingsState.page = ",
    ] {
        assert_eq!(numbered(&source, pattern), Vec::<&str>::new());
    }
}

/// The name of each row kind in `RowKinds`.
fn row_kind_name(kind: RowKind) -> &'static str {
    match kind {
        RowKind::Text => "text",
        RowKind::Flag => "flag",
        RowKind::Choice => "choice",
        RowKind::Number => "number",
        RowKind::Color => "color",
        RowKind::ColorMode => "color-mode",
        RowKind::File => "file",
        RowKind::Folder => "folder",
    }
}

/// The name of each line kind in `LineKinds`.
fn line_kind_name(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Heading => "heading",
        LineKind::Sensor => "sensor",
        LineKind::Empty => "empty",
    }
}

#[test]
fn the_window_numbers_rows_and_lines_as_rust_does() {
    use RowKind::*;
    let source = slint_source();
    let rows = [Text, Flag, Choice, Number, Color, ColorMode, File, Folder];
    let rows: Vec<(String, i32)> = rows
        .iter()
        .map(|&kind| (row_kind_name(kind).to_owned(), kind.code()))
        .collect();
    assert_eq!(global_ints(&source, "RowKinds"), rows);
    let lines = [LineKind::Heading, LineKind::Sensor, LineKind::Empty];
    let lines: Vec<(String, i32)> = lines
        .iter()
        .map(|&kind| (line_kind_name(kind).to_owned(), kind.code()))
        .collect();
    assert_eq!(global_ints(&source, "LineKinds"), lines);
    for pattern in ["kind == ", "kind != "] {
        assert_eq!(numbered(&source, pattern), Vec::<&str>::new());
    }
}

/// The calls `SettingsState.{callback}("path", …)` in `source` of the callbacks that take a
/// settings path: each callback and path.
fn path_calls(source: &str) -> Vec<(&str, &str)> {
    const CALLBACKS: [&str; 8] = [
        "set-flag",
        "set-choice",
        "set-number",
        "set-color",
        "set-text",
        "set-font",
        "set-font-style",
        "reset",
    ];
    CALLBACKS
        .iter()
        .flat_map(|callback| {
            let call = format!("SettingsState.{callback}(");
            source
                .match_indices(&call)
                .map(|(i, _)| {
                    let args = &source[i + call.len()..];
                    let path = args
                        .strip_prefix('"')
                        .and_then(|rest| rest.split('"').next())
                        .unwrap_or_else(|| panic!("{callback} without a literal path"));
                    (*callback, path)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A value of the kind `callback` saves at `path`, as Rust makes it.
fn value_for(callback: &str, path: &str) -> Value {
    match callback {
        "set-flag" => Value::Bool(true),
        "set-number" => Value::from(10),
        "set-color" => Value::from("#123456"),
        "set-text" => Value::from("x"),
        "set-font" => Value::from("Arial"),
        "set-font-style" => Value::from(model::FONT_STYLES[0]),
        "set-choice" => {
            let choice = model::choice(path).unwrap_or_else(|| panic!("{path}: not in CHOICES"));
            Value::from(choice.values[0])
        }
        "reset" => Value::Null,
        other => panic!("{other} takes no path"),
    }
}

#[test]
fn every_settings_path_the_window_passes_exists() {
    let source = slint_source();
    let calls = path_calls(&source);
    assert!(
        calls.len() > 30,
        "only {} paths; is the scan broken?",
        calls.len()
    );
    let defaults = Settings::default();
    for (callback, path) in calls {
        let saved = apply_settings_patch(&defaults, &model::patch(path, value_for(callback, path)));
        assert!(saved.is_ok(), "{callback}(\"{path}\"): {:?}", saved.err());
    }
}

#[test]
fn every_settings_path_rust_saves_exists() {
    let defaults = Settings::default();
    let stored = serde_json::to_value(&defaults).expect("settings serialise");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/settings");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("src/settings") {
        let path = entry.expect("an entry").path();
        let source = std::fs::read_to_string(&path).expect("a source file");
        for (i, m) in source.match_indices("model::patch(\"") {
            let key = source[i + m.len()..].split('"').next().unwrap_or_default();
            // The value there now: a path that exists takes it.
            let value = key
                .split('.')
                .try_fold(&stored, |v, k| v.get(k))
                .unwrap_or_else(|| panic!("{}: no setting {key}", path.display()));
            let saved = apply_settings_patch(&defaults, &model::patch(key, value.clone()));
            assert!(saved.is_ok(), "{key}: {:?}", saved.err());
            checked += 1;
        }
    }
    assert!(checked > 5, "only {checked} paths; is the scan broken?");
}

/// The message keys Settings and the Power sensor build from ids (`format!`), which the
/// catalog's test of literal keys cannot see.
#[test]
fn every_built_key_exists() {
    use crate::look::skins::{BuiltIn, CUSTOM};
    use crate::platform::PowerAction;
    let mut keys: Vec<String> = Vec::new();
    let ids = BuiltIn::ALL.iter().map(|skin| skin.id()).chain([CUSTOM]);
    keys.extend(ids.map(|id| format!("skins.name.{id}")));
    keys.extend(model::FONT_STYLES.iter().map(|s| format!("fontStyle.{s}")));
    keys.extend((model::MATERIALS.iter()).map(|m| format!("settings.appearance.material.{m}")));
    keys.extend((super::form::modes(true).iter()).map(|m| format!("settings.props.colorMode.{m}")));
    for action in PowerAction::ALL {
        keys.push(format!("sensor.power.mode.{}", action.id()));
        keys.push(format!("sensor.power.confirm.{}", action.id()));
    }
    let missing: Vec<&String> = keys.iter().filter(|k| !crate::i18n::exists(k)).collect();
    assert!(missing.is_empty(), "missing in en.json: {missing:?}");
}
