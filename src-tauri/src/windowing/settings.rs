//! The Settings window (SPEC §4.1, §10.7): created on demand, one instance.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::i18n::t;

pub const LABEL: &str = "settings";
/// Sent to an already open Settings window to switch pages.
pub const EVENT_NAVIGATE: &str = "settings-navigate";

/// Opens Settings on `route` (e.g. `bar`), or focuses it and switches to `route`.
pub fn open(app: &AppHandle, route: &str) -> tauri::Result<()> {
    let route: String = route
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '/'))
        .collect();
    if let Some(window) = app.get_webview_window(LABEL) {
        window.emit_to(LABEL, EVENT_NAVIGATE, &route)?;
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }
    WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App(format!("settings/{route}").into()),
    )
    .title(t("settings.title"))
    .inner_size(760.0, 560.0)
    .min_inner_size(560.0, 420.0)
    .center()
    .build()?;
    Ok(())
}
