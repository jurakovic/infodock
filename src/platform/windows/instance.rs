//! Data folders and the single-instance lock (SPEC §11.1, §6.5).
//!
//! The lock is a named mutex in the session namespace. A second launch finds the running bar
//! window by its class, lets it take the foreground (the second launch has that right, since the
//! user just started it) and posts it the activation message; the bar thread then shows the bar
//! and opens Settings.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_RoamingAppData, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
    RegisterWindowMessageW,
};
use windows::core::{GUID, HSTRING, w};

use super::window::CLASS_NAME;
use crate::platform::{AppDirs, Instance, InstanceGuard};

/// Posted by a second launch to the running instance's bar window.
pub(super) fn activate_message() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    // SAFETY: plain call with a static string.
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("Infodock.Activate")) })
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: the returned string is owned by us and freed with CoTaskMemFree.
    unsafe {
        let path = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let result = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.0.cast()));
        result
    }
}

fn folder(id: &GUID, env: &str) -> PathBuf {
    known_folder(id)
        .or_else(|| std::env::var_os(env).map(PathBuf::from))
        .unwrap_or_default()
}

pub fn app_dirs(identifier: &str) -> AppDirs {
    AppDirs {
        config: folder(&FOLDERID_RoamingAppData, "APPDATA").join(identifier),
        logs: folder(&FOLDERID_LocalAppData, "LOCALAPPDATA")
            .join(identifier)
            .join("logs"),
    }
}

/// Closes the mutex when the primary instance ends.
struct MutexHandle(isize);

impl Drop for MutexHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed once.
        unsafe {
            let _ = CloseHandle(HANDLE(self.0 as *mut _));
        }
    }
}

pub fn single_instance(identifier: &str) -> Instance {
    let name = HSTRING::from(format!("Local\\{identifier}"));
    // SAFETY: plain call; the last error is read right after it.
    let created = unsafe { CreateMutexW(None, true, &name) };
    match created {
        // SAFETY: reads the error code CreateMutexW left behind.
        Ok(handle) if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS => {
            // SAFETY: our own handle, not used again.
            unsafe {
                let _ = CloseHandle(handle);
            }
            Instance::Secondary
        }
        Ok(handle) => Instance::Primary(InstanceGuard::new(MutexHandle(handle.0 as isize))),
        Err(e) => {
            eprintln!("single-instance lock unavailable: {e}");
            Instance::Primary(InstanceGuard::new(()))
        }
    }
}

pub fn activate_running_instance() {
    for _ in 0..30 {
        // SAFETY: plain window lookup by our own class name.
        if let Ok(bar) = unsafe { FindWindowW(CLASS_NAME, None) } {
            let mut pid = 0;
            // SAFETY: plain calls; failures only mean the bar cannot take the foreground.
            unsafe {
                GetWindowThreadProcessId(bar, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(bar), activate_message(), WPARAM(0), LPARAM(0));
            }
            return;
        }
        // The running instance may still be starting.
        std::thread::sleep(Duration::from_millis(100));
    }
}
