//! Noticing a hotkey combination another app holds while Settings records one (SPEC §6.5).
//!
//! A combination registered with `RegisterHotKey` goes to its app as `WM_HOTKEY` and never
//! reaches the Settings window, so recording would wait for keys that never come. While it
//! records, a low-level keyboard hook (which sees keys before Windows routes hotkeys) tries to
//! register each combination pressed: when that fails, someone holds it, and the hook reports it
//! and keeps the keys from that app. Other keys go on to the window as usual.

use std::cell::RefCell;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    RegisterHotKey, UnregisterHotKey, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::platform::keys::{KeyCombo, code_of_vk};

/// The id of the test registration; thread hotkeys take ids up to 0xBFFF.
const PROBE_ID: i32 = 0xB1F0;

/// Told of each combination someone holds.
type Taken = Box<dyn Fn(KeyCombo)>;

thread_local! {
    /// The listener while a hook is installed on this thread.
    static TAKEN: RefCell<Option<Taken>> = const { RefCell::new(None) };
}

/// The installed hook; dropping it removes it.
pub(crate) struct Hook(HHOOK);

impl Drop for Hook {
    fn drop(&mut self) {
        // SAFETY: the hook was installed by `watch` on this thread and is removed once.
        let _ = unsafe { UnhookWindowsHookEx(self.0) };
        TAKEN.with(|t| *t.borrow_mut() = None);
    }
}

/// Watches the keyboard on this thread (Settings', which pumps messages) until the hook is
/// dropped; `taken` hears of each combination someone holds.
pub(crate) fn watch(taken: Taken) -> Option<Hook> {
    // SAFETY: a low-level hook with a static procedure; this module is its instance.
    let hook = unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(instance.into()), 0)
    };
    match hook {
        Ok(hook) => {
            TAKEN.with(|t| *t.borrow_mut() = Some(taken));
            Some(Hook(hook))
        }
        Err(e) => {
            tracing::warn!("cannot watch for hotkeys other apps hold: {e}");
            None
        }
    }
}

unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    if code == HC_ACTION as i32 && down && lparam.0 != 0 {
        // SAFETY: for a low-level keyboard hook lParam points to a KBDLLHOOKSTRUCT.
        let vk = unsafe { (*(lparam.0 as *const KBDLLHOOKSTRUCT)).vkCode };
        if let Some(combo) = combo(vk).filter(|c| c.could_be_hotkey())
            && held_elsewhere(&combo, vk)
        {
            let reported = TAKEN.with(|t| {
                t.try_borrow()
                    .ok()
                    .and_then(|t| t.as_ref().map(|taken| taken(combo)))
            });
            if reported.is_some() {
                // Kept from the app that holds it: the user is choosing a hotkey.
                return LRESULT(1);
            }
        }
    }
    // SAFETY: passes the event on, as every hook must.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// The key `vk` with the modifiers held now.
fn combo(vk: u32) -> Option<KeyCombo> {
    // SAFETY: plain key-state queries; the high bit is set while the key is down.
    let down = |key: VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0;
    Some(KeyCombo {
        ctrl: down(VK_CONTROL),
        alt: down(VK_MENU),
        shift: down(VK_SHIFT),
        win: down(VK_LWIN) || down(VK_RWIN),
        code: code_of_vk(vk)?,
    })
}

/// Someone holds `combo` (the key `vk`): it cannot be registered. Infodock's own hotkey counts
/// too; Settings takes that as choosing it again.
pub(crate) fn held_elsewhere(combo: &KeyCombo, vk: u32) -> bool {
    let mut modifiers = MOD_NOREPEAT;
    for (held, flag) in [
        (combo.ctrl, MOD_CONTROL),
        (combo.alt, MOD_ALT),
        (combo.shift, MOD_SHIFT),
        (combo.win, MOD_WIN),
    ] {
        if held {
            modifiers = HOT_KEY_MODIFIERS(modifiers.0 | flag.0);
        }
    }
    // SAFETY: a thread hotkey registered and at once removed again.
    unsafe {
        if RegisterHotKey(None, PROBE_ID, modifiers, vk).is_ok() {
            let _ = UnregisterHotKey(None, PROBE_ID);
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "registers a hotkey on this machine"]
    fn a_combination_another_thread_holds_is_seen_as_held() {
        // Ctrl+Alt+Shift+F23 is held by another thread, as by another app.
        let (held, release) = std::sync::mpsc::channel::<bool>();
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            let modifiers = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0);
            // SAFETY: a thread hotkey for the test, removed below.
            let ok = unsafe { RegisterHotKey(None, 1, modifiers, 0x86) }.is_ok();
            let _ = held.send(ok);
            let _ = finished.recv();
            // SAFETY: as above.
            let _ = unsafe { UnregisterHotKey(None, 1) };
        });
        assert!(
            release.recv().expect("registered"),
            "the holder registered it"
        );
        let f23 = KeyCombo {
            ctrl: true,
            alt: true,
            shift: true,
            win: false,
            code: "F23",
        };
        assert!(held_elsewhere(&f23, 0x86));
        let f22 = KeyCombo { code: "F22", ..f23 };
        assert!(!held_elsewhere(&f22, 0x85), "a free combination");
        let _ = done.send(());
        holder.join().expect("holder");
        assert!(!held_elsewhere(&f23, 0x86), "free again once released");
    }
}
