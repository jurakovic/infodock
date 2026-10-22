//! The legacy XML import on the sensor side (SPEC §11.4): each type turns a legacy sensor's
//! properties into its props (`SensorType::convert_legacy`), mostly by naming its XML keys
//! ([`LegacyKey`]), whose values are read by the encoding of the field they set
//! (`config/legacy.rs`); [`incoming`] turns a parsed file into what the import adds.

use serde_json::{Value, json};

use super::registry::Registry;
use super::schema::{self, FieldKind, Lit, PropField};
use super::{DefaultsCtx, Props, State};
use crate::config::exchange::{Incoming, IncomingSensor, SkipReason, Skipped, Warning};
use crate::config::legacy::{self, LegacyProps, LegacySensor};
use crate::config::model::LegacyUnsupported;

/// Legacy types without an Infodock type yet: kept in `legacy.unsupported` for later versions
/// (feeds, mail, hardware monitors, radio) and reported as "not supported yet".
pub const UNSUPPORTED: &[&str] = &[
    "DockRSSSensor",
    "DockStandardMail",
    "DockGMail",
    "DockGMailForDomains",
    "DockYMail",
    "DockSpeedFan",
    "DockRadio",
];

/// An XML key of a legacy type and the prop it sets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LegacyKey {
    pub xml: &'static str,
    /// The prop's key; empty for [`LegacyValue::Ignored`].
    pub prop: &'static str,
    pub value: LegacyValue,
}

/// How a legacy value becomes the prop's value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LegacyValue {
    /// By the prop's field type: bool, colour mode, opacity, interval, slider, text, file and
    /// folder paths, font, font style, drive, colour, a select whose XML values are its own, a
    /// power mode by its id, a hidden field by its default's kind.
    Field,
    /// A select some of whose options the XML names by other values (ignoring case); other
    /// values are read as [`LegacyValue::Field`].
    Options(&'static [(&'static str, Lit)]),
    /// A graph's width in px.
    GraphWidth,
    /// A property Infodock has no use for; read without a warning.
    Ignored,
    /// A property Infodock has no counterpart for; not imported, with a warning unless it is
    /// empty.
    Dropped,
}

impl LegacyKey {
    /// `xml` sets `prop` by the field's encoding.
    pub const fn new(xml: &'static str, prop: &'static str) -> Self {
        Self {
            xml,
            prop,
            value: LegacyValue::Field,
        }
    }

    /// `xml` chooses an option of the select `prop` by these XML values.
    pub const fn options(
        xml: &'static str,
        prop: &'static str,
        options: &'static [(&'static str, Lit)],
    ) -> Self {
        Self {
            xml,
            prop,
            value: LegacyValue::Options(options),
        }
    }

    /// `xml` is a graph width for `prop`.
    pub const fn graph_width(xml: &'static str, prop: &'static str) -> Self {
        Self {
            xml,
            prop,
            value: LegacyValue::GraphWidth,
        }
    }

    /// `xml` is known and not imported.
    pub const fn ignored(xml: &'static str) -> Self {
        Self {
            xml,
            prop: "",
            value: LegacyValue::Ignored,
        }
    }

    /// `xml` is known and not imported, which the report says when it has a value.
    pub const fn dropped(xml: &'static str) -> Self {
        Self {
            xml,
            prop: "",
            value: LegacyValue::Dropped,
        }
    }
}

/// What converting one legacy sensor may use, and what it reports.
pub struct LegacyCtx<'a> {
    /// The sensor's legacy type, e.g. `DockText`.
    pub legacy_type: &'a str,
    pub defaults: &'a DefaultsCtx<'a>,
    pub warnings: Vec<Warning>,
    /// Runtime state for the new sensor, e.g. the uptime record.
    pub state: State,
}

impl LegacyCtx<'_> {
    /// Warns that property `key` has a `value` that cannot be read.
    pub fn invalid(&mut self, key: &str, value: &str) {
        self.warnings.push(Warning::InvalidProperty {
            legacy: self.legacy_type.to_owned(),
            key: key.to_owned(),
            value: value.to_owned(),
        });
    }

    /// Warns that property `key` is not known.
    pub fn unknown(&mut self, key: &str) {
        self.warnings.push(Warning::UnknownProperty {
            legacy: self.legacy_type.to_owned(),
            key: key.to_owned(),
        });
    }

    /// Warns that property `key` is not imported.
    pub fn dropped(&mut self, key: &str) {
        self.warnings.push(Warning::DroppedProperty {
            legacy: self.legacy_type.to_owned(),
            key: key.to_owned(),
        });
    }
}

