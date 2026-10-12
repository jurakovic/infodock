//! Tray icon and menu (SPEC §4.1, §6.5): Show/Hide bar · Lock · Popups · Settings… · Quit.
//! Left click toggles the bar.

use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::bar::BarController;
use crate::config::ConfigStore;
use crate::config::model::Settings;
use crate::i18n::t;

const ID_TOGGLE: &str = "toggle-bar";
const ID_LOCK: &str = "lock";
const ID_POPUPS: &str = "popups";
const ID_SETTINGS: &str = "settings";
const ID_QUIT: &str = "quit";

/// Managed as Tauri state so other code can refresh it via [`sync`].
pub struct Tray {
    icon: TrayIcon,
    toggle: MenuItem<Wry>,
    lock: CheckMenuItem<Wry>,
    popups: CheckMenuItem<Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let settings = app.state::<ConfigStore>().get().settings;
    let toggle = MenuItem::with_id(app, ID_TOGGLE, t("tray.hideBar"), true, None::<&str>)?;
    let lock = CheckMenuItem::with_id(
        app,
        ID_LOCK,
        t("tray.lock"),
        true,
        settings.locked,
        None::<&str>,
    )?;
    let popups = CheckMenuItem::with_id(
        app,
        ID_POPUPS,
        t("tray.popups"),
        true,
        settings.popups_enabled,
        None::<&str>,
    )?;
    let open_settings =
        MenuItem::with_id(app, ID_SETTINGS, t("tray.settings"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, t("tray.quit"), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &lock,
            &popups,
            &PredefinedMenuItem::separator(app)?,
            &open_settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(t("tray.tooltip"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                super::toggle_bar(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let icon = builder.build(app)?;
    app.manage(Tray {
        icon,
        toggle,
        lock,
        popups,
    });

    let handle = app.clone();
    app.state::<ConfigStore>()
        .on_change(Box::new(move |old, new| {
            let s = (&old.settings, &new.settings);
            if s.0.locked != s.1.locked
                || s.0.popups_enabled != s.1.popups_enabled
                || s.0.show_tray_icon != s.1.show_tray_icon
            {
                sync(&handle);
            }
        }));
    sync(app);
    Ok(())
}

/// Updates the menu (labels, check marks) and tray visibility from the settings and the bar.
pub fn sync(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    let settings = app.state::<ConfigStore>().get().settings;
    let bar_visible = app
        .try_state::<std::sync::Arc<BarController>>()
        .is_none_or(|b| b.is_visible());
    let label = if bar_visible {
        t("tray.hideBar")
    } else {
        t("tray.showBar")
    };
    let results = [
        tray.toggle.set_text(label),
        tray.lock.set_checked(settings.locked),
        tray.popups.set_checked(settings.popups_enabled),
        tray.icon.set_visible(settings.show_tray_icon),
    ];
    for e in results.into_iter().filter_map(Result::err) {
        tracing::warn!("tray update failed: {e}");
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        ID_TOGGLE => super::toggle_bar(app),
        ID_LOCK => change_settings(app, |s| s.locked = !s.locked),
        ID_POPUPS => change_settings(app, |s| s.popups_enabled = !s.popups_enabled),
        ID_SETTINGS => {
            if let Err(e) = super::settings::open(app, "general") {
                tracing::warn!("could not open settings: {e}");
            }
        }
        ID_QUIT => app.exit(0),
        _ => {}
    }
}

fn change_settings(app: &AppHandle, change: impl FnOnce(&mut Settings)) {
    let result = app.state::<ConfigStore>().update(|c| {
        change(&mut c.settings);
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!("could not change settings from the tray: {e}");
    }
    // The menu toggled its own check mark; re-sync in case the store disagreed.
    sync(app);
}
