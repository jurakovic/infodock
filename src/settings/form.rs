//! The properties form (SPEC §6.7): a sensor's schema fields as rows for Settings (§8.2), the
//! user's edits, and the changes to preview and save. Pure; the window glue is in `mod.rs`.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::model::FONT_STYLES;
use crate::config::model::SensorInstance;
use crate::i18n::t;
use crate::sensors::Props;
use crate::sensors::registry::Registry;
use crate::sensors::schema::{self, FieldKind, PropField};

/// How a row is edited; the window shows one control per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Text,
    Flag,
    /// A list: `select`, `font` and `fontStyle` fields.
    Choice,
    /// A slider: `slider`, `interval` and `opacity` fields.
    Number,
    Color,
    /// Standard / Custom (/ Dynamic) with a colour for Custom.
    ColorMode,
    File,
    Folder,
}

impl RowKind {
    /// The kind's number in the window (`PropRow.kind` in `settings.slint`).
    pub fn code(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormRow {
    pub key: &'static str,
    pub kind: RowKind,
    pub label: String,
    pub help: String,
    pub visible: bool,
    pub text: String,
    pub checked: bool,
    pub number: f32,
    pub minimum: f32,
    pub maximum: f32,
    pub step: f32,
    /// A message key with a `{value}` placeholder.
    pub unit: &'static str,
    pub options: Vec<String>,
    pub index: i32,
    /// The colour of a colour-mode row.
    pub color: String,
    /// Why the value typed last cannot be used; empty if it can.
    pub error: String,
}

/// The colour modes a field offers, in list order (SPEC §8.4).
fn modes(dynamic: bool) -> &'static [&'static str] {
    if dynamic {
        &["standard", "custom", "dynamic"]
    } else {
        &["standard", "custom"]
    }
}

/// What the form edits.
pub struct Form {
    pub id: String,
    pub title: String,
    /// Added by the picker just now: Cancel removes it again (SPEC §6.6).
    pub new: bool,
    schema: &'static [PropField],
    /// The resolved props with the edits applied.
    props: Props,
    /// The edited keys and their values, for previewing and saving.
    changes: Props,
    /// Fields whose last edit was invalid: they show an error and block saving (SPEC §6.7).
    invalid: BTreeSet<&'static str>,
}

impl Form {
    /// The form for `instance`; `None` if its type is unknown.
    pub fn open(registry: &Registry, instance: &SensorInstance, new: bool) -> Option<Self> {
        let ty = registry.get(&instance.kind)?;
        let (props, _) = schema::resolve(ty.schema(), &instance.props, &Props::new());
        Some(Self {
            id: instance.id.clone(),
            title: t(ty.name()),
            new,
            schema: ty.schema(),
            props,
            changes: Props::new(),
            invalid: BTreeSet::new(),
        })
    }

    pub fn changes(&self) -> &Props {
        &self.changes
    }

    /// Whether the edits can be saved: no visible field holds an invalid value.
    pub fn valid(&self) -> bool {
        self.schema
            .iter()
            .filter(|f| self.invalid.contains(f.key))
            .all(|f| !f.visible(&self.props))
    }

    /// The rows for the fields Settings can edit, in schema order. `fonts` is the list of
    /// installed families.
    pub fn rows(&self, fonts: &[String]) -> Vec<FormRow> {
        self.schema
            .iter()
            .filter_map(|field| self.row(field, fonts))
            .collect()
    }

