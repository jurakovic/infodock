//! Windows implementation of the platform layer (SPEC §4.6, Appendix A).

pub mod appbar;
mod com;
mod cpu;
mod device;
mod dialog;
mod disk;
mod hint;
mod icons;
pub mod image;
pub mod instance;
mod locale;
mod memory;
mod menu;
mod monitors;
mod net;
mod power;
pub mod render;
pub mod shell;
pub mod text;
pub mod toast;
mod tray;
pub mod window;
mod wlan;

use std::mem::size_of;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::FOLDERID_Programs;
use windows::core::{PCWSTR, w};

use super::{Capabilities, Color, Locale, MonitorInfo, OsAppearance, Platform};
use crate::config::model::DateLocale;
use crate::sensors::dates::{Date, LocalTime};

pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    /// Read once: what the system offers does not change while Infodock runs (a Wi-Fi adapter
    /// plugged in later is found at the next start).
    fn capabilities(&self) -> Capabilities {
        static CAPABILITIES: std::sync::OnceLock<Capabilities> = std::sync::OnceLock::new();
        CAPABILITIES
            .get_or_init(|| {
                let capabilities = Capabilities {
                    appbar: true,
                    recycle_bin: shell::has_recycle_bin(),
                    start_menu: instance::known_folder(&FOLDERID_Programs)
                        .is_some_and(|p| p.is_dir()),
                    wifi: wlan::has_interface(),
                    battery: power::has_battery(),
                    power_modes: power::modes(),
                    // Ejecting needs no hardware to exist; Eject USB lists what there is.
                    eject: true,
                    per_core_cpu: cpu::per_processor_counters(),
                };
                tracing::info!("capabilities: {capabilities:?}");
                capabilities
            })
            .clone()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        monitors::list()
    }

    fn locale(&self, which: DateLocale) -> Locale {
        locale::read(which)
    }

    fn cpu_reader(&self) -> Option<Box<dyn super::CpuReader>> {
        Some(Box::new(cpu::WinCpu::new()))
    }

    fn logical_processors(&self) -> u32 {
        cpu::logical_processors()
    }

    fn memory(&self) -> Option<super::MemoryStatus> {
        memory::status()
    }

    fn page_files(&self) -> Option<Vec<super::PageFile>> {
        memory::page_files()
    }

    fn volumes(&self) -> Vec<super::Volume> {
        disk::volumes()
    }

    fn disk_space(&self, root: &str) -> Option<super::DiskSpace> {
        disk::space(root)
    }

    fn path_kind(&self, path: &str) -> super::PathKind {
        disk::path_kind(path)
    }

    fn net_interfaces(&self) -> Vec<super::NetInterface> {
        net::interfaces()
    }

    fn net_interface(&self, id: Option<&str>) -> Option<super::NetInterface> {
        net::interface(id)
    }

    fn local_time(&self) -> LocalTime {
        // SAFETY: no arguments; it only reads the clock.
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let date = Date::new(i32::from(t.wYear), t.wMonth as u8, t.wDay as u8)
            .unwrap_or_else(|| Date::from_days(0));
        LocalTime {
            date,
            hour: t.wHour as u8,
            minute: t.wMinute as u8,
            second: t.wSecond as u8,
            millis: t.wMilliseconds,
        }
    }

    fn launch(&self, launch: &super::Launch) {
        shell::launch(launch);
    }

    fn shell_icon(&self, path: &str, px: u32) -> Option<super::Bitmap> {
        icons::shell_icon(path, px)
    }

    fn notify(&self, title: &str, body: &str) {
        toast::notify(title, body);
    }

    fn power(&self, action: super::PowerAction, force: bool) {
        power::power(action, force);
    }

    fn play_sound(&self, wav: &'static [u8]) {
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        // SAFETY: with SND_MEMORY the "name" is the sound in memory; it is static, so it
        // outlives the asynchronous playback.
        let played = unsafe {
            PlaySoundW(
                PCWSTR(wav.as_ptr().cast()),
                None,
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            )
        };
        if !played.as_bool() {
            tracing::warn!("could not play a sound");
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`): a server without Wi-Fi or a
    /// battery, with a shell.
    #[test]
    #[ignore = "reads this machine's capabilities"]
    fn capabilities_of_this_machine() {
        let caps = WindowsPlatform.capabilities();
        assert!(caps.appbar && caps.eject, "{caps:?}");
        assert!(caps.recycle_bin, "{caps:?}");
        assert!(caps.start_menu, "{caps:?}");
        // Asked again: the same, without reading again.
        assert_eq!(WindowsPlatform.capabilities(), caps);
    }
}
