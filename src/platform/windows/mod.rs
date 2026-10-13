//! Windows implementation of the platform layer (SPEC §4.6, Appendix A).

pub mod appbar;
mod dialog;
mod hint;
pub mod image;
pub mod instance;
mod locale;
mod menu;
mod monitors;
pub mod render;
pub mod shell;
pub mod text;
mod tray;
pub mod window;

use std::mem::size_of;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{PCWSTR, w};

use super::{Capabilities, Color, Locale, MonitorInfo, OsAppearance, Platform};
use crate::config::model::DateLocale;

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

    fn locale(&self, which: DateLocale) -> Locale {
        locale::read(which)
    }

    fn font_families(&self) -> Vec<String> {
        text::font_families().unwrap_or_else(|e| {
            tracing::warn!("the font list is not available: {e}");
            Vec::new()
        })
    }

    fn appearance(&self) -> OsAppearance {
        const PERSONALIZE: PCWSTR =
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        const DWM: PCWSTR = w!(r"Software\Microsoft\Windows\DWM");
        let defaults = OsAppearance::default();
        OsAppearance {
            // Missing values: apps use light mode and transparency is on (the OS defaults).
            dark: read_dword(PERSONALIZE, w!("AppsUseLightTheme")).is_some_and(|v| v == 0),
            transparency: read_dword(PERSONALIZE, w!("EnableTransparency")).is_none_or(|v| v != 0),
            // The accent colour as 0xAABBGGRR.
            accent: read_dword(DWM, w!("AccentColor")).map_or(defaults.accent, |v| {
                let [r, g, b, _] = v.to_le_bytes();
                Color::rgba(r, g, b, 1.0)
            }),
            animations: client_area_animation().unwrap_or(defaults.animations),
        }
    }

    fn uptime(&self) -> std::time::Duration {
        // SAFETY: no arguments; it only reads the tick count.
        let ms = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
        std::time::Duration::from_millis(ms)
    }
}

/// A DWORD value under `HKEY_CURRENT_USER`, if present.
/// "Animation effects" (Settings ▸ Accessibility ▸ Visual effects).
fn client_area_animation() -> Option<bool> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut on = windows::core::BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to the pointer.
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut on).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    Some(on.as_bool())
}

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
