//! UI commands (SPEC §4.5 "ui"). Hint and popup commands arrive in M1/M3.

use tauri::AppHandle;

use crate::error::{CmdError, CmdResult};
use crate::windowing::settings;

#[tauri::command]
pub async fn open_settings(app: AppHandle, route: String) -> CmdResult<()> {
    settings::open(&app, &route).map_err(|e| CmdError::new("window", e.to_string()))
}
