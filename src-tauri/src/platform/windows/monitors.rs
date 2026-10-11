//! Monitor enumeration: device name, friendly name, bounds, work area and effective DPI.

use std::collections::HashMap;
use std::mem::size_of;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::BOOL;

use super::from_wide;
use crate::platform::{MonitorInfo, Rect};

pub fn list() -> Vec<MonitorInfo> {
    let friendly = friendly_names();
    handles()
        .into_iter()
        .filter_map(|h| info(h, &friendly))
        .collect()
}

/// Details for one monitor handle; `friendly` maps GDI device names to friendly names.
pub fn info(monitor: HMONITOR, friendly: &HashMap<String, String>) -> Option<MonitorInfo> {
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `mi` is a MONITORINFOEXW whose cbSize says so, as GetMonitorInfoW requires.
    let ok = unsafe { GetMonitorInfoW(monitor, (&raw mut mi).cast::<MONITORINFO>()) };
    if !ok.as_bool() {
        return None;
    }
    let device = from_wide(&mi.szDevice);
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: plain out-parameters.
    let scale = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .map_or(1.0, |()| f64::from(dpi_x) / 96.0);
    let name = friendly
        .get(&device)
        .cloned()
        .unwrap_or_else(|| device.trim_start_matches(r"\\.\").to_owned());
    let primary = mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;
    Some(MonitorInfo::new(
        device,
        name,
        to_rect(mi.monitorInfo.rcMonitor),
        to_rect(mi.monitorInfo.rcWork),
        scale,
        primary,
    ))
}

pub fn to_rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

pub fn to_win_rect(r: Rect) -> RECT {
    RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

fn handles() -> Vec<HMONITOR> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` is the `&mut Vec<HMONITOR>` passed below, alive for the whole call.
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        BOOL(1)
    }
    let mut list: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only runs during this call, while `list` is alive.
    let _ =
        unsafe { EnumDisplayMonitors(None, None, Some(collect), LPARAM(&raw mut list as isize)) };
    list
}

/// GDI device name (`\\.\DISPLAY1`) → monitor friendly name (`DELL U2720Q`), from the display
/// configuration API. Empty if the API is unavailable (e.g. in some remote sessions).
fn friendly_names() -> HashMap<String, String> {
    let mut names = HashMap::new();
    let (mut path_count, mut mode_count) = (0u32, 0u32);
    // SAFETY: out-parameters only.
    if unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
    } != ERROR_SUCCESS
    {
        return names;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: the buffers have the sizes reported above and the counts say so.
    let status = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return names;
    }
    paths.truncate(path_count as usize);
    for path in &paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            },
            ..Default::default()
        };
        // SAFETY: each header starts a correctly sized request packet of the type it names.
        let ok = unsafe {
            DisplayConfigGetDeviceInfo(&mut source.header) == 0
                && DisplayConfigGetDeviceInfo(&mut target.header) == 0
        };
        let name = from_wide(&target.monitorFriendlyDeviceName);
        if ok && !name.is_empty() {
            names.insert(from_wide(&source.viewGdiDeviceName), name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`).
    #[test]
    #[ignore = "needs a Windows desktop session"]
    fn lists_at_least_one_sane_monitor() {
        let monitors = list();
        assert!(!monitors.is_empty(), "no monitors");
        assert_eq!(
            monitors.iter().filter(|m| m.primary).count(),
            1,
            "{monitors:?}"
        );
        for m in &monitors {
            assert!(m.bounds.width() > 0 && m.bounds.height() > 0, "{m:?}");
            assert!(m.scale >= 1.0 && m.scale <= 5.0, "{m:?}");
            assert!(m.device.starts_with(r"\\.\"), "{m:?}");
        }
    }
}
