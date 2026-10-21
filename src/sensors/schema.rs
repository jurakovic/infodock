//! Property schemas (SPEC §8.2) and the storage rules of §11.2.
//!
//! A sensor type declares its properties as a static list of [`PropField`]s. The list drives
//! the generated Settings form, validation, and the static defaults:
//!
//! * [`resolve`] gives a sensor its full props: every field's default, overlaid with the valid
//!   stored values.
//! * [`check`] normalises one value: colours upper case, numbers clamped to their range. A value
//!   of the wrong type is invalid: on load it is dropped with a warning, an update rejects it.
//! * [`to_stored`] keeps only what differs from the static defaults, plus machine-derived
//!   fields, which are always stored. Keys the schema does not know (from a newer version) are
//!   kept as they were.

use serde_json::{Map, Number, Value, json};

use super::{MsgKey, Props};
use crate::config::model::normalize_color;

/// A constant value, for defaults and choices in static schemas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lit {
    Null,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(&'static str),
    /// A `colorMode` value: `{ "mode": …, "color": "#RRGGBB" }`.
    ColorMode(&'static str, &'static str),
}

impl Lit {
    pub fn to_value(self) -> Value {
        match self {
            Lit::Null => Value::Null,
            Lit::Bool(b) => Value::Bool(b),
            Lit::Int(i) => Value::from(i),
            Lit::Num(n) => number(n),
            Lit::Str(s) => Value::from(s),
            Lit::ColorMode(mode, color) => {
                let mut m = Map::new();
                m.insert("mode".into(), Value::from(mode));
                m.insert("color".into(), Value::from(color));
                Value::Object(m)
            }
        }
    }
}

/// `n` as a JSON number, integral when it has no fraction (so `9` is not stored as `9.0`).
fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        Value::from(n as i64)
    } else {
        Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectOption {
    pub value: Lit,
    pub label: MsgKey,
}

/// The field type, which picks the Settings control and the validation (SPEC §8.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldKind {
    /// With `empty_is_default`, an empty string means the default.
    Text {
        empty_is_default: bool,
    },
    Bool,
    Select {
        options: &'static [SelectOption],
    },
    Slider {
        min: f64,
        max: f64,
        step: f64,
        unit: Option<MsgKey>,
    },
    /// Seconds.
    Interval {
        min: f64,
        max: f64,
    },
    /// `#RRGGBB`.
    Color,
    /// `{ mode, color }`; `dynamic` allows the dynamic gradient (§8.4).
    ColorMode {
        dynamic: bool,
    },
    /// Integer percent, `min`–100.
    Opacity {
        min: u32,
    },
    /// A path; existence is checked when used.
    File {
        extensions: &'static [&'static str],
    },
    Folder,
    /// A font family; `""` follows the global sensor font.
    Font,
    /// One of the §8.2 font style ids.
    FontStyle,
    /// `X:\`.
    Drive {
        hide_optical: bool,
    },
    /// `{ id, name }`, or `null` for automatic.
    NetAdapter,
    /// A battery index.
    Battery,
    /// A CPU series: `total`, `user`, `system` or `core<i>` (SPEC §9.9). Settings lists this
    /// computer's processors.
    CpuSeries,
    /// A power mode id (`lock`, `sleep`, `shutDown` …; SPEC §9.21). Settings lists the modes
    /// this computer offers.
    PowerMode,
    /// Internal values; not shown, any JSON.
    Hidden,
}

/// Shows a field only while another field has one of `values`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleIf {
    pub key: &'static str,
    pub values: &'static [Lit],
}

/// A field that means nothing while it is empty (SPEC §8.2): the Properties form says so on its
/// row and does not save. Stored and imported sensors may still hold it empty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Required {
    /// What the row says while the field is empty.
    pub message: MsgKey,
    /// Another field whose value makes this one optional, e.g. a Shortcut's URL for its target.
    pub unless: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropField {
    pub key: &'static str,
    pub kind: FieldKind,
    pub label: MsgKey,
    pub description: Option<MsgKey>,
    /// The static default; for a machine-derived field, the value used when the type gives
    /// none.
    pub default: Lit,
    /// The default comes from [`super::SensorType::machine_defaults`] and is always stored.
    pub machine_default: bool,
    pub visible_if: Option<VisibleIf>,
    pub required: Option<Required>,
}

