//! Settings ▸ Appearance (SPEC §10.3, §10.7): the theme, the bar's background and, for the
//! classic theme, the skin gallery with previews rendered by the bar's renderer
//! (`skin_preview`), the custom image, a notice when the configured skin is missing, and the
//! skin packs: installed from a folder or a `.zip`, listed with their problems, removed to the
//! Recycle Bin (SPEC §12.4).

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{Open, WINDOW, model, pick, save};
use crate::app::App;
use crate::config::model::{
    BackgroundSettings, ClassicSettings, ColorScheme, Settings, Theme as ThemeId, ThemeDefaults,
};
use crate::i18n::{t, t_args};
use crate::look::skins::pack::{self, PackEntry};
use crate::look::skins::{self, BuiltIn, CUSTOM};
use crate::look::theme::Theme;
use crate::platform::Bitmap;
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

/// Every folder in the skins folder for the list of packs: its id, its name (the id when it
/// cannot be used) and why it cannot be used.
fn pack_rows(packs: &[PackEntry]) -> Vec<(String, String, String)> {
    packs
        .iter()
        .map(|entry| match &entry.pack {
            Ok(pack) => (entry.id.clone(), pack.name.clone(), String::new()),
            Err(e) => (entry.id.clone(), entry.id.clone(), e.message()),
        })
        .collect()
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
pub(super) struct PreviewKey {
    pub(super) settings: Settings,
    /// The skin's pack as listed.
    pub(super) pack: Option<PackEntry>,
    /// Logical px.
    pub(super) width: f32,
    pub(super) scale: f32,
}

#[derive(Clone)]
enum Preview {
    Image(slint::Image),
    /// Where there is no renderer: the skin's colour.
    Swatch(slint::Color),
}

/// Previews of the bar's background kept with what they were drawn from.
#[derive(Default)]
pub(super) struct Previews(Vec<(PreviewKey, Preview)>);

impl Previews {
    /// A tile for each of `items` (what its preview shows, its name, and whether it is the one
    /// chosen), drawing only the previews not kept; afterwards only these are kept.
    pub(super) fn tiles(
        &mut self,
        app: &App,
        items: Vec<(PreviewKey, String, bool)>,
    ) -> Vec<ui::SkinTile> {
        let mut kept = Vec::with_capacity(items.len());
        let mut tiles = Vec::with_capacity(items.len());
        for (key, name, selected) in items {
            let shown = self
                .0
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, p)| p.clone());
            let image = shown.unwrap_or_else(|| preview(app, &key));
            let (preview, has_preview, swatch) = match &image {
                Preview::Image(image) => (image.clone(), true, slint::Color::default()),
                Preview::Swatch(color) => (slint::Image::default(), false, *color),
            };
            tiles.push(ui::SkinTile {
                name: name.into(),
                preview,
                has_preview,
                swatch,
                selected,
            });
            kept.push((key, image));
        }
        self.0 = kept;
        tiles
    }
}

/// The gallery as last shown.
#[derive(Default)]
pub(super) struct Gallery {
    /// The skins folder as last listed.
    packs: Vec<PackEntry>,
    listed: bool,
    entries: Vec<SkinEntry>,
    previews: Previews,
    /// The custom image's path as last checked, and its problem (empty when it can be used).
    custom: Option<(String, String)>,
}

impl Gallery {
    /// Lists the skins folder again, e.g. when the page is shown or a pack was installed.
    /// Files may have changed in place (a pack installed again under its name, the custom image
    /// fixed), so the previews are drawn and the custom image checked again too.
    pub(super) fn refresh(&mut self, app: &App) {
        self.packs = pack::list(&app.skins);
        self.listed = true;
        self.previews = Previews::default();
        self.custom = None;
    }

