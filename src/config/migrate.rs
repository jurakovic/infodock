//! Schema migrations (SPEC §11.2). Each step is a pure function from schema `n` to `n + 1`.

use serde_json::Value;

use super::model::SCHEMA_VERSION;

/// A migration step: takes a config at schema `n` and returns it at schema `n + 1`.
pub type Step = fn(Value) -> Value;

/// `STEPS[i]` migrates schema `i + 1` to `i + 2`. Empty while only schema 1 exists.
const STEPS: &[Step] = &[];

#[derive(Debug, PartialEq, Eq)]
pub struct Migrated {
    pub value: Value,
    /// The schema the file had, if it was older than the current one.
    pub from: Option<u32>,
    /// The file was written by a newer Infodock (schema above ours); loaded best-effort.
    pub newer: bool,
}

pub fn migrate(value: Value) -> Migrated {
    migrate_with(value, STEPS, SCHEMA_VERSION)
}

/// Runs the steps needed to bring `value` from its `schema` to `target`. A missing schema
/// counts as 1.
pub fn migrate_with(mut value: Value, steps: &[Step], target: u32) -> Migrated {
    let start = value
        .get("schema")
        .and_then(Value::as_u64)
        .and_then(|s| u32::try_from(s).ok())
        .unwrap_or(1)
        .max(1);
    if start > target {
        return Migrated {
            value,
            from: None,
            newer: true,
        };
    }
    for schema in start..target {
        let step = steps[(schema - 1) as usize];
        value = step(value);
        if let Some(obj) = value.as_object_mut() {
            obj.insert("schema".into(), Value::from(schema + 1));
        }
    }
    Migrated {
        value,
        from: (start < target).then_some(start),
        newer: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn add_a(mut v: Value) -> Value {
        v["a"] = json!(1);
        v
    }
    fn rename_a_to_b(mut v: Value) -> Value {
        let a = v.as_object_mut().unwrap().remove("a").unwrap();
        v["b"] = a;
        v
    }

    #[test]
    fn current_schema_is_untouched() {
        let m = migrate(json!({ "schema": SCHEMA_VERSION, "x": 1 }));
        assert_eq!(m.value, json!({ "schema": SCHEMA_VERSION, "x": 1 }));
        assert_eq!(m.from, None);
        assert!(!m.newer);
    }

    #[test]
    fn runs_steps_in_order_and_bumps_schema() {
        let m = migrate_with(json!({ "schema": 1 }), &[add_a, rename_a_to_b], 3);
        assert_eq!(m.value, json!({ "schema": 3, "b": 1 }));
        assert_eq!(m.from, Some(1));
    }

    #[test]
    fn starts_from_the_files_schema() {
        let m = migrate_with(json!({ "schema": 2, "a": 5 }), &[add_a, rename_a_to_b], 3);
        assert_eq!(m.value, json!({ "schema": 3, "b": 5 }));
        assert_eq!(m.from, Some(2));
    }

    #[test]
    fn missing_schema_counts_as_one() {
        let m = migrate_with(json!({}), &[add_a], 2);
        assert_eq!(m.value, json!({ "schema": 2, "a": 1 }));
    }

    #[test]
    fn newer_schema_is_flagged_not_changed() {
        let m = migrate_with(json!({ "schema": 9, "z": 1 }), &[], 1);
        assert!(m.newer);
        assert_eq!(m.value, json!({ "schema": 9, "z": 1 }));
    }

    #[test]
    fn steps_cover_every_schema_below_current() {
        assert_eq!(STEPS.len() as u32, SCHEMA_VERSION - 1);
    }
}
