//! The global hotkey (SPEC §6.5): brings the bar to the front or toggles it.
//!
//! The hotkey manager lives on the main thread, whose event loop delivers its messages.

use std::cell::RefCell;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::app::App;
use crate::config::model::HotkeyAction;

struct State {
    current: Option<HotKey>,
    /// Why the configured hotkey is not registered, e.g. another app owns it.
    error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    current: None,
    error: None,
});

thread_local! {
    static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
}

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Creates the hotkey manager and registers the configured hotkey. A taken hotkey does not stop
/// the app; Settings shows the error (SPEC §6.5). Main thread only.
pub fn init(app: &Arc<App>) {
    match GlobalHotKeyManager::new() {
        Ok(manager) => MANAGER.with(|m| *m.borrow_mut() = Some(manager)),
        Err(e) => {
            tracing::warn!("global hotkeys unavailable: {e}");
            state().error = Some(e.to_string());
            return;
        }
    }
    let handler_app = Arc::clone(app);
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        let ours = state().current.is_some_and(|h| h.id() == event.id);
        if ours && event.state == HotKeyState::Pressed {
            on_pressed(&handler_app);
        }
    }));
    let _ = set(&app.store.get().settings.hotkey);
}

fn on_pressed(app: &Arc<App>) {
    match app.store.get().settings.hotkey_action {
        HotkeyAction::BringToFront => app.bring_bar_to_front(),
        HotkeyAction::ToggleVisibility => app.toggle_bar(),
    }
}

/// Parses an accelerator such as `Ctrl+Alt+Shift+I`.
pub fn parse(accelerator: &str) -> Result<HotKey, String> {
    HotKey::from_str(accelerator.trim()).map_err(|e| e.to_string())
}

/// Registers `accelerator` in place of the current hotkey. On failure the previous hotkey stays
/// registered and the error is returned (and kept for [`error`]). Main thread only.
pub fn set(accelerator: &str) -> Result<(), String> {
    let result = register(accelerator);
    state().error = result.as_ref().err().cloned();
    if let Err(e) = &result {
        tracing::warn!("hotkey {accelerator:?} not registered: {e}");
    }
    result
}

fn register(accelerator: &str) -> Result<(), String> {
    let hotkey = parse(accelerator)?;
    if state().current == Some(hotkey) {
        return Ok(());
    }
    MANAGER.with(|m| {
        let manager = m.borrow();
        let Some(manager) = manager.as_ref() else {
            return Err("global hotkeys are unavailable".to_owned());
        };
        manager.register(hotkey).map_err(|e| e.to_string())?;
        if let Some(old) = state().current.replace(hotkey) {
            let _ = manager.unregister(old);
        }
        Ok(())
    })
}

/// Why the configured hotkey is not registered, if it is not.
pub fn error() -> Option<String> {
    state().error.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_and_common_accelerators() {
        for a in [
            "Ctrl+Alt+Shift+I",
            "ctrl+alt+shift+i",
            "Alt+Space",
            "Ctrl+Shift+F12",
            " Super+B ",
        ] {
            assert!(parse(a).is_ok(), "{a}: {:?}", parse(a));
        }
    }

    #[test]
    fn rejects_garbage() {
        for a in ["", "Ctrl+", "Hyper+Banana", "Ctrl+Alt"] {
            assert!(parse(a).is_err(), "{a} should not parse");
        }
    }
}
