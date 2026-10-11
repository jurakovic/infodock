//! Config commands. Every change is broadcast as `config-changed` by the store listener
//! registered in `lib.rs`, whichever command made it.

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::config::patch::apply_settings_patch;
use crate::config::{Config, ConfigStore};
use crate::error::{CmdError, CmdResult};
use crate::hotkey;

#[tauri::command]
pub async fn get_config(store: State<'_, ConfigStore>) -> CmdResult<Config> {
    Ok(store.get())
}

/// Merges a deep partial of `Settings` into the settings and returns the new config.
///
/// A changed hotkey is registered first; if that fails (e.g. another app owns the key) the
/// error has code `hotkey` and nothing is saved (SPEC §6.5).
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    store: State<'_, ConfigStore>,
    patch: Value,
) -> CmdResult<Config> {
    let current = store.get().settings;
    let next = apply_settings_patch(&current, &patch)?;
    if next.hotkey != current.hotkey {
        hotkey::set(&app, &next.hotkey).map_err(|e| CmdError::new("hotkey", e))?;
    }
    let (_, config) = store.update(|c| {
        c.settings = apply_settings_patch(&c.settings, &patch)?;
        Ok(())
    })?;
    Ok(config)
}
