//! Layout exchange files (SPEC §11.3): the export of the whole config or of its layout, and what
//! an imported file brings before each sensor is checked against its type
//! (`sensors::import`). The legacy XML import (§11.4) brings the same [`Incoming`].

use std::path::Path;

use serde_json::{Map, Value};

use super::migrate::migrate;
use super::model::{Config, LegacyUnsupported, SCHEMA_VERSION, Zone};
use crate::i18n::{t, t_args};

/// The extension of exchange files, after the file's name.
pub const EXTENSION: &str = "infodock.json";

/// Exchange files larger than this are refused; a layout takes a few kB.
const MAX_FILE: u64 = 16 * 1024 * 1024;

const ZONES: [(Zone, &str); 3] = [
    (Zone::Left, "left"),
    (Zone::Center, "center"),
    (Zone::Right, "right"),
];

/// What kind of file an import reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    /// An exchange file, `.infodock.json`.
    Json,
    /// A legacy XML layout (SPEC §11.4).
    Legacy,
    /// Either, told apart by the file's name: a `.json` file is an exchange file, any other a
    /// legacy layout (the welcome page, §11.5).
    Any,
}

impl ImportFormat {
    /// The kind of file at `path`: `Any` decided by its name.
    pub fn of(self, path: &std::path::Path) -> Self {
        match self {
            Self::Any => {
                let json = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"));
                if json { Self::Json } else { Self::Legacy }
            }
            format => format,
        }
    }
}

/// What an export holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportScope {
    /// The whole config.
    Config,
    /// `{ schema, layout, state }`, with `legacy` while it holds unsupported legacy sensors.
    Layout,
}

/// What an import does with the current layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// The imported sensors take the place of the layout, its state and its unsupported legacy
    /// sensors.
    Replace,
    /// The imported sensors are appended to their zones.
    Merge,
}

/// A sensor an import brings, before its type checks it.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingSensor {
    pub zone: Zone,
    pub kind: String,
    pub props: Map<String, Value>,
    /// Its runtime state, e.g. the uptime record.
    pub state: Map<String, Value>,
}

/// What an imported file brings, in zone and document order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Incoming {
    pub sensors: Vec<IncomingSensor>,
    /// Legacy sensors without an Infodock type yet (SPEC §11.4).
    pub unsupported: Vec<LegacyUnsupported>,
    pub skipped: Vec<Skipped>,
    pub warnings: Vec<Warning>,
}

/// The outcome of an import (SPEC §11.3).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportReport {
    pub added: usize,
    pub skipped: Vec<Skipped>,
    pub warnings: Vec<Warning>,
}

/// An entry of the file that was not added to the layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Skipped {
    /// The entry's type as the file names it; empty if it names none.
    pub kind: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Not an object with a `type`.
    NotASensor,
    /// A legacy type kept in `legacy.unsupported` for later versions (SPEC §11.4).
    NotSupportedYet,
    /// A legacy type Infodock does not know.
    UnknownType,
    /// A legacy sensor without a side, or with one other than left, center or right.
    NoSide,
}

impl SkipReason {
    pub fn message(self) -> String {
        t(match self {
            SkipReason::NotASensor => "import.skipped.notASensor",
            SkipReason::NotSupportedYet => "import.skipped.notSupportedYet",
            SkipReason::UnknownType => "import.skipped.unknownType",
            SkipReason::NoSide => "import.skipped.noSide",
        })
    }
}

/// Something the user should know about an added sensor.
#[derive(Debug, Clone, PartialEq)]
pub enum Warning {
    /// A value its type cannot use; the default is used instead. `sensor` names the sensor as
    /// the report shows it: its type's name.
    InvalidValue { sensor: String, key: String },
    /// A type this version does not know; kept and shown as a placeholder (SPEC §11.2).
    UnknownType { kind: String },
    /// A legacy property its type does not name; ignored (SPEC §11.4).
    UnknownProperty { legacy: String, key: String },
    /// A legacy property whose value cannot be read; the default is used instead.
    InvalidProperty {
        legacy: String,
        key: String,
        value: String,
    },
    /// A legacy property Infodock has no counterpart for; not imported (SPEC §11.4).
    DroppedProperty { legacy: String, key: String },
    /// A legacy network adapter this computer does not have; Automatic is used instead
    /// (SPEC §9.13).
    UnknownAdapter { legacy: String, adapter: String },
}

