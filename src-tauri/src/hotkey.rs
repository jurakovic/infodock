//! The global hotkey (SPEC §6.5): brings the bar to the front or toggles it.

use std::str::FromStr;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use ts_rs::TS;

use crate::config::ConfigStore;
use crate::config::model::HotkeyAction;
use crate::windowing;

/// The registered hotkey and the last registration error. Managed as Tauri state.
#[derive(Default)]
pub struct HotkeyState {
    current: Mutex<Option<Shortcut>>,
    error: Mutex<Option<String>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HotkeyStatus {
    pub accelerator: String,
    pub registered: bool,
    /// Why registration failed, e.g. the key is used by another app.
    pub error: Option<String>,
}

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                on_pressed(app);
            }
        })
        .build()
}

fn on_pressed(app: &AppHandle) {
    match app.state::<ConfigStore>().get().settings.hotkey_action {
        HotkeyAction::BringToFront => windowing::bring_bar_to_front(app),
        HotkeyAction::ToggleVisibility => windowing::toggle_bar(app),
    }
}

/// Parses an accelerator such as `Ctrl+Alt+Shift+I`.
pub fn parse(accelerator: &str) -> Result<Shortcut, String> {
    Shortcut::from_str(accelerator.trim()).map_err(|e| e.to_string())
}

/// Registers `accelerator` in place of the current hotkey. On failure the previous hotkey stays
/// registered and the error is returned (and kept for [`status`]).
pub fn set(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    let state = app.state::<HotkeyState>();
    let result = register(app, &state, accelerator);
    *lock(&state.error) = result.as_ref().err().cloned();
    if let Err(e) = &result {
        tracing::warn!("hotkey {accelerator:?} not registered: {e}");
    }
    result
}

fn register(app: &AppHandle, state: &HotkeyState, accelerator: &str) -> Result<(), String> {
    let shortcut = parse(accelerator)?;
    let mut current = lock(&state.current);
    if *current == Some(shortcut) {
        return Ok(());
    }
    let manager = app.global_shortcut();
    manager.register(shortcut).map_err(|e| e.to_string())?;
    if let Some(old) = current.replace(shortcut) {
        let _ = manager.unregister(old);
    }
    Ok(())
}

pub fn status(app: &AppHandle) -> HotkeyStatus {
    let state = app.state::<HotkeyState>();
    HotkeyStatus {
        accelerator: app.state::<ConfigStore>().get().settings.hotkey,
        registered: lock(&state.current).is_some(),
        error: lock(&state.error).clone(),
    }
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
