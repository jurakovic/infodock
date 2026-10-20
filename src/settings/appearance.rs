//! Settings ▸ Appearance (SPEC §10.3, §10.7): the theme, the bar's background and, for the
//! classic theme, the skin gallery with previews rendered by the bar's renderer
//! (`skin_preview`), the custom image and a notice when the configured skin is missing.

use std::sync::Arc;

use serde_json::Value;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{Open, WINDOW, model, save};
use crate::app::App;
use crate::bar::skins::pack::{self, PackEntry};
use crate::bar::skins::{self, BuiltIn, CUSTOM};
use crate::bar::theme::Theme;
use crate::config::model::{ClassicSettings, Settings, Theme as ThemeId, ThemeDefaults};
use crate::i18n::{t, t_args};
use crate::platform::{Bitmap, PathRequest};
use crate::ui;

/// The page index of Appearance in the window.
pub(super) const PAGE: i32 = 2;
/// The previews' width, logical px.
const PREVIEW_WIDTH: f32 = 160.0;
/// The file types the custom image may have (SPEC §10.3).
const IMAGE_TYPES: &[&str] = &["png", "jpg", "jpeg"];

/// A skin the gallery offers.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinEntry {
    /// What `classic.skin` stores.
    pub id: String,
    pub name: String,
}

/// The gallery: the built-in skins, the custom image, then the packs that can be used, by name
/// (SPEC §12.4).
pub fn entries(packs: &[PackEntry]) -> Vec<SkinEntry> {
    let built_in = BuiltIn::ALL
        .iter()
        .map(|skin| skin.id())
        .chain([CUSTOM])
        .map(|id| SkinEntry {
            id: id.into(),
            name: t(&format!("skins.name.{id}")),
        });
    let installed = packs.iter().filter_map(|entry| {
        let pack = entry.pack.as_ref().ok()?;
        Some(SkinEntry {
            id: pack.id.clone(),
            name: pack.name.clone(),
        })
    });
    built_in.chain(installed).collect()
}

/// Why the bar does not draw the configured skin `id`: it is not installed, or its pack cannot
/// be used, so Gloss stands in (SPEC §12.4); `None` when it is drawn.
pub fn notice(id: &str, packs: &[PackEntry]) -> Option<String> {
    if BuiltIn::from_id(id).is_some() || id == CUSTOM {
        return None;
    }
    match packs.iter().find(|entry| entry.id == id).map(|e| &e.pack) {
        Some(Ok(_)) => None,
        Some(Err(e)) => Some(t_args(
            "settings.appearance.skin.unusable",
            &[("name", id), ("reason", &e.message())],
        )),
        None => Some(t_args("settings.appearance.skin.missing", &[("name", id)])),
    }
}

/// The settings a skin's preview is drawn with: the classic theme with that skin, and what of
/// `settings` the bar's background depends on.
fn preview_settings(settings: &Settings, id: &str) -> Settings {
    let mut preview = Settings {
        theme: ThemeId::Classic,
        background: settings.background.clone(),
        classic: ClassicSettings {
            skin: id.into(),
            // Only the custom image's preview depends on it.
            custom_skin: if id == CUSTOM {
                settings.classic.custom_skin.clone()
            } else {
                String::new()
            },
        },
        ..Settings::default()
    };
    preview.bar.height = settings.bar.height;
    preview.bar.edge = settings.bar.edge;
    preview
}

/// What a preview was drawn from, so it is drawn again only when that changed.
#[derive(Debug, Clone, PartialEq)]
struct PreviewKey {
    settings: Settings,
    /// The skin's pack as listed.
    pack: Option<PackEntry>,
    scale: f32,
}

#[derive(Clone)]
enum Preview {
    Image(slint::Image),
    /// Where there is no renderer: the skin's colour.
    Swatch(slint::Color),
}

/// The gallery as last shown.
#[derive(Default)]
pub(super) struct Gallery {
    /// The skins folder as last listed.
    packs: Vec<PackEntry>,
    listed: bool,
    entries: Vec<SkinEntry>,
    previews: Vec<(PreviewKey, Preview)>,
}

impl Gallery {
    /// Lists the skins folder again, e.g. when the page is shown.
    pub(super) fn refresh(&mut self, app: &App) {
        self.packs = pack::list(&app.skins);
        self.listed = true;
    }
}