impl Warning {
    pub fn message(&self) -> String {
        match self {
            Warning::InvalidValue { sensor, key } => t_args(
                "import.warning.invalidValue",
                &[("sensor", sensor), ("key", key)],
            ),
            Warning::UnknownType { kind } => {
                t_args("import.warning.unknownType", &[("type", kind)])
            }
            Warning::UnknownProperty { legacy, key } => t_args(
                "import.warning.unknownProperty",
                &[("type", legacy), ("key", key)],
            ),
            Warning::InvalidProperty { legacy, key, value } => t_args(
                "import.warning.invalidProperty",
                &[("type", legacy), ("key", key), ("value", value)],
            ),
            Warning::DroppedProperty { legacy, key } => t_args(
                "import.warning.droppedProperty",
                &[("type", legacy), ("key", key)],
            ),
            Warning::UnknownAdapter { legacy, adapter } => t_args(
                "import.warning.unknownAdapter",
                &[("type", legacy), ("adapter", adapter)],
            ),
        }
    }
}

/// Why a file cannot be imported.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the file is too large")]
    TooLarge,
    #[error("not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no layout in the file")]
    NotALayout,
    #[error("written by a newer Infodock (schema {0})")]
    Newer(u64),
    #[error("no <sensors> element")]
    NoLegacyLayout,
    #[error("XML: {0}")]
    Xml(String),
}

impl ImportError {
    /// The sentence Settings or a dialog shows.
    pub fn message(&self) -> String {
        match self {
            ImportError::Io(e) => t_args("import.error.io", &[("error", &e.to_string())]),
            ImportError::TooLarge => t("import.error.tooLarge"),
            ImportError::Json(_) | ImportError::NotALayout => t("import.error.notALayout"),
            ImportError::Newer(_) => t("import.error.newer"),
            ImportError::NoLegacyLayout => t("import.error.noLegacyLayout"),
            ImportError::Xml(e) => t_args("import.error.xml", &[("error", e)]),
        }
    }
}

/// The text of an exchange file: the whole config, or its layout with the state of its sensors.
pub fn export(config: &Config, scope: ExportScope) -> serde_json::Result<String> {
    let value = match scope {
        ExportScope::Config => serde_json::to_value(config)?,
        ExportScope::Layout => {
            let state: Map<String, Value> = config
                .state
                .iter()
                .filter(|(id, _)| config.layout.find(id).is_some())
                .map(|(id, state)| (id.clone(), Value::Object(state.clone())))
                .collect();
            let mut file = Map::new();
            file.insert("schema".into(), Value::from(SCHEMA_VERSION));
            file.insert("layout".into(), serde_json::to_value(&config.layout)?);
            file.insert("state".into(), Value::Object(state));
            if !config.legacy.unsupported.is_empty() {
                file.insert("legacy".into(), serde_json::to_value(&config.legacy)?);
            }
            Value::Object(file)
        }
    };
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    Ok(text)
}

/// Reads an exchange file; see [`parse`].
pub fn read(path: &Path) -> Result<Incoming, ImportError> {
    parse(&String::from_utf8_lossy(&read_bytes(path)?))
}

/// The bytes of a file to import, refusing large ones.
pub fn read_bytes(path: &Path) -> Result<Vec<u8>, ImportError> {
    if std::fs::metadata(path)?.len() > MAX_FILE {
        return Err(ImportError::TooLarge);
    }
    Ok(std::fs::read(path)?)
}

