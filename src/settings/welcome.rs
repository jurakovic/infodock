//! The welcome page (SPEC §11.5), opened once, on first run: the theme with a preview of the
//! bar in each, the edge, starting with Windows, importing a layout and a few tips.
//! `meta.firstRunDone` is set once the page is left (*Get started*, another page, or closing
//! Settings), so a run that ends while it is open shows it again next time.

use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, VecModel};

use super::Open;
use super::appearance::{PreviewKey, Previews};
use crate::app::App;
use crate::config::exchange::ImportFormat;
use crate::config::model::{ColorScheme, Settings, Theme as ThemeId};
use crate::i18n::t;
use crate::platform::NativeDialogs;
use crate::ui;

/// The page index of the welcome page in the window; it is not in the navigation.
pub(super) const PAGE: i32 = 8;
/// The previews' width, logical px.
const PREVIEW_WIDTH: f32 = 240.0;

/// The welcome page while it is open.
#[derive(Default)]
pub(super) struct Welcome {
    /// The page was opened and has not been left.
    pub(super) shown: bool,
    previews: Previews,
}

/// The settings the preview of `theme` is drawn with: what of `settings` the bar's background
/// depends on, in that theme, with the colour scheme the OS's mode (`os_dark`) gives, so a
/// change of the mode draws it again.
pub fn preview_settings(settings: &Settings, theme: ThemeId, os_dark: bool) -> Settings {
    let mut preview = Settings {
        theme,
        color_scheme: match settings.color_scheme {
            ColorScheme::System if os_dark => ColorScheme::Dark,
            ColorScheme::System => ColorScheme::Light,
            scheme => scheme,
        },
        background: settings.background.clone(),
        classic: settings.classic.clone(),
        ..Settings::default()
    };
    preview.bar.height = settings.bar.height;
    preview.bar.edge = settings.bar.edge;
    preview
}

/// Shows the welcome page's theme tiles while it is open; their previews are drawn again only
/// when what they show changed.
pub(super) fn fill(open: &mut Open, app: &App, settings: &Settings) {
    if !open.welcome.shown {
        return;
    }
    let scale = open.window.window().scale_factor();
    let os_dark = app.platform.appearance().dark;
    let items = [
        (ThemeId::Modern, "settings.appearance.theme.modern"),
        (ThemeId::Classic, "settings.appearance.theme.classic"),
    ]
    .into_iter()
    .map(|(theme, name)| {
        let key = PreviewKey {
            settings: preview_settings(settings, theme, os_dark),
            // On first run the skin is a built-in one.
            pack: None,
            width: PREVIEW_WIDTH,
            scale,
        };
        (key, t(name), settings.theme == theme)
    })
    .collect();
    let tiles = open.welcome.previews.tiles(app, items);
    let state = open.window.global::<ui::SettingsState>();
    state.set_welcome_themes(ModelRc::new(VecModel::from(tiles)));
}

/// Opens the page: it shows until it is left.
pub(super) fn show(open: &mut Open, app: &App) {
    open.welcome.shown = true;
    let settings = app.store.get().settings;
    fill(open, app, &settings);
    open.window.global::<ui::SettingsState>().set_page(PAGE);
}

/// The page was left: it is not opened again (SPEC §11.5).
pub(super) fn left(open: &mut Open, app: &App) {
    if !std::mem::take(&mut open.welcome.shown) {
        return;
    }
    open.welcome.previews = Previews::default();
    let result = app.store.update(|c| {
        c.meta.first_run_done = true;
        Ok(())
    });
    match result {
        Ok(_) => tracing::info!("first run done"),
        Err(e) => tracing::warn!("could not note that the first run is done: {e}"),
    }
}

pub(super) fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    let state = window.global::<ui::SettingsState>();
    let a = Arc::clone(app);
    state.on_welcome_import(move || {
        let dialogs = NativeDialogs {
            owner: super::owner(),
        };
        crate::transfer::import(&a, &dialogs, ImportFormat::Any);
    });
    let a = Arc::clone(app);
    state.on_welcome_done(move || super::close(&a));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_show_the_bar_in_each_theme() {
        let mut settings = Settings {
            hotkey: "Ctrl+K".into(),
            theme: ThemeId::Classic,
            ..Settings::default()
        };
        settings.bar.edge = crate::config::model::Edge::Top;
        settings.classic.skin = "carbon".into();
        let modern = preview_settings(&settings, ThemeId::Modern, true);
        assert_eq!(modern.theme, ThemeId::Modern);
        assert_eq!(modern.bar.edge, settings.bar.edge);
        // The OS's mode decides the look while the scheme follows it.
        assert_eq!(modern.color_scheme, ColorScheme::Dark);
        let light = preview_settings(&settings, ThemeId::Modern, false);
        assert_eq!(light.color_scheme, ColorScheme::Light);
        settings.color_scheme = ColorScheme::Dark;
        let forced = preview_settings(&settings, ThemeId::Modern, false);
        assert_eq!(forced.color_scheme, ColorScheme::Dark);
        let classic = preview_settings(&settings, ThemeId::Classic, false);
        assert_eq!(classic.classic.skin, "carbon");
        // Settings the background does not depend on do not draw it again.
        assert_eq!(
            classic,
            preview_settings(
                &Settings {
                    hotkey: "Ctrl+J".into(),
                    ..settings.clone()
                },
                ThemeId::Classic,
                false
            )
        );
    }
}