/// A premultiplied bitmap as a Slint image.
fn to_image(bitmap: &Bitmap) -> slint::Image {
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        bitmap.pixels(),
        bitmap.width(),
        bitmap.height(),
    );
    slint::Image::from_rgba8_premultiplied(buffer)
}

fn preview(app: &App, key: &PreviewKey) -> Preview {
    let os = app.platform.appearance();
    match crate::bar::skin_preview(&key.settings, os, &app.skins, PREVIEW_WIDTH, key.scale) {
        Ok(bitmap) => Preview::Image(to_image(&bitmap)),
        Err(_) => {
            let c = Theme::resolve(&key.settings, os).bar_background;
            Preview::Swatch(slint::Color::from_argb_f32(c.a, c.r, c.g, c.b))
        }
    }
}

/// Copies the Appearance settings into the window; the gallery's previews are drawn again only
/// where what they show changed.
pub(super) fn fill(open: &mut Open, app: &App, settings: &Settings) {
    let state = open.window.global::<ui::SettingsState>();
    let json = serde_json::to_value(settings).unwrap_or_default();
    let choice = |path: &str| model::choice(path).map_or(0, |c| model::choice_index(&json, c));
    let defaults = ThemeDefaults::of(settings.theme);
    let background = &settings.background;

    state.set_theme(choice("theme"));
    state.set_background_mode(choice("background.colorMode"));
    state.set_background_color(
        background
            .color
            .as_deref()
            .unwrap_or(&defaults.background_color)
            .into(),
    );
    state.set_background_color_customised(background.color.is_some());
    state.set_background_color_default(defaults.background_color.as_str().into());
    state.set_background_opacity(background.opacity.unwrap_or(defaults.background_opacity) as f32);
    state.set_background_opacity_customised(background.opacity.is_some());
    state.set_background_opacity_default(defaults.background_opacity as i32);
    let id = |value| {
        serde_json::to_value(value)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
    };
    let labels: Vec<SharedString> =
        model::material_labels(&id(defaults.background_material).unwrap_or_default())
            .iter()
            .map(SharedString::from)
            .collect();
    state.set_materials(ModelRc::new(VecModel::from(labels)));
    state.set_material(model::material_index(
        background.material.and_then(id).as_deref(),
    ));
    state.set_material_available(app.platform.appearance().transparency);

    let classic = &settings.classic;
    state.set_custom_skin(classic.custom_skin.as_str().into());
    state.set_custom_skin_shown(classic.skin == CUSTOM);
    let custom_error = match classic.custom_skin.as_str() {
        "" => String::new(),
        path => skins::read_custom(path)
            .err()
            .map_or_else(String::new, |e| e.message()),
    };
    state.set_custom_skin_error(custom_error.into());
    state.set_skin_preview_height(settings.bar_height() as f32);

    // The gallery only shows with the classic theme.
    if settings.theme != ThemeId::Classic {
        state.set_skin_notice(SharedString::new());
        state.set_skins(ModelRc::default());
        return;
    }
    let gallery = &mut open.gallery;
    if !gallery.listed {
        gallery.refresh(app);
    }
    state.set_skin_notice(
        notice(&classic.skin, &gallery.packs)
            .unwrap_or_default()
            .into(),
    );
    gallery.entries = entries(&gallery.packs);
    let scale = open.window.window().scale_factor();
    let mut previews = Vec::with_capacity(gallery.entries.len());
    let mut tiles = Vec::with_capacity(gallery.entries.len());
    for entry in &gallery.entries {
        let key = PreviewKey {
            settings: preview_settings(settings, &entry.id),
            pack: gallery.packs.iter().find(|p| p.id == entry.id).cloned(),
            scale,
        };
        let shown = gallery
            .previews
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, p)| p.clone());
        let image = shown.unwrap_or_else(|| preview(app, &key));
        let (preview, has_preview, swatch) = match &image {
            Preview::Image(image) => (image.clone(), true, slint::Color::default()),
            Preview::Swatch(color) => (slint::Image::default(), false, *color),
        };
        tiles.push(ui::SkinTile {
            name: entry.name.as_str().into(),
            preview,
            has_preview,
            swatch,
            selected: entry.id == classic.skin,
        });
        previews.push((key, image));
    }
    gallery.previews = previews;
    let selected = gallery.entries.iter().position(|e| e.id == classic.skin);
    state.set_skin_selected(selected.map_or(-1, |i| i as i32));
    state.set_skins(ModelRc::new(VecModel::from(tiles)));
}

