//! The properties form (SPEC §6.7): a sensor's schema fields as rows for Settings (§8.2), the
//! user's edits, and the changes to preview and save. Pure; the window glue is in `mod.rs`.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::model::FONT_STYLES;
use crate::config::model::SensorInstance;
use crate::i18n::{t, t_args};
use crate::platform::{DriveKind, NetInterface, Volume};
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
    /// The resolved props as stored when the form opened: a stored drive, adapter or
    /// processor this computer lacks stays listed after another one is chosen.
    opened: Props,
    /// The edited keys and their values, for previewing and saving.
    changes: Props,
    /// Fields whose last edit was invalid: they show an error and block saving (SPEC §6.7).
    invalid: BTreeSet<&'static str>,
    /// This computer's logical processors, for `cpuSeries` fields.
    processors: u32,
    /// This computer's volumes, for `drive` fields; `None` until the sampler's list arrives.
    volumes: Option<Vec<Volume>>,
    /// This computer's network connections, for `netAdapter` fields.
    adapters: Vec<NetInterface>,
    /// The power modes this computer offers, for `powerMode` fields.
    power_modes: Vec<String>,
    /// This computer's batteries by name, in the driver's order, for `battery` fields; `None`
    /// until the sampler's list arrives.
    batteries: Option<Vec<String>>,
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
            opened: props.clone(),
            props,
            changes: Props::new(),
            invalid: BTreeSet::new(),
            processors: 1,
            volumes: None,
            adapters: Vec::new(),
            power_modes: Vec::new(),
            batteries: None,
        })
    }

    /// Whether a field picks a battery, so the form needs the batteries.
    pub fn has_battery_field(&self) -> bool {
        self.schema
            .iter()
            .any(|f| matches!(f.kind, FieldKind::Battery))
    }

    /// Sets the batteries `battery` fields offer, by name in the driver's order (SPEC §8.2).
    pub fn set_batteries(&mut self, names: Vec<String>) {
        self.batteries = Some(names);
    }

    /// Sets the power modes `powerMode` fields offer (SPEC §9.21).
    pub fn set_power_modes(&mut self, modes: Vec<String>) {
        self.power_modes = modes;
    }

    /// Sets the number of logical processors `cpuSeries` fields offer (SPEC §9.9).
    pub fn set_processors(&mut self, processors: u32) {
        self.processors = processors.max(1);
    }

    /// Whether a field picks a drive, so the form needs the volumes (reading them can take a
    /// moment with network drives).
    pub fn has_drive_field(&self) -> bool {
        self.schema
            .iter()
            .any(|f| matches!(f.kind, FieldKind::Drive { .. }))
    }

    /// Whether a field picks a network connection, so the form needs the list.
    pub fn has_adapter_field(&self) -> bool {
        self.schema
            .iter()
            .any(|f| matches!(f.kind, FieldKind::NetAdapter))
    }

    /// Sets the connections `netAdapter` fields offer (SPEC §8.2).
    pub fn set_adapters(&mut self, adapters: Vec<NetInterface>) {
        self.adapters = adapters;
    }

    /// Sets the volumes `drive` fields offer (SPEC §8.2).
    pub fn set_volumes(&mut self, volumes: Vec<Volume>) {
        self.volumes = Some(volumes);
    }

    pub fn changes(&self) -> &Props {
        &self.changes
    }

    /// Whether the edits can be saved: no visible field holds an invalid value or is required
    /// and empty (SPEC §8.2).
    pub fn valid(&self) -> bool {
        self.schema
            .iter()
            .filter(|f| self.invalid.contains(f.key) || f.missing(&self.props))
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
            } else if field.missing(&self.props) {
                field.required.map(|r| t(r.message)).unwrap_or_default()
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
            FieldKind::CpuSeries => {
                let current = value.as_str().unwrap_or("total");
                let series = cpu_series(self.processors, self.opened_str(field.key));
                row.index = series.iter().position(|(v, _)| v == current).unwrap_or(0) as i32;
                row.options = series.into_iter().map(|(_, label)| label).collect();
            }
            FieldKind::Drive { hide_optical } => {
                let current = value.as_str().unwrap_or("");
                let drives = drives(
                    self.volumes.as_deref(),
                    hide_optical,
                    self.opened_str(field.key),
                );
                row.index = drives.iter().position(|(v, _)| v == current).unwrap_or(0) as i32;
                row.options = drives.into_iter().map(|(_, label)| label).collect();
            }
            FieldKind::PowerMode => {
                let current = value.as_str().unwrap_or("");
                let modes = power_modes(&self.power_modes, self.opened_str(field.key));
                row.index = modes.iter().position(|(v, _)| v == current).unwrap_or(0) as i32;
                row.options = modes.into_iter().map(|(_, label)| label).collect();
            }
            FieldKind::Battery => {
                let current = value.as_u64().unwrap_or(0);
                let list = batteries(self.batteries.as_deref(), self.opened_index(field.key));
                row.index = list.iter().position(|(v, _)| *v == current).unwrap_or(0) as i32;
                row.options = list.into_iter().map(|(_, label)| label).collect();
            }
            FieldKind::NetAdapter => {
                let adapters = adapters(&self.adapters, self.opened.get(field.key));
                row.index = adapters
                    .iter()
                    .position(|(v, _)| adapter_id(v) == adapter_id(&value))
                    .unwrap_or(0) as i32;
                row.options = adapters.into_iter().map(|(_, label)| label).collect();
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
            FieldKind::CpuSeries => cpu_series(self.processors, self.opened_str(key))
                .into_iter()
                .nth(index)
                .map(|(value, _)| Value::from(value)),
            FieldKind::Drive { hide_optical } => {
                drives(self.volumes.as_deref(), hide_optical, self.opened_str(key))
                    .into_iter()
                    .nth(index)
                    .map(|(value, _)| Value::from(value))
            }
            FieldKind::NetAdapter => adapters(&self.adapters, self.opened.get(key))
                .into_iter()
                .nth(index)
                .map(|(value, _)| value),
            FieldKind::PowerMode => power_modes(&self.power_modes, self.opened_str(key))
                .into_iter()
                .nth(index)
                .map(|(value, _)| Value::from(value)),
            FieldKind::Battery => batteries(self.batteries.as_deref(), self.opened_index(key))
                .into_iter()
                .nth(index)
                .map(|(value, _)| Value::from(value)),
            _ => None,
        };
        value.is_some_and(|v| self.set(key, v))
    }

    /// The text of `key` when the form opened.
    fn opened_str(&self, key: &str) -> &str {
        self.opened.get(key).and_then(Value::as_str).unwrap_or("")
    }

    /// The number of `key` when the form opened.
    fn opened_index(&self, key: &str) -> u64 {
        self.opened.get(key).and_then(Value::as_u64).unwrap_or(0)
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

/// How Settings edits `field`; `None` for fields it does not show (hidden ones).
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
        FieldKind::CpuSeries
        | FieldKind::Drive { .. }
        | FieldKind::NetAdapter
        | FieldKind::PowerMode
        | FieldKind::Battery => RowKind::Choice,
        FieldKind::Folder => RowKind::Folder,
        FieldKind::Hidden => return None,
    })
}