/// The props `raw` sets through `keys`, each value read by the encoding of its field in
/// `schema`; a property `keys` does not name and a value that cannot be read are reported. Props
/// are not yet in their stored form: the import checks them against the type (§11.2).
pub fn convert(
    schema: &[PropField],
    keys: &[LegacyKey],
    raw: &LegacyProps,
    cx: &mut LegacyCtx,
) -> Props {
    let mut props = Props::new();
    for (xml, value) in raw {
        let Some(key) = keys.iter().find(|k| k.xml == xml) else {
            cx.unknown(xml);
            continue;
        };
        match key.value {
            LegacyValue::Ignored => continue,
            LegacyValue::Dropped => {
                if !value.trim().is_empty() {
                    cx.dropped(xml);
                }
                continue;
            }
            _ => {}
        }
        let Some(field) = schema::field(schema, key.prop) else {
            tracing::error!("{}: {xml} names no field {}", cx.legacy_type, key.prop);
            continue;
        };
        match read_value(field, key.value, value) {
            Some(v) => {
                props.insert(key.prop.to_owned(), v);
            }
            None => cx.invalid(xml, value),
        }
    }
    props
}

/// `value` as a valid value of `field`.
fn read_value(field: &PropField, how: LegacyValue, value: &str) -> Option<Value> {
    let read = match how {
        LegacyValue::Options(options) => options
            .iter()
            .find(|(xml, _)| xml.eq_ignore_ascii_case(value.trim()))
            .map(|(_, lit)| lit.to_value())
            .or_else(|| by_field(field, value)),
        LegacyValue::GraphWidth => legacy::graph_width(value).map(Value::from),
        LegacyValue::Field => by_field(field, value),
        LegacyValue::Ignored | LegacyValue::Dropped => None,
    };
    read.and_then(|v| schema::check(field, &v))
}

fn by_field(field: &PropField, value: &str) -> Option<Value> {
    match field.kind {
        FieldKind::Bool => legacy::bool(value).map(Value::Bool),
        FieldKind::ColorMode { .. } => {
            let (mode, color) = legacy::color_mode(value)?;
            // Standard keeps the field's default colour (SPEC §11.4).
            let color = match (color, field.default) {
                (Some(color), _) => color,
                (None, Lit::ColorMode(_, default)) => default.to_owned(),
                (None, _) => "#FFFFFF".to_owned(),
            };
            Some(json!({ "mode": mode, "color": color }))
        }
        FieldKind::Opacity { .. } => legacy::opacity(value).map(Value::from),
        FieldKind::Interval { .. } | FieldKind::Slider { .. } => {
            legacy::number(value).map(Value::from)
        }
        FieldKind::Battery => legacy::number(value)
            .filter(|n| *n >= 0.0 && n.fract() == 0.0 && *n <= f64::from(u32::MAX))
            .map(|n| Value::from(n as u64)),
        FieldKind::Text { .. } => Some(Value::from(value)),
        FieldKind::File { .. } | FieldKind::Folder => Some(Value::from(legacy::path(value))),
        FieldKind::Font => Some(Value::from(value.trim())),
        FieldKind::FontStyle => legacy::font_style(value).map(Value::from),
        FieldKind::Drive { .. } => legacy::drive(value).map(Value::from),
        FieldKind::Color => Some(Value::from(value.trim())),
        FieldKind::Select { options } => options
            .iter()
            .map(|o| o.value)
            .find(|lit| match lit {
                Lit::Str(s) => s.eq_ignore_ascii_case(value.trim()),
                Lit::Int(i) => legacy::number(value) == Some(*i as f64),
                _ => false,
            })
            .map(Lit::to_value),
        FieldKind::CpuSeries => Some(Value::from(value.trim())),
        // Ids ignoring case, as selects (`Shutdown` is `shutDown`).
        FieldKind::PowerMode => {
            let value = value.trim();
            let id = crate::platform::PowerAction::ALL
                .into_iter()
                .map(crate::platform::PowerAction::id)
                .find(|id| id.eq_ignore_ascii_case(value));
            Some(Value::from(id.unwrap_or(value)))
        }
        FieldKind::Hidden => match field.default {
            Lit::Int(_) => legacy::number(value).map(|n| Value::from(n.round() as i64)),
            Lit::Bool(_) => legacy::bool(value).map(Value::Bool),
            Lit::Str(_) => Some(Value::from(value)),
            _ => None,
        },
        FieldKind::NetAdapter => None,
    }
}

