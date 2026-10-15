//! `shortcut` — opens a program, document, folder or web address (SPEC §9.17). The bar shows
//! its icon: a custom image, the target's shell icon, or the built-in globe for web addresses.
//! The target is checked on the `path/<path>` topic, and the sensor is inactive while it is
//! missing.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use super::common::icon::{self, Icon};
use super::schema::{FieldKind, Lit, PropField, SelectOption};
use super::{
    Category, ClickResult, ColorChoice, DrawCtx, HintContent, IconSource, LayoutCtx, Props, Sensor,
    SensorCtx, SensorType, ShellIcon, State, typed,
};
use crate::i18n::{t, t_args};
use crate::platform::{Bitmap, Canvas, Launch, PathKind, load_image};
use crate::sampler::{Sample, Topic, TopicRequest};

/// The icon for web addresses (Appendix C).
const GLOBE: &[u8] = include_bytes!("../../assets/icons/globe.svg");
/// The icon when the shell has none, e.g. for a missing target (Appendix C).
const APP: &[u8] = include_bytes!("../../assets/icons/app.svg");

/// How often the target is checked (SPEC §9.17).
const CHECK_EVERY: Duration = Duration::from_secs(60);
/// An automatic title from a web address keeps this many characters at each end.
const URL_KEEP: usize = 75;

const SCHEMA: &[PropField] = &[
    PropField::new(
        "target",
        FieldKind::File {
            extensions: &["exe", "cmd", "bat", "lnk", "url", "ps1"],
        },
        "sensor.shortcut.field.target",
        Lit::Str(""),
    )
    .describe("sensor.shortcut.field.target.help"),
    PropField::new(
        "args",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.shortcut.field.args",
        Lit::Str(""),
    )
    .describe("sensor.shortcut.field.args.help"),
    PropField::new(
        "url",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.shortcut.field.url",
        Lit::Str(""),
    )
    .describe("sensor.shortcut.field.url.help"),
    PropField::new(
        "workDir",
        FieldKind::Folder,
        "sensor.shortcut.field.workDir",
        Lit::Str(""),
    )
    .describe("sensor.shortcut.field.workDir.help"),
    PropField::new(
        "runAs",
        FieldKind::Select {
            options: &[
                SelectOption {
                    value: Lit::Str("normal"),
                    label: "sensor.shortcut.runAs.normal",
                },
                SelectOption {
                    value: Lit::Str("administrator"),
                    label: "sensor.shortcut.runAs.administrator",
                },
            ],
        },
        "sensor.shortcut.field.runAs",
        Lit::Str("normal"),
    ),
    PropField::new(
        "title",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.shortcut.field.title",
        Lit::Str(""),
    )
    .describe("sensor.shortcut.field.title.help"),
    PropField::new(
        "image",
        FieldKind::File {
            extensions: &["ico", "png", "gif", "jpg", "jpeg", "svg"],
        },
        "sensor.shortcut.field.image",
        Lit::Str(""),
    )
    .describe("sensor.shortcut.field.image.help"),
    PropField::new(
        "color",
        FieldKind::ColorMode { dynamic: false },
        "sensor.field.color",
        Lit::ColorMode("standard", "#FFFFFF"),
    )
    .describe("sensor.shortcut.field.color.help"),
    PropField::new(
        "opacity",
        FieldKind::Opacity { min: 20 },
        "sensor.field.opacity",
        Lit::Int(100),
    ),
];

pub struct ShortcutType;

