//! System commands (SPEC §4.5 "system").

use std::sync::Arc;

use tauri::State;

use crate::error::CmdResult;
use crate::platform::{Capabilities, MonitorInfo, Platform};

#[tauri::command]
pub async fn capabilities(platform: State<'_, Arc<dyn Platform>>) -> CmdResult<Capabilities> {
    Ok(platform.capabilities())
}

#[tauri::command]
pub async fn list_monitors(platform: State<'_, Arc<dyn Platform>>) -> CmdResult<Vec<MonitorInfo>> {
    Ok(platform.monitors())
}
