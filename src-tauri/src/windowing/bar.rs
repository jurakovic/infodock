//! The bar window: creation, placement through the platform AppBar, and visibility (SPEC §5).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use crate::config::ConfigStore;
use crate::config::model::{Material, Settings, ThemeDefaults};
use crate::i18n::t;
use crate::platform::{self, AppBar, AppBarEvent, DockRequest, Platform};

pub const LABEL: &str = "bar";
/// Bar rect, edge, monitor and scale after every placement (SPEC §4.5).
pub const EVENT_BAR_GEOMETRY: &str = "bar-geometry";

/// Owns the bar's placement. Managed as Tauri state.
pub struct BarController {
    appbar: Arc<dyn AppBar>,
    window: WebviewWindow,
    visible: AtomicBool,
    /// The window material currently applied (SPEC §10.3), to avoid redundant re-application.
    material: Mutex<Option<Material>>,
    app: AppHandle,
}

impl BarController {
    /// Creates the bar window (hidden until placed) and docks it. Call on the main thread after
    /// the [`ConfigStore`] is managed.
    pub fn create(app: &AppHandle) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("bar".into()))
            .title(t("app.name"))
            .decorations(false)
            .transparent(true)
            .skip_taskbar(true)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .shadow(false)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            .inner_size(800.0, 28.0)
            .build()?;

        // Closing the bar (graceful taskkill, Restart Manager) quits the app the normal way, so
        // the reservation is released and the config flushed. Alt+F4 is blocked in the AppBar.
        let quitter = app.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                quitter.exit(0);
            }
        });

        let emitter = app.clone();
        let appbar = platform::attach_appbar(
            &window,
            Box::new(move |event| match event {
                AppBarEvent::Placed(geometry) => {
                    if let Err(e) = emitter.emit(EVENT_BAR_GEOMETRY, &geometry) {
                        tracing::warn!("could not send bar geometry: {e}");
                    }
                }
                // Hint and popup windows arrive in M1; they will hide here (SPEC §5.3).
                AppBarEvent::Fullscreen(_) => {}
            }),
        )?;

        let controller = Arc::new(Self {
            appbar,
            window,
            visible: AtomicBool::new(true),
            material: Mutex::new(None),
            app: app.clone(),
        });
        let store = app.state::<ConfigStore>();
        let settings = store.get().settings;
        controller.apply(&settings);
        controller.apply_material(&settings);

        let weak = Arc::downgrade(&controller);
        store.on_change(Box::new(move |old, new| {
            let Some(c) = weak.upgrade() else {
                return;
            };
            if DockRequest::from_settings(&old.settings, true)
                != DockRequest::from_settings(&new.settings, true)
            {
                c.apply(&new.settings);
            }
            c.apply_material(&new.settings);
        }));

        let appbar = Arc::clone(&controller.appbar);
        crate::logging::on_fatal_panic(Box::new(move || appbar.release()));
        Ok(controller)
    }

    fn apply(&self, settings: &Settings) {
        self.appbar.configure(DockRequest::from_settings(
            settings,
            self.visible.load(Ordering::SeqCst),
        ));
    }

    /// Applies Acrylic/Mica (or none) as configured; off while OS transparency effects are off.
    pub fn apply_material(&self, settings: &Settings) {
        let transparency = self.app.state::<Arc<dyn Platform>>().transparency_effects();
        let material = effective_material(settings, transparency);
        let previous = self
            .material
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(material);
        if previous == Some(material) {
            return;
        }
        let window = self.window.clone();
        let result = self
            .window
            .run_on_main_thread(move || set_material(&window, material));
        if let Err(e) = result {
            tracing::warn!("could not apply the window material: {e}");
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }

    /// Shows or hides the bar; a hidden bar reserves no space.
    pub fn set_visible(&self, visible: bool) {
        if self.visible.swap(visible, Ordering::SeqCst) != visible {
            self.apply(&self.app.state::<ConfigStore>().get().settings);
        }
    }

    pub fn toggle_visible(&self) {
        self.set_visible(!self.is_visible());
    }

    /// Shows the bar if hidden and brings it to the front (hotkey).
    pub fn bring_to_front(&self) {
        self.set_visible(true);
        self.appbar.raise();
    }

    /// Releases the reserved screen space. Call on exit.
    pub fn release(&self) {
        self.appbar.release();
    }
}

/// The material to use: the setting, else the theme default; none without OS transparency.
fn effective_material(settings: &Settings, transparency: bool) -> Material {
    if !transparency {
        return Material::None;
    }
    settings
        .background
        .material
        .unwrap_or_else(|| ThemeDefaults::of(settings.theme).background_material)
}

#[cfg(windows)]
fn set_material(window: &WebviewWindow, material: Material) {
    use window_vibrancy::{apply_acrylic, apply_mica, clear_acrylic, clear_mica};
    let _ = clear_acrylic(window);
    let _ = clear_mica(window);
    // The CSS background draws the tint (SPEC §12.2), so the backdrop itself stays untinted.
    let result = match material {
        Material::None => Ok(()),
        Material::Acrylic => apply_acrylic(window, Some((0, 0, 0, 0))),
        Material::Mica => apply_mica(window, None),
    };
    match result {
        Ok(()) => tracing::debug!("window material: {material:?}"),
        Err(e) => tracing::warn!("window material {material:?} not available: {e}"),
    }
}

#[cfg(not(windows))]
fn set_material(_window: &WebviewWindow, _material: Material) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::Theme;

    #[test]
    fn material_follows_setting_theme_and_transparency() {
        let mut s = Settings::default();
        assert_eq!(effective_material(&s, true), Material::Acrylic);
        assert_eq!(effective_material(&s, false), Material::None);
        s.theme = Theme::Classic;
        assert_eq!(effective_material(&s, true), Material::None);
        s.background.material = Some(Material::Mica);
        assert_eq!(effective_material(&s, true), Material::Mica);
    }
}
