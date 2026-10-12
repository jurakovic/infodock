//! The Settings window and the tray (SPEC §4.2; named `windowing` so it cannot be confused with
//! the `windows` crate). The bar lives in `crate::bar`.

pub mod settings;
pub mod tray;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::bar::BarController;

/// Shows or hides the bar and updates the tray menu.
pub fn toggle_bar(app: &AppHandle) {
    if let Some(bar) = app.try_state::<Arc<BarController>>() {
        bar.toggle_visible();
    }
    tray::sync(app);
}

/// Shows the bar if it was hidden, without taking focus (second launch).
pub fn show_bar(app: &AppHandle) {
    if let Some(bar) = app.try_state::<Arc<BarController>>() {
        bar.set_visible(true);
    }
    tray::sync(app);
}

/// Shows the bar if needed and brings it to the front (hotkey).
pub fn bring_bar_to_front(app: &AppHandle) {
    if let Some(bar) = app.try_state::<Arc<BarController>>() {
        bar.bring_to_front();
    }
    tray::sync(app);
}