/// Lets the user pick the custom image.
fn browse(app: &Arc<App>) {
    let owner = WINDOW.with(|w| {
        let w = w.try_borrow().ok()?;
        super::properties::owner(&w.as_ref()?.window)
    });
    let (title, types_name, all_files) = (
        t("settings.props.pickFile"),
        t("settings.props.supportedFiles"),
        t("settings.props.allFiles"),
    );
    // The picker runs its own modal loop; no window state is borrowed meanwhile.
    let picked = crate::platform::pick_path(
        owner,
        &PathRequest {
            title: &title,
            folder: false,
            extensions: IMAGE_TYPES,
            types_name: &types_name,
            all_files: &all_files,
        },
    );
    if let Some(path) = picked {
        save(app, model::patch("classic.customSkin", Value::String(path)));
    }
}

/// Handles the page's callbacks.
pub(super) fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    let state = window.global::<ui::SettingsState>();
    let a = Arc::clone(app);
    state.on_set_material(move |index| {
        let material = usize::try_from(index)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| model::MATERIALS.get(i));
        save(
            &a,
            model::patch(
                "background.material",
                material.map_or(Value::Null, |m| Value::from(*m)),
            ),
        );
    });
    let a = Arc::clone(app);
    state.on_set_skin(move |index| {
        let id = WINDOW.with(|w| {
            let w = w.borrow();
            let open = w.as_ref()?;
            let index = usize::try_from(index).ok()?;
            open.gallery.entries.get(index).map(|e| e.id.clone())
        });
        if let Some(id) = id {
            save(&a, model::patch("classic.skin", Value::String(id)));
        }
    });
    let a = Arc::clone(app);
    state.on_browse_custom_skin(move || browse(&a));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::skins::tests::pack as write_pack;

    fn ids(entries: &[SkinEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn gallery_lists_built_in_skins_then_usable_packs() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_pack(dir.path(), "broken", "{");
        let ok = write_pack(
            dir.path(),
            "ocean",
            r#"{ "name": "Ocean blue", "image": "t.png" }"#,
        );
        crate::bar::skins::tests::rows_png(&ok.join("t.png"), 2, &[100; 25]);
        let packs = pack::list(dir.path());
        let entries = entries(&packs);
        assert_eq!(
            ids(&entries),
            [
                "flat", "gloss", "brushed", "carbon", "glass", "custom", "ocean"
            ]
        );
        assert_eq!(entries[2].name, "Brushed metal");
        assert_eq!(entries[5].name, "Custom image");
        assert_eq!(entries[6].name, "Ocean blue");

        // Built-in skins, the custom image and usable packs are drawn.
        for id in ["gloss", "custom", "ocean"] {
            assert_eq!(notice(id, &packs), None, "{id}");
        }
        assert_eq!(
            notice("gone", &packs).as_deref(),
            Some("The skin “gone” is not installed on this computer, so Gloss is used.")
        );
        let broken = notice("broken", &packs).expect("a notice");
        assert!(
            broken.starts_with("The skin “broken” cannot be used, so Gloss is used: skin.json"),
            "{broken}"
        );
    }

    #[test]
    fn previews_depend_only_on_what_the_skin_shows() {
        let mut settings = Settings {
            hotkey: "Ctrl+K".into(),
            ..Settings::default()
        };
        settings.classic.custom_skin = "wood.png".into();
        settings.bar.height = Some(30);
        let gloss = preview_settings(&settings, "gloss");
        assert_eq!(gloss.theme, ThemeId::Classic);
        assert_eq!(gloss.classic.skin, "gloss");
        assert_eq!(gloss.bar_height(), 30);
        // Unrelated settings, and the custom image for another skin, do not draw it again.
        let mut plain = Settings::default();
        plain.bar.height = Some(30);
        assert_eq!(gloss, preview_settings(&plain, "gloss"));
        assert_eq!(
            preview_settings(&settings, CUSTOM).classic.custom_skin,
            "wood.png"
        );
    }
}
