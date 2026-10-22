//! `update_settings` patches: a deep partial of [`Settings`] merged into the current value.

use serde_json::Value;

use super::ConfigError;
use super::model::Settings;

/// Deep-merges `patch` into `target`. Objects merge key by key; anything else (including
/// `null`, which resets theme-dependent settings) replaces the target value.
pub fn merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                match t.get_mut(k) {
                    Some(tv) if tv.is_object() && v.is_object() => merge(tv, v),
                    _ => {
                        t.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (t, p) => *t = p.clone(),
    }
}

/// Applies a settings patch, validating types and rejecting keys that `Settings` does not have.
pub fn apply_settings_patch(current: &Settings, patch: &Value) -> Result<Settings, ConfigError> {
    if !patch.is_object() {
        return Err(ConfigError::Invalid(
            "settings patch must be an object".into(),
        ));
    }
    let reference = serde_json::to_value(current)?;
    let mut unknown = Vec::new();
    collect_unknown(patch, &reference, "", &mut unknown);
    if !unknown.is_empty() {
        return Err(ConfigError::Invalid(format!(
            "unknown settings: {}",
            unknown.join(", ")
        )));
    }
    let mut merged = reference;
    merge(&mut merged, patch);
    let mut next: Settings =
        serde_json::from_value(merged).map_err(|e| ConfigError::Invalid(e.to_string()))?;
    next.sanitize();
    Ok(next)
}

/// Lists dotted paths in `patch` that do not exist in `reference`. Values under a key whose
/// reference is `null` (an unset optional object such as `bar.floating`) are not checked here;
/// deserialization validates them.
fn collect_unknown(patch: &Value, reference: &Value, prefix: &str, out: &mut Vec<String>) {
    let (Value::Object(p), Value::Object(r)) = (patch, reference) else {
        return;
    };
    for (k, v) in p {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match r.get(k) {
            None => out.push(path),
            Some(rv) => collect_unknown(v, rv, &path, out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{Edge, ThemeId};
    use serde_json::json;

    #[test]
    fn merge_is_deep_and_null_replaces() {
        let mut t = json!({ "a": { "b": 1, "c": 2 }, "d": 3 });
        merge(&mut t, &json!({ "a": { "b": 9 }, "d": null }));
        assert_eq!(t, json!({ "a": { "b": 9, "c": 2 }, "d": null }));
    }

    #[test]
    fn patch_changes_only_given_keys() {
        let s = Settings::default();
        let next = apply_settings_patch(
            &s,
            &json!({ "bar": { "edge": "top", "height": 30 }, "locked": true }),
        )
        .unwrap();
        assert_eq!(next.bar.edge, Edge::Top);
        assert_eq!(next.bar.height, Some(30));
        assert!(next.locked);
        assert_eq!(next.bar.reserve_space, s.bar.reserve_space);
        assert_eq!(next.theme, ThemeId::Modern);
    }

    #[test]
    fn null_resets_theme_dependent_setting() {
        let mut s = Settings::default();
        s.bar.height = Some(35);
        let next = apply_settings_patch(&s, &json!({ "bar": { "height": null } })).unwrap();
        assert_eq!(next.bar.height, None);
    }

    #[test]
    fn patch_is_sanitized() {
        let next =
            apply_settings_patch(&Settings::default(), &json!({ "sensorSpacing": 99 })).unwrap();
        assert_eq!(next.sensor_spacing, 20);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = apply_settings_patch(
            &Settings::default(),
            &json!({ "bar": { "edgee": "top" }, "nope": 1 }),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("bar.edgee") && err.contains("nope"), "{err}");
    }

    #[test]
    fn wrong_types_are_rejected() {
        assert!(
            apply_settings_patch(&Settings::default(), &json!({ "bar": { "edge": "left" } }))
                .is_err()
        );
        assert!(apply_settings_patch(&Settings::default(), &json!({ "locked": "yes" })).is_err());
        assert!(apply_settings_patch(&Settings::default(), &json!([1])).is_err());
    }

    #[test]
    fn optional_object_can_be_set() {
        let next = apply_settings_patch(
            &Settings::default(),
            &json!({ "bar": { "floating": { "x": 10, "y": 20, "width": 800 } } }),
        )
        .unwrap();
        assert_eq!(next.bar.floating.map(|f| f.width), Some(800));
    }
}