impl PropField {
    pub const fn new(key: &'static str, kind: FieldKind, label: MsgKey, default: Lit) -> Self {
        Self {
            key,
            kind,
            label,
            description: None,
            default,
            machine_default: false,
            visible_if: None,
            required: None,
        }
    }

    pub const fn describe(mut self, description: MsgKey) -> Self {
        self.description = Some(description);
        self
    }

    pub const fn machine(mut self) -> Self {
        self.machine_default = true;
        self
    }

    pub const fn visible_if(mut self, key: &'static str, values: &'static [Lit]) -> Self {
        self.visible_if = Some(VisibleIf { key, values });
        self
    }

    /// The field means nothing while empty; the form's row says `message`.
    pub const fn required(mut self, message: MsgKey) -> Self {
        self.required = Some(Required {
            message,
            unless: None,
        });
        self
    }

    /// As [`Self::required`], while field `other` is empty too.
    pub const fn required_unless(mut self, other: &'static str, message: MsgKey) -> Self {
        self.required = Some(Required {
            message,
            unless: Some(other),
        });
        self
    }

    /// Whether this field is required and empty in `props`: absent, `null` or blank text.
    pub fn missing(&self, props: &Props) -> bool {
        let Some(required) = self.required else {
            return false;
        };
        let empty = |key: &str| {
            props.get(key).is_none_or(|value| match value {
                Value::Null => true,
                Value::String(s) => s.trim().is_empty(),
                _ => false,
            })
        };
        empty(self.key) && required.unless.is_none_or(empty)
    }

    /// Whether Settings shows this field.
    pub fn visible(&self, props: &Props) -> bool {
        if matches!(self.kind, FieldKind::Hidden) {
            return false;
        }
        self.visible_if.is_none_or(|v| {
            props
                .get(v.key)
                .is_some_and(|value| v.values.iter().any(|lit| lit.to_value() == *value))
        })
    }
}

const FONT_STYLES: [&str; 6] = [
    "normal",
    "semibold",
    "bold",
    "italic",
    "semiboldItalic",
    "boldItalic",
];

fn number_in(value: &Value, min: f64, max: f64) -> Option<Value> {
    let n = value.as_f64()?;
    n.is_finite().then(|| number(n.clamp(min, max)))
}

/// The normalised `value` for `field`, or `None` if it has the wrong type or shape.
pub fn check(field: &PropField, value: &Value) -> Option<Value> {
    match field.kind {
        FieldKind::Text { .. } | FieldKind::File { .. } | FieldKind::Folder | FieldKind::Font => {
            value.is_string().then(|| value.clone())
        }
        FieldKind::Bool => value.is_boolean().then(|| value.clone()),
        FieldKind::Select { options } => options
            .iter()
            .map(|o| o.value.to_value())
            .find(|v| v == value),
        // Whole seconds, and whole numbers for sliders with whole steps: the typed props
        // read them as integers (a hand-edited 2.5 would not load).
        FieldKind::Interval { min, max } => number_in(value, min, max)
            .and_then(|v| v.as_f64())
            .map(|n| number(n.round())),
        FieldKind::Slider { min, max, step, .. } => {
            let whole = step.fract() == 0.0;
            number_in(value, min, max)
                .and_then(|v| v.as_f64())
                .map(|n| number(if whole { n.round() } else { n }))
        }
        FieldKind::Color => value
            .as_str()
            .and_then(|s| normalize_color(s.to_owned()))
            .map(Value::from),
        FieldKind::ColorMode { dynamic } => {
            let mode = value.get("mode")?.as_str()?;
            let allowed = matches!(mode, "standard" | "custom") || (dynamic && mode == "dynamic");
            let color = value
                .get("color")?
                .as_str()
                .and_then(|s| normalize_color(s.to_owned()))?;
            allowed.then(|| json!({ "mode": mode, "color": color }))
        }
        FieldKind::Opacity { min } => number_in(value, f64::from(min), 100.0)
            .and_then(|v| v.as_f64())
            .map(|n| Value::from(n.round() as i64)),
        FieldKind::FontStyle => value
            .as_str()
            .filter(|s| FONT_STYLES.contains(s))
            .map(Value::from),
        FieldKind::Drive { .. } => {
            let s = value.as_str()?;
            let bytes = s.as_bytes();
            (bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && &s[1..] == ":\\")
                .then(|| Value::from(s.to_ascii_uppercase()))
        }
        FieldKind::NetAdapter => match value {
            Value::Null => Some(Value::Null),
            Value::Object(o) => {
                let id = o.get("id")?.as_str()?;
                let name = o.get("name")?.as_str()?;
                let mut m = Map::new();
                m.insert("id".into(), Value::from(id));
                m.insert("name".into(), Value::from(name));
                Some(Value::Object(m))
            }
            _ => None,
        },
        // The battery's index is a `u32`; a larger number could not be read by its type.
        FieldKind::Battery => value
            .as_u64()
            .filter(|n| u32::try_from(*n).is_ok())
            .map(Value::from),
        FieldKind::CpuSeries => value
            .as_str()
            .filter(|s| {
                matches!(*s, "total" | "user" | "system")
                    || s.strip_prefix("core")
                        .is_some_and(|n| !n.is_empty() && n.parse::<u32>().is_ok())
            })
            .map(Value::from),
        FieldKind::PowerMode => value
            .as_str()
            .filter(|s| crate::platform::PowerAction::from_id(s).is_some())
            .map(Value::from),
        FieldKind::Hidden => Some(value.clone()),
    }
}

