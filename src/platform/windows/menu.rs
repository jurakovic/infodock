//! Native popup menus (SPEC §6.4), shown at the pointer or beside the focused sensor from the
//! bar thread.

use std::mem::size_of;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetForegroundWindow, GetSystemMetrics,
    HMENU, IsWindow, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW,
    SM_MENUDROPALIGNMENT, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TPM_TOPALIGN, TPM_VERTICAL, TPMPARAMS,
    TrackPopupMenuEx, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR};

use crate::platform::{MenuEntry, Rect};

/// A popup menu with `entries`; submenus are owned by it and destroyed with it.
fn build(entries: &[MenuEntry]) -> Option<HMENU> {
    // SAFETY: plain menu construction; the caller destroys the returned menu.
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
                MenuEntry::Submenu { label, entries } => match build(entries) {
                    Some(sub) => AppendMenuW(
                        menu,
                        MF_POPUP,
                        sub.0 as usize,
                        &HSTRING::from(label.as_str()),
                    ),
                    None => Ok(()),
                },
            };
            if let Err(e) = result {
                tracing::warn!("menu entry not added: {e}");
            }
        }
        Some(menu)
    }
}

/// Shows `entries` at the pointer, or beside `near` (physical screen px) without covering it,
/// and returns the chosen id. Runs a modal loop. The bar takes the foreground for it; a menu
/// closed without a choice (Esc, a click on the bar) gives the foreground back to the window
/// that had it, as Esc on the focused bar does (SPEC §6.4, §14.4).
pub(super) fn show(hwnd: HWND, entries: &[MenuEntry], near: Option<Rect>) -> Option<u32> {
    // SAFETY: the menu is created, shown and destroyed here; `hwnd` is the bar window.
    unsafe {
        let menu = build(entries)?;
        let right = GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0;
        let horizontal = if right { TPM_RIGHTALIGN } else { TPM_LEFTALIGN };
        let mut flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | horizontal;
        let (x, y, params) = match near {
            // Below the rect where there is room, else above it (a bar at the bottom).
            Some(r) => {
                flags |= TPM_TOPALIGN | TPM_VERTICAL;
                let params = TPMPARAMS {
                    cbSize: size_of::<TPMPARAMS>() as u32,
                    rcExclude: RECT {
                        left: r.left,
                        top: r.top,
                        right: r.right,
                        bottom: r.bottom,
                    },
                };
                (if right { r.right } else { r.left }, r.bottom, Some(params))
            }
            None => {
                let mut pointer = POINT::default();
                let _ = GetCursorPos(&mut pointer);
                flags |= TPM_BOTTOMALIGN;
                (pointer.x, pointer.y, None)
            }
        };
        // The owner must be in the foreground, or the menu does not close when the user
        // clicks elsewhere; the WM_NULL afterwards is the documented companion (KB135788).
        let before = GetForegroundWindow();
        let _ = SetForegroundWindow(hwnd);
        let params = params.as_ref().map(std::ptr::from_ref);
        let chosen = TrackPopupMenuEx(menu, flags.0, x, y, hwnd, params);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        let chosen = u32::try_from(chosen.0).ok().filter(|id| *id != 0);
        // Not one of the bar thread's own windows (the bar keeps the foreground while the
        // keyboard is on it; a popup closed before the menu).
        let other = !before.is_invalid()
            && !super::window::on_this_thread(before)
            && IsWindow(Some(before)).as_bool();
        if give_back(chosen.is_some(), other, GetForegroundWindow() == hwnd)
            && !SetForegroundWindow(before).as_bool()
        {
            tracing::debug!("the focus could not go back after the menu");
        }
        chosen
    }
}

/// Whether the window that had the foreground before a menu gets it back: the menu closed
/// without a choice, another app's window had it (not the bar, which keeps it while the
/// keyboard is on the bar), and the bar still has it (a click elsewhere gave it to that
/// window already).
fn give_back(chosen: bool, other_had_it: bool, bar_has_it: bool) -> bool {
    !chosen && other_had_it && bar_has_it
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_focus_goes_back_only_after_a_menu_closed_on_the_bar() {
        assert!(
            give_back(false, true, true),
            "Esc: back to the user's window"
        );
        assert!(!give_back(true, true, true), "a choice does what it does");
        assert!(
            !give_back(false, false, true),
            "the bar had the keyboard: it keeps it"
        );
        assert!(
            !give_back(false, true, false),
            "a click elsewhere chose the window"
        );
    }
}