    fn row(&self, field: &PropField, fonts: &[String]) -> Option<FormRow> {
        let kind = kind(field)?;
        let value = self.props.get(field.key).cloned().unwrap_or(Value::Null);
        let mut row = FormRow {
            key: field.key,
            kind,
            label: t(field.label),
            help: field.description.map(t).unwrap_or_default(),
            visible: field.visible(&self.props),
            error: if self.invalid.contains(field.key) {
                t(match kind {
                    RowKind::Color | RowKind::ColorMode => "settings.color.invalid",
                    _ => "settings.props.invalid",
                })
            } else {
                String::new()
            },
            text: value.as_str().unwrap_or_default().to_owned(),
            checked: value.as_bool().unwrap_or(false),
            number: value.as_f64().unwrap_or(0.0) as f32,
            minimum: 0.0,
            maximum: 100.0,
            step: 1.0,
            unit: "common.px",
            options: Vec::new(),
            index: 0,
            color: String::new(),
        };
        match field.kind {
            FieldKind::Select { options } => {
                row.options = options.iter().map(|o| t(o.label)).collect();
                row.index = options
                    .iter()
                    .position(|o| o.value.to_value() == value)
                    .unwrap_or(0) as i32;
            }
            FieldKind::Font => {
                row.options = font_options(fonts, row.text.as_str());
                row.index = if row.text.is_empty() {
                    0
                } else {
                    row.options
                        .iter()
                        .skip(1)
                        .position(|f| *f == row.text)
                        .map_or(0, |i| i as i32 + 1)
                };
            }
            FieldKind::FontStyle => {
                row.options = FONT_STYLES
                    .iter()
                    .map(|s| t(&format!("fontStyle.{s}")))
                    .collect();
                row.index = FONT_STYLES
                    .iter()
                    .position(|s| Some(*s) == value.as_str())
                    .unwrap_or(0) as i32;
            }
            FieldKind::Slider {
                min,
                max,
                step,
                unit,
            } => {
                (row.minimum, row.maximum, row.step) = (min as f32, max as f32, step as f32);
                row.unit = unit.unwrap_or("common.number");
            }
            FieldKind::Interval { min, max } => {
                (row.minimum, row.maximum) = (min as f32, max as f32);
                row.unit = "common.seconds";
            }
            FieldKind::Opacity { min } => {
                row.minimum = min as f32;
                row.unit = "common.percent";
            }
            FieldKind::ColorMode { dynamic } => {
                let mode = value
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("standard");
                row.options = modes(dynamic)
                    .iter()
                    .map(|m| t(&format!("settings.props.colorMode.{m}")))
                    .collect();
                row.index = modes(dynamic).iter().position(|m| *m == mode).unwrap_or(0) as i32;
                row.color = value
                    .get("color")
                    .and_then(Value::as_str)
                    .unwrap_or("#FFFFFF")
                    .to_owned();
            }
            _ => {}
        }
        Some(row)
    }

    /// Applies an edit of `key`; `false` if the value is invalid, which leaves the value as it
    /// was and marks the field invalid until a valid edit.
    pub fn set(&mut self, key: &str, value: Value) -> bool {
        let Some(field) = schema::field(self.schema, key) else {
            return false;
        };
        let Some(value) = schema::normalize(field, &value, &field.default.to_value()) else {
            self.invalid.insert(field.key);
            return false;
        };
        self.invalid.remove(field.key);
        self.props.insert(key.to_owned(), value.clone());
        self.changes.insert(key.to_owned(), value);
        true
    }

    /// Applies the choice at `index` of a list row (`fonts` as for [`Self::rows`]).
    pub fn choose(&mut self, key: &str, index: usize, fonts: &[String]) -> bool {
        let Some(field) = schema::field(self.schema, key) else {
            return false;
        };
        let value = match field.kind {
            FieldKind::Select { options } => options.get(index).map(|o| o.value.to_value()),
            FieldKind::Font => {
                let current = self.props.get(key).and_then(Value::as_str).unwrap_or("");
                let options = font_options(fonts, current);
                match index {
                    0 => Some(Value::from("")),
                    i => options.get(i).map(|f| Value::from(f.as_str())),
                }
            }
            FieldKind::FontStyle => FONT_STYLES.get(index).map(|s| Value::from(*s)),
            _ => None,
        };
        value.is_some_and(|v| self.set(key, v))
    }