/// The power modes offered, as `(id, label)`, in menu order; the stored mode stays listed when
/// this computer lacks it (e.g. hibernate after hibernation was turned off).
fn power_modes(offered: &[String], stored: &str) -> Vec<(String, String)> {
    crate::platform::PowerAction::ALL
        .into_iter()
        .map(|a| a.id())
        .filter(|id| offered.iter().any(|o| o == id) || *id == stored)
        .map(|id| (id.to_owned(), t(&format!("sensor.power.mode.{id}"))))
        .collect()
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

/// The series a CPU dual graph offers (SPEC §9.9), as (value, label): total, user, system and
/// each logical processor; a `stored` processor this computer lacks stays listed, marked as
/// unavailable.
fn cpu_series(processors: u32, stored: &str) -> Vec<(String, String)> {
    let mut series: Vec<(String, String)> = [
        ("total", "sensor.cpu.series.total"),
        ("user", "sensor.cpu.series.user"),
        ("system", "sensor.cpu.series.system"),
    ]
    .into_iter()
    .map(|(value, label)| (value.to_owned(), t(label)))
    .collect();
    let processor = |i: u32, key: &str| t_args(key, &[("n", &(u64::from(i) + 1).to_string())]);
    series.extend(
        (0..processors).map(|i| (format!("core{i}"), processor(i, "sensor.cpu.series.core"))),
    );
    if !series.iter().any(|(value, _)| value == stored)
        && let Some(i) = stored
            .strip_prefix("core")
            .and_then(|n| n.parse::<u32>().ok())
    {
        series.push((stored.to_owned(), processor(i, "sensor.cpu.series.missing")));
    }
    series
}

/// The drives a `drive` field offers, as (root, label): this computer's volumes (optical ones
/// left out with `hide_optical`, unless `stored`), e.g. `Local Disk (C:)` or `Data (D:)` by
/// their label; a stored drive that is not connected stays listed, marked so. Until the list
/// is known (`None`), only the stored drive, by its root.
fn drives(volumes: Option<&[Volume]>, hide_optical: bool, stored: &str) -> Vec<(String, String)> {
    let Some(volumes) = volumes else {
        return (!stored.is_empty())
            .then(|| (stored.to_owned(), stored.to_owned()))
            .into_iter()
            .collect();
    };
    let mut drives: Vec<(String, String)> = volumes
        .iter()
        .filter(|v| !(hide_optical && v.kind == DriveKind::Optical) || v.root == stored)
        .map(|v| (v.root.clone(), crate::sensors::disk::volume_name(v)))
        .collect();
    if !stored.is_empty() && !drives.iter().any(|(root, _)| root == stored) {
        let label = t_args("volume.missing", &[("drive", stored)]);
        drives.push((stored.to_owned(), label));
    }
    drives
}

/// The batteries a `battery` field offers, as (index, label): `1. {name}` for each of this
/// computer's batteries (`1. Battery` when it has no name); a `stored` battery it lacks stays
/// listed, marked so. Until the list is known (`None`), only the stored battery, unmarked.
fn batteries(names: Option<&[String]>, stored: u64) -> Vec<(u64, String)> {
    let number = |i: u64| (i + 1).to_string();
    let Some(names) = names else {
        let name = t("sensor.battery.name");
        let label = t_args(
            "sensor.battery.option",
            &[("n", &number(stored)), ("name", &name)],
        );
        return vec![(stored, label)];
    };
    let mut list: Vec<(u64, String)> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let i = i as u64;
            let name = if name.is_empty() {
                t("sensor.battery.name")
            } else {
                name.clone()
            };
            let label = t_args(
                "sensor.battery.option",
                &[("n", &number(i)), ("name", &name)],
            );
            (i, label)
        })
        .collect();
    if !list.iter().any(|(i, _)| *i == stored) {
        let label = t_args("sensor.battery.option.missing", &[("n", &number(stored))]);
        list.push((stored, label));
    }
    list
}

