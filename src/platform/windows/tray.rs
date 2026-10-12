//! The notification-area icon (SPEC §4.1, §6.5), registered for the bar window. Its callback
//! message arrives in the bar window procedure and becomes [`UiEvent::TrayClick`] or
//! [`UiEvent::TrayMenu`](crate::platform::UiEvent).

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{LIM_SMALL, LoadIconMetric};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_SELECT, NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONDATAW_0,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HICON, RegisterWindowMessageW, WM_APP, WM_CONTEXTMENU,
};
use windows::core::{PCWSTR, w};

use crate::platform::{TrayIcon, UiEvent};

/// Broadcast by Explorer when the taskbar is (re)created; icons must be added again.
pub(super) fn taskbar_created() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    // SAFETY: plain call with a static string.
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

/// The tray icon's callback message to the bar window.
pub(super) const WM_TRAY: u32 = WM_APP + 2;
const ICON_ID: u32 = 1;
/// `NIN_SELECT | NINF_KEY`: the icon was selected with the keyboard.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

/// The exe's icon (resource 1, `build.rs`) at the small-icon size for the system DPI.
fn icon() -> HICON {
    static ICON: OnceLock<isize> = OnceLock::new();
    let raw = *ICON.get_or_init(|| {
        // SAFETY: loads a resource of our own module; the icon lives as long as the process.
        unsafe {
            let module = GetModuleHandleW(None).ok();
            // MAKEINTRESOURCE(1): a resource ID in pointer form, never dereferenced as memory.
            let id = PCWSTR(std::ptr::without_provenance(1));
            LoadIconMetric(module.map(Into::into), id, LIM_SMALL).map_or(0, |icon| icon.0 as isize)
        }
    });
    HICON(raw as *mut _)
}

/// Explorer reports Enter on the icon as two keyboard selections (Space as one). A keyboard
/// selection that follows an accepted one within `window` is taken as its duplicate and dropped.
#[derive(Default)]
struct KeySelectFilter {
    /// When the last accepted keyboard selection arrived, until its duplicate is seen.
    pending: Option<Instant>,
}

impl KeySelectFilter {
    fn accept(&mut self, now: Instant, window: Duration) -> bool {
        match self.pending.take() {
            Some(last) if now.saturating_duration_since(last) <= window => false,
            _ => {
                self.pending = Some(now);
                true
            }
        }
    }
}

pub(super) struct Tray {
    hwnd: isize,
    shown: AtomicBool,
    tooltip: Mutex<String>,
    key_select: Mutex<KeySelectFilter>,
}

impl Tray {
    pub(super) fn new(hwnd: HWND) -> Self {
        Self {
            hwnd: hwnd.0 as isize,
            shown: AtomicBool::new(false),
            tooltip: Mutex::new(String::new()),
            key_select: Mutex::new(KeySelectFilter::default()),
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(self.hwnd as *mut _),
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon(),
            Anonymous: NOTIFYICONDATAW_0 {
                uVersion: NOTIFYICON_VERSION_4,
            },
            ..Default::default()
        };
        let tooltip = self.tooltip.lock().unwrap_or_else(PoisonError::into_inner);
        // At most 127 UTF-16 units plus the terminating NUL.
        for (slot, unit) in data.szTip.iter_mut().zip(tooltip.encode_utf16().take(127)) {
            *slot = unit;
        }
        data
    }

    fn notify(&self, message: NOTIFY_ICON_MESSAGE) -> bool {
        let data = self.data();
        // SAFETY: `data` describes our own icon and outlives the call.
        unsafe { Shell_NotifyIconW(message, &data) }.as_bool()
    }

    /// Adds the icon, or updates its tooltip if it is shown already.
    pub(super) fn show(&self, tooltip: &str) {
        *self.tooltip.lock().unwrap_or_else(PoisonError::into_inner) = tooltip.to_owned();
        if self.shown.load(Ordering::SeqCst) {
            self.notify(NIM_MODIFY);
        } else {
            self.add();
        }
    }

    fn add(&self) {
        let added = self.notify(NIM_ADD);
        if added {
            // Version 4: the callback reports NIN_SELECT and WM_CONTEXTMENU.
            self.notify(NIM_SETVERSION);
        } else {
            tracing::warn!("the tray icon could not be added");
        }
        self.shown.store(added, Ordering::SeqCst);
    }

    pub(super) fn hide(&self) {
        if self.shown.swap(false, Ordering::SeqCst) {
            self.notify(NIM_DELETE);
        }
    }

    /// Explorer restarted and forgot the icon (`TaskbarCreated`).
    pub(super) fn restore(&self) {
        if self.shown.load(Ordering::SeqCst) {
            self.add();
        }
    }

    /// Maps the callback message's `lParam` to an event.
    pub(super) fn event(&self, lparam: LPARAM) -> Option<UiEvent> {
        match (lparam.0 & 0xFFFF) as u32 {
            NIN_SELECT => Some(UiEvent::TrayClick),
            NIN_KEYSELECT => {
                // SAFETY: plain call without arguments.
                let window = Duration::from_millis(unsafe { GetDoubleClickTime() }.into());
                let mut filter = self
                    .key_select
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                filter
                    .accept(Instant::now(), window)
                    .then_some(UiEvent::TrayClick)
            }
            WM_CONTEXTMENU => Some(UiEvent::TrayMenu),
            _ => None,
        }
    }
}

impl TrayIcon for Tray {
    fn remove(&self) {
        self.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_duplicate_keyboard_selection_is_dropped() {
        let window = Duration::from_millis(500);
        let mut filter = KeySelectFilter::default();
        let t0 = Instant::now();
        // Enter: two selections at once, one toggle.
        assert!(filter.accept(t0, window));
        assert!(!filter.accept(t0 + Duration::from_millis(5), window));
        // The next press is accepted again, with its own duplicate dropped.
        let t1 = t0 + Duration::from_millis(300);
        assert!(filter.accept(t1, window));
        assert!(!filter.accept(t1 + Duration::from_millis(5), window));
        // Space: one selection per press, far enough apart.
        assert!(filter.accept(t1 + Duration::from_secs(1), window));
        assert!(filter.accept(t1 + Duration::from_secs(2), window));
    }
}
