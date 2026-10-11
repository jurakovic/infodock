//! Windows implementation of the platform layer (SPEC §4.6, Appendix A).

pub mod appbar;
mod monitors;

use std::mem::size_of;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

use super::{Capabilities, MonitorInfo, Platform};

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

    fn transparency_effects(&self) -> bool {
        let mut value: u32 = 1;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: `value` is a DWORD-sized out buffer and `size` says so.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
                w!("EnableTransparency"),
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut value).cast()),
                Some(&mut size),
            )
        };
        // Missing value: transparency is on by default.
        status != ERROR_SUCCESS || value != 0
    }
}

/// A NUL-terminated UTF-16 buffer as a `String`.
fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