/// A `netAdapter` value's id; `None` for Automatic.
fn adapter_id(value: &Value) -> Option<&str> {
    value.get("id").and_then(Value::as_str)
}

/// The connections a `netAdapter` field offers, as (stored value, label): Automatic (`null`),
/// then this computer's connections as `{ id, name }` named like `Wi-Fi (Intel(R) …)`, matched
/// by id (a renamed connection is the same one); a `stored` connection that is not there stays
/// listed, marked so.
fn adapters(interfaces: &[NetInterface], stored: Option<&Value>) -> Vec<(Value, String)> {
    let mut adapters = vec![(Value::Null, t("sensor.network.automatic"))];
    adapters.extend(interfaces.iter().map(|i| {
        let value = json!({ "id": i.id, "name": i.alias });
        let label = if i.description.is_empty() || i.description == i.alias {
            i.alias.clone()
        } else {
            t_args(
                "sensor.network.hint.named",
                &[("alias", &i.alias), ("description", &i.description)],
            )
        };
        (value, label)
    }));
    if let Some(stored) = stored
        && let Some(id) = adapter_id(stored)
        && !interfaces.iter().any(|i| i.id == id)
    {
        let name = stored.get("name").and_then(Value::as_str).unwrap_or(id);
        let label = t_args("sensor.network.missing", &[("name", name)]);
        adapters.push((stored.clone(), label));
    }
    adapters
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

    fn error(form: &Form, key: &str) -> String {
        form.rows(&fonts())
            .into_iter()
            .find(|r| r.key == key)
            .map(|r| r.error)
            .unwrap_or_default()
    }

    #[test]
    fn required_fields_show_their_message_and_block_saving() {
        let mut form =
            Form::open(registry(), &instance("folder-menu", json!({})), true).expect("known");
        assert!(!form.valid());
        assert_eq!(
            error(&form, "folder"),
            t("sensor.folderMenu.field.folder.required")
        );
        assert!(form.set("folder", json!("  ")));
        assert!(!form.valid(), "blank is empty");
        assert!(form.set("folder", json!("C:\\Users\\Public")));
        assert!(form.valid());
        assert_eq!(error(&form, "folder"), "");

        // A Shortcut needs a target or a web address.
        let mut form =
            Form::open(registry(), &instance("shortcut", json!({})), true).expect("known");
        assert!(!form.valid());
        assert_eq!(
            error(&form, "target"),
            t("sensor.shortcut.field.target.required")
        );
        assert!(form.set("url", json!("https://example.com")));
        assert!(form.valid());
        assert_eq!(error(&form, "target"), "");
        let target = json!({ "target": "C:\\Windows\\notepad.exe" });
        let form = Form::open(registry(), &instance("shortcut", target), false).expect("known");
        assert!(form.valid());

        // A Text sensor without text has nothing to show.
        let mut form = Form::open(registry(), &instance("text", json!({})), false).expect("known");
        assert!(form.valid());
        assert!(form.set("text", json!("")));
        assert!(!form.valid());
        assert_eq!(error(&form, "text"), t("sensor.text.field.text.required"));
    }

    #[test]
    fn hidden_required_fields_do_not_block_saving() {
        static SCHEMA: [PropField; 2] = [
            PropField::new("custom", FieldKind::Bool, "label", Lit::Bool(true)),
            PropField::new("path", FieldKind::Folder, "label", Lit::Str(""))
                .visible_if("custom", &[Lit::Bool(true)])
                .required("label"),
        ];
        let mut form = Form {
            id: "s_test1".into(),
            title: "Test".into(),
            new: false,
            schema: &SCHEMA,
            props: schema::resolve(&SCHEMA, &Props::new(), &Props::new()).0,
            opened: Props::new(),
            changes: Props::new(),
            invalid: BTreeSet::new(),
            processors: 1,
            volumes: None,
            adapters: Vec::new(),
            power_modes: Vec::new(),
            batteries: None,
        };
        assert!(!form.valid());
        assert!(form.set("custom", json!(false)));
        assert!(form.valid(), "the folder is hidden now");
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
            opened: Props::new(),
            changes: Props::new(),
            invalid: BTreeSet::new(),
            processors: 1,
            volumes: None,
            adapters: Vec::new(),
            power_modes: Vec::new(),
            batteries: None,
        };
        assert!(!form.set("color", json!("#12")));
        assert!(!form.valid());
        assert!(form.set("custom", json!(false)));
        assert!(form.valid(), "the colour is hidden now");
    }

    #[test]
    fn power_modes_list_what_this_computer_offers() {
        let instance = SensorInstance {
            id: "s_p".into(),
            kind: "power".into(),
            props: json!({ "mode": "hibernate" })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        };
        let mut form = Form::open(registry(), &instance, false).expect("a known type");
        form.set_power_modes(vec!["lock".into(), "sleep".into(), "shutDown".into()]);
        let rows = form.rows(&[]);
        // Hibernate is stored, so it stays listed though this computer cannot hibernate.
        assert_eq!(rows[0].options, ["Lock", "Sleep", "Hibernate", "Shut down"]);
        assert_eq!(rows[0].index, 2);
        assert!(form.choose("mode", 3, &[]));
        assert_eq!(form.changes()["mode"], json!("shutDown"));
        assert!(form.rows(&[])[2].visible, "force applies to shutting down");
    }

    #[test]
    fn cpu_series_list_this_computers_processors() {
        let stored = json!({ "bottomSeries": "core7" });
        let mut form =
            Form::open(registry(), &instance("cpu-graph-dual", stored), false).expect("known");
        form.set_processors(2);
        let rows = form.rows(&fonts());
        let top = rows.iter().find(|r| r.key == "topSeries").expect("top");
        assert_eq!(
            top.options,
            [
                "Total",
                "User tasks",
                "System",
                "Processor 1",
                "Processor 2"
            ]
        );
        assert_eq!(top.index, 3, "core0");
        let bottom = rows
            .iter()
            .find(|r| r.key == "bottomSeries")
            .expect("bottom");
        assert_eq!(
            bottom.options.last().map(String::as_str),
            Some("Processor 8 (not on this computer: shows the total)")
        );
        assert_eq!(bottom.index, 5);
        assert!(form.choose("bottomSeries", 4, &fonts()));
        assert_eq!(form.changes()["bottomSeries"], json!("core1"));
        assert!(form.choose("bottomSeries", 5, &fonts()), "still listed");
        assert_eq!(form.changes()["bottomSeries"], json!("core7"));
        // A huge processor number does not overflow.
        let huge = json!({ "topSeries": "core4294967295" });
        let form = Form::open(registry(), &instance("cpu-graph-dual", huge), false).expect("known");
        let rows = form.rows(&fonts());
        let top = rows.iter().find(|r| r.key == "topSeries").expect("top");
        assert!(top.options.last().is_some_and(|o| o.contains("4294967296")));
    }

    #[test]
    fn batteries_are_numbered_with_their_names() {
        let instance = SensorInstance {
            id: "b".into(),
            kind: "battery".into(),
            props: json!({ "battery": 2 })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        };
        let mut form = Form::open(registry(), &instance, false).expect("known type");
        assert!(form.has_battery_field() && !form.has_drive_field());
        // Until the sampler's list arrives, only the stored battery, not marked missing.
        let rows = form.rows(&fonts());
        let battery = rows.iter().find(|r| r.key == "battery").expect("battery");
        assert_eq!(
            (battery.options.as_slice(), battery.index),
            (&["3. Battery".to_owned()][..], 0)
        );
        form.set_batteries(vec!["SMP 5B10".into(), String::new()]);
        let rows = form.rows(&fonts());
        let battery = rows.iter().find(|r| r.key == "battery").expect("battery");
        assert_eq!(battery.kind, RowKind::Choice);
        assert_eq!(
            battery.options,
            ["1. SMP 5B10", "2. Battery", "3. (not connected)"]
        );
        assert_eq!(battery.index, 2);
        assert!(form.choose("battery", 0, &fonts()));
        assert_eq!(form.changes()["battery"], json!(0));
    }

    #[test]
    fn drives_list_this_computers_volumes() {
        let volume = |root: &str, kind, label: &str| Volume::new(root, kind, label);
        let stored = json!({ "drive": "F:\\" });
        let mut form =
            Form::open(registry(), &instance("disk-meter", stored), false).expect("known");
        // Until the sampler's list arrives, only the stored drive.
        let rows = form.rows(&fonts());
        let drive = rows.iter().find(|r| r.key == "drive").expect("drive");
        assert_eq!(
            (drive.options.as_slice(), drive.index),
            (&["F:\\".to_owned()][..], 0)
        );
        form.set_volumes(vec![
            volume("C:\\", DriveKind::Fixed, ""),
            volume("D:\\", DriveKind::Fixed, "Data"),
            volume("E:\\", DriveKind::Optical, ""),
        ]);
        let rows = form.rows(&fonts());
        let drive = rows.iter().find(|r| r.key == "drive").expect("drive");
        assert_eq!(drive.kind, RowKind::Choice);
        assert_eq!(
            drive.options,
            ["Local Disk (C:)", "Data (D:)", "F:\\ (not connected)"],
            "optical drives hidden"
        );
        assert_eq!(drive.index, 2);
        assert!(form.choose("drive", 1, &fonts()));
        assert_eq!(form.changes()["drive"], json!("D:\\"));
        // The stored drive stays listed, so it can be chosen again.
        let rows = form.rows(&fonts());
        let drive = rows.iter().find(|r| r.key == "drive").expect("drive");
        assert_eq!((drive.options.len(), drive.index), (3, 1));
        assert!(form.choose("drive", 2, &fonts()));
        assert_eq!(form.changes()["drive"], json!("F:\\"));
        // A stored optical drive that is there is named by its kind.
        let stored = json!({ "drive": "E:\\" });
        let mut form =
            Form::open(registry(), &instance("disk-meter", stored), false).expect("known");
        form.set_volumes(vec![volume("E:\\", DriveKind::Optical, "")]);
        let rows = form.rows(&fonts());
        let drive = rows.iter().find(|r| r.key == "drive").expect("drive");
        assert_eq!(drive.options, ["CD Drive (E:)"]);
    }

    #[test]
    fn adapters_list_automatic_and_this_computers_connections() {
        let stored = json!({ "adapter": { "id": "GONE", "name": "Ethernet 2" } });
        let mut form =
            Form::open(registry(), &instance("network-graph", stored), false).expect("known");
        assert!(form.has_adapter_field() && !form.has_drive_field());
        let wifi = NetInterface {
            id: "WIFI".into(),
            alias: "Wi-Fi".into(),
            description: "Intel(R) Wi-Fi 6E AX211".into(),
            ..NetInterface::default()
        };
        form.set_adapters(vec![wifi]);
        let rows = form.rows(&fonts());
        let adapter = rows.iter().find(|r| r.key == "adapter").expect("adapter");
        assert_eq!(adapter.kind, RowKind::Choice);
        assert_eq!(
            adapter.options,
            [
                "Automatic",
                "Wi-Fi (Intel(R) Wi-Fi 6E AX211)",
                "Ethernet 2 (not connected)"
            ]
        );
        assert_eq!(adapter.index, 2);
        assert!(form.choose("adapter", 1, &fonts()));
        assert_eq!(
            form.changes()["adapter"],
            json!({ "id": "WIFI", "name": "Wi-Fi" })
        );
        assert!(form.choose("adapter", 0, &fonts()));
        assert_eq!(form.changes()["adapter"], Value::Null, "Automatic");
        assert!(
            form.choose("adapter", 2, &fonts()),
            "the stored one, still listed"
        );
        assert_eq!(form.changes()["adapter"]["id"], json!("GONE"));

        // A connection renamed since it was stored is still the same one.
        let stored = json!({ "adapter": { "id": "WIFI", "name": "Old name" } });
        let mut form =
            Form::open(registry(), &instance("network-graph", stored), false).expect("known");
        form.set_adapters(vec![NetInterface {
            id: "WIFI".into(),
            alias: "Wi-Fi".into(),
            ..NetInterface::default()
        }]);
        let rows = form.rows(&fonts());
        let adapter = rows.iter().find(|r| r.key == "adapter").expect("adapter");
        assert_eq!((adapter.options.len(), adapter.index), (2, 1));
    }

    #[test]
    fn unknown_types_have_no_form() {
        assert!(Form::open(registry(), &instance("mail", json!({})), false).is_none());
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