/// What an exchange file brings: a whole config or a layout export, migrated to this schema.
/// Only its layout, the state of its sensors and its unsupported legacy sensors are imported;
/// settings are not (SPEC §11.3). Entries that are not sensors are skipped.
pub fn parse(text: &str) -> Result<Incoming, ImportError> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
    if !value.is_object() {
        return Err(ImportError::NotALayout);
    }
    let migrated = migrate(value);
    if migrated.newer {
        let schema = migrated.value.get("schema").and_then(Value::as_u64);
        return Err(ImportError::Newer(schema.unwrap_or_default()));
    }
    let file = migrated.value;
    let layout = file
        .get("layout")
        .and_then(Value::as_object)
        .ok_or(ImportError::NotALayout)?;
    let state = file.get("state").and_then(Value::as_object);

    let mut incoming = Incoming::default();
    for (zone, key) in ZONES {
        let entries = layout.get(key).and_then(Value::as_array);
        for entry in entries.into_iter().flatten() {
            match sensor_entry(entry) {
                Some((id, kind, props)) => incoming.sensors.push(IncomingSensor {
                    zone,
                    kind,
                    props,
                    state: id
                        .and_then(|id| state?.get(id)?.as_object().cloned())
                        .unwrap_or_default(),
                }),
                None => incoming.skipped.push(Skipped {
                    kind: entry
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    reason: SkipReason::NotASensor,
                }),
            }
        }
    }
    let unsupported = file
        .pointer("/legacy/unsupported")
        .and_then(Value::as_array);
    incoming.unsupported = unsupported
        .into_iter()
        .flatten()
        .filter_map(|e| serde_json::from_value(e.clone()).ok())
        .collect();
    Ok(incoming)
}