impl SensorType for ShortcutType {
    fn id(&self) -> &'static str {
        "shortcut"
    }

    fn category(&self) -> Category {
        Category::Launchers
    }

    fn name(&self) -> &'static str {
        "sensor.shortcut.name"
    }

    fn description(&self) -> &'static str {
        "sensor.shortcut.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockShortcut"]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Shortcut {
            props: typed(props)?,
            kind: None,
            refresh_icon: false,
            custom: None,
            icon: None,
        }))
    }

    /// The title, or what it is made of: the web address or the target's file name.
    fn summary(&self, props: &Props) -> Option<String> {
        let text = |key: &str| props.get(key).and_then(|v| v.as_str()).unwrap_or_default();
        let summary = [text("title"), text("url")]
            .into_iter()
            .find(|s| !s.is_empty())
            .map(|s| middle_truncated(s, URL_KEEP))
            .unwrap_or_else(|| file_name(text("target")).to_owned());
        (!summary.is_empty()).then_some(summary)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum RunAs {
    Normal,
    Administrator,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShortcutProps {
    target: String,
    args: String,
    /// Takes precedence over the target.
    url: String,
    /// Empty: the target's folder.
    work_dir: String,
    run_as: RunAs,
    /// Empty: automatic.
    title: String,
    /// Empty: the target's icon.
    image: String,
    /// Custom tints the icon (SPEC §8.4).
    color: ColorChoice,
    /// Percent.
    opacity: u32,
}

/// The last component of a Windows or POSIX path (`C:\Tools\app.exe` → `app.exe`), or the
/// path itself when it has none (`C:\`).
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rsplit(['\\', '/']).next() {
        Some(name) if !name.is_empty() && !name.ends_with(':') => name,
        _ => path,
    }
}

/// `name` without its extension (`app.exe` → `app`); a leading dot is no extension.
fn without_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// `text` with its middle cut out when it is longer than twice `keep` characters: the first and
/// last `keep` with `…` between them (SPEC §9.17).
pub fn middle_truncated(text: &str, keep: usize) -> String {
    let count = text.chars().count();
    if count <= 2 * keep {
        return text.to_owned();
    }
    let head: String = text.chars().take(keep).collect();
    let tail: String = text.chars().skip(count - keep).collect();
    format!("{head}…{tail}")
}

/// The automatic title (SPEC §9.17): a web address middle-truncated to 150 characters, a
/// folder's name, or a file's name without its extension.
pub fn auto_title(url: &str, target: &str, kind: Option<PathKind>) -> String {
    if !url.is_empty() {
        return middle_truncated(url, URL_KEEP);
    }
    let name = file_name(target);
    match kind {
        Some(PathKind::Folder) => name.to_owned(),
        _ => without_extension(name).to_owned(),
    }
}

/// Whether `target` is a bare program name such as `notepad.exe`, which the shell finds on its
/// own (App Paths, `PATH`); such a target is not checked.
pub fn is_bare_name(target: &str) -> bool {
    !target.contains(['\\', '/', ':'])
}

/// The working folder for starting `target`: `work_dir`, or else the target's folder (SPEC
/// §9.17); `None` for a bare program name.
pub fn work_dir(target: &str, work_dir: &str) -> Option<String> {
    if !work_dir.is_empty() {
        return Some(work_dir.to_owned());
    }
    let trimmed = target.trim_end_matches(['\\', '/']);
    let cut = trimmed.rfind(['\\', '/'])?;
    let parent = &trimmed[..cut];
    // `C:\app.exe` starts in `C:\`, not `C:`.
    Some(if parent.ends_with(':') || parent.is_empty() {
        trimmed[..=cut].to_owned()
    } else {
        parent.to_owned()
    })
}

struct Shortcut {
    props: ShortcutProps,
    /// What the target is, from the `path/<path>` topic; `None` before its first sample.
    kind: Option<PathKind>,
    /// The target changed what it is: its shell icon is read again at the next measure.
    refresh_icon: bool,
    /// The custom image, decoded at a height in physical px (`None` inside if it cannot be
    /// read).
    custom: Option<(u32, Option<Bitmap>)>,
    /// What is drawn; `None` while the shell icon is being read.
    icon: Option<Icon>,
}

impl Shortcut {
    fn has_url(&self) -> bool {
        !self.props.url.is_empty()
    }

    /// Nothing to open.
    fn unset(&self) -> bool {
        !self.has_url() && self.props.target.is_empty()
    }

    /// The target is checked: a path, not a web address or a bare program name.
    fn checks_target(&self) -> bool {
        !self.has_url() && !self.props.target.is_empty() && !is_bare_name(&self.props.target)
    }

    fn title(&self) -> String {
        if self.props.title.is_empty() {
            auto_title(&self.props.url, &self.props.target, self.kind)
        } else {
            self.props.title.clone()
        }
    }

    /// The custom image `px` physical px tall, if it is set and can be read.
    fn custom_image(&mut self, px: u32) -> Option<Bitmap> {
        if self.props.image.is_empty() {
            return None;
        }
        if self.custom.as_ref().is_none_or(|(h, _)| *h != px) {
            let image = load_image(Path::new(&self.props.image), Some(px))
                .map_err(|e| tracing::warn!("image {:?} cannot be shown: {e}", self.props.image))
                .ok();
            self.custom = Some((px, image));
        }
        self.custom.as_ref().and_then(|(_, image)| image.clone())
    }

    /// The icon `px` physical px tall: the custom image, the globe for a web address, or the
    /// target's shell icon (the generic app icon when it has none). `None` while loading.
    /// The bitmap says whether it is one of Infodock's built-in icons.
    fn bitmap(&mut self, px: u32, icons: &dyn IconSource) -> Option<(Bitmap, bool)> {
        if let Some(image) = self.custom_image(px) {
            return Some((image, false));
        }
        if self.has_url() {
            return Icon::svg(GLOBE, px).map(|b| (b, true));
        }
        if self.props.target.is_empty() {
            return Icon::svg(APP, px).map(|b| (b, true));
        }
        if std::mem::take(&mut self.refresh_icon) {
            icons.forget(&self.props.target);
        }
        match icons.shell_icon(&self.props.target, px) {
            ShellIcon::Ready(icon) => Some((icon, false)),
            ShellIcon::Loading => None,
            ShellIcon::None => Icon::svg(APP, px).map(|b| (b, true)),
        }
    }
}

impl Sensor for Shortcut {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        // Bar height − 4, centred (SPEC §9.17).
        let logical = cx.theme.bar_height.saturating_sub(4).max(1) as f32;
        let px = icon::physical(logical, cx.scale);
        let opacity = self.props.opacity.min(100) as f32 / 100.0;
        let inactive = self.inactive().then_some(cx.theme.inactive);
        self.icon = self.bitmap(px, cx.icons).map(|(bitmap, built_in)| {
            let tint = icon::tint(&self.props.color, built_in, cx.theme);
            Icon::new(bitmap, tint, opacity, inactive)
        });
        // While the shell icon loads, its square is kept free.
        self.icon
            .as_ref()
            .map_or((px as f32 / cx.scale).ceil(), |i| i.width(cx.scale))
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        if let Some(icon) = &self.icon {
            icon.draw(canvas, cx);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        let text = if !self.props.title.is_empty() {
            self.props.title.clone()
        } else if self.unset() {
            t("sensor.shortcut.hint.empty")
        } else if cx.clicks() {
            t_args("sensor.shortcut.hint.open", &[("title", &self.title())])
        } else {
            self.title()
        };
        Some(HintContent::Text(text))
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        let props = &self.props;
        if self.has_url() {
            cx.platform.open(&props.url);
        } else if props.target.is_empty() {
            return ClickResult::None;
        } else if self.kind == Some(PathKind::Folder) {
            cx.platform.open(&props.target);
        } else {
            let dir = work_dir(&props.target, &props.work_dir);
            cx.platform.launch(&Launch {
                target: &props.target,
                args: &props.args,
                dir: dir.as_deref(),
                admin: props.run_as == RunAs::Administrator,
            });
        }
        ClickResult::Handled
    }

    /// While the target is missing; web addresses and bare program names are never checked.
    fn inactive(&self) -> bool {
        self.checks_target() && self.kind == Some(PathKind::Missing)
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.title()
    }

    fn topics(&self) -> Vec<TopicRequest> {
        if !self.checks_target() {
            return Vec::new();
        }
        vec![TopicRequest {
            topic: Topic::Path(self.props.target.clone()),
            interval: CHECK_EVERY,
        }]
    }

    fn on_sample(&mut self, topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let (Topic::Path(path), Sample::Path(sample)) = (topic, sample) else {
            return false;
        };
        if *path != self.props.target || self.kind == Some(sample.kind) {
            return false;
        }
        // A target that appeared (or changed) has another icon.
        self.refresh_icon = true;
        self.kind = Some(sample.kind);
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::bar::theme::Theme;
    use crate::config::model::{ClicksWhenLocked, Settings};
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::platform::{Platform, RectF};
    use crate::sampler::PathSample;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, theme};

    fn create(stored: serde_json::Value) -> Shortcut {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        Shortcut {
            props: typed(&props).expect("valid props"),
            kind: None,
            refresh_icon: false,
            custom: None,
            icon: None,
        }
    }

    /// Hands out a 24 px icon for `C:\Tools\app.exe` and nothing else; records what it is
    /// asked.
    #[derive(Default)]
    struct Icons {
        loading: bool,
        log: std::cell::RefCell<Vec<String>>,
    }

    impl IconSource for Icons {
        fn shell_icon(&self, path: &str, px: u32) -> ShellIcon {
            self.log.borrow_mut().push(format!("icon {path} {px}"));
            if self.loading {
                ShellIcon::Loading
            } else if path == "C:\\Tools\\app.exe" {
                let pixels = vec![255; (px * px * 4) as usize];
                ShellIcon::Ready(Bitmap::from_rgba(px, px, pixels).expect("a bitmap"))
            } else {
                ShellIcon::None
            }
        }

        fn forget(&self, path: &str) {
            self.log.borrow_mut().push(format!("forget {path}"));
        }
    }

    fn measure_with(sensor: &mut Shortcut, icons: &Icons, scale: f32) -> f32 {
        sensor.measure(&LayoutCtx {
            theme: &theme(),
            text: &FixedMetrics,
            scale,
            numbers: Default::default(),
            icons,
        })
    }

    fn drawn(sensor: &Shortcut) -> String {
        let mut canvas = RecordingCanvas::new(1.0);
        sensor.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme(),
                rect: RectF::new(7.0, 0.0, 31.0, 28.0),
            },
        );
        canvas.dump()
    }

    fn path(kind: PathKind) -> Sample {
        Sample::Path(PathSample { kind })
    }

    struct Harness {
        platform: FakeClock,
        settings: Settings,
        theme: Theme,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                platform: FakeClock::default(),
                settings: Settings::default(),
                theme: theme(),
            }
        }

        fn ctx(&self) -> SensorCtx<'_> {
            SensorCtx {
                id: "s_1",
                settings: &self.settings,
                theme: &self.theme,
                locked: self.settings.locked,
                alt: false,
                animate: false,
                platform: &self.platform as &dyn Platform,
            }
        }

        fn hint(&self, sensor: &Shortcut) -> String {
            match sensor.hint(&self.ctx()) {
                Some(HintContent::Text(text)) => text,
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn titles_and_names() {
        assert_eq!(file_name("C:\\Tools\\app.exe"), "app.exe");
        assert_eq!(file_name("C:\\Users\\me\\Downloads\\"), "Downloads");
        assert_eq!(file_name("C:\\"), "C:\\");
        assert_eq!(file_name("notepad.exe"), "notepad.exe");
        assert_eq!(auto_title("", "C:\\Tools\\app.exe", None), "app");
        assert_eq!(auto_title("", "C:\\Tools\\.config", None), ".config");
        assert_eq!(
            auto_title("", "D:\\Photos\\2026.10", Some(PathKind::Folder)),
            "2026.10"
        );
        assert_eq!(
            auto_title("https://example.com/", "C:\\x.exe", None),
            "https://example.com/"
        );
        // 150 characters stay; longer keeps 75 at each end.
        let exact = "a".repeat(150);
        assert_eq!(middle_truncated(&exact, 75), exact);
        let long = format!("https://{}{}", "b".repeat(100), "c".repeat(100));
        let title = auto_title(&long, "", None);
        assert_eq!(title.chars().count(), 151);
        assert!(title.starts_with("https://bbb"), "{title}");
        assert!(title.contains("b…c") && title.ends_with("cc"), "{title}");
    }

    #[test]
    fn working_folder() {
        assert_eq!(work_dir("C:\\Tools\\app.exe", ""), Some("C:\\Tools".into()));
        assert_eq!(work_dir("C:\\app.exe", ""), Some("C:\\".into()));
        assert_eq!(work_dir("C:\\app.exe", "D:\\Work"), Some("D:\\Work".into()));
        assert_eq!(work_dir("notepad.exe", ""), None);
        assert!(is_bare_name("notepad.exe"));
        assert!(!is_bare_name("C:\\Windows\\notepad.exe"));
    }

    #[test]
    fn shell_icon_at_bar_height_less_four() {
        let icons = Icons::default();
        let mut sensor = create(json!({ "target": "C:\\Tools\\app.exe" }));
        assert_eq!(measure_with(&mut sensor, &icons, 1.0), 24.0);
        assert_eq!(drawn(&sensor), "bitmap 24×24 into 7,2 24×24 opacity 1\n");
        // At 150 % the shell is asked for 36 physical px.
        assert_eq!(measure_with(&mut sensor, &icons, 1.5), 24.0);
        assert_eq!(
            *icons.log.borrow(),
            ["icon C:\\Tools\\app.exe 24", "icon C:\\Tools\\app.exe 36"]
        );
    }

    #[test]
    fn loading_keeps_the_place_free() {
        let icons = Icons {
            loading: true,
            ..Icons::default()
        };
        let mut sensor = create(json!({ "target": "C:\\Tools\\app.exe" }));
        assert_eq!(measure_with(&mut sensor, &icons, 1.0), 24.0);
        assert_eq!(drawn(&sensor), "");
    }

    #[test]
    fn web_addresses_show_the_globe_and_custom_colours_tint() {
        let icons = Icons::default();
        let mut sensor = create(json!({
            "url": "https://example.com/",
            "color": { "mode": "custom", "color": "#FF0000" },
            "opacity": 50,
        }));
        assert_eq!(measure_with(&mut sensor, &icons, 1.0), 24.0);
        assert!(icons.log.borrow().is_empty(), "no shell icon for a URL");
        assert!(drawn(&sensor).ends_with("opacity 0.5\n"));
        let bitmap = sensor.icon.as_ref().expect("an icon").bitmap();
        assert!(
            bitmap
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| px[1] == 0 && px[2] == 0)
        );
        assert!(!sensor.inactive());
        assert!(sensor.topics().is_empty());
    }

    #[test]
    fn missing_target_is_inactive_with_the_generic_icon() {
        let h = Harness::new();
        let icons = Icons::default();
        let mut sensor = create(json!({ "target": "Q:\\Gone\\tool.exe" }));
        assert_eq!(
            sensor.topics(),
            [TopicRequest {
                topic: Topic::Path("Q:\\Gone\\tool.exe".into()),
                interval: CHECK_EVERY,
            }]
        );
        assert!(!sensor.inactive(), "not known yet");
        let topic = Topic::Path("Q:\\Gone\\tool.exe".into());
        assert!(sensor.on_sample(&topic, &path(PathKind::Missing), &h.ctx()));
        assert!(!sensor.on_sample(&topic, &path(PathKind::Missing), &h.ctx()));
        assert!(sensor.inactive());
        measure_with(&mut sensor, &icons, 1.0);
        // The generic icon, faded to the inactive opacity (78 %).
        assert_eq!(drawn(&sensor), "bitmap 24×24 into 7,2 24×24 opacity 0.78\n");
        // It comes back: its icon is read again, and it is active.
        assert!(sensor.on_sample(&topic, &path(PathKind::File), &h.ctx()));
        measure_with(&mut sensor, &icons, 1.0);
        assert_eq!(
            icons.log.borrow().last().map(String::as_str),
            Some("icon Q:\\Gone\\tool.exe 24")
        );
        assert!(
            icons
                .log
                .borrow()
                .contains(&"forget Q:\\Gone\\tool.exe".into())
        );
        assert!(!sensor.inactive());
        // A sample of another path is not for it.
        assert!(!sensor.on_sample(
            &Topic::Path("C:\\".into()),
            &path(PathKind::Missing),
            &h.ctx()
        ));
    }

    #[test]
    fn bare_program_names_are_not_checked() {
        let sensor = create(json!({ "target": "notepad.exe" }));
        assert!(sensor.topics().is_empty());
        assert!(!sensor.inactive());
        assert!(
            !create(json!({})).inactive(),
            "nothing set, nothing missing"
        );
    }

    #[test]
    fn hints() {
        let mut h = Harness::new();
        let mut sensor = create(json!({ "target": "C:\\Tools\\app.exe" }));
        assert_eq!(h.hint(&sensor), "Open 'app'");
        let topic = Topic::Path("C:\\Tools\\app.exe".into());
        sensor.on_sample(&topic, &path(PathKind::File), &h.ctx());
        assert_eq!(sensor.accessible_value(&h.ctx()), "app");
        // A set title is the whole hint.
        assert_eq!(h.hint(&create(json!({ "title": "My tool" }))), "My tool");
        assert_eq!(h.hint(&create(json!({}))), "No target set");
        // Clicks need Alt: only the title.
        h.settings.locked = true;
        h.settings.clicks_when_locked = ClicksWhenLocked::AltOnly;
        assert_eq!(h.hint(&sensor), "app");
    }

    #[test]
    fn clicks_start_the_target_with_its_parameters() {
        let h = Harness::new();
        let mut file = create(json!({
            "target": "C:\\Tools\\app.exe", "args": "--fast \"a b\"", "runAs": "administrator"
        }));
        assert_eq!(file.click(&h.ctx()), ClickResult::Handled);
        let mut folder = create(json!({ "target": "D:\\Photos", "args": "ignored" }));
        let topic = Topic::Path("D:\\Photos".into());
        folder.on_sample(&topic, &path(PathKind::Folder), &h.ctx());
        assert_eq!(folder.click(&h.ctx()), ClickResult::Handled);
        let mut url = create(json!({ "url": "ms-settings:display", "target": "C:\\x.exe" }));
        assert_eq!(url.click(&h.ctx()), ClickResult::Handled);
        let mut dir = create(json!({ "target": "C:\\Tools\\app.exe", "workDir": "E:\\Work" }));
        dir.click(&h.ctx());
        assert_eq!(create(json!({})).click(&h.ctx()), ClickResult::None);
        assert_eq!(
            h.platform.log(),
            [
                "launch C:\\Tools\\app.exe args=--fast \"a b\" dir=C:\\Tools admin=true",
                "open D:\\Photos",
                "open ms-settings:display",
                "launch C:\\Tools\\app.exe args= dir=E:\\Work admin=false",
            ]
        );
    }

    #[test]
    fn summary() {
        let summary = |stored: serde_json::Value| {
            let stored = stored.as_object().cloned().unwrap_or_default();
            ShortcutType.summary(&resolve(SCHEMA, &stored, &Props::new()).0)
        };
        assert_eq!(summary(json!({})), None);
        assert_eq!(
            summary(json!({ "target": "C:\\Tools\\app.exe" })),
            Some("app.exe".into())
        );
        assert_eq!(
            summary(json!({ "target": "C:\\x.exe", "url": "https://example.com/" })),
            Some("https://example.com/".into())
        );
        assert_eq!(
            summary(json!({ "target": "C:\\x.exe", "title": "X" })),
            Some("X".into())
        );
    }
}
