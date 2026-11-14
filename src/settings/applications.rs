//! Settings ▸ Applications (SPEC §10.6): the calendar app a click on a Date sensor opens, and
//! the task manager a click on a CPU or Memory sensor opens.

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
    state
        .on_browse_calendar_app(move || browse(&a, "apps.calendar", "settings.apps.calendar.pick"));
    let a = Arc::clone(app);
    state.on_browse_task_manager_app(move || {
        browse(&a, "apps.taskManager", "settings.apps.taskManager.pick");
    });
}

/// Lets the user pick the app saved at `path`, titled `title`; programs are offered first. A
/// shortcut picked gives its target.
fn browse(app: &Arc<App>, path: &str, title: &str) {
    let picked = pick(&t(title), false, &["exe"], &t("settings.apps.programs"));
    if let Some(picked) = picked {
        save(app, model::patch(path, Value::String(picked)));
    }
}