/// The id, type and props of a layout entry; `None` unless it is an object with a non-empty
/// `type` and, if any, object `props`.
fn sensor_entry(entry: &Value) -> Option<(Option<&str>, String, Map<String, Value>)> {
    let entry = entry.as_object()?;
    let kind = entry.get("type")?.as_str().filter(|k| !k.is_empty())?;
    let props = match entry.get("props") {
        None | Some(Value::Null) => Map::new(),
        Some(props) => props.as_object()?.clone(),
    };
    let id = entry.get("id").and_then(Value::as_str);
    Some((id, kind.to_owned(), props))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::model::Edge;

    fn object(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    fn config() -> Config {
        let mut c = Config::first_run("1.0.0");
        let date = c.layout.right[0].id.clone();
        c.state.insert(date, object(json!({ "seen": 1 })));
        c.state
            .insert("s_gone0".into(), object(json!({ "old": true })));
        c.settings.bar.edge = Edge::Top;
        c
    }

    #[test]
    fn exports_the_whole_config() {
        let c = config();
        let text = export(&c, ExportScope::Config).expect("exported");
        let back: Config = serde_json::from_str(&text).expect("a config");
        assert_eq!(back, c);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn exports_the_layout_with_its_state() {
        let mut c = config();
        let text = export(&c, ExportScope::Layout).expect("exported");
        let v: Value = serde_json::from_str(&text).expect("JSON");
        let mut keys: Vec<&str> = v
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        keys.sort_unstable();
        assert_eq!(keys, ["layout", "schema", "state"]);
        assert_eq!(v["schema"], SCHEMA_VERSION);
        assert_eq!(
            v["layout"],
            serde_json::to_value(&c.layout).expect("layout")
        );
        // Only the state of sensors in the layout.
        let date = &c.layout.right[0].id;
        assert_eq!(v["state"], json!({ date.as_str(): { "seen": 1 } }));

        c.legacy.unsupported.push(LegacyUnsupported {
            kind: "DockRSSSensor".into(),
            zone: Zone::Center,
            props: [("url".to_owned(), "https://example.com".to_owned())].into(),
        });
        let v: Value = serde_json::from_str(&export(&c, ExportScope::Layout).expect("exported"))
            .expect("JSON");
        assert_eq!(v["legacy"]["unsupported"][0]["type"], "DockRSSSensor");
    }

    #[test]
    fn an_export_imports_as_it_was() {
        let mut c = config();
        c.legacy.unsupported.push(LegacyUnsupported {
            kind: "DockGMail".into(),
            zone: Zone::Left,
            props: Default::default(),
        });
        for scope in [ExportScope::Config, ExportScope::Layout] {
            let incoming = parse(&export(&c, scope).expect("exported")).expect("parsed");
            let kinds: Vec<(Zone, &str)> = incoming
                .sensors
                .iter()
                .map(|s| (s.zone, s.kind.as_str()))
                .collect();
            assert_eq!(
                kinds,
                [
                    (Zone::Left, "cpu-graph"),
                    (Zone::Left, "memory-meter"),
                    (Zone::Left, "network-graph"),
                    (Zone::Center, "image"),
                    (Zone::Right, "date"),
                    (Zone::Right, "time"),
                ]
            );
            assert_eq!(incoming.sensors[3].props["popupText"], "Infodock %v");
            assert_eq!(incoming.sensors[4].state, object(json!({ "seen": 1 })));
            assert!(incoming.sensors[5].state.is_empty());
            assert_eq!(incoming.unsupported, c.legacy.unsupported);
            assert!(incoming.skipped.is_empty() && incoming.warnings.is_empty());
        }
    }

    #[test]
    fn skips_entries_that_are_not_sensors() {
        let incoming = parse(
            r#"{ "layout": {
                "left": [ 3, { "props": {} }, { "type": "" }, { "type": "text", "props": [1] },
                          { "type": "spacer" } ],
                "center": "nope",
                "right": [ { "id": 7, "type": "text", "props": null } ]
            }, "state": { "7": { "x": 1 } } }"#,
        )
        .expect("parsed");
        let kinds: Vec<&str> = incoming.sensors.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(kinds, ["spacer", "text"]);
        assert!(incoming.sensors[1].state.is_empty());
        let skipped: Vec<&str> = incoming.skipped.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(skipped, ["", "", "", "text"]);
        assert!(
            incoming
                .skipped
                .iter()
                .all(|s| s.reason == SkipReason::NotASensor)
        );
    }

    #[test]
    fn refuses_what_is_not_a_layout() {
        for text in ["", "[]", "{ \"layout\": 1 }", "{ \"settings\": {} }", "{"] {
            let e = parse(text).expect_err(text);
            assert!(
                matches!(e, ImportError::Json(_) | ImportError::NotALayout),
                "{text}: {e}"
            );
            assert_eq!(
                e.message(),
                "The file does not hold an Infodock layout.",
                "{text}"
            );
        }
    }

    #[test]
    fn refuses_a_newer_schema() {
        let e = parse(&format!(
            r#"{{ "schema": {}, "layout": {{}} }}"#,
            SCHEMA_VERSION + 1
        ))
        .expect_err("newer");
        assert!(matches!(e, ImportError::Newer(s) if s == u64::from(SCHEMA_VERSION) + 1));
    }

    #[test]
    fn takes_a_byte_order_mark_and_a_missing_schema() {
        let incoming =
            parse("\u{feff}{ \"layout\": { \"left\": [ { \"type\": \"text\" } ] } }").expect("ok");
        assert_eq!(incoming.sensors.len(), 1);
    }

    #[test]
    fn reads_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(format!("a.{EXTENSION}"));
        std::fs::write(
            &path,
            export(&config(), ExportScope::Layout).expect("exported"),
        )
        .expect("written");
        assert_eq!(read(&path).expect("read").sensors.len(), 6);
        let missing = read(&dir.path().join("none.json")).expect_err("missing");
        assert!(matches!(missing, ImportError::Io(_)));
        assert!(missing.message().starts_with("The file could not be read"));
    }

    #[test]
    fn messages_exist() {
        for key in [
            "import.skipped.notASensor",
            "import.skipped.notSupportedYet",
            "import.skipped.unknownType",
            "import.skipped.noSide",
            "import.warning.invalidValue",
            "import.warning.unknownProperty",
            "import.warning.invalidProperty",
            "import.warning.unknownAdapter",
            "import.warning.droppedProperty",
            "import.error.noLegacyLayout",
            "import.error.xml",
            "import.warning.unknownType",
            "import.error.io",
            "import.error.tooLarge",
            "import.error.notALayout",
            "import.error.newer",
        ] {
            assert!(crate::i18n::exists(key), "{key}");
        }
    }
}
