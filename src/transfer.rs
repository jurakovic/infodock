//! Importing and exporting layouts as the user does it (SPEC §6.4, §11.3–11.4): the file pickers,
//! the *Replace* / *Merge* question, the report and the errors. Runs on the main thread, as
//! Settings' pickers do, so the bar goes on while a picker is open; the Tools menu starts it.
//!
//! A clean import is announced with a toast (§7.4); one that skipped entries or has warnings
//! shows the report in a dialog, so nothing goes unseen.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use crate::app::App;
use crate::config::exchange::{
    self, EXTENSION, ExportScope, ImportFormat, ImportMode, ImportReport, Incoming,
};
use crate::config::legacy;
use crate::i18n::{t, t_args, t_count};
use crate::platform::{Choice, Dialogs, Message, MessageKind, PathRequest, SaveRequest};

/// The report dialog lists at most this many skipped entries and warnings.
const MAX_LINES: usize = 15;

thread_local! {
    /// One import or export at a time (all run on the main thread): a menu click while a
    /// picker is open does nothing.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

struct Busy;

impl Busy {
    fn take() -> Option<Self> {
        (!BUSY.replace(true)).then_some(Busy)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.set(false);
    }
}

/// Asks for a file, reads it, asks *Replace* or *Merge*, imports it and reports. Returns the
/// report, or `None` if the user cancelled or the file could not be imported (said in a
/// message).
pub fn import(app: &App, dialogs: &dyn Dialogs, format: ImportFormat) -> Option<ImportReport> {
    let _busy = Busy::take()?;
    let path = pick_file(dialogs, format)?;
    let incoming = match read(app, &path, format) {
        Ok(incoming) => incoming,
        Err(reason) => {
            failed(dialogs, "import.failed", &reason);
            return None;
        }
    };
    if incoming.sensors.is_empty() {
        // Nothing this version can show (perhaps only sensors it does not support yet): no
        // question whose *Replace* would empty the bar, but what was skipped and why.
        dialogs.show_message(&Message {
            kind: MessageKind::Info,
            text: &t("import.empty"),
            detail: &report_lines(&incoming.skipped, &incoming.warnings).join("\n"),
        });
        return None;
    }
    let mode = ask_mode(dialogs, &incoming)?;
    match app.import(incoming, mode) {
        Ok(report) => {
            show_report(app, dialogs, &report);
            Some(report)
        }
        Err(e) => {
            failed(dialogs, "import.failed", &e.to_string());
            None
        }
    }
}

fn pick_file(dialogs: &dyn Dialogs, format: ImportFormat) -> Option<PathBuf> {
    let (title, types_name, extensions): (_, _, &[&str]) = match format {
        ImportFormat::Json => (t("import.pick.title"), t("import.pick.types"), &[EXTENSION]),
        ImportFormat::Legacy => (
            t("import.pick.legacyTitle"),
            t("import.pick.legacyTypes"),
            &["xml"],
        ),
        ImportFormat::Any => (
            t("import.pick.title"),
            t("import.pick.anyTypes"),
            &[EXTENSION, "xml"],
        ),
    };
    let all_files = t("settings.props.allFiles");
    dialogs
        .open_path(&PathRequest {
            title: &title,
            folder: false,
            extensions,
            types_name: &types_name,
            all_files: &all_files,
        })
        .map(PathBuf::from)
}

/// What the file brings, or why it cannot be imported.
fn read(app: &App, path: &Path, format: ImportFormat) -> Result<Incoming, String> {
    let code_pages = |bytes: &[u8], code_page| app.platform.decode_code_page(bytes, code_page);
    match format.of(path) {
        ImportFormat::Json | ImportFormat::Any => exchange::read(path),
        ImportFormat::Legacy => {
            legacy::read(path, &code_pages).map(|sensors| app.legacy_incoming(sensors))
        }
    }
    .and_then(Incoming::checked)
    .map_err(|e| e.message())
}

/// The network locations the question names, at most this many.
const MAX_NETWORK_PATHS: usize = 3;

fn ask_mode(dialogs: &dyn Dialogs, incoming: &Incoming) -> Option<ImportMode> {
    let question = t("import.mode.question");
    let mut detail = t_count("import.mode.detail", incoming.sensors.len() as u64, &[]);
    // The bar opens these as soon as the sensors are on it, which connects to their servers
    // (and hands them the user's sign-in): said before, so a layout from elsewhere can be
    // left alone.
    let paths = incoming.network_paths();
    if !paths.is_empty() {
        let mut lines: Vec<String> = paths
            .iter()
            .take(MAX_NETWORK_PATHS)
            .map(|p| exchange::shown(p))
            .collect();
        if paths.len() > MAX_NETWORK_PATHS {
            let more = (paths.len() - MAX_NETWORK_PATHS) as u64;
            lines.push(t_count("import.report.more", more, &[]));
        }
        detail.push_str("\n\n");
        detail.push_str(&t_args(
            "import.mode.network",
            &[("paths", &lines.join("\n"))],
        ));
    }
    let choices = [t("import.mode.replace"), t("import.mode.merge")];
    let choice = dialogs.choose(&Choice {
        question: &question,
        detail: &detail,
        choices: &[&choices[0], &choices[1]],
    })?;
    Some(match choice {
        0 => ImportMode::Replace,
        _ => ImportMode::Merge,
    })
}

fn show_report(app: &App, dialogs: &dyn Dialogs, report: &ImportReport) {
    let added = t_count("import.report.added", report.added as u64, &[]);
    if report.skipped.is_empty() && report.warnings.is_empty() {
        app.platform.notify(&t("import.report.title"), &added);
        return;
    }
    dialogs.show_message(&Message {
        kind: MessageKind::Warning,
        text: &added,
        detail: &report_lines(&report.skipped, &report.warnings).join("\n"),
    });
}

/// The report's lines: each skipped entry, then the warnings, at most [`MAX_LINES`].
fn report_lines(skipped: &[exchange::Skipped], warnings: &[exchange::Warning]) -> Vec<String> {
    let skipped = skipped.iter().map(|s| {
        let kind = if s.kind.is_empty() {
            t("import.report.noType")
        } else {
            exchange::shown(&s.kind)
        };
        t_args(
            "import.report.skipped",
            &[("type", &kind), ("reason", &s.reason.message())],
        )
    });
    let warnings = warnings.iter().map(exchange::Warning::message);
    let mut lines: Vec<String> = skipped.chain(warnings).collect();
    if lines.len() > MAX_LINES {
        let more = lines.len() - (MAX_LINES - 1);
        lines.truncate(MAX_LINES - 1);
        lines.push(t_count("import.report.more", more as u64, &[]));
    }
    lines
}

fn failed(dialogs: &dyn Dialogs, key: &str, reason: &str) {
    dialogs.show_message(&Message {
        kind: MessageKind::Error,
        text: &t(key),
        detail: reason,
    });
}

/// Asks where to save, and whether the layout or the whole config (the file type chosen in the
/// picker), and writes the file. Returns its path, or `None` if the user cancelled or it could
/// not be written (said in a message).
pub fn export(app: &App, dialogs: &dyn Dialogs) -> Option<PathBuf> {
    let _busy = Busy::take()?;
    let (title, file_name) = (t("export.pick.title"), t("export.pick.fileName"));
    let (layout, config) = (t("export.types.layout"), t("export.types.config"));
    let (path, type_index) = dialogs.save_path(&SaveRequest {
        title: &title,
        file_name: &file_name,
        types: &[(&layout, EXTENSION), (&config, EXTENSION)],
    })?;
    let scope = match type_index {
        1 => ExportScope::Config,
        _ => ExportScope::Layout,
    };
    let path = export_path(path);
    let written = app
        .export(scope)
        .map_err(std::io::Error::other)
        .and_then(|text| std::fs::write(&path, text));
    if let Err(e) = written {
        failed(dialogs, "export.failed", &e.to_string());
        return None;
    }
    Some(path)
}

/// `path` with `.infodock.json` added when it does not end in `.json`.
fn export_path(path: String) -> PathBuf {
    if path.to_lowercase().ends_with(".json") {
        PathBuf::from(path)
    } else {
        PathBuf::from(format!("{path}.{EXTENSION}"))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, PoisonError};

    use super::*;
    use crate::config::ConfigStore;
    use crate::config::exchange::{MAX_SENSORS, SkipReason, Skipped, Warning};
    use crate::config::model::{LegacyUnsupported, Zone};
    use crate::platform::{Capabilities, MonitorInfo, OsAppearance, Platform};

    /// Answers with a script and records what it was asked.
    #[derive(Default)]
    struct Script {
        open: RefCell<Option<String>>,
        save: RefCell<Option<(String, usize)>>,
        choices: RefCell<VecDeque<Option<usize>>>,
        asked: RefCell<Vec<String>>,
    }

    impl Dialogs for Script {
        fn open_path(&self, request: &PathRequest) -> Option<String> {
            self.asked.borrow_mut().push(format!(
                "open {} ({}: {:?})",
                request.title, request.types_name, request.extensions
            ));
            self.open.borrow_mut().take()
        }
        fn save_path(&self, request: &SaveRequest) -> Option<(String, usize)> {
            let types: Vec<String> = request
                .types
                .iter()
                .map(|(name, ext)| format!("{name}: {ext}"))
                .collect();
            self.asked.borrow_mut().push(format!(
                "save {} as {} ({})",
                request.title,
                request.file_name,
                types.join(", ")
            ));
            self.save.borrow_mut().take()
        }
        fn choose(&self, choice: &Choice) -> Option<usize> {
            self.asked.borrow_mut().push(format!(
                "choose {} {} {:?}",
                choice.question, choice.detail, choice.choices
            ));
            self.choices.borrow_mut().pop_front().flatten()
        }
        fn show_message(&self, message: &Message) {
            self.asked.borrow_mut().push(format!(
                "{:?}: {} | {}",
                message.kind, message.text, message.detail
            ));
        }
    }

    impl Script {
        fn asked(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }
    }

    /// Records toasts.
    #[derive(Default)]
    struct Toasts(Mutex<Vec<String>>);

    impl Platform for Toasts {
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn monitors(&self) -> Vec<MonitorInfo> {
            Vec::new()
        }
        fn appearance(&self) -> OsAppearance {
            OsAppearance::default()
        }
        fn uptime(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }
        fn notify(&self, title: &str, body: &str) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("{title}: {body}"));
        }
    }

    struct Fixture {
        app: App,
        toasts: Arc<Toasts>,
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let (store, _) = ConfigStore::open(dir.path().join("config"), "test");
            let toasts = Arc::new(Toasts::default());
            let app = App::new(store, toasts.clone(), dir.path().join("skins"));
            Self { app, toasts, dir }
        }

        fn file(&self, name: &str, text: &str) -> String {
            let path = self.dir.path().join(name);
            std::fs::write(&path, text).expect("written");
            path.to_string_lossy().into_owned()
        }

        fn toasts(&self) -> Vec<String> {
            self.toasts
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        fn kinds(&self, zone: Zone) -> Vec<String> {
            let layout = self.app.store.get().layout;
            layout.zone(zone).iter().map(|s| s.kind.clone()).collect()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.app.store.shutdown();
        }
    }

    const LAYOUT: &str = r#"{ "schema": 1, "layout": {
        "left": [ { "id": "s_a", "type": "text", "props": { "text": "Hi" } } ],
        "right": [ { "id": "s_b", "type": "spacer" } ]
    } }"#;

    #[test]
    fn imports_with_replace_and_announces_it() {
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(f.file("a.infodock.json", LAYOUT));
        script.choices.borrow_mut().push_back(Some(0));
        let report = import(&f.app, &script, ImportFormat::Json).expect("imported");
        assert_eq!(report.added, 2);
        assert_eq!(f.kinds(Zone::Left), ["text"]);
        assert!(f.kinds(Zone::Center).is_empty());
        assert_eq!(f.kinds(Zone::Right), ["spacer"]);
        assert_eq!(
            script.asked(),
            [
                "open Import layout (Infodock layouts: [\"infodock.json\"])",
                "choose Replace the current layout, or add the imported sensors to it? \
                 The file holds 2 sensors. [\"Replace the layout\\nThe sensors now on the bar \
                 are removed.\", \"Add to the layout\\nThe imported sensors are added at the \
                 end of their zones.\"]",
            ]
        );
        assert_eq!(f.toasts(), ["Layout imported: 2 sensors added."]);
    }

    #[test]
    fn merges_and_reports_what_was_skipped() {
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(f.file(
            "b.json",
            r#"{ "layout": { "center": [ 1, { "type": "image", "props": { "opacity": "x" } } ] } }"#,
        ));
        script.choices.borrow_mut().push_back(Some(1));
        let report = import(&f.app, &script, ImportFormat::Json).expect("imported");
        assert_eq!(report.added, 1);
        assert_eq!(f.kinds(Zone::Center), ["image", "image"]);
        assert_eq!(
            script.asked().last().map(String::as_str),
            Some(
                "Warning: 1 sensor added. | Not imported: an entry without a type (not a \
                 sensor entry)\nImage: the value of “opacity” cannot be used, so its default \
                 is used instead."
            )
        );
        assert!(f.toasts().is_empty());
    }

    #[test]
    fn cancelling_changes_nothing() {
        let f = Fixture::new();
        let before = f.app.store.get();
        // No file chosen.
        let script = Script::default();
        assert!(import(&f.app, &script, ImportFormat::Json).is_none());
        assert_eq!(script.asked().len(), 1);
        // A file, then Cancel.
        *script.open.borrow_mut() = Some(f.file("c.json", LAYOUT));
        script.choices.borrow_mut().push_back(None);
        assert!(import(&f.app, &script, ImportFormat::Json).is_none());
        assert_eq!(f.app.store.get(), before);
        assert!(f.toasts().is_empty());
    }

    #[test]
    fn says_why_a_file_cannot_be_imported() {
        let f = Fixture::new();
        let script = Script::default();
        let newer = r#"{ "schema": 99, "layout": {} }"#;
        for (name, text, reason) in [
            (
                "newer.json",
                newer,
                "The file was saved by a newer version of Infodock. Update Infodock to import it.",
            ),
            (
                "bad.json",
                "<xml/>",
                "The file does not hold an Infodock layout.",
            ),
        ] {
            *script.open.borrow_mut() = Some(f.file(name, text));
            assert!(import(&f.app, &script, ImportFormat::Json).is_none());
            assert_eq!(
                script.asked().last().cloned(),
                Some(format!(
                    "Error: The layout could not be imported. | {reason}"
                ))
            );
        }
        *script.open.borrow_mut() = Some(f.file("empty.json", r#"{ "layout": {} }"#));
        assert!(import(&f.app, &script, ImportFormat::Json).is_none());
        assert_eq!(
            script.asked().last().map(String::as_str),
            Some("Info: The file holds no sensors this version of Infodock can show. | ")
        );
        // Only sensors this version does not support: nothing to replace the bar with, and the
        // message says why (review M4).
        let only_rss = r#"<sensors><sensor side="left" type="DockRSSSensor"/>
            <sensor type="DockText"/></sensors>"#;
        *script.open.borrow_mut() = Some(f.file("rss.xml", only_rss));
        assert!(import(&f.app, &script, ImportFormat::Legacy).is_none());
        assert_eq!(
            script.asked().last().map(String::as_str),
            Some(
                "Info: The file holds no sensors this version of Infodock can show. | \
                 Not imported: DockRSSSensor (not supported yet)\n\
                 Not imported: DockText (no side: left, center or right)"
            )
        );
        // More sensors than a bar can hold.
        let many: Vec<&str> = vec![r#"{ "type": "spacer" }"#; MAX_SENSORS + 1];
        let many = format!(r#"{{ "layout": {{ "left": [{}] }} }}"#, many.join(","));
        *script.open.borrow_mut() = Some(f.file("many.json", &many));
        assert!(import(&f.app, &script, ImportFormat::Json).is_none());
        assert_eq!(
            script.asked().last().cloned(),
            Some(format!(
                "Error: The layout could not be imported. | The file holds more than \
                 {MAX_SENSORS} sensors, more than a bar can show."
            ))
        );
        assert!(f.toasts().is_empty());
    }

    #[test]
    fn the_question_names_network_locations() {
        let f = Fixture::new();
        let script = Script::default();
        let file = r#"{ "layout": { "left": [
            { "type": "image", "props": { "file": "\\\\server\\share\\logo.png" } },
            { "type": "folder-menu", "props": { "folder": "\\\\Server\\Share\\logo.png" } },
            { "type": "folder-menu", "props": { "folder": "C:\\Users" } }
        ] } }"#;
        *script.open.borrow_mut() = Some(f.file("net.json", file));
        script.choices.borrow_mut().push_back(None);
        assert!(import(&f.app, &script, ImportFormat::Json).is_none());
        let question = script.asked().last().cloned().unwrap_or_default();
        // Each location once (names ignore case), local paths not at all.
        assert!(
            question.contains("trust where it came from.\n\\\\server\\share\\logo.png ["),
            "{question}"
        );
        assert!(!question.contains("C:\\Users"));
    }

    #[test]
    fn exports_the_chosen_scope() {
        let f = Fixture::new();
        let script = Script::default();
        let target = f.dir.path().join("mine");
        *script.save.borrow_mut() = Some((target.to_string_lossy().into_owned(), 0));
        let path = export(&f.app, &script).expect("exported");
        assert_eq!(path, f.dir.path().join("mine.infodock.json"));
        assert_eq!(
            script.asked(),
            [
                "save Export layout as Infodock layout (Infodock layout: infodock.json, \
              Infodock settings and layout: infodock.json)"
            ]
        );
        let layout: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("JSON");
        assert!(layout.get("settings").is_none());
        assert_eq!(layout["layout"]["left"][0]["type"], "cpu-graph");

        let whole = f.dir.path().join("all.json");
        *script.save.borrow_mut() = Some((whole.to_string_lossy().into_owned(), 1));
        assert_eq!(export(&f.app, &script), Some(whole.clone()));
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&whole).expect("read")).expect("JSON");
        assert_eq!(config["settings"]["bar"]["edge"], "bottom");

        let unwritable = f.dir.path().join("no-such-folder").join("x.json");
        *script.save.borrow_mut() = Some((unwritable.to_string_lossy().into_owned(), 0));
        assert!(export(&f.app, &script).is_none());
        assert!(
            script
                .asked()
                .last()
                .is_some_and(|m| m.starts_with("Error: The layout could not be saved. | "))
        );
    }

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy");
        path.join(name).to_string_lossy().into_owned()
    }

    fn sensors(f: &Fixture, zone: Zone) -> Vec<(String, serde_json::Value)> {
        let layout = f.app.store.get().layout;
        layout
            .zone(zone)
            .iter()
            .map(|s| (s.kind.clone(), serde_json::Value::Object(s.props.clone())))
            .collect()
    }

    #[test]
    fn imports_a_legacy_layout() {
        use serde_json::json;
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(fixture("general.xml"));
        script.choices.borrow_mut().push_back(Some(0));
        let report = import(&f.app, &script, ImportFormat::Legacy).expect("imported");
        assert_eq!(
            sensors(&f, Zone::Left),
            [
                (
                    "text".into(),
                    json!({
                        "text": "Hello & welcome", "font": "Tahoma", "fontStyle": "boldItalic",
                        "fontSize": 25, "shadow": true
                    })
                ),
                ("spacer".into(), json!({ "width": 20 })),
            ]
        );
        assert_eq!(
            sensors(&f, Zone::Center),
            [(
                "image".into(),
                json!({
                    "image": "C:\\Pictures\\logo.png",
                    "color": { "mode": "custom", "color": "#3A6EA5" },
                    "opacity": 50,
                    "popupText": "My logo"
                })
            )]
        );
        assert_eq!(
            sensors(&f, Zone::Right),
            [
                ("image".into(), json!({})),
                ("text".into(), json!({ "text": "Plain" }))
            ]
        );
        let unsupported = f.app.store.get().legacy.unsupported;
        assert_eq!(unsupported.len(), 1);
        assert_eq!(unsupported[0].kind, "DockStandardMail");
        assert_eq!(unsupported[0].zone, Zone::Center);
        assert_eq!(
            unsupported[0].props.keys().collect::<Vec<_>>(),
            ["server", "user"]
        );
        assert_eq!(report.added, 5);
        let asked = script.asked();
        assert_eq!(
            asked[0],
            "open Import legacy XML layout (XML layouts: [\"xml\"])"
        );
        assert!(
            asked[1].contains("The file holds 5 sensors."),
            "{}",
            asked[1]
        );
        assert_eq!(
            asked[2],
            "Warning: 5 sensors added. | \
             Not imported: DockStandardMail (not supported yet)\n\
             Not imported: DockWeather (not a sensor type Infodock knows)\n\
             DockImage: the property “blink” is not known, so it was ignored.\n\
             DockText: “maybe” is not a valid value of “shadow”, so its default is used instead."
        );
        assert!(f.toasts().is_empty());
    }

    #[test]
    fn imports_a_layout_inside_a_settings_dump() {
        use serde_json::json;
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(fixture("settings-dump.txt"));
        script.choices.borrow_mut().push_back(Some(1));
        let report = import(&f.app, &script, ImportFormat::Legacy).expect("imported");
        assert_eq!(report.added, 2);
        // Merged after the first-run layout.
        assert_eq!(
            sensors(&f, Zone::Center).last(),
            Some(&("text".into(), json!({ "text": "Café – open" })))
        );
        assert_eq!(
            sensors(&f, Zone::Right).last(),
            Some(&("spacer".into(), json!({ "showDivider": true })))
        );
        assert_eq!(f.toasts(), ["Layout imported: 2 sensors added."]);
    }

    /// The welcome page offers both kinds in one picker; the name tells them apart.
    #[test]
    fn imports_either_kind_by_its_name() {
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(f.file("a.infodock.json", LAYOUT));
        script.choices.borrow_mut().push_back(Some(0));
        let report = import(&f.app, &script, ImportFormat::Any).expect("imported");
        assert_eq!(report.added, 2);
        assert_eq!(
            script.asked().first().map(String::as_str),
            Some(
                "open Import layout (Infodock and legacy XML layouts: \
                 [\"infodock.json\", \"xml\"])"
            )
        );
        // Any other name is a legacy layout, e.g. a settings dump.
        *script.open.borrow_mut() = Some(fixture("settings-dump.txt"));
        script.choices.borrow_mut().push_back(Some(0));
        let report = import(&f.app, &script, ImportFormat::Any).expect("imported");
        assert_eq!(report.added, 2);
        assert_eq!(f.kinds(Zone::Center), ["text"]);
    }

    #[test]
    fn imports_the_legacy_system_sensors() {
        use serde_json::json;
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(fixture("system.xml"));
        script.choices.borrow_mut().push_back(Some(0));
        let report = import(&f.app, &script, ImportFormat::Legacy).expect("imported");
        assert_eq!(
            sensors(&f, Zone::Left),
            [
                (
                    "date".into(),
                    json!({
                        "weekday": "short", "month": "short", "year": "bottom", "shadow": true,
                        "calendar": "sunday"
                    })
                ),
                (
                    "time".into(),
                    json!({ "use24h": false, "showSeconds": true, "chime": 30 })
                ),
                (
                    "uptime".into(),
                    json!({ "text": "Up", "fontStyle": "normal", "fontSize": 14, "shadow": true })
                ),
            ]
        );
        assert_eq!(
            sensors(&f, Zone::Center),
            [
                (
                    "cpu-meter".into(),
                    json!({ "interval": 2, "color": { "mode": "dynamic", "color": "#00FF00" } })
                ),
                (
                    "cpu-graph".into(),
                    json!({
                        "width": 100,
                        "bkColor": { "mode": "custom", "color": "#202020" },
                        "bkOpacity": 50,
                        "graphColor": { "mode": "dynamic", "color": "#000000" },
                        "graphOpacity": 100
                    })
                ),
                (
                    "cpu-graph-dual".into(),
                    json!({
                        "interval": 3,
                        "gridColor": { "mode": "custom", "color": "#FFFFFF" },
                        "gridOpacity": 20,
                        "bottomColor": { "mode": "dynamic", "color": "#000000" },
                        "bottomOpacity": 88,
                        "flipBottom": false
                    })
                ),
                (
                    "memory-meter".into(),
                    json!({ "interval": 5, "color": { "mode": "custom", "color": "#FF8000" } })
                ),
                ("memory-graph".into(), json!({ "display": "percent" })),
                ("swap-meter".into(), json!({ "interval": 1 })),
                (
                    "swap-graph".into(),
                    json!({ "graphColor": { "mode": "custom", "color": "#C0C0C0" } })
                ),
            ]
        );
        assert_eq!(
            sensors(&f, Zone::Right),
            [
                (
                    "disk-meter".into(),
                    json!({ "drive": "D:\\", "mode": "freeBytes", "interval": 300 })
                ),
                (
                    "network-graph".into(),
                    json!({ "bandwidth": "12500000", "width": 150 })
                ),
            ]
        );
        let config = f.app.store.get();
        let uptime = &config.layout.left[2].id;
        assert_eq!(
            config
                .state
                .get(uptime)
                .map(|s| serde_json::Value::Object(s.clone())),
            Some(json!({ "longestSecs": 3 * 86_400 + 4 * 3_600 + 12 * 60 }))
        );
        assert_eq!(config.state.len(), 1);
        assert_eq!(report.added, 12);
        assert!(report.skipped.is_empty());
        let warnings: Vec<String> = report.warnings.iter().map(|w| w.message()).collect();
        // This computer has no network adapters.
        assert_eq!(
            warnings,
            [
                "DockSwapGraph: “2” is not a valid value of “display-mode”, so its default is \
                 used instead.",
                "DockNetwork: this computer has no network adapter “Realtek PCIe GbE Family \
                 Controller”, so Automatic is used instead."
            ]
        );
    }

    /// SPEC Appendix B, the exit criterion of M4 (§17).
    #[test]
    fn imports_appendix_b_as_specified() {
        use serde_json::json;
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(fixture("example.xml"));
        script.choices.borrow_mut().push_back(Some(0));
        let report = import(&f.app, &script, ImportFormat::Legacy).expect("imported");
        assert_eq!(
            sensors(&f, Zone::Left),
            [
                ("battery".into(), json!({})),
                ("spacer".into(), json!({ "showDivider": true })),
                (
                    "disk-meter".into(),
                    json!({ "title": "System", "drive": "C:\\", "interval": 10 })
                ),
                (
                    "recycle-bin".into(),
                    json!({
                        "imageEmpty": "D:\\Icons\\Recycle-Empty.ico",
                        "imageFull": "D:\\Icons\\Recycle.ico", "opacity": 77
                    })
                ),
                (
                    "shortcut".into(),
                    json!({
                        "target": "D:\\Documents", "title": "Documents",
                        "image": "D:\\Icons\\Documents.ico"
                    })
                ),
            ]
        );
        assert!(sensors(&f, Zone::Center).is_empty());
        assert_eq!(
            sensors(&f, Zone::Right),
            [
                (
                    "memory-graph".into(),
                    json!({
                        "width": 70, "gridOpacity": 7,
                        "graphColor": { "mode": "custom", "color": "#C0C0C0" },
                        "graphOpacity": 54
                    })
                ),
                (
                    "cpu-graph-dual".into(),
                    json!({
                        "width": 70, "gridOpacity": 7,
                        "topColor": { "mode": "custom", "color": "#666666" },
                        "topOpacity": 88,
                        "bottomColor": { "mode": "custom", "color": "#C0C0C0" },
                        "bottomOpacity": 88
                    })
                ),
                (
                    "uptime".into(),
                    json!({ "text": "", "font": "Arial", "fontSize": 11 })
                ),
                (
                    "date".into(),
                    json!({ "weekday": "none", "month": "number" })
                ),
            ]
        );
        let config = f.app.store.get();
        assert_eq!(
            config.legacy.unsupported,
            [LegacyUnsupported {
                kind: "DockRSSSensor".into(),
                zone: Zone::Center,
                props: [
                    ("title".to_owned(), "News".to_owned()),
                    ("url".to_owned(), "https://example.com/feed.xml".to_owned())
                ]
                .into(),
            }]
        );
        assert!(config.state.is_empty(), "{:?}", config.state);
        assert_eq!(report.added, 9);
        assert_eq!(
            report.skipped,
            [Skipped {
                kind: "DockRSSSensor".into(),
                reason: SkipReason::NotSupportedYet
            }]
        );
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn imports_the_legacy_launchers() {
        use serde_json::json;
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(fixture("launchers.xml"));
        script.choices.borrow_mut().push_back(Some(0));
        let report = import(&f.app, &script, ImportFormat::Legacy).expect("imported");
        assert_eq!(
            sensors(&f, Zone::Left),
            [
                (
                    "wifi".into(),
                    json!({ "title": "Home", "interval": 5, "display": "dbm" })
                ),
                (
                    "disk".into(),
                    json!({
                        "drive": "E:\\", "title": "Stick",
                        "color": { "mode": "custom", "color": "#80C0FF" }
                    })
                ),
            ]
        );
        assert_eq!(
            sensors(&f, Zone::Center),
            [
                (
                    "folder-menu".into(),
                    json!({
                        "folder": "C:\\Users\\Public\\Downloads", "title": "Downloads",
                        "image": "D:\\Icons\\Folder.ico", "opacity": 90,
                        "popupWidth": 300, "popupHeight": 450
                    })
                ),
                ("start-menu".into(), json!({ "popupHeight": 500 })),
            ]
        );
        assert_eq!(
            sensors(&f, Zone::Right),
            [
                (
                    "power".into(),
                    json!({ "mode": "shutDown", "confirm": false })
                ),
                ("power".into(), json!({ "mode": "lock" })),
                ("eject-usb".into(), json!({ "opacity": 70 })),
            ]
        );
        assert_eq!(report.added, 7);
        let warnings: Vec<String> = report.warnings.iter().map(|w| w.message()).collect();
        assert_eq!(
            warnings,
            ["DockPower: Infodock has no use for the property “path”, so it was not imported."]
        );
    }

    #[test]
    fn says_why_a_legacy_file_cannot_be_imported() {
        let f = Fixture::new();
        let script = Script::default();
        *script.open.borrow_mut() = Some(f.file("none.xml", "<layout/>"));
        assert!(import(&f.app, &script, ImportFormat::Legacy).is_none());
        assert_eq!(
            script.asked().last().map(String::as_str),
            Some("Error: The layout could not be imported. | The file holds no legacy XML layout.")
        );
    }

    #[test]
    fn long_reports_are_cut() {
        let report = ImportReport {
            added: 0,
            skipped: vec![
                Skipped {
                    kind: "DockRSSSensor".into(),
                    reason: SkipReason::NotSupportedYet,
                };
                20
            ],
            warnings: vec![Warning::UnknownType {
                kind: "mail".into(),
            }],
        };
        let lines = report_lines(&report.skipped, &report.warnings);
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines[0], "Not imported: DockRSSSensor (not supported yet)");
        assert_eq!(lines[MAX_LINES - 1], "… and 7 more");
    }

    #[test]
    fn one_at_a_time() {
        let busy = Busy::take().expect("free");
        assert!(Busy::take().is_none());
        drop(busy);
        assert!(Busy::take().is_some());
    }
}
