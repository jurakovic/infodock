//! The running app: state shared by all threads, bar visibility, and quitting.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::bar::{BarController, BarHost};
use crate::config::ConfigStore;
use crate::platform::Platform;
use crate::{settings, tray};

/// How long a requested quit may take before the process ends anyway.
const QUIT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct App {
    pub store: ConfigStore,
    pub platform: Arc<dyn Platform>,
    bar: OnceLock<Arc<BarController>>,
    shut_down: AtomicBool,
}

impl App {
    pub fn new(store: ConfigStore, platform: Arc<dyn Platform>) -> Self {
        Self {
            store,
            platform,
            bar: OnceLock::new(),
            shut_down: AtomicBool::new(false),
        }
    }

    pub fn set_bar(&self, bar: Arc<BarController>) {
        let _ = self.bar.set(bar);
    }

    pub fn bar_visible(&self) -> bool {
        self.bar.get().is_none_or(|b| b.is_visible())
    }

    /// Shows or hides the bar (tray click, hotkey) and updates the tray menu.
    pub fn toggle_bar(self: &Arc<Self>) {
        if let Some(bar) = self.bar.get() {
            bar.toggle_visible();
        }
        tray::request_sync(self);
    }

    /// Shows the bar if hidden and brings it to the front (hotkey).
    pub fn bring_bar_to_front(self: &Arc<Self>) {
        if let Some(bar) = self.bar.get() {
            bar.bring_to_front();
        }
        tray::request_sync(self);
    }

    /// Ends the event loop from any thread; the main thread then cleans up. If the loop does not
    /// end in time (or is gone), the process ends here after the same cleanup.
    pub fn request_quit(self: &Arc<Self>) {
        let queued = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        })
        .is_ok();
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            if queued {
                std::thread::sleep(QUIT_TIMEOUT);
                tracing::warn!("the event loop did not end; quitting anyway");
            }
            app.shutdown();
            std::process::exit(0);
        });
    }

    /// Releases the reserved screen space, writes pending config changes and closes the log.
    /// Runs once, whichever path ends the app.
    pub fn shutdown(&self) {
        if self.shut_down.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(bar) = self.bar.get() {
            bar.release();
        }
        self.store.shutdown();
        tracing::info!("Infodock exiting");
        crate::logging::shutdown();
    }
}

/// The bar's view of the app (called on the bar thread).
pub struct AppHost(pub Arc<App>);

impl BarHost for AppHost {
    fn quit(&self) {
        self.0.request_quit();
    }

    fn activate(&self) {
        let app = &self.0;
        if let Some(bar) = app.bar.get() {
            bar.set_visible(true);
        }
        tray::request_sync(app);
        let app = Arc::clone(app);
        let _ = slint::invoke_from_event_loop(move || settings::open(&app, "general"));
    }
}
