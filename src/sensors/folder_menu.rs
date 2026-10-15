//! `folder-menu` — Folder Menu (SPEC §9.19): a folder icon that opens a popup listing the
//! folder's contents (`common/folder_browser.rs`), lined up with the sensor's left edge.

use serde::Deserialize;
use serde_json::Value;

use super::common::folder_browser::{FolderBrowser, Options};
use super::common::icon::{self, Icon, ImageFile};
use super::drop::{self, DragInfo, DropAction, Dropped};
use super::schema::{FieldKind, Lit, PropField};
use super::shortcut::file_name;
use super::{
    Category, ClickResult, ColorChoice, DrawCtx, HintContent, LayoutCtx, MenuId, MenuItem, Props,
    Sensor, SensorCtx, SensorType, State, typed,
};
use crate::i18n::t;
use crate::platform::{Canvas, DropEffect, FileOp};

/// The built-in folder icon (Appendix C).
const FOLDER: &[u8] = include_bytes!("../../assets/icons/folder.svg");
/// The icon's height, logical px (SPEC §9.19).
const ICON: f32 = 21.0;
/// The *Open folder* menu item.
const OPEN: MenuId = 0;

const SCHEMA: &[PropField] = &[
    PropField::new(
        "folder",
        FieldKind::Folder,
        "sensor.folderMenu.field.folder",
        Lit::Str(""),
    )
    .describe("sensor.folderMenu.field.folder.help"),
    PropField::new(
        "title",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.folderMenu.field.title",
        Lit::Str(""),
    )
    .describe("sensor.folderMenu.field.title.help"),
    PropField::new(
        "image",
        FieldKind::File {
            extensions: &["ico", "png", "gif", "jpg", "jpeg", "svg"],
        },
        "sensor.shortcut.field.image",
        Lit::Str(""),
    )
    .describe("sensor.folderMenu.field.image.help"),
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
    PropField::new(
        "showHidden",
        FieldKind::Bool,
        "sensor.folderMenu.field.showHidden",
        Lit::Bool(false),
    )
    .describe("sensor.folderMenu.field.showHidden.help"),
    PropField::new(
        "popupWidth",
        FieldKind::Hidden,
        "sensor.folderMenu.field.popupWidth",
        Lit::Int(240),
    ),
    PropField::new(
        "popupHeight",
        FieldKind::Hidden,
        "sensor.folderMenu.field.popupHeight",
        Lit::Int(320),
    ),
];

pub struct FolderMenuType;

