//! Message catalogs and `t()` (SPEC §13). Every user-visible string comes from here: the bar,
//! menus and dialogs call it directly, the Slint UI through its `I18n` global. Only English exists
//! in v1.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, PoisonError};

use serde_json::Value;

static EN: &str = include_str!("../i18n/en.json");

static CATALOG: LazyLock<HashMap<String, Value>> =
    LazyLock::new(|| serde_json::from_str(EN).expect("src/i18n/en.json is valid JSON"));

static REPORTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// The message for `key`, or the key itself (logged once) if it is missing.
pub fn t(key: &str) -> String {
    t_args(key, &[])
}

/// Like [`t`], replacing `{name}` placeholders. Plural entries use their `other` form.
pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    let template = match CATALOG.get(key) {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Object(forms)) => forms.get("other").and_then(Value::as_str).unwrap_or(key),
        _ => {
            if REPORTED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key.to_owned())
            {
                tracing::warn!("missing message key: {key}");
            }
            key
        }
    };
    args.iter().fold(template.to_owned(), |s, (name, value)| {
        s.replace(&format!("{{{name}}}"), value)
    })
}

/// Like [`t_args`] for a message that depends on `count`: its plural form is chosen by the UI
/// language's rules (SPEC §13; English: `one` for exactly 1, else `other`), and `{count}` is
/// replaced by `count`.
pub fn t_count(key: &str, count: u64, args: &[(&str, &str)]) -> String {
    let form = if count == 1 { "one" } else { "other" };
    let count = count.to_string();
    let mut args = args.to_vec();
    args.push(("count", &count));
    match CATALOG.get(key) {
        Some(Value::Object(forms)) => match forms.get(form).and_then(Value::as_str) {
            Some(template) => args.iter().fold(template.to_owned(), |s, (name, value)| {
                s.replace(&format!("{{{name}}}"), value)
            }),
            None => t_args(key, &args),
        },
        _ => t_args(key, &args),
    }
}

/// Whether the catalog has `key`, for tests of code that names keys.
#[cfg(test)]
pub(crate) fn exists(key: &str) -> bool {
    CATALOG.contains_key(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_and_formats() {
        assert_eq!(t("tray.quit"), "Quit Infodock");
        assert_eq!(t_args("common.items", &[("count", "3")]), "3 items");
    }

    #[test]
    fn plural_forms_follow_the_count() {
        assert_eq!(t_count("common.items", 1, &[]), "1 item");
        assert_eq!(t_count("common.items", 0, &[]), "0 items");
        assert_eq!(t_count("common.items", 21, &[]), "21 items");
        assert_eq!(
            t_count("tray.quit", 2, &[]),
            "Quit Infodock",
            "not a plural entry"
        );
    }

    #[test]
    fn missing_key_returns_key() {
        assert_eq!(t("no.such.key"), "no.such.key");
    }

    /// Keys in `I18n.tr("…")` and `I18n.tr-with("…", …)` calls of the Slint UI.
    fn slint_keys(source: &str) -> Vec<&str> {
        ["I18n.tr(\"", "I18n.tr-with(\""]
            .iter()
            .flat_map(|call| source.match_indices(call).map(move |(i, _)| i + call.len()))
            .filter_map(|start| source[start..].split('"').next())
            .collect()
    }

    #[test]
    fn every_key_used_in_the_slint_ui_exists() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        let mut checked = 0;
        for entry in std::fs::read_dir(&ui).expect("ui folder") {
            let path = entry.expect("ui entry").path();
            if path.extension().is_none_or(|e| e != "slint") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("slint source");
            for key in slint_keys(&source) {
                assert!(
                    CATALOG.contains_key(key),
                    "{}: unknown key {key}",
                    path.display()
                );
                checked += 1;
            }
        }
        assert!(
            checked > 20,
            "only {checked} keys found; is the scan broken?"
        );
    }
}
