//! Close requests (INTERNALS.md, Threads and windows): a `WM_CLOSE` posted to any window of a
//! thread that watches for them asks the app to quit, as one to the bar window does.
//!
//! `taskkill` without `/f` posts `WM_CLOSE` to one top-level window of the process, the first it
//! finds in z-order. With the bar kept behind other windows that is a hidden one (winit's, the
//! hotkey's, the drive watch's) or Settings, whose procedures would destroy the window, ignore
//! the message or close Settings alone. So each thread with windows hooks the messages it takes
//! from its queue (`WH_GETMESSAGE`) and hands a posted `WM_CLOSE` to the bar window. A sent one,
//! as a window's close button sends it, never passes the hook and closes that window only.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetClassNameW, HC_ACTION, MSG, PM_REMOVE, PostMessageW, SetWindowsHookExW,
    WH_GETMESSAGE, WM_CLOSE, WM_NULL,
};

use crate::platform::guard::guard;

/// The bar window, where the threads that watch after it was created hand their close requests;
/// 0 until then.
static BAR: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    /// Where this thread hands its close requests; 0 while it does not watch.
    static TARGET: Cell<isize> = const { Cell::new(0) };
}

/// The bar window was created on this thread: close requests to the thread's other windows, and
/// to those of the threads that watch from now on, go to it.
pub(super) fn bar_created(bar: HWND) {
    BAR.store(bar.0 as isize, Ordering::SeqCst);
    watch(bar);
}

/// Hands a `WM_CLOSE` posted to any of this thread's windows to the bar window, if it exists.
pub(crate) fn watch_for_bar() {
    let bar = BAR.load(Ordering::SeqCst);
    if bar != 0 {
        watch(HWND(bar as *mut c_void));
    }
}

/// From now on, a `WM_CLOSE` posted to one of this thread's windows other than `to` goes to `to`
/// instead, and the window it was posted to gets nothing (`WM_NULL`). Once per thread; the hook
/// lasts as long as the thread.
fn watch(to: HWND) {
    if TARGET.get() != 0 {
        return;
    }
    TARGET.set(to.0 as isize);
    // SAFETY: a hook for this thread only, whose procedure is in this module (no module handle).
    let hook =
        unsafe { SetWindowsHookExW(WH_GETMESSAGE, Some(get_message), None, GetCurrentThreadId()) };
    if let Err(e) = hook {
        TARGET.set(0);
        tracing::warn!("close requests to this thread's windows are not watched: {e}");
    }
}

unsafe extern "system" fn get_message(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let removed = wparam.0 & PM_REMOVE.0 as usize != 0;
    if code == HC_ACTION as i32 && removed && lparam.0 != 0 {
        guard(
            "the close request hook",
            || (),
            || {
                // SAFETY: for WH_GETMESSAGE lParam points to the message being taken from the
                // queue, which the hook may change.
                hand_over(unsafe { &mut *(lparam.0 as *mut MSG) });
            },
        );
    }
    // SAFETY: passes the message on to the thread's other hooks.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Hands `msg` to this thread's target when it is a close request to another window.
fn hand_over(msg: &mut MSG) {
    let to = TARGET.get();
    if msg.message != WM_CLOSE || msg.hwnd.is_invalid() || msg.hwnd.0 as isize == to || to == 0 {
        return;
    }
    let target = HWND(to as *mut c_void);
    // SAFETY: posting to the bar window, which lives as long as the process.
    if unsafe { PostMessageW(Some(target), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok() {
        tracing::info!(
            "a close request to the {} window quits the app",
            class_name(msg.hwnd)
        );
        msg.message = WM_NULL;
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut name = [0u16; 64];
    // SAFETY: the buffer is valid for its length.
    let len = unsafe { GetClassNameW(hwnd, &mut name) };
    String::from_utf16_lossy(&name[..usize::try_from(len).unwrap_or(0)])
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, IsWindow, MSG, PM_REMOVE, PeekMessageW,
        PostMessageW, RegisterClassExW, SendMessageW, WINDOW_EX_STYLE, WM_CLOSE, WNDCLASSEXW,
        WNDPROC, WS_POPUP,
    };
    use windows::core::{PCWSTR, w};

    use super::watch;

    /// The `WM_CLOSE`s the target window received.
    static RECEIVED: AtomicU32 = AtomicU32::new(0);

    /// The target's procedure: counts the `WM_CLOSE`s, as the bar window quits on one.
    extern "system" fn target_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CLOSE {
            RECEIVED.fetch_add(1, Ordering::SeqCst);
            return LRESULT(0);
        }
        // SAFETY: default processing.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Default processing only, which destroys the window on `WM_CLOSE`.
    extern "system" fn plain_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: default processing.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// A hidden top-level window of `class` on this thread.
    fn window(class: PCWSTR, proc: WNDPROC) -> HWND {
        // SAFETY: plain class registration and window creation for the test.
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None).expect("module").into();
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: proc,
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )
            .expect("window")
        }
    }

    /// Handles this thread's messages until `received` close requests reached the target, then
    /// a little longer, so one that should not come has had its chance; at most 2 s.
    fn pump_until(received: u32) {
        let until = Instant::now() + Duration::from_secs(2);
        let mut settle = 10;
        while settle > 0 && Instant::now() < until {
            if RECEIVED.load(Ordering::SeqCst) >= received {
                settle -= 1;
            }
            let mut msg = MSG::default();
            // SAFETY: plain message loop calls.
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "needs a Windows desktop session"]
    fn a_close_request_posted_to_another_window_goes_to_the_target() {
        let target = window(w!("InfodockTest.CloseTarget"), Some(target_proc));
        let other = window(w!("InfodockTest.CloseOther"), Some(plain_proc));
        watch(target);

        // SAFETY: posting to this thread's own windows.
        unsafe { PostMessageW(Some(other), WM_CLOSE, WPARAM(0), LPARAM(0)) }.expect("posted");
        pump_until(1);
        assert_eq!(RECEIVED.load(Ordering::SeqCst), 1, "handed to the target");
        // SAFETY: plain query.
        assert!(
            unsafe { IsWindow(Some(other)) }.as_bool(),
            "the other window got nothing"
        );

        // One posted to the target reaches it once, not again through the hook.
        // SAFETY: as above.
        unsafe { PostMessageW(Some(target), WM_CLOSE, WPARAM(0), LPARAM(0)) }.expect("posted");
        pump_until(2);
        assert_eq!(RECEIVED.load(Ordering::SeqCst), 2);

        // A sent one, as a close button sends it, is the window's own.
        // SAFETY: as above.
        unsafe { SendMessageW(other, WM_CLOSE, None, None) };
        // SAFETY: plain query.
        assert!(
            !unsafe { IsWindow(Some(other)) }.as_bool(),
            "closed by its own procedure"
        );
        assert_eq!(RECEIVED.load(Ordering::SeqCst), 2);
    }
}
