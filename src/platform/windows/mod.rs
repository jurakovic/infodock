//! Windows implementation of the platform layer (SPEC §4.6, Appendix A).

pub mod appbar;
mod autostart;
mod band;
mod battery;
mod com;
mod cpu;
mod device;
pub mod dialog;
mod disk;
mod droptarget;
mod eject;
mod fileops;
mod hint;
mod icons;
pub mod image;
pub mod instance;
mod locale;
mod memory;
mod menu;
mod monitors;
mod net;
mod popup;
mod power;
pub mod render;
pub mod shell;
pub mod text;
pub mod toast;
mod tray;
mod uia;
pub mod window;
mod wlan;

use std::mem::size_of;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::{FOLDERID_CommonPrograms, FOLDERID_Programs};
use windows::core::{PCWSTR, w};

use super::{Capabilities, Color, Locale, MonitorInfo, OsAppearance, Platform};
use crate::config::model::DateLocale;
use crate::sensors::dates::{Date, LocalTime};

pub struct WindowsPlatform;

/// Starts `probe` on a thread of `scope` and returns what waits for its answer; runs it there
/// and then when no thread can be started. A probe that panics answers its default (none).
fn side_by_side<'scope, T, F>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    probe: F,
) -> impl FnOnce() -> T + 'scope
where
    T: Send + Default + 'scope,
    F: FnOnce() -> T + Send + Copy + 'scope,
{
    let thread = std::thread::Builder::new()
        .name("probe".into())
        .spawn_scoped(scope, probe);
    move || match thread {
        Ok(thread) => thread.join().unwrap_or_default(),
        Err(_) => probe(),
    }
}

/// What the system offers, probed once: it does not change while Infodock runs (a Wi-Fi adapter
/// plugged in later is found at the next start).
static CAPABILITIES: std::sync::OnceLock<Capabilities> = std::sync::OnceLock::new();

impl Platform for WindowsPlatform {
    /// Probed once, when first asked (at start, on a thread of its own).
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
            .get_or_init(|| {
                let started = std::time::Instant::now();
                // The slow probes (PDH loads its counter names, the WLAN client calls its
                // service, the shell loads) run side by side, the shell's in an apartment.
                let (per_core_cpu, wifi, (recycle_bin, start_menu)) = std::thread::scope(|s| {
                    let per_core = side_by_side(s, cpu::per_processor_counters);
                    let wifi = side_by_side(s, wlan::has_interface);
                    let shell = side_by_side(s, || {
                        com::enter();
                        let programs = instance::known_folder(&FOLDERID_Programs);
                        (
                            shell::has_recycle_bin(),
                            programs.is_some_and(|p| p.is_dir()),
                        )
                    });
                    (per_core(), wifi(), shell())
                });
                let capabilities = Capabilities {
                    appbar: true,
                    recycle_bin,
                    start_menu,
                    wifi,
                    battery: power::has_battery(),
                    power_modes: power::modes(),
                    // Ejecting needs no hardware to exist; Eject USB lists what there is.
                    eject: true,
                    per_core_cpu,
                };
                tracing::info!(
                    "capabilities: {capabilities:?} (probed in {} ms)",
                    started.elapsed().as_millis()
                );
                crate::logging::startup_step("capabilities");
                capabilities
            })
            .clone()
    }

    fn probed_capabilities(&self) -> Option<Capabilities> {
        CAPABILITIES.get().cloned()
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

    fn memory(&self) -> Result<super::MemoryStatus, String> {
        memory::status()
    }

    fn page_files(&self) -> Result<Vec<super::PageFile>, String> {
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

    fn list_dir(&self, path: &str) -> Result<Vec<super::DirItem>, String> {
        disk::list_dir(path)
    }

    fn compare_names(&self, a: &str, b: &str) -> std::cmp::Ordering {
        locale::compare_names(a, b)
    }

    fn reveal(&self, path: &str) {
        shell::reveal(path);
    }

    fn file_operation(
        &self,
        op: super::FileOp,
        paths: Vec<String>,
        folder: String,
        done: Box<dyn FnOnce() + Send>,
    ) {
        fileops::file_operation(op, paths, folder, done);
    }

    fn recycle(&self, paths: Vec<String>, done: Box<dyn FnOnce() + Send>) {
        fileops::recycle(paths, done);
    }

    fn known_folder(&self, folder: super::KnownFolder) -> Option<String> {
        let id = match folder {
            super::KnownFolder::Programs => &FOLDERID_Programs,
            super::KnownFolder::CommonPrograms => &FOLDERID_CommonPrograms,
        };
        instance::known_folder(id).map(|p| p.to_string_lossy().into_owned())
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

    fn wifi_reader(
        &self,
        changed: Box<dyn Fn() + Send + Sync>,
    ) -> Option<Box<dyn super::WifiReader>> {
        wlan::WinWifi::open(changed).map(|w| Box::new(w) as Box<dyn super::WifiReader>)
    }

    fn power_status(&self) -> Result<super::PowerStatus, String> {
        battery::status()
    }

    fn recycle_bin(&self) -> Result<super::RecycleBin, String> {
        shell::recycle_bin()
    }

    fn empty_recycle_bin(&self, done: Box<dyn FnOnce() + Send>) {
        shell::empty_recycle_bin(done);
    }

    fn power(&self, action: super::PowerAction, force: bool) {
        power::power(action, force);
    }

    fn eject(&self, root: &str, report: super::EjectReport) {
        let root = root.to_owned();
        let spawned = std::thread::Builder::new()
            .name("eject".into())
            .spawn(move || {
                let result = eject::eject(&root);
                tracing::info!("eject {root}: {result:?}");
                let (title, body) = report(&result);
                toast::notify(&title, &body);
            });
        if let Err(e) = spawned {
            tracing::warn!("could not start ejecting a drive: {e}");
        }
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

    fn autostart(&self, name: &str) -> Option<String> {
        autostart::read(name)
    }

    fn decode_code_page(&self, bytes: &[u8], code_page: Option<u32>) -> Option<String> {
        locale::decode(bytes, code_page)
    }

    fn set_autostart(&self, name: &str, change: &super::AutostartChange) -> Result<(), String> {
        autostart::change(name, change)
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
