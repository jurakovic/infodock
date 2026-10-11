//! Config commands. Every change is broadcast as `config-changed` by the store listener
//! registered in `lib.rs`, whichever command made it.

use serde_json::Value;
use tauri::State;

use crate::config::patch::apply_settings_patch;
use crate::config::{Config, ConfigStore};
use crate::error::CmdResult;

#[tauri::command]
pub async fn get_config(store: State<'_, ConfigStore>) -> CmdResult<Config> {
    Ok(store.get())
}

/// Merges a deep partial of `Settings` into the settings and returns the new config.
#[tauri::command]
pub async fn update_settings(store: State<'_, ConfigStore>, patch: Value) -> CmdResult<Config> {
    let (_, config) = store.update(|c| {
        c.settings = apply_settings_patch(&c.settings, &patch)?;
        Ok(())
    })?;
    Ok(config)
}