/// What a parsed legacy file brings (SPEC §11.4): each sensor of a known type converted by
/// its type, unsupported types kept for later (any property whose name contains `password`
/// dropped), unknown types and sensors without a side skipped.
pub fn incoming(registry: &Registry, ctx: &DefaultsCtx, sensors: Vec<LegacySensor>) -> Incoming {
    let mut incoming = Incoming::default();
    for LegacySensor { zone, kind, props } in sensors {
        let skip = |reason| Skipped {
            kind: kind.clone(),
            reason,
        };
        let Some(zone) = zone else {
            incoming.skipped.push(skip(SkipReason::NoSide));
            continue;
        };
        if UNSUPPORTED.contains(&kind.as_str()) {
            incoming.skipped.push(skip(SkipReason::NotSupportedYet));
            let props = props
                .into_iter()
                .filter(|(key, _)| !key.to_ascii_lowercase().contains("password"))
                .collect();
            incoming
                .unsupported
                .push(LegacyUnsupported { kind, zone, props });
            continue;
        }
        let Some(ty) = registry.by_legacy_type(&kind) else {
            incoming.skipped.push(skip(SkipReason::UnknownType));
            continue;
        };
        let mut cx = LegacyCtx {
            legacy_type: &kind,
            defaults: ctx,
            warnings: Vec::new(),
            state: State::new(),
        };
        let props = ty.convert_legacy(&props, &mut cx);
        let LegacyCtx {
            warnings, state, ..
        } = cx;
        incoming.warnings.extend(warnings);
        incoming.sensors.push(IncomingSensor {
            zone,
            kind: ty.id().to_owned(),
            props,
            state,
        });
    }
    incoming
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::exchange::{ImportMode, ImportReport};
    use crate::config::model::{Config, Zone};
    use crate::sensors::registry::registry;
    use crate::sensors::schema::SelectOption;
    use crate::test_util::NO_PLATFORM;

    static CTX: DefaultsCtx = DefaultsCtx {
        platform: &NO_PLATFORM,
    };

    const MODES: &[SelectOption] = &[
        SelectOption {
            value: Lit::Str("none"),
            label: "x",
        },
        SelectOption {
            value: Lit::Str("long"),
            label: "x",
        },
    ];

    const CHIMES: &[SelectOption] = &[
        SelectOption {
            value: Lit::Int(0),
            label: "x",
        },
        SelectOption {
            value: Lit::Int(30),
            label: "x",
        },
    ];

    const SCHEMA: &[PropField] = &[
        PropField::new("on", FieldKind::Bool, "x", Lit::Bool(false)),
        PropField::new(
            "color",
            FieldKind::ColorMode { dynamic: true },
            "x",
            Lit::ColorMode("standard", "#FFFFFF"),
        ),
        PropField::new(
            "plain",
            FieldKind::ColorMode { dynamic: false },
            "x",
            Lit::ColorMode("standard", "#FFFFFF"),
        ),
        PropField::new(
            "opacity",
            FieldKind::Opacity { min: 20 },
            "x",
            Lit::Int(100),
        ),
        PropField::new(
            "interval",
            FieldKind::Interval {
                min: 1.0,
                max: 10.0,
            },
            "x",
            Lit::Int(1),
        ),
        PropField::new(
            "width",
            FieldKind::Slider {
                min: 50.0,
                max: 150.0,
                step: 5.0,
                unit: None,
            },
            "x",
            Lit::Int(70),
        ),
        PropField::new(
            "image",
            FieldKind::File { extensions: &[] },
            "x",
            Lit::Str(""),
        ),
        PropField::new("style", FieldKind::FontStyle, "x", Lit::Str("normal")),
        PropField::new(
            "drive",
            FieldKind::Drive {
                hide_optical: false,
            },
            "x",
            Lit::Str("C:\\"),
        ),
        PropField::new(
            "mode",
            FieldKind::Select { options: MODES },
            "x",
            Lit::Str("long"),
        ),
        PropField::new(
            "chime",
            FieldKind::Select { options: CHIMES },
            "x",
            Lit::Int(0),
        ),
        PropField::new("size", FieldKind::Hidden, "x", Lit::Int(240)),
        PropField::new("battery", FieldKind::Battery, "x", Lit::Int(0)),
        PropField::new("power", FieldKind::PowerMode, "x", Lit::Str("sleep")),
    ];

    const KEYS: &[LegacyKey] = &[
        LegacyKey::new("on", "on"),
        LegacyKey::new("colorize", "color"),
        LegacyKey::new("plain", "plain"),
        LegacyKey::new("opacity", "opacity"),
        LegacyKey::new("interval", "interval"),
        LegacyKey::graph_width("width", "width"),
        LegacyKey::new("image", "image"),
        LegacyKey::new("fontstyle", "style"),
        LegacyKey::new("disk", "drive"),
        LegacyKey::options(
            "dayname",
            "mode",
            &[("0", Lit::Str("none")), ("2", Lit::Str("long"))],
        ),
        LegacyKey::new("chime", "chime"),
        LegacyKey::options("alert", "chime", &[("-1", Lit::Int(0))]),
        LegacyKey::new("popup-width", "size"),
        LegacyKey::new("battery", "battery"),
        LegacyKey::options("mode", "power", &[("0", Lit::Str("lock"))]),
        LegacyKey::ignored("legacy-only"),
        LegacyKey::dropped("path"),
    ];

    fn raw(pairs: &[(&str, &str)]) -> LegacyProps {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn convert_raw(pairs: &[(&str, &str)]) -> (Value, Vec<String>) {
        let mut cx = LegacyCtx {
            legacy_type: "DockTest",
            defaults: &CTX,
            warnings: Vec::new(),
            state: State::new(),
        };
        let props = convert(SCHEMA, KEYS, &raw(pairs), &mut cx);
        let warnings = cx.warnings.iter().map(Warning::message).collect();
        (Value::Object(props), warnings)
    }

    #[test]
    fn values_are_read_by_their_fields_encoding() {
        let (props, warnings) = convert_raw(&[
            ("on", "yes"),
            ("colorize", "dyn:#00ff00"),
            ("plain", "off:#123456"),
            ("opacity", "196"),
            ("interval", "2.5"),
            ("width", "73"),
            ("image", "D:/Icons/a.ico"),
            ("fontstyle", "bold, italic"),
            ("disk", "d:"),
            ("dayname", "0"),
            ("chime", "30"),
            ("legacy-only", "whatever"),
        ]);
        assert_eq!(
            props,
            json!({
                "on": true,
                "color": { "mode": "dynamic", "color": "#00FF00" },
                "plain": { "mode": "standard", "color": "#FFFFFF" },
                "opacity": 77,
                "interval": 3,
                "width": 70,
                "image": "D:\\Icons\\a.ico",
                "style": "boldItalic",
                "drive": "D:\\",
                "mode": "none",
                "chime": 30,
            })
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn select_tables_fall_back_to_the_fields_values() {
        let read = |pairs: &[(&str, &str)]| convert_raw(pairs);
        assert_eq!(read(&[("alert", "-1")]), (json!({ "chime": 0 }), vec![]));
        assert_eq!(read(&[("alert", "30")]), (json!({ "chime": 30 }), vec![]));
        assert_eq!(
            read(&[("dayname", "LONG")]),
            (json!({ "mode": "long" }), vec![])
        );
        assert_eq!(read(&[("mode", "0")]), (json!({ "power": "lock" }), vec![]));
        assert_eq!(
            read(&[("mode", "restart")]),
            (json!({ "power": "restart" }), vec![])
        );
        // Power modes ignoring case, as selects (review M4).
        assert_eq!(
            read(&[("mode", "LOCK")]),
            (json!({ "power": "lock" }), vec![])
        );
        let (props, warnings) = read(&[("alert", "45"), ("mode", "9")]);
        assert_eq!(props, json!({}));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn hidden_fields_are_read_by_their_defaults_kind() {
        let (props, warnings) = convert_raw(&[("popup-width", "300.4")]);
        assert_eq!(props, json!({ "size": 300 }));
        assert!(warnings.is_empty());
        let (props, warnings) = convert_raw(&[("popup-width", "wide")]);
        assert_eq!(props, json!({}));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn battery_indices_are_whole_numbers() {
        assert_eq!(
            convert_raw(&[("battery", "1")]),
            (json!({ "battery": 1 }), vec![])
        );
        for bad in ["1.5", "-1"] {
            let (props, warnings) = convert_raw(&[("battery", bad)]);
            assert_eq!((props, warnings.len()), (json!({}), 1), "{bad}");
        }
    }

    #[test]
    fn dropped_properties_are_reported_when_they_have_a_value() {
        let (props, warnings) = convert_raw(&[("path", "C:/x.exe")]);
        assert_eq!(props, json!({}));
        assert_eq!(
            warnings,
            ["DockTest: Infodock has no use for the property “path”, so it was not imported."]
        );
        assert_eq!(convert_raw(&[("path", " ")]), (json!({}), vec![]));
    }

    #[test]
    fn values_are_clamped_to_the_fields_range() {
        let (props, warnings) = convert_raw(&[("opacity", "18"), ("interval", "60")]);
        assert_eq!(props, json!({ "opacity": 20, "interval": 10 }));
        assert!(warnings.is_empty());
    }

    #[test]
    fn unreadable_values_and_unknown_properties_are_reported() {
        let (props, warnings) = convert_raw(&[
            ("on", "maybe"),
            ("plain", "dyn:#00FF00"),
            ("dayname", "7"),
            ("chime", "45"),
            ("disk", "C:\\Windows"),
            ("blink", "yes"),
        ]);
        assert_eq!(props, json!({}));
        let invalid = |value: &str, key: &str| {
            format!(
                "DockTest: “{value}” is not a valid value of “{key}”, so its default is used \
                 instead."
            )
        };
        assert_eq!(
            warnings,
            [
                "DockTest: the property “blink” is not known, so it was ignored.".to_owned(),
                invalid("45", "chime"),
                invalid("7", "dayname"),
                invalid("C:\\Windows", "disk"),
                invalid("maybe", "on"),
                invalid("dyn:#00FF00", "plain"),
            ]
        );
    }

    fn sensor(zone: Option<Zone>, kind: &str, pairs: &[(&str, &str)]) -> LegacySensor {
        LegacySensor {
            zone,
            kind: kind.into(),
            props: raw(pairs),
        }
    }

    /// The import of `sensors` into an empty config with Replace.
    fn imported(sensors: Vec<LegacySensor>) -> (Config, ImportReport) {
        let mut config = Config::default();
        let incoming = incoming(registry(), &CTX, sensors);
        let prepared = crate::sensors::import::prepare(registry(), &CTX, incoming);
        let report = crate::sensors::import::import(&mut config, prepared, ImportMode::Replace);
        (config, report)
    }

    #[test]
    fn unsupported_types_are_kept_without_passwords() {
        let (config, report) = imported(vec![sensor(
            Some(Zone::Center),
            "DockGMail",
            &[
                ("user", "me"),
                ("password", "secret"),
                ("smtp-Password", "x"),
            ],
        )]);
        assert!(config.layout.center.is_empty());
        assert_eq!(
            config.legacy.unsupported,
            [LegacyUnsupported {
                kind: "DockGMail".into(),
                zone: Zone::Center,
                props: [("user".to_owned(), "me".to_owned())].into(),
            }]
        );
        assert_eq!(report.added, 0);
        assert_eq!(
            report.skipped,
            [Skipped {
                kind: "DockGMail".into(),
                reason: SkipReason::NotSupportedYet
            }]
        );
    }

    #[test]
    fn unknown_types_and_sides_are_skipped() {
        let (config, report) = imported(vec![
            sensor(Some(Zone::Left), "DockWeather", &[("city", "Zagreb")]),
            sensor(None, "DockText", &[]),
        ]);
        assert!(config.layout.all().next().is_none());
        assert!(config.legacy.unsupported.is_empty());
        let reasons: Vec<(&str, SkipReason)> = report
            .skipped
            .iter()
            .map(|s| (s.kind.as_str(), s.reason))
            .collect();
        assert_eq!(
            reasons,
            [
                ("DockWeather", SkipReason::UnknownType),
                ("DockText", SkipReason::NoSide)
            ]
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn every_legacy_type_belongs_to_one_type() {
        let mut seen = std::collections::HashSet::new();
        for ty in registry().types() {
            for legacy in ty.legacy_types() {
                assert!(seen.insert(*legacy), "{legacy} twice");
                assert!(!UNSUPPORTED.contains(legacy), "{legacy}");
                assert_eq!(
                    registry().by_legacy_type(legacy).map(|t| t.id()),
                    Some(ty.id())
                );
            }
        }
    }

    #[test]
    fn every_type_with_legacy_types_names_its_keys() {
        for ty in registry().types() {
            assert_eq!(
                ty.legacy_types().is_empty(),
                ty.legacy_keys().is_empty(),
                "{}",
                ty.id()
            );
        }
    }

    #[test]
    fn every_legacy_key_names_a_field() {
        for ty in registry().types() {
            for key in ty.legacy_keys() {
                if !matches!(key.value, LegacyValue::Ignored | LegacyValue::Dropped) {
                    assert!(
                        schema::field(ty.schema(), key.prop).is_some(),
                        "{}: {} → {}",
                        ty.id(),
                        key.xml,
                        key.prop
                    );
                }
            }
        }
    }
}
