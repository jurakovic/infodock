//! Noticing a hotkey combination another app holds while Settings records one (SPEC §6.5).
//!
//! A combination registered with `RegisterHotKey` goes to its app as `WM_HOTKEY` and never
//! reaches the Settings window, so recording would wait for keys that never come. While it
//! records, a low-level keyboard hook (which sees keys before Windows routes hotkeys) tries to
//! register each combination pressed: when that fails, someone holds it, and the hook reports it
//! and keeps the keys from that app. Other keys go on to the window as usual.
//!
//! The hook runs on a thread of its own with a plain message loop, so it answers at once while
//! Settings' thread draws. Windows calls none of the low-level keyboard hooks while the newest is
//! a process's that reads the keyboard's raw input, so the app turns winit's off (`lib.rs`,
//! `NoDeviceEvents`; S14x2, D120).

use std::cell::RefCell;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    RegisterHotKey, UnregisterHotKey, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_NOREMOVE,
    PeekMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
};

use crate::platform::guard::guard;
use crate::platform::keys::{KeyCombo, code_of_vk};

/// The id of the test registration; thread hotkeys take ids up to 0xBFFF.
const PROBE_ID: i32 = 0xB1F0;

/// Told of each combination someone holds, on the watch's thread.
type Taken = Box<dyn Fn(KeyCombo) + Send>;

thread_local! {
    /// The listener, on the watch's thread while its hook is installed.
    static TAKEN: RefCell<Option<Taken>> = const { RefCell::new(None) };
}

/// The running watch; dropping it removes the hook and ends its thread.
pub(crate) struct Watch {
    thread: u32,
    handle: Option<JoinHandle<()>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: posts to the watch's thread, whose message queue exists (`watch`).
        let _ = unsafe { PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        tracing::debug!("no longer watching for hotkeys other apps hold");
    }
}

/// Watches the keyboard from a thread of its own until the watch is dropped; `taken` hears of
/// each combination someone holds, on that thread. Returns once the hook is installed, so keys
/// pressed from then on are seen.
pub(crate) fn watch(taken: Taken) -> Option<Watch> {
    let (ready, installed) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("hotkey-watch".into())
        .spawn(move || {
            let mut msg = MSG::default();
            // SAFETY: creates this thread's message queue, so the WM_QUIT that ends it is kept.
            let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
            // SAFETY: a low-level hook with a static procedure; this module is its instance.
            let hook = unsafe {
                GetModuleHandleW(None).and_then(|instance| {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(instance.into()), 0)
                })
            };
            let hook = match hook {
                Ok(hook) => hook,
                Err(e) => {
                    tracing::warn!("cannot watch for hotkeys other apps hold: {e}");
                    let _ = ready.send(None);
                    return;
                }
            };
            TAKEN.with(|t| *t.borrow_mut() = Some(taken));
            // SAFETY: a plain query.
            let thread = unsafe { GetCurrentThreadId() };
            let _ = ready.send(Some(thread));
            tracing::debug!("watching for hotkeys other apps hold (thread {thread})");
            // The hook is called while this thread waits for messages; WM_QUIT ends the wait.
            // SAFETY: a plain message loop for a thread without windows.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {}
            // SAFETY: the hook was installed above, on this thread, and is removed once.
            let _ = unsafe { UnhookWindowsHookEx(hook) };
            TAKEN.with(|t| *t.borrow_mut() = None);
        });
    let handle = match spawned {
        Ok(handle) => handle,
        Err(e) => {
            tracing::warn!("cannot watch for hotkeys other apps hold: {e}");
            return None;
        }
    };
    match installed.recv() {
        Ok(Some(thread)) => Some(Watch {
            thread,
            handle: Some(handle),
        }),
        _ => {
            let _ = handle.join();
            None
        }
    }
}

unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let kept = guard(
        "the keyboard hook",
        || false,
        || {
            if code != HC_ACTION as i32 || lparam.0 == 0 {
                return false;
            }
            // SAFETY: for a low-level keyboard hook lParam points to a KBDLLHOOKSTRUCT.
            let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            down && keep(event.vkCode, event.flags.0 & LLKHF_INJECTED.0 != 0)
        },
    );
    if kept {
        // Kept from the app that holds it: the user is choosing a hotkey.
        return LRESULT(1);
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
    use std::time::Duration;

    use windows::Win32::UI::Input::KeyboardAndMouse::{
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_F24, keybd_event,
    };
    use windows::Win32::UI::WindowsAndMessaging::WM_HOTKEY;

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
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
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
        // Ctrl+Alt+Shift+F24 is held by another thread, as by another app (F23 is the other
        // test's, which may run at the same time). Watched from this test's thread, which pumps
        // no messages: the watch has its own.
        let (holder, registered, holder_thread) = with_message_loop(|| {
            let modifiers = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0);
            // SAFETY: a thread hotkey for the test; it goes with the thread.
            (
                unsafe { RegisterHotKey(None, 1, modifiers, u32::from(VK_F24.0)) }.is_ok(),
                (),
            )
        });
        assert!(
            registered.recv().expect("registered"),
            "the holder holds it"
        );
        let (combos, reports) = mpsc::channel();
        let watching = watch(Box::new(move |combo| {
            let _ = combos.send(combo);
        }))
        .expect("the hook is installed");

        for vk in [VK_CONTROL, VK_MENU, VK_SHIFT, VK_F24] {
            key(vk, false);
        }
        for vk in [VK_F24, VK_SHIFT, VK_MENU, VK_CONTROL] {
            key(vk, true);
        }
        let reported = reports.recv_timeout(Duration::from_secs(2));

        drop(watching);
        quit(holder);
        let hotkeys = holder_thread.join().expect("holder");
        assert_eq!(
            reported.ok(),
            Some(KeyCombo {
                ctrl: true,
                alt: true,
                shift: true,
                win: false,
                code: "F24",
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