impl SensorType for FolderMenuType {
    fn id(&self) -> &'static str {
        "folder-menu"
    }

    fn category(&self) -> Category {
        Category::Launchers
    }

    fn name(&self) -> &'static str {
        "sensor.folderMenu.name"
    }

    fn description(&self) -> &'static str {
        "sensor.folderMenu.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["TurboFolder"]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(FolderMenu {
            props: typed(props)?,
            image: ImageFile::default(),
            icon: None,
        }))
    }

    /// The title, or the folder's name.
    fn summary(&self, props: &Props) -> Option<String> {
        let text = |key: &str| props.get(key).and_then(Value::as_str).unwrap_or_default();
        let summary = match text("title") {
            "" => file_name(text("folder")),
            title => title,
        };
        (!summary.is_empty()).then(|| summary.to_owned())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderProps {
    folder: String,
    /// Empty: the folder's name.
    title: String,
    /// Empty: the built-in folder icon.
    image: String,
    /// Custom tints the icon (SPEC §8.4).
    color: ColorChoice,
    /// Percent.
    opacity: u32,
    show_hidden: bool,
    /// The popup's size, logical px.
    popup_width: Value,
    popup_height: Value,
}

struct FolderMenu {
    props: FolderProps,
    image: ImageFile,
    icon: Option<Icon>,
}

/// What a drop into the folder does (SPEC §9.19).
#[derive(Debug, Clone, PartialEq)]
enum DropPlan {
    /// Internet shortcuts (`.url`) to the links.
    Links,
    /// Copy, move or make shortcuts to the files and folders.
    Files(FileOp, Vec<String>),
    /// Nothing can be done: an effect the source does not allow, or a folder into itself.
    Refused,
}

impl FolderMenu {
    /// What dropping `items` would do; `None` when they are not all files and folders or all
    /// links, so the bar inserts new sensors instead.
    fn drop_plan(&self, items: &[Dropped], drag: DragInfo, cx: &SensorCtx) -> Option<DropPlan> {
        if self.unset() || items.is_empty() {
            return None;
        }
        let folder = &self.props.folder;
        if items.iter().all(|i| matches!(i, Dropped::Url { .. })) {
            let allowed = drag.allowed.link || drag.allowed.copy;
            return Some(if allowed {
                DropPlan::Links
            } else {
                DropPlan::Refused
            });
        }
        let paths: Vec<String> = items
            .iter()
            .map(|i| i.path().map(str::to_owned))
            .collect::<Option<_>>()?;
        let volume = cx.platform.volume_of(folder);
        let same_volume =
            volume.is_some() && paths.iter().all(|p| cx.platform.volume_of(p) == volume);
        let Some(op) = drop::file_op(drag.modifiers, same_volume, drag.allowed) else {
            return Some(DropPlan::Refused);
        };
        // A folder into itself, or items moved to where they are: nothing to do.
        let pointless = op != FileOp::Link
            && paths.iter().any(|p| {
                drop::is_within(folder, p)
                    || (op == FileOp::Move && drop::same_folder(drop::parent(p), folder))
            });
        Some(if pointless {
            DropPlan::Refused
        } else {
            DropPlan::Files(op, paths)
        })
    }

    /// The title, or the folder's name (`Downloads`, `C:\`).
    fn title(&self) -> String {
        if self.props.title.is_empty() {
            file_name(&self.props.folder).to_owned()
        } else {
            self.props.title.clone()
        }
    }

    fn unset(&self) -> bool {
        self.props.folder.is_empty()
    }

    /// A stored size, or `default` if it is not a number.
    fn size(value: &Value, default: f32) -> f32 {
        value.as_f64().map_or(default, |v| v as f32)
    }
}

impl Sensor for FolderMenu {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let px = icon::physical(ICON, cx.scale);
        let opacity = self.props.opacity.min(100) as f32 / 100.0;
        let bitmap = match self.image.get(&self.props.image, px) {
            Some(image) => Some((image, false)),
            None => Icon::svg(FOLDER, px).map(|b| (b, true)),
        };
        self.icon = bitmap.map(|(bitmap, built_in)| {
            let tint = icon::tint(&self.props.color, built_in, cx.theme);
            Icon::new(bitmap, tint, opacity, None)
        });
        self.icon.as_ref().map_or(ICON, |i| i.width(cx.scale))
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        if let Some(icon) = &self.icon {
            icon.draw(canvas, cx);
        }
    }

    /// The title (the bar shows no hints while the popup is open).
    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        Some(HintContent::Text(if self.unset() {
            t("sensor.folderMenu.hint.empty")
        } else {
            self.title()
        }))
    }

    /// Opens the `folder-browser` popup (SPEC §9.19).
    fn click(&mut self, _cx: &SensorCtx) -> ClickResult {
        if self.unset() {
            return ClickResult::None;
        }
        ClickResult::Popup(Box::new(FolderBrowser::new(Options {
            folders: vec![self.props.folder.clone()],
            title: self.title(),
            show_hidden: self.props.show_hidden,
            width: Self::size(&self.props.popup_width, 240.0),
            height: Self::size(&self.props.popup_height, 320.0),
        })))
    }

    fn menu(&self, _cx: &SensorCtx) -> Vec<MenuItem> {
        vec![MenuItem::new(OPEN, t("sensor.folderMenu.menu.open")).enabled(!self.unset())]
    }

    fn on_menu(&mut self, item: MenuId, cx: &SensorCtx) -> bool {
        if item == OPEN && !self.unset() {
            cx.platform.open(&self.props.folder);
        }
        false
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.title()
    }

    /// Files and folders are copied, moved or linked into the folder, links become Internet
    /// shortcuts (SPEC §9.19).
    fn drop_action(&self, items: &[Dropped], drag: DragInfo, cx: &SensorCtx) -> Option<DropAction> {
        let title = self.title();
        Some(match self.drop_plan(items, drag, cx)? {
            DropPlan::Links => DropAction {
                effect: if drag.allowed.link {
                    DropEffect::Link
                } else {
                    DropEffect::Copy
                },
                hint: Some(drop::hint("drop.urlShortcut", items, &title)),
            },
            DropPlan::Files(op, _) => {
                let key = match op {
                    FileOp::Copy => "drop.copy",
                    FileOp::Move => "drop.move",
                    FileOp::Link => "drop.link",
                };
                DropAction {
                    effect: op.effect(),
                    hint: Some(drop::hint(key, items, &title)),
                }
            }
            DropPlan::Refused => DropAction::REFUSED,
        })
    }

    fn drop_items(&mut self, items: &[Dropped], drag: DragInfo, cx: &SensorCtx) {
        let folder = &self.props.folder;
        match self.drop_plan(items, drag, cx) {
            Some(DropPlan::Links) => {
                for item in items {
                    if let Dropped::Url { url, title } = item {
                        let stem = drop::url_file_stem(url, title.as_deref());
                        cx.platform.create_url_file(folder, &stem, url);
                    }
                }
            }
            Some(DropPlan::Files(op, paths)) => {
                cx.platform
                    .file_operation(op, paths, folder.clone(), Box::new(|| {}));
            }
            Some(DropPlan::Refused) | None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::model::Settings;
    use crate::platform::Platform;
    use crate::sampler::ChangeSink;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, draw, measure, theme};

    fn create(stored: serde_json::Value) -> FolderMenu {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        FolderMenu {
            props: typed(&props).expect("valid props"),
            image: ImageFile::default(),
            icon: None,
        }
    }

    fn with_ctx<R>(f: impl FnOnce(&SensorCtx, &FakeClock) -> R) -> R {
        let platform = FakeClock::default();
        let settings = Settings::default();
        let theme = theme();
        let cx = SensorCtx {
            id: "s_1",
            settings: &settings,
            theme: &theme,
            locked: false,
            alt: false,
            animate: false,
            platform: &platform as &dyn Platform,
            changes: &ChangeSink::NONE,
        };
        f(&cx, &platform)
    }

    #[test]
    fn a_folder_icon_that_opens_the_browser() {
        let mut menu = create(json!({ "folder": "C:\\Users\\me\\Downloads" }));
        assert_eq!(measure(&mut menu), 21.0);
        assert_eq!(draw(&menu, 21.0), "bitmap 21×21 into 7,4 21×21 opacity 1\n");
        with_ctx(|cx, platform| {
            assert_eq!(menu.hint(cx), Some(HintContent::Text("Downloads".into())));
            match menu.click(cx) {
                ClickResult::Popup(popup) => assert_eq!(popup.kind(), "folder-browser"),
                other => panic!("{other:?}"),
            }
            assert_eq!(menu.menu(cx), [MenuItem::new(OPEN, "Open folder")]);
            menu.on_menu(OPEN, cx);
            assert_eq!(platform.log(), ["open C:\\Users\\me\\Downloads"]);
        });
        assert_eq!(
            create(json!({ "folder": "D:\\", "title": "Data" })).title(),
            "Data"
        );
        assert_eq!(create(json!({ "folder": "D:\\" })).title(), "D:\\");
    }

    #[test]
    fn without_a_folder_nothing_opens() {
        let mut menu = create(json!({}));
        with_ctx(|cx, _| {
            assert_eq!(menu.click(cx), ClickResult::None);
            assert_eq!(
                menu.hint(cx),
                Some(HintContent::Text("No folder set".into()))
            );
            assert!(!menu.menu(cx)[0].enabled);
        });
        assert_eq!(FolderMenuType.summary(&Props::new()), None);
    }

    #[test]
    fn drops_copy_move_link_or_make_url_files() {
        use crate::platform::{DropEffects, Modifiers};
        let mut menu = create(json!({ "folder": "C:\\Users\\me\\Downloads" }));
        let all = DragInfo {
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        let file = |p: &str| Dropped::Path(p.into());
        with_ctx(|cx, platform| {
            // Same volume: moved.
            let local = [file("C:\\Data\\report.pdf")];
            assert_eq!(
                menu.drop_action(&local, all, cx),
                Some(DropAction {
                    effect: DropEffect::Move,
                    hint: Some("Move 'report.pdf' to 'Downloads'".into())
                })
            );
            // Another volume: copied; with Alt: shortcuts.
            let usb = [file("E:\\a.txt"), Dropped::Path("E:\\Photos".into())];
            assert_eq!(
                menu.drop_action(&usb, all, cx).and_then(|a| a.hint),
                Some("Copy 2 items to 'Downloads'".into())
            );
            let alt = DragInfo {
                modifiers: Modifiers {
                    alt: true,
                    ..Modifiers::default()
                },
                ..all
            };
            assert_eq!(
                menu.drop_action(&usb, alt, cx).map(|a| a.effect),
                Some(DropEffect::Link)
            );
            // Already there, or the folder itself: refused.
            let here = [file("C:\\Users\\me\\Downloads\\x.zip")];
            assert_eq!(menu.drop_action(&here, all, cx), Some(DropAction::REFUSED));
            let parent = [Dropped::Path("C:\\Users\\me".into())];
            assert_eq!(
                menu.drop_action(&parent, all, cx),
                Some(DropAction::REFUSED)
            );
            // A drive among them: the bar inserts sensors instead.
            let drive = [file("C:\\a"), Dropped::Drive("E:\\".into())];
            assert_eq!(menu.drop_action(&drive, all, cx), None);
            let link = [Dropped::Url {
                url: "https://example.com/".into(),
                title: Some("Example Domain".into()),
            }];
            assert_eq!(
                menu.drop_action(&link, all, cx).and_then(|a| a.hint),
                Some("Create shortcut to 'https://example.com/' in 'Downloads'".into())
            );
            menu.drop_items(&local, all, cx);
            menu.drop_items(&usb, alt, cx);
            menu.drop_items(&link, all, cx);
            menu.drop_items(&here, all, cx);
            assert_eq!(
                platform.log(),
                [
                    "Move C:\\Data\\report.pdf into C:\\Users\\me\\Downloads",
                    "Link E:\\a.txt, E:\\Photos into C:\\Users\\me\\Downloads",
                    "url file C:\\Users\\me\\Downloads\\Example Domain.url https://example.com/",
                ]
            );
        });
    }

    #[test]
    fn summary_is_the_title_or_the_folder() {
        let props = |v: serde_json::Value| v.as_object().cloned().unwrap_or_default();
        assert_eq!(
            FolderMenuType.summary(&props(json!({ "folder": "C:\\Tools", "title": "" }))),
            Some("Tools".into())
        );
        assert_eq!(
            FolderMenuType.summary(&props(json!({ "folder": "C:\\Tools", "title": "Kit" }))),
            Some("Kit".into())
        );
    }
}
