//! `folder-menu` — Folder Menu (SPEC §9.19): a folder icon that opens a popup listing the
//! folder's contents (`common/folder_browser.rs`), lined up with the sensor's left edge.

use serde::Deserialize;
use serde_json::Value;

use super::common::folder_browser::{FolderBrowser, Options};
use super::common::icon::{self, Icon, ImageFile};
use super::schema::{FieldKind, Lit, PropField};
use super::shortcut::file_name;
use super::{
    Category, ClickResult, ColorChoice, DrawCtx, HintContent, LayoutCtx, MenuId, MenuItem, Props,
    Sensor, SensorCtx, SensorType, State, typed,
};
use crate::i18n::t;
use crate::platform::Canvas;

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

impl FolderMenu {
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
            folder: self.props.folder.clone(),
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
