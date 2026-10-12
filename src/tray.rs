//! The tray icon (SPEC §4.1, §6.5), a Slint `SystemTrayIcon` on the main thread. Left click
//! toggles the bar; the menu mirrors the bar's visibility and the Lock/Popups settings.

use std::cell::RefCell;
use std::sync::Arc;

use crate::app::App;
use crate::config::model::Settings;
use crate::{settings, ui};

thread_local! {
    static TRAY: RefCell<Option<ui::Tray>> = const { RefCell::new(None) };
}

/// Creates the tray icon. Main thread only; a failure is logged and the app runs without it.
pub fn create(app: &Arc<App>) {
    let tray = match ui::Tray::new() {
        Ok(tray) => tray,
        Err(e) => {
            tracing::warn!("tray icon unavailable: {e}");
            return;
        }
    };
    crate::install_i18n!(tray);

    let a = Arc::clone(app);
    tray.on_toggle_bar(move || a.toggle_bar());
    let a = Arc::clone(app);
    tray.on_toggle_lock(move || change_settings(&a, |s| s.locked = !s.locked));
    let a = Arc::clone(app);
    tray.on_toggle_popups(move || change_settings(&a, |s| s.popups_enabled = !s.popups_enabled));
    let a = Arc::clone(app);
    tray.on_open_settings(move || settings::open(&a, "general"));
    let a = Arc::clone(app);
    tray.on_quit(move || a.request_quit());

    sync_into(&tray, app);
    TRAY.with(|t| *t.borrow_mut() = Some(tray));

    let a = Arc::clone(app);
    app.store.on_change(Box::new(move |old, new| {
        let (o, n) = (&old.settings, &new.settings);
        if o.locked != n.locked
            || o.popups_enabled != n.popups_enabled
            || o.show_tray_icon != n.show_tray_icon
        {
            request_sync(&a);
        }
    }));
}

/// Removes the tray icon. Main thread only.
pub fn destroy() {
    TRAY.with(|t| t.borrow_mut().take());
}

/// Updates the tray from the settings and the bar, from any thread.
pub fn request_sync(app: &Arc<App>) {
    let app = Arc::clone(app);
    let _ = slint::invoke_from_event_loop(move || {
        TRAY.with(|t| {
            if let Some(tray) = t.borrow().as_ref() {
                sync_into(tray, &app);
            }
        });
    });
}

fn sync_into(tray: &ui::Tray, app: &App) {
    let settings = app.store.get().settings;
    tray.set_bar_visible(app.bar_visible());
    tray.set_locked(settings.locked);
    tray.set_popups_enabled(settings.popups_enabled);
    tray.set_icon_visible(settings.show_tray_icon);
}

fn change_settings(app: &Arc<App>, change: impl FnOnce(&mut Settings)) {
    let result = app.store.update(|c| {
        change(&mut c.settings);
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!("could not change settings from the tray: {e}");
    }
    // The menu toggled its own check mark; re-sync in case the store disagreed.
    request_sync(app);
}
