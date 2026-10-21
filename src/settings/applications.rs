//! Settings ▸ Applications (SPEC §10.6): the calendar app a click on a Date sensor opens.

use std::sync::Arc;

use serde_json::Value;
use slint::ComponentHandle;

use super::{model, pick, save};
use crate::app::App;
use crate::i18n::t;
use crate::ui;

pub(super) fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    let state = window.global::<ui::SettingsState>();
    let a = Arc::clone(app);
    state.on_browse_calendar_app(move || browse(&a));
}

/// Lets the user pick the calendar app; programs are offered first. A shortcut picked gives
/// its target.
fn browse(app: &Arc<App>) {
    let picked = pick(
        &t("settings.apps.calendar.pick"),
        false,
        &["exe"],
        &t("settings.apps.programs"),
    );
    if let Some(path) = picked {
        save(app, model::patch("apps.calendar", Value::String(path)));
    }
}