    /// Sets a colour-mode row to the mode at `index` with `color` (`#RRGGBB`).
    pub fn color_mode(&mut self, key: &str, index: usize, color: &str) -> bool {
        let Some(FieldKind::ColorMode { dynamic }) =
            schema::field(self.schema, key).map(|f| f.kind)
        else {
            return false;
        };
        match modes(dynamic).get(index) {
            Some(mode) => self.set(key, json!({ "mode": mode, "color": color.trim() })),
            None => false,
        }
    }
}

/// How Settings edits `field`; `None` for fields it does not show (hidden ones, and the
/// drive, adapter and battery pickers, which arrive with their sensors in M2 and M3).
fn kind(field: &PropField) -> Option<RowKind> {
    Some(match field.kind {
        FieldKind::Text { .. } => RowKind::Text,
        FieldKind::Bool => RowKind::Flag,
        FieldKind::Select { .. } | FieldKind::Font | FieldKind::FontStyle => RowKind::Choice,
        FieldKind::Slider { .. } | FieldKind::Interval { .. } | FieldKind::Opacity { .. } => {
            RowKind::Number
        }
        FieldKind::Color => RowKind::Color,
        FieldKind::ColorMode { .. } => RowKind::ColorMode,
        FieldKind::File { .. } => RowKind::File,
        FieldKind::Folder => RowKind::Folder,
        FieldKind::Drive { .. }
        | FieldKind::NetAdapter
        | FieldKind::Battery
        | FieldKind::Hidden => return None,
    })
}

/// "Default" (the global sensor font, SPEC §8.2), the installed families, and `current` if it
/// is not installed.
fn font_options(fonts: &[String], current: &str) -> Vec<String> {
    let mut options = vec![t("settings.props.font.default")];
    options.extend(fonts.iter().cloned());
    if !current.is_empty() && !fonts.iter().any(|f| f == current) {
        options.push(current.to_owned());
    }
    options
}

/// The file types a `file` field accepts, e.g. `["png", "jpg"]`.
pub fn extensions(registry: &Registry, kind: &str, key: &str) -> &'static [&'static str] {
    registry
        .get(kind)
        .and_then(|ty| schema::field(ty.schema(), key))
        .and_then(|f| match f.kind {
            FieldKind::File { extensions } => Some(extensions),
            _ => None,
        })
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::registry::registry;
    use crate::sensors::schema::Lit;

    fn instance(kind: &str, props: Value) -> SensorInstance {
        SensorInstance {
            id: "s_test1".into(),
            kind: kind.into(),
            props: props.as_object().cloned().unwrap_or_default(),
        }
    }

    fn fonts() -> Vec<String> {
        vec!["Arial".into(), "Consolas".into()]
    }

    #[test]
    fn text_sensor_rows() {
        let form = Form::open(
            registry(),
            &instance("text", json!({ "font": "Gone" })),
            false,
        )
        .expect("known type");
        assert_eq!(form.title, "Text");
        let rows = form.rows(&fonts());
        let kinds: Vec<_> = rows.iter().map(|r| (r.key, r.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("text", RowKind::Text),
                ("font", RowKind::Choice),
                ("fontStyle", RowKind::Choice),
                ("fontSize", RowKind::Number),
                ("shadow", RowKind::Flag),
            ]
        );
        assert_eq!(rows[0].text, "Text");
        assert_eq!(rows[1].options, ["Default", "Arial", "Consolas", "Gone"]);
        assert_eq!(rows[1].index, 3);
        assert_eq!(rows[2].options[0], "Normal");
        assert_eq!(
            (rows[3].number, rows[3].minimum, rows[3].maximum),
            (13.0, 9.0, 25.0)
        );
        assert_eq!(rows[3].unit, "common.px");
    }

    #[test]
    fn image_sensor_rows() {
        let form = Form::open(registry(), &instance("image", json!({})), true).expect("known");
        let rows = form.rows(&fonts());
        let color = rows.iter().find(|r| r.key == "color").expect("colour row");
        assert_eq!(color.kind, RowKind::ColorMode);
        assert_eq!(color.options, ["Standard", "Custom"]);
        assert_eq!((color.index, color.color.as_str()), (0, "#FFFFFF"));
        let opacity = rows
            .iter()
            .find(|r| r.key == "opacity")
            .expect("opacity row");
        assert_eq!((opacity.minimum, opacity.unit), (20.0, "common.percent"));
        assert_eq!(rows[0].kind, RowKind::File);
        assert_eq!(extensions(registry(), "image", "image").len(), 7);
        assert!(form.new);
    }

    #[test]
    fn edits_are_validated_and_collected() {
        let mut form = Form::open(registry(), &instance("text", json!({})), false).expect("known");
        assert!(form.set("text", json!("Hi")));
        assert!(form.set("fontSize", json!(99)), "clamped");
        assert!(!form.set("shadow", json!("yes")));
        assert!(!form.set("bogus", json!(1)));
        assert!(form.choose("font", 2, &fonts()));
        assert!(form.choose("fontStyle", 2, &fonts()));
        assert!(form.choose("font", 0, &fonts()), "back to the global font");
        assert_eq!(
            Value::Object(form.changes().clone()),
            json!({ "text": "Hi", "fontSize": 25, "font": "", "fontStyle": "bold" })
        );
        assert_eq!(form.rows(&fonts())[3].number, 25.0);
    }

    #[test]
    fn colour_modes() {
        let mut form = Form::open(registry(), &instance("image", json!({})), false).expect("known");
        assert!(form.color_mode("color", 1, "#ff0000"));
        assert_eq!(
            form.changes()["color"],
            json!({ "mode": "custom", "color": "#FF0000" })
        );
        assert!(
            !form.color_mode("color", 2, "#FF0000"),
            "no dynamic mode for images"
        );
        assert!(!form.color_mode("color", 1, "red"));
    }

    #[test]
    fn invalid_values_show_an_error_and_block_saving() {
        let mut form = Form::open(registry(), &instance("image", json!({})), false).expect("known");
        let error = |form: &Form| {
            let rows = form.rows(&fonts());
            rows.into_iter()
                .find(|r| r.key == "color")
                .map(|r| r.error)
                .unwrap_or_default()
        };
        assert!(form.valid());
        assert!(!form.color_mode("color", 1, "#12345"));
        assert!(!form.valid());
        assert_eq!(error(&form), t("settings.color.invalid"));
        assert!(form.changes().is_empty(), "the invalid colour is not kept");
        assert!(form.color_mode("color", 1, "#123456"));
        assert!(form.valid());
        assert_eq!(error(&form), "");
    }

    #[test]
    fn hidden_invalid_fields_do_not_block_saving() {
        static SCHEMA: [PropField; 2] = [
            PropField::new("custom", FieldKind::Bool, "label", Lit::Bool(true)),
            PropField::new("color", FieldKind::Color, "label", Lit::Str("#FFFFFF"))
                .visible_if("custom", &[Lit::Bool(true)]),
        ];
        let mut form = Form {
            id: "s_test1".into(),
            title: "Test".into(),
            new: false,
            schema: &SCHEMA,
            props: schema::resolve(&SCHEMA, &Props::new(), &Props::new()).0,
            changes: Props::new(),
            invalid: BTreeSet::new(),
        };
        assert!(!form.set("color", json!("#12")));
        assert!(!form.valid());
        assert!(form.set("custom", json!(false)));
        assert!(form.valid(), "the colour is hidden now");
    }

    #[test]
    fn unknown_types_have_no_form() {
        assert!(Form::open(registry(), &instance("cpu-graph", json!({})), false).is_none());
    }

    #[test]
    fn every_registered_field_is_shown_and_labelled() {
        for ty in registry().types() {
            let form =
                Form::open(registry(), &instance(ty.id(), json!({})), false).expect("registered");
            for row in form.rows(&fonts()) {
                assert!(
                    !row.label.contains('.'),
                    "{}.{}: {}",
                    ty.id(),
                    row.key,
                    row.label
                );
            }
        }
    }
}