/// [`check`], with an empty text of an `empty_is_default` field replaced by `default`.
pub fn normalize(field: &PropField, raw: &Value, default: &Value) -> Option<Value> {
    let value = check(field, raw)?;
    let empty_means_default = matches!(
        field.kind,
        FieldKind::Text {
            empty_is_default: true
        }
    ) && value.as_str() == Some("");
    Some(if empty_means_default {
        default.clone()
    } else {
        value
    })
}

/// A stored value that was ignored, for the log.
#[derive(Debug, Clone, PartialEq)]
pub struct Ignored {
    pub key: String,
    pub value: Value,
}

/// The full props for `schema`: each field's default (`machine` for machine-derived fields),
/// overlaid with the valid values of `stored`. Invalid stored values are reported.
pub fn resolve(schema: &[PropField], stored: &Props, machine: &Props) -> (Props, Vec<Ignored>) {
    let mut props = Props::new();
    let mut ignored = Vec::new();
    for field in schema {
        let default = field
            .machine_default
            .then(|| machine.get(field.key).cloned())
            .flatten()
            .unwrap_or_else(|| field.default.to_value());
        let value = match stored.get(field.key) {
            None => default,
            Some(raw) => normalize(field, raw, &default).unwrap_or_else(|| {
                ignored.push(Ignored {
                    key: field.key.to_owned(),
                    value: raw.clone(),
                });
                default
            }),
        };
        props.insert(field.key.to_owned(), value);
    }
    (props, ignored)
}

/// What to store for `props` (SPEC §11.2): values that differ from the static defaults and
/// all machine-derived fields. Keys of `previous` that the schema does not know are kept.
pub fn to_stored(schema: &[PropField], props: &Props, previous: &Props) -> Props {
    let mut stored: Props = previous
        .iter()
        .filter(|(key, _)| !schema.iter().any(|f| f.key == key.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for field in schema {
        if let Some(value) = props.get(field.key)
            && (field.machine_default || *value != field.default.to_value())
        {
            stored.insert(field.key.to_owned(), value.clone());
        }
    }
    stored
}

/// Machine-derived fields of `schema` that `stored` lacks.
pub fn missing_machine_fields<'a>(schema: &'a [PropField], stored: &Props) -> Vec<&'a PropField> {
    schema
        .iter()
        .filter(|f| f.machine_default && !stored.contains_key(f.key))
        .collect()
}