    /// The custom image's problem as a sentence, empty when it can be used; read only when the
    /// path differs from the one last checked, not at every refresh of the page (each typed
    /// character saves the path).
    fn custom_problem(&mut self, path: &str) -> String {
        match &self.custom {
            Some((checked, problem)) if checked == path => problem.clone(),
            _ => {
                let problem = skins::read_custom(path)
                    .err()
                    .map_or_else(String::new, |e| e.message());
                self.custom = Some((path.to_owned(), problem.clone()));
                problem
            }
        }
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
    match crate::bar::skin_preview(&key.settings, os, &app.skins, key.width, key.scale) {
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
    apply_scheme(open, settings.color_scheme);
    let state = open.window.global::<ui::SettingsState>();
    let json = serde_json::to_value(settings).unwrap_or_default();
    let choice = |path: &str| model::choice(path).map_or(0, |c| model::choice_index(&json, c));
    let defaults = ThemeDefaults::of(settings.theme);
    let background = &settings.background;

    state.set_theme(choice("theme"));
    state.set_color_scheme(choice("colorScheme"));
    state.set_background_mode(choice("background.colorMode"));
    // What the theme gives, which in the modern bar's light look is its light colour.
    let uncustomised = Settings {
        background: BackgroundSettings {
            color: None,
            ..background.clone()
        },
        ..settings.clone()
    };
    let os_dark = app.platform.appearance().dark;
    let theme_color = model::in_effect(&uncustomised, os_dark, "background.color")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| defaults.background_color.clone());
    state.set_background_color(background.color.as_deref().unwrap_or(&theme_color).into());
    state.set_background_color_customised(background.color.is_some());
    state.set_background_color_default(theme_color.as_str().into());
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
    // Checked only while the image is the skin, so a path kept from before costs nothing.
    let custom_error = match classic.custom_skin.as_str() {
        path if !path.is_empty() && classic.skin == CUSTOM => open.gallery.custom_problem(path),
        _ => String::new(),
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
    let rows: Vec<ui::SkinPackRow> = pack_rows(&gallery.packs)
        .into_iter()
        .map(|(id, name, problem)| ui::SkinPackRow {
            id: id.into(),
            name: name.into(),
            problem: problem.into(),
        })
        .collect();
    state.set_skin_packs(ModelRc::new(VecModel::from(rows)));
    state.set_skin_notice(
        notice(&classic.skin, &gallery.packs)
            .unwrap_or_default()
            .into(),
    );
    gallery.entries = entries(&gallery.packs);
    let scale = open.window.window().scale_factor();
    let items = gallery
        .entries
        .iter()
        .map(|entry| {
            let key = PreviewKey {
                settings: preview_settings(settings, &entry.id),
                pack: gallery.packs.iter().find(|p| p.id == entry.id).cloned(),
                width: PREVIEW_WIDTH,
                scale,
            };
            (key, entry.name.clone(), entry.id == classic.skin)
        })
        .collect();
    let tiles = gallery.previews.tiles(app, items);
    let selected = gallery.entries.iter().position(|e| e.id == classic.skin);
    state.set_skin_selected(selected.map_or(-1, |i| i as i32));
    state.set_skins(ModelRc::new(VecModel::from(tiles)));
}

/// Gives the window's title bar the colour scheme (SPEC §10.3); the Fluent palette follows
/// `color-scheme` in the UI. Done again until the native window exists.
pub(super) fn apply_scheme(open: &mut Open, scheme: ColorScheme) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::window::Theme as WindowTheme;
    if open.scheme == Some(scheme) {
        return;
    }
    let theme = match scheme {
        ColorScheme::System => None,
        ColorScheme::Light => Some(WindowTheme::Light),
        ColorScheme::Dark => Some(WindowTheme::Dark),
    };
    let applied = open
        .window
        .window()
        .with_winit_window(|w| w.set_theme(theme))
        .is_some();
    if applied {
        open.scheme = Some(scheme);
    }
}

/// Lets the user pick the custom image.
fn browse(app: &Arc<App>) {
    let picked = pick(
        &t("settings.props.pickFile"),
        false,
        IMAGE_TYPES,
        &t("settings.props.supportedFiles"),
    );
    if let Some(path) = picked {
        save(app, model::patch("classic.customSkin", Value::String(path)));
    }
}

/// The skins folder changed: the bar draws its skin again, the page lists the folder again.
pub(super) fn packs_changed(app: &App) {
    app.skins_changed();
    WINDOW.with(|w| {
        if let Some(open) = w.borrow_mut().as_mut() {
            open.gallery.refresh(app);
            let settings = app.store.get().settings;
            super::fill(open, app, &settings);
        }
    });
}

/// Lets the user pick a pack's folder, or a `.zip` holding one, and installs it (SPEC §12.4).
fn install(app: &Arc<App>, folder: bool) {
    let picked = if folder {
        pick(&t("settings.appearance.packs.pickFolder"), true, &[], "")
    } else {
        pick(
            &t("settings.appearance.packs.pickZip"),
            false,
            &["zip"],
            &t("settings.appearance.packs.zipFiles"),
        )
    };
    let Some(path) = picked else {
        return;
    };
    let (message, failed) = match skins::install::install(&app.skins, Path::new(&path)) {
        Ok(pack) => {
            tracing::info!("installed the skin pack {:?} from {path:?}", pack.id);
            let message = t_args(
                "settings.appearance.packs.installed",
                &[("name", &pack.name)],
            );
            (message, false)
        }
        Err(e) => {
            let reason = e.message();
            tracing::warn!("could not install the skin pack {path:?}: {reason}");
            let message = t_args(
                "settings.appearance.packs.installFailed",
                &[("reason", &reason)],
            );
            (message, true)
        }
    };
    WINDOW.with(|w| {
        if let Some(open) = w.borrow().as_ref() {
            let state = open.window.global::<ui::SettingsState>();
            state.set_skin_install_message(message.into());
            state.set_skin_install_failed(failed);
        }
    });
    packs_changed(app);
}

/// Moves the pack `id`, one the page lists, to the Recycle Bin (SPEC §12.4).
fn remove(app: &Arc<App>, id: &str) {
    let listed = WINDOW.with(|w| {
        w.borrow()
            .as_ref()
            .is_some_and(|open| open.gallery.packs.iter().any(|p| p.id == id))
    });
    if !listed {
        return;
    }
    let path = app.skins.join(id).display().to_string();
    let (a, id) = (Arc::clone(app), id.to_owned());
    app.platform.recycle(
        vec![path],
        Box::new(move || {
            let queued = slint::invoke_from_event_loop(move || removed(&a, &id));
            if queued.is_err() {
                tracing::debug!("a skin pack was removed after the event loop ended");
            }
        }),
    );
}

/// After removing the pack `id` (or not: the user may have said no): the bar no longer shows it.
fn removed(app: &Arc<App>, id: &str) {
    if !app.skins.join(id).exists() {
        tracing::info!("removed the skin pack {id:?}");
        // Folder names ignore case on Windows: `ocean` in the config names `Ocean`.
        if app.store.get().settings.classic.skin.to_lowercase() == id.to_lowercase() {
            save(
                app,
                model::patch("classic.skin", Value::from(skins::FALLBACK.id())),
            );
        }
    }
    packs_changed(app);
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
    let a = Arc::clone(app);
    state.on_install_skin(move |folder| install(&a, folder));
    let a = Arc::clone(app);
    state.on_remove_skin(move |id| remove(&a, &id));
    let a = Arc::clone(app);
    state.on_open_skins_folder(move || {
        if let Err(e) = std::fs::create_dir_all(&a.skins) {
            tracing::warn!("could not create the skins folder: {e}");
        }
        a.platform.open(&a.skins.display().to_string());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::skins::tests::pack as write_pack;

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
        crate::look::skins::tests::rows_png(&ok.join("t.png"), 2, &[100; 25]);
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
        let rows = pack_rows(&packs);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].0.as_str(), rows[0].1.as_str()),
            ("broken", "broken")
        );
        assert!(
            rows[0].2.starts_with("skin.json is not valid"),
            "{}",
            rows[0].2
        );
        assert_eq!(
            rows[1],
            ("ocean".into(), "Ocean blue".into(), String::new())
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
