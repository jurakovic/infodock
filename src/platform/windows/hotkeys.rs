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
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    RegisterHotKey, UnregisterHotKey, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
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
        tracing::debug!("no longer watching for hotkeys other apps hold");
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
            // SAFETY: a plain query; the hook procedure runs on this thread.
            let thread = unsafe { GetCurrentThreadId() };
            tracing::debug!("watching for hotkeys other apps hold (thread {thread})");
            Some(Hook(hook))
        }
        Err(e) => {
            tracing::warn!("cannot watch for hotkeys other apps hold: {e}");
            None
        }
    }
}

unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: for a low-level keyboard hook lParam points to a KBDLLHOOKSTRUCT.
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if down && keep(event.vkCode, event.flags.0 & LLKHF_INJECTED.0 != 0) {
            // Kept from the app that holds it: the user is choosing a hotkey.
            return LRESULT(1);
        }
    }
    // SAFETY: passes the event on, as every hook must.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Whether the key `vk` just pressed makes a combination someone holds, which is then reported
/// and kept from them. Each step is logged (trace), as the hook is hard to watch otherwise.
fn keep(vk: u32, injected: bool) -> bool {
    // SAFETY: a plain query; the hook runs on the thread that installed it.
    let thread = unsafe { GetCurrentThreadId() };
    let Some(combo) = combo(vk).filter(|c| c.could_be_hotkey()) else {
        tracing::trace!(vk, injected, thread, "hook: not a hotkey combination");
        return false;
    };
    let held = held_elsewhere(&combo, vk);
    let listening = TAKEN.with(|t| t.try_borrow().map(|t| t.is_some()).unwrap_or(false));
    tracing::trace!(
        vk,
        injected,
        thread,
        ?combo,
        held,
        listening,
        "hook: a combination"
    );
    if !held {
        return false;
    }
    TAKEN
        .with(|t| {
            t.try_borrow()
                .ok()
                .and_then(|t| t.as_ref().map(|taken| taken(combo)))
        })
        .is_some()
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
    use std::sync::mpsc;
    use std::time::Duration;

    use windows::Win32::UI::Input::KeyboardAndMouse::{
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_F23, keybd_event,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_HOTKEY, WM_QUIT,
    };

    use super::*;

    /// Runs `body` on a new thread with a message loop until [`quit`] posts it `WM_QUIT`.
    /// `body` runs first: the first part of its result comes back, the second is kept on the
    /// thread until the loop ends. The thread's id, the result and the number of `WM_HOTKEY`s
    /// the loop saw (from `join`) come back.
    fn with_message_loop<T: Send + 'static, K>(
        body: impl FnOnce() -> (T, K) + Send + 'static,
    ) -> (u32, mpsc::Receiver<T>, std::thread::JoinHandle<u32>) {
        let (ready, started) = mpsc::channel();
        let (result, results) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut msg = MSG::default();
            // SAFETY: creates this thread's message queue, so a posted WM_QUIT is not lost.
            let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
            let (sent, _kept) = body();
            let _ = result.send(sent);
            // SAFETY: as above.
            let _ = ready.send(unsafe { GetCurrentThreadId() });
            let mut hotkeys = 0;
            // SAFETY: a plain message loop for this thread.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                if msg.message == WM_HOTKEY {
                    hotkeys += 1;
                }
            }
            hotkeys
        });
        let id = started.recv().expect("the thread started");
        (id, results, thread)
    }

    fn quit(thread: u32) {
        // SAFETY: posts to a thread of this test that has a message queue.
        let _ = unsafe { PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }

    fn key(vk: VIRTUAL_KEY, up: bool) {
        let flags = if up {
            KEYEVENTF_KEYUP
        } else {
            KEYBD_EVENT_FLAGS(0)
        };
        // SAFETY: an injected key event, as a user's.
        unsafe { keybd_event(vk.0 as u8, 0, flags, 0) };
    }

    #[test]
    #[ignore = "registers a hotkey and injects keys on this machine"]
    fn a_held_combination_pressed_while_watching_is_reported_and_kept_from_its_holder() {
        // Ctrl+Alt+Shift+F23 is held by another thread, as by another app.
        let (holder, registered, holder_thread) = with_message_loop(|| {
            let modifiers = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0);
            // SAFETY: a thread hotkey for the test; it goes with the thread.
            (
                unsafe { RegisterHotKey(None, 1, modifiers, u32::from(VK_F23.0)) }.is_ok(),
                (),
            )
        });
        assert!(
            registered.recv().expect("registered"),
            "the holder holds it"
        );
        let (combos, reports) = mpsc::channel();
        let (watcher, hook, watcher_thread) = with_message_loop(move || {
            let hook = watch(Box::new(move |combo| {
                let _ = combos.send(combo);
            }));
            (hook.is_some(), hook)
        });
        assert!(hook.recv().expect("watching"), "the hook is installed");

        for vk in [VK_CONTROL, VK_MENU, VK_SHIFT, VK_F23] {
            key(vk, false);
        }
        for vk in [VK_F23, VK_SHIFT, VK_MENU, VK_CONTROL] {
            key(vk, true);
        }
        let reported = reports.recv_timeout(Duration::from_secs(2));

        quit(watcher);
        let _ = watcher_thread.join();
        quit(holder);
        let hotkeys = holder_thread.join().expect("holder");
        assert_eq!(
            reported.ok(),
            Some(KeyCombo {
                ctrl: true,
                alt: true,
                shift: true,
                win: false,
                code: "F23",
            }),
            "the hook reports the combination"
        );
        assert_eq!(hotkeys, 0, "the holder never gets it");
    }

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
