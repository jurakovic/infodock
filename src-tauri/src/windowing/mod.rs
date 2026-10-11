//! App windows (SPEC §4.2 `windows/`; named `windowing` so it cannot be confused with the
//! `windows` crate) and the tray.

pub mod bar;
pub mod settings;
pub mod tray;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use bar::BarController;

/// Shows or hides the bar and updates the tray menu.
pub fn toggle_bar(app: &AppHandle) {
    if let Some(bar) = app.try_state::<Arc<BarController>>() {
        bar.toggle_visible();
    }
    tray::sync(app);
}

/// Shows the bar if needed and brings it to the front (hotkey, second launch).
pub fn bring_bar_to_front(app: &AppHandle) {
    if let Some(bar) = app.try_state::<Arc<BarController>>() {
        bar.bring_to_front();
    }
    tray::sync(app);
}