/// A field of `schema` by key.
pub fn field<'a>(schema: &'a [PropField], key: &str) -> Option<&'a PropField> {
    schema.iter().find(|f| f.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MODES: &[SelectOption] = &[
        SelectOption {
            value: Lit::Str("none"),
            label: "x",
        },
        SelectOption {
            value: Lit::Str("number"),
            label: "x",
        },
        SelectOption {
            value: Lit::Int(30),
            label: "x",
        },
    ];

    const SCHEMA: &[PropField] = &[
        PropField::new(
            "text",
            FieldKind::Text {
                empty_is_default: true,
            },
            "x",
            Lit::Str("Text"),
        ),
        PropField::new("shadow", FieldKind::Bool, "x", Lit::Bool(false)),
        PropField::new(
            "width",
            FieldKind::Slider {
                min: 3.0,
                max: 145.0,
                step: 1.0,
                unit: None,
            },
            "x",
            Lit::Int(9),
        ),
        PropField::new(
            "mode",
            FieldKind::Select { options: MODES },
            "x",
            Lit::Str("none"),
        ),
        PropField::new(
            "color",
            FieldKind::ColorMode { dynamic: false },
            "x",
            Lit::ColorMode("standard", "#FFFFFF"),
        ),
        PropField::new(
            "opacity",
            FieldKind::Opacity { min: 20 },
            "x",
            Lit::Int(100),
        )
        .visible_if("mode", &[Lit::Str("number"), Lit::Int(30)]),
        PropField::new("use24h", FieldKind::Bool, "x", Lit::Bool(true)).machine(),
        PropField::new("size", FieldKind::Hidden, "x", Lit::Null),
    ];

    fn props(v: Value) -> Props {
        match v {
            Value::Object(m) => m,
            _ => panic!("object expected"),
        }
    }

    #[test]
    fn resolves_defaults_and_stored_values() {
        let machine = props(json!({ "use24h": false }));
        let (p, ignored) = resolve(SCHEMA, &props(json!({ "width": 20 })), &machine);
        assert!(ignored.is_empty());
        assert_eq!(
            Value::Object(p),
            json!({
                "text": "Text", "shadow": false, "width": 20, "mode": "none",
                "color": { "mode": "standard", "color": "#FFFFFF" }, "opacity": 100,
                "use24h": false, "size": null
            })
        );
    }

    #[test]
    fn a_battery_index_fits_its_type() {
        let field = PropField::new("battery", FieldKind::Battery, "x", Lit::Int(0));
        assert_eq!(check(&field, &json!(2)), Some(json!(2)));
        assert_eq!(
            check(&field, &json!(u64::from(u32::MAX))),
            Some(json!(u32::MAX))
        );
        assert_eq!(check(&field, &json!(u64::from(u32::MAX) + 1)), None);
        assert_eq!(check(&field, &json!(-1)), None);
    }

    #[test]
    fn required_fields_are_missing_while_empty() {
        let folder = PropField::new("folder", FieldKind::Folder, "x", Lit::Str("")).required("x");
        let target = PropField::new(
            "target",
            FieldKind::File { extensions: &[] },
            "x",
            Lit::Str(""),
        )
        .required_unless("url", "x");
        let optional = PropField::new("title", FieldKind::Folder, "x", Lit::Str(""));
        for empty in [json!({}), json!({ "folder": "" }), json!({ "folder": " " })] {
            assert!(folder.missing(&props(empty.clone())), "{empty}");
            assert!(!optional.missing(&props(empty)));
        }
        assert!(folder.missing(&props(json!({ "folder": null }))));
        assert!(!folder.missing(&props(json!({ "folder": "C:\\" }))));
        assert!(target.missing(&props(json!({ "target": "", "url": "" }))));
        assert!(!target.missing(&props(json!({ "url": "https://example.com" }))));
        assert!(!target.missing(&props(json!({ "target": "a.exe" }))));
    }

    #[test]
    fn normalises_and_ignores_invalid_values() {
        let stored = props(json!({
            "text": "",
            "shadow": "yes",
            "width": 500,
            "mode": 30,
            "color": { "mode": "custom", "color": "#a0ff9b" },
            "opacity": 5.4,
        }));
        let (p, ignored) = resolve(SCHEMA, &stored, &Props::new());
        // Empty text means the default here.
        assert_eq!(p["text"], "Text");
        assert_eq!(p["shadow"], false);
        assert_eq!(p["width"], 145);
        assert_eq!(p["mode"], 30);
        assert_eq!(p["color"], json!({ "mode": "custom", "color": "#A0FF9B" }));
        assert_eq!(p["opacity"], 20);
        assert_eq!(p["use24h"], true, "static fallback without a machine value");
        assert_eq!(
            ignored,
            vec![Ignored {
                key: "shadow".into(),
                value: json!("yes")
            }]
        );
    }

    #[test]
    fn checks_each_field_type() {
        let f = |kind| PropField::new("k", kind, "x", Lit::Null);
        let dynamic = f(FieldKind::ColorMode { dynamic: true });
        let plain = f(FieldKind::ColorMode { dynamic: false });
        let dyn_value = json!({ "mode": "dynamic", "color": "#00FF00" });
        assert_eq!(check(&dynamic, &dyn_value), Some(dyn_value.clone()));
        assert_eq!(check(&plain, &dyn_value), None);
        assert_eq!(check(&plain, &json!({ "mode": "custom" })), None);
        assert_eq!(
            check(&f(FieldKind::Color), &json!("#abcdef")),
            Some(json!("#ABCDEF"))
        );
        assert_eq!(check(&f(FieldKind::Color), &json!("red")), None);
        assert_eq!(
            check(&f(FieldKind::FontStyle), &json!("boldItalic")),
            Some(json!("boldItalic"))
        );
        assert_eq!(check(&f(FieldKind::FontStyle), &json!("heavy")), None);
        let drive = f(FieldKind::Drive { hide_optical: true });
        assert_eq!(check(&drive, &json!("c:\\")), Some(json!("C:\\")));
        assert_eq!(check(&drive, &json!("C:")), None);
        let interval = f(FieldKind::Interval {
            min: 1.0,
            max: 60.0,
        });
        assert_eq!(check(&interval, &json!(0.5)), Some(json!(1)));
        assert_eq!(
            check(&interval, &json!(2.5)),
            Some(json!(3)),
            "whole seconds"
        );
        let width = f(FieldKind::Slider {
            min: 50.0,
            max: 150.0,
            step: 5.0,
            unit: None,
        });
        assert_eq!(
            check(&width, &json!(57.4)),
            Some(json!(57)),
            "whole numbers"
        );
        let fine = f(FieldKind::Slider {
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: None,
        });
        assert_eq!(check(&fine, &json!(0.25)), Some(json!(0.25)));
        assert_eq!(check(&interval, &json!("2")), None);
        let adapter = f(FieldKind::NetAdapter);
        assert_eq!(check(&adapter, &Value::Null), Some(Value::Null));
        assert_eq!(
            check(
                &adapter,
                &json!({ "id": "{1}", "name": "Ethernet", "x": 1 })
            ),
            Some(json!({ "id": "{1}", "name": "Ethernet" }))
        );
        assert_eq!(check(&f(FieldKind::Battery), &json!(-1)), None);
        let mode = f(FieldKind::PowerMode);
        assert_eq!(check(&mode, &json!("shutDown")), Some(json!("shutDown")));
        assert_eq!(check(&mode, &json!("explode")), None);
        assert_eq!(check(&mode, &json!(5)), None);
        assert_eq!(
            check(&f(FieldKind::Hidden), &json!([1, 2])),
            Some(json!([1, 2]))
        );
        assert_eq!(check(&f(FieldKind::Font), &json!("")), Some(json!("")));
    }

    #[test]
    fn stores_only_changes_and_machine_fields() {
        let machine = props(json!({ "use24h": true }));
        let previous = props(json!({ "width": 20, "fromNewerVersion": 1 }));
        let (mut p, _) = resolve(SCHEMA, &previous, &machine);
        p.insert("shadow".into(), json!(true));
        p.insert("width".into(), json!(9));
        let stored = to_stored(SCHEMA, &p, &previous);
        assert_eq!(
            Value::Object(stored),
            json!({ "shadow": true, "use24h": true, "fromNewerVersion": 1 })
        );
    }

    #[test]
    fn finds_missing_machine_fields() {
        let missing: Vec<_> = missing_machine_fields(SCHEMA, &Props::new())
            .iter()
            .map(|f| f.key)
            .collect();
        assert_eq!(missing, ["use24h"]);
        let stored = props(json!({ "use24h": false }));
        assert!(missing_machine_fields(SCHEMA, &stored).is_empty());
    }

    #[test]
    fn visibility() {
        let (mut p, _) = resolve(SCHEMA, &Props::new(), &Props::new());
        let opacity = field(SCHEMA, "opacity").expect("field");
        assert!(!opacity.visible(&p));
        p.insert("mode".into(), json!(30));
        assert!(opacity.visible(&p));
        assert!(!field(SCHEMA, "size").expect("field").visible(&p));
        assert!(field(SCHEMA, "text").expect("field").visible(&p));
    }

    #[test]
    fn numbers_stay_integral() {
        assert_eq!(number(9.0), json!(9));
        assert_eq!(number(2.5), json!(2.5));
        assert_eq!(Lit::Num(0.5).to_value(), json!(0.5));
    }
}
