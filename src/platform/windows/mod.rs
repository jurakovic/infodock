//! Windows implementation of the platform layer (SPEC §4.6, Appendix A).

pub mod appbar;
pub mod instance;
mod menu;
mod monitors;
mod render;
mod tray;
pub mod window;

use std::mem::size_of;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{PCWSTR, w};

use super::{Capabilities, MonitorInfo, OsAppearance, Platform};

pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            appbar: true,
            ..Capabilities::default()
        }
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        monitors::list()
    }

    fn appearance(&self) -> OsAppearance {
        const PERSONALIZE: PCWSTR =
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        OsAppearance {
            // Missing values: apps use light mode and transparency is on (the OS defaults).
            dark: read_dword(PERSONALIZE, w!("AppsUseLightTheme")).is_some_and(|v| v == 0),
            transparency: read_dword(PERSONALIZE, w!("EnableTransparency")).is_none_or(|v| v != 0),
        }
    }
}

/// A DWORD value under `HKEY_CURRENT_USER`, if present.
fn read_dword(key: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `data` is a DWORD-sized out buffer and `size` says so.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

/// A NUL-terminated UTF-16 buffer as a `String`.
fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
