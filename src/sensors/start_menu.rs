//! `start-menu` — Start Menu (SPEC §9.20): a Folder Menu whose list merges the user's and all
//! users' Start Menu programs (`common/folder_browser.rs`).

use serde::Deserialize;
use serde_json::Value;

use super::common::folder_browser::{FolderBrowser, Options};
use super::common::icon::{self, Icon};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, ClickResult, ColorChoice, DrawCtx, Fetched, HintContent, LayoutCtx, MenuId, MenuItem,
    Props, Sensor, SensorCtx, SensorType, State, typed,
};
use crate::i18n::t;
use crate::platform::{Canvas, Capability, KnownFolder, Platform};

/// The built-in Start Menu icon (Appendix C).
const START_MENU: &[u8] = include_bytes!("../../assets/icons/start-menu.svg");
/// The icon's height, logical px (as the Folder Menu's, SPEC §9.19).
const ICON: f32 = 21.0;
/// The menu items: the user's Programs folder, and all users'.
const OPEN: MenuId = 0;
const OPEN_ALL: MenuId = 1;

const SCHEMA: &[PropField] = &[
    PropField::new(
        "title",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.folderMenu.field.title",
        Lit::Str(""),
    )
    .describe("sensor.startMenu.field.title.help"),
    PropField::new(
        "image",
        FieldKind::File {
            extensions: &["ico", "png", "gif", "jpg", "jpeg", "svg"],
        },
        "sensor.shortcut.field.image",
        Lit::Str(""),
    )
    .describe("sensor.startMenu.field.image.help"),
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

/// SPEC §9.20: the Folder Menu's keys; the folders are this computer's Start Menu.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::ignored("folder"),
    LegacyKey::new("text", "title"),
    LegacyKey::new("image", "image"),
    LegacyKey::new("colorize", "color"),
    LegacyKey::new("opacity", "opacity"),
    LegacyKey::new("popup-width", "popupWidth"),
    LegacyKey::new("popup-height", "popupHeight"),
];

pub struct StartMenuType;

impl SensorType for StartMenuType {
    fn id(&self) -> &'static str {
        "start-menu"
    }

    fn category(&self) -> Category {
        Category::Launchers
    }

    fn name(&self) -> &'static str {
        "sensor.startMenu.name"
    }

    fn description(&self) -> &'static str {
        "sensor.startMenu.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["TurboFolderStartMenu"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn requires(&self) -> &'static [Capability] {
        &[Capability::StartMenu]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(StartMenu {
            props: typed(props)?,
            icon: None,
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        props
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartMenuProps {
    /// Empty: "Start Menu".
    title: String,
    /// Empty: the built-in Start Menu icon.
    image: String,
    /// Custom tints the icon (SPEC §8.4).
    color: ColorChoice,
    /// Percent.
    opacity: u32,
    /// The popup's size, logical px.
    popup_width: Value,
    popup_height: Value,
}

struct StartMenu {
    props: StartMenuProps,
    icon: Option<Icon>,
}

impl StartMenu {
    fn title(&self) -> String {
        if self.props.title.is_empty() {
            t("sensor.startMenu.name")
        } else {
            self.props.title.clone()
        }
    }

    /// The Start Menu's program folders, the user's first (theirs win, SPEC §9.20).
    fn folders(platform: &dyn Platform) -> Vec<String> {
        [KnownFolder::Programs, KnownFolder::CommonPrograms]
            .into_iter()
            .filter_map(|f| platform.known_folder(f))
            .collect()
    }

    /// A stored size, or `default` if it is not a number.
    fn size(value: &Value, default: f32) -> f32 {
        value.as_f64().map_or(default, |v| v as f32)
    }
}

impl Sensor for StartMenu {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let px = icon::physical(ICON, cx.scale);
        let opacity = self.props.opacity.min(100) as f32 / 100.0;
        let bitmap = match icon::custom_image(&self.props.image, px, cx.icons) {
            Fetched::Ready(image) => Some((image, false)),
            // Its square stays free until it arrives.
            Fetched::Loading => None,
            Fetched::None => Icon::svg(START_MENU, px).map(|b| (b, true)),
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

    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        Some(HintContent::Text(self.title()))
    }

    /// Opens the `folder-browser` popup on the merged program folders (SPEC §9.20).
    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        let folders = Self::folders(cx.platform);
        if folders.is_empty() {
            return ClickResult::None;
        }
        ClickResult::Popup(Box::new(FolderBrowser::new(Options {
            folders,
            title: self.title(),
            show_hidden: false,
            width: Self::size(&self.props.popup_width, 240.0),
            height: Self::size(&self.props.popup_height, 320.0),
        })))
    }

    fn menu(&self, cx: &SensorCtx) -> Vec<MenuItem> {
        let known = |f| cx.platform.known_folder(f).is_some();
        vec![
            MenuItem::new(OPEN, t("sensor.folderMenu.menu.open"))
                .enabled(known(KnownFolder::Programs)),
            MenuItem::new(OPEN_ALL, t("sensor.startMenu.menu.openAll"))
                .enabled(known(KnownFolder::CommonPrograms)),
        ]
    }

    fn on_menu(&mut self, item: MenuId, cx: &SensorCtx) -> bool {
        let folder = match item {
            OPEN => KnownFolder::Programs,
            OPEN_ALL => KnownFolder::CommonPrograms,
            _ => return false,
        };
        if let Some(path) = cx.platform.known_folder(folder) {
            cx.platform.open(&path);
        }
        false
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.title()
    }

    fn has_popup(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::model::Settings;
    use crate::sampler::ChangeSink;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, draw, measure, theme};

    fn create(stored: serde_json::Value) -> StartMenu {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        StartMenu {
            props: typed(&props).expect("valid props"),
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
    fn an_icon_that_opens_the_merged_programs() {
        let mut menu = create(json!({}));
        assert_eq!(measure(&mut menu), 21.0);
        assert_eq!(draw(&menu, 21.0), "bitmap 21×21 into 7,4 21×21 opacity 1\n");
        with_ctx(|cx, platform| {
            assert_eq!(menu.hint(cx), Some(HintContent::Text("Start Menu".into())));
            match menu.click(cx) {
                ClickResult::Popup(popup) => assert_eq!(popup.kind(), "folder-browser"),
                other => panic!("{other:?}"),
            }
            assert_eq!(
                menu.menu(cx),
                [
                    MenuItem::new(OPEN, "Open folder"),
                    MenuItem::new(OPEN_ALL, "Open (all users) folder"),
                ]
            );
            menu.on_menu(OPEN, cx);
            menu.on_menu(OPEN_ALL, cx);
            assert_eq!(
                platform.log(),
                [
                    "open C:\\Users\\me\\Programs",
                    "open C:\\ProgramData\\Programs"
                ]
            );
        });
        assert_eq!(create(json!({ "title": "Apps" })).title(), "Apps");
        assert_eq!(StartMenuType.summary(&Props::new()), None);
    }
}
