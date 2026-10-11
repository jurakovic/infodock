//! App commands (SPEC §4.5 "app").

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

use crate::hotkey::{self, HotkeyStatus};
use ts_rs::TS;

use crate::error::{CmdError, CmdResult};

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub version: String,
    /// OS locale as a BCP 47 tag, e.g. `en-US`.
    pub locale: String,
    pub data_dir: String,
    pub log_dir: String,
    pub skins_dir: String,
}

#[tauri::command]
pub async fn app_info<R: Runtime>(app: AppHandle<R>) -> CmdResult<AppInfo> {
    let path = app.path();
    let io = |e: tauri::Error| CmdError::new("io", e.to_string());
    let data_dir = path.app_config_dir().map_err(io)?;
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        locale: sys_locale::get_locale().unwrap_or_else(|| "en-US".into()),
        skins_dir: data_dir.join("skins").display().to_string(),
        data_dir: data_dir.display().to_string(),
        log_dir: path.app_log_dir().map_err(io)?.display().to_string(),
    })
}

#[tauri::command]
pub async fn quit(app: AppHandle) -> CmdResult<()> {
    app.exit(0);
    Ok(())
}

/// The global hotkey and why it could not be registered, if so (SPEC §6.5).
#[tauri::command]
pub async fn hotkey_status(app: AppHandle) -> CmdResult<HotkeyStatus> {
    Ok(hotkey::status(&app))
}
