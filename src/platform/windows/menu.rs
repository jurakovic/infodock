//! Native popup menus (SPEC §6.4), shown at the pointer from the bar thread.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, MF_CHECKED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, SM_MENUDROPALIGNMENT, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR};

use crate::platform::MenuEntry;

/// Shows `entries` at the pointer and returns the chosen id. Runs a modal loop.
pub(super) fn show(hwnd: HWND, entries: &[MenuEntry]) -> Option<u32> {
    // SAFETY: the menu is created, shown and destroyed here; `hwnd` is the bar window.
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for entry in entries {
            let result = match entry {
                MenuEntry::Item {
                    id,
                    label,
                    checked,
                    enabled,
                } => {
                    let mut flags = MF_STRING;
                    if *checked {
                        flags |= MF_CHECKED;
                    }
                    if !*enabled {
                        flags |= MF_GRAYED;
                    }
                    AppendMenuW(menu, flags, *id as usize, &HSTRING::from(label.as_str()))
                }
                MenuEntry::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()),
            };
            if let Err(e) = result {
                tracing::warn!("menu entry not added: {e}");
            }
        }

        let mut pointer = POINT::default();
        let _ = GetCursorPos(&mut pointer);
        let horizontal = if GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0 {
            TPM_RIGHTALIGN
        } else {
            TPM_LEFTALIGN
        };
        // The owner must be in the foreground, or the menu does not close when the user
        // clicks elsewhere; the WM_NULL afterwards is the documented companion (KB135788).
        let _ = SetForegroundWindow(hwnd);
        let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | horizontal;
        let chosen = TrackPopupMenuEx(menu, flags.0, pointer.x, pointer.y, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        u32::try_from(chosen.0).ok().filter(|id| *id != 0)
    }
}
