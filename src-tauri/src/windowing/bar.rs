//! The bar window: creation, placement through the platform AppBar, and visibility (SPEC §5).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::config::ConfigStore;
use crate::config::model::Settings;
use crate::i18n::t;
use crate::platform::{self, AppBar, AppBarEvent, DockRequest};

pub const LABEL: &str = "bar";
/// Bar rect, edge, monitor and scale after every placement (SPEC §4.5).
pub const EVENT_BAR_GEOMETRY: &str = "bar-geometry";

/// Owns the bar's placement. Managed as Tauri state.
pub struct BarController {
    appbar: Arc<dyn AppBar>,
    visible: AtomicBool,
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
            visible: AtomicBool::new(true),
            app: app.clone(),
        });
        let store = app.state::<ConfigStore>();
        controller.apply(&store.get().settings);

        let weak = Arc::downgrade(&controller);
        store.on_change(Box::new(move |old, new| {
            if let Some(c) = weak.upgrade()
                && DockRequest::from_settings(&old.settings, true)
                    != DockRequest::from_settings(&new.settings, true)
            {
                c.apply(&new.settings);
            }
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

    #[allow(dead_code, reason = "tray and hotkey arrive in M0 S4")]
    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }

    /// Shows or hides the bar; a hidden bar reserves no space.
    #[allow(dead_code, reason = "tray and hotkey arrive in M0 S4")]
    pub fn set_visible(&self, visible: bool) {
        if self.visible.swap(visible, Ordering::SeqCst) != visible {
            self.apply(&self.app.state::<ConfigStore>().get().settings);
        }
    }

    #[allow(dead_code, reason = "tray and hotkey arrive in M0 S4")]
    pub fn toggle_visible(&self) {
        self.set_visible(!self.is_visible());
    }

    /// Shows the bar if hidden and brings it to the front (hotkey, second launch).
    #[allow(dead_code, reason = "tray and hotkey arrive in M0 S4")]
    pub fn bring_to_front(&self) {
        self.set_visible(true);
        self.appbar.raise();
    }

    /// Re-negotiates the placement (e.g. after a resume).
    #[allow(dead_code, reason = "tray and hotkey arrive in M0 S4")]
    pub fn refresh(&self) {
        self.appbar.refresh();
    }

    /// Releases the reserved screen space. Call on exit.
    pub fn release(&self) {
        self.appbar.release();
    }
}
