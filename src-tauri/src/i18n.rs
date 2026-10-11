//! Strings shown by the Rust side (tray menu, native dialogs), read from the same catalog as the
//! UI (SPEC §13). Only English exists in v1.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, PoisonError};

use serde_json::Value;

static EN: &str = include_str!("../../src/i18n/en.json");

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_and_formats() {
        assert_eq!(t("tray.quit"), "Quit Infodock");
        assert_eq!(t_args("common.items", &[("count", "3")]), "3 items");
    }

    #[test]
    fn missing_key_returns_key() {
        assert_eq!(t("no.such.key"), "no.such.key");
    }
}
