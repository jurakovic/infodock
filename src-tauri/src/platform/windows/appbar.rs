//! The Windows AppBar: work-area reservation, z-order and full-screen handling (SPEC §5.2–5.3).
//!
//! The AppBar is registered for the bar window itself, and the bar window is subclassed
//! (`SetWindowSubclass`) to receive the AppBar callback message and the broadcasts it needs
//! (`TaskbarCreated`, `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`). A message-only window would not
//! receive broadcasts, and registering a different window than the visible bar would decouple the
//! shell's z-order handling from it.
//!
//! Threading: the subclass procedure runs on the UI thread. Other threads only store a request
//! and post a message. State used by the window procedure is behind short-lived locks that are
//! never held across a Win32 call, because `SetWindowPos` re-enters the procedure synchronously
//! (`WM_WINDOWPOSCHANGING`).

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_TOP, ABM_ACTIVATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
    ABM_WINDOWPOSCHANGED, ABN_FULLSCREENAPP, ABN_POSCHANGED, ABN_STATECHANGE, APPBARDATA,
    DefSubclassProc, RemoveWindowSubclass, SHAppBarMessage, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, HWND_BOTTOM,
    HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, MSGFLT_ALLOW, PostMessageW, RegisterWindowMessageW,
    SET_WINDOW_POS_FLAGS, SPI_SETWORKAREA, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    WA_INACTIVE, WINDOWPOS, WM_ACTIVATE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_NCDESTROY,
    WM_SETTINGCHANGE, WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING, WS_EX_TOOLWINDOW,
};
use windows::core::w;

use super::monitors::{self, to_rect, to_win_rect};
use crate::config::model::{BarMode, Edge, ZOrder};
use crate::platform::geometry::{Rect, dock_strip, keep_height, select_monitor, to_physical};
use crate::platform::{AppBar, AppBarEvent, AppBarSink, BarGeometry, DockRequest, PlatformError};

const SUBCLASS_ID: usize = 0x1D0C;
/// `wParam` of the private "reposition" message when it asks to raise the bar instead.
const RAISE: usize = 1;

struct Messages {
    /// AppBar notifications (ABN_*).
    callback: u32,
    /// Private: recompute the placement (or raise, see [`RAISE`]) on the UI thread.
    reposition: u32,
    /// Broadcast by Explorer when the taskbar is (re)created.
    taskbar_created: u32,
}

static MESSAGES: OnceLock<Messages> = OnceLock::new();

fn messages() -> &'static Messages {
    MESSAGES.get_or_init(|| {
        // SAFETY: plain calls with static strings.
        unsafe {
            Messages {
                callback: RegisterWindowMessageW(w!("Infodock.AppBar.Callback")),
                reposition: RegisterWindowMessageW(w!("Infodock.AppBar.Reposition")),
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            }
        }
    })
}

struct Shared {
    /// The bar window. Stored as an integer because `HWND` is not `Send`.
    hwnd: isize,
    /// Registered with the shell (`ABM_NEW` done, no `ABM_REMOVE` since). Atomic so the panic
    /// hook can release the reservation without taking a lock.
    registered: AtomicBool,
    /// The latest request; written by any thread.
    request: Mutex<Option<DockRequest>>,
    /// UI-thread state. Never hold this lock across a Win32 call.
    ui: Mutex<UiState>,
    sink: AppBarSink,
}

#[derive(Default)]
struct UiState {
    /// The last placement: request and window rect (physical px).
    applied: Option<(DockRequest, Rect)>,
    /// A full-screen app is open on the bar's monitor.
    fullscreen: bool,
    /// Brought to the front by the hotkey; cleared when the bar loses focus.
    raised: bool,
    /// Our own `SetWindowPos` is in progress, so `WM_WINDOWPOSCHANGING` must not interfere.
    positioning: bool,
    /// A placement is running; a nested request is re-posted instead of run re-entrantly.
    busy: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }

    fn ui<R>(&self, f: impl FnOnce(&mut UiState) -> R) -> R {
        f(&mut lock(&self.ui))
    }

    fn post(&self, wparam: usize) {
        // SAFETY: posting to our own window; fails only if the window is gone.
        let _ = unsafe {
            PostMessageW(
                Some(self.hwnd()),
                messages().reposition,
                WPARAM(wparam),
                LPARAM(0),
            )
        };
    }

    fn appbar_data(&self) -> APPBARDATA {
        APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd(),
            uCallbackMessage: messages().callback,
            ..Default::default()
        }
    }

    fn register(&self) {
        if !self.registered.swap(true, Ordering::SeqCst) {
            let mut abd = self.appbar_data();
            // SAFETY: `abd` is a fully initialised APPBARDATA for our window.
            if unsafe { SHAppBarMessage(ABM_NEW, &mut abd) } == 0 {
                self.registered.store(false, Ordering::SeqCst);
                tracing::warn!("ABM_NEW failed; the bar will not reserve screen space");
            } else {
                tracing::debug!("appbar registered");
            }
        }
    }

    fn unregister(&self) {
        if self.registered.swap(false, Ordering::SeqCst) {
            let mut abd = self.appbar_data();
            // SAFETY: as above.
            unsafe { SHAppBarMessage(ABM_REMOVE, &mut abd) };
            tracing::debug!("appbar removed");
        }
    }

    /// Computes and applies the placement for the current request (SPEC §5.2 steps 1–2).
    fn reposition(&self) {
        if self.ui(|u| std::mem::replace(&mut u.busy, true)) {
            self.post(0);
            return;
        }
        let request = lock(&self.request).clone();
        if let Some(request) = request {
            self.place(&request);
        }
        self.ui(|u| u.busy = false);
    }

    fn place(&self, req: &DockRequest) {
        let hwnd = self.hwnd();
        if !req.visible {
            self.unregister();
            // SAFETY: hiding our own window.
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            self.ui(|u| u.applied = None);
            return;
        }
        // Floating mode arrives in M1; until then it behaves like a docked bar without reservation.
        let reserve = req.reserve_space && req.mode == BarMode::Docked;
        if !reserve {
            // Remove first so the work area below no longer contains our old reservation.
            self.unregister();
        }
        let monitors = monitors::list();
        let Some(monitor) = select_monitor(&monitors, &req.monitor) else {
            tracing::warn!("no monitors found; bar not placed");
            return;
        };
        let height = to_physical(req.height, monitor.scale);
        let rect = if reserve {
            self.register();
            self.negotiate(monitor.bounds, req.edge, height)
        } else {
            dock_strip(monitor.work_area, req.edge, height)
        };

        let (fullscreen, raised) = self.ui(|u| (u.fullscreen, u.raised));
        let insert_after = insert_after(req, fullscreen, raised);
        self.set_window_pos(insert_after, rect, SWP_NOACTIVATE | SWP_SHOWWINDOW);

        let changed = self.ui(|u| {
            let changed = u
                .applied
                .as_ref()
                .is_none_or(|(r, a)| r != req || *a != rect);
            u.applied = Some((req.clone(), rect));
            changed
        });
        if changed {
            tracing::info!(
                "bar placed at {rect:?} on {} (scale {})",
                monitor.key,
                monitor.scale
            );
            (self.sink)(AppBarEvent::Placed(BarGeometry {
                rect,
                edge: req.edge,
                monitor: monitor.clone(),
                scale: monitor.scale,
            }));
        }
    }

    /// `ABM_QUERYPOS` → adjust → `ABM_SETPOS`; returns the rect the shell granted.
    fn negotiate(&self, bounds: Rect, edge: Edge, height: i32) -> Rect {
        let mut abd = self.appbar_data();
        abd.uEdge = match edge {
            Edge::Top => ABE_TOP,
            Edge::Bottom => ABE_BOTTOM,
        };
        abd.rc = to_win_rect(dock_strip(bounds, edge, height));
        // SAFETY: `abd` is initialised for our registered window.
        unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut abd) };
        abd.rc = to_win_rect(keep_height(to_rect(abd.rc), edge, height));
        // SAFETY: as above.
        unsafe { SHAppBarMessage(ABM_SETPOS, &mut abd) };
        keep_height(to_rect(abd.rc), edge, height)
    }

    fn set_window_pos(&self, insert_after: HWND, rect: Rect, flags: SET_WINDOW_POS_FLAGS) {
        self.ui(|u| u.positioning = true);
        // SAFETY: moving our own window.
        let result = unsafe {
            SetWindowPos(
                self.hwnd(),
                Some(insert_after),
                rect.left,
                rect.top,
                rect.width(),
                rect.height(),
                flags,
            )
        };
        self.ui(|u| u.positioning = false);
        if let Err(e) = result {
            tracing::warn!("SetWindowPos failed: {e}");
        }
    }

    /// Re-applies only the z-order (full-screen changes, losing focus while raised).
    fn restack(&self) {
        let Some(req) = lock(&self.request).clone() else {
            return;
        };
        let (fullscreen, raised) = self.ui(|u| (u.fullscreen, u.raised));
        self.set_window_pos(
            insert_after(&req, fullscreen, raised),
            Rect::default(),
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }

    fn raise_now(&self) {
        self.ui(|u| u.raised = true);
        let Some(req) = lock(&self.request).clone() else {
            return;
        };
        let z = if req.z_order == ZOrder::Topmost {
            HWND_TOPMOST
        } else {
            HWND_TOP
        };
        self.set_window_pos(z, Rect::default(), SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        // SAFETY: plain call; may be refused by the foreground lock, which is harmless.
        let _ = unsafe { SetForegroundWindow(self.hwnd()) };
    }

    /// ABN_* notifications (SPEC §5.2 step 3).
    fn on_notification(&self, code: u32, lparam: LPARAM) {
        match code {
            ABN_POSCHANGED | ABN_STATECHANGE => self.post(0),
            ABN_FULLSCREENAPP => {
                let opening = lparam.0 != 0;
                // The notification does not say which monitor; compare with the foreground window.
                // SAFETY: plain queries.
                let on_our_monitor = unsafe {
                    MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST)
                        == MonitorFromWindow(self.hwnd(), MONITOR_DEFAULTTONEAREST)
                };
                let fullscreen = opening && on_our_monitor;
                if self.ui(|u| std::mem::replace(&mut u.fullscreen, fullscreen)) != fullscreen {
                    tracing::debug!(
                        "full-screen app {}",
                        if fullscreen { "opened" } else { "closed" }
                    );
                    self.restack();
                    (self.sink)(AppBarEvent::Fullscreen(fullscreen));
                }
            }
            _ => {}
        }
    }

    /// Keeps other code (Aero Snap, DPI changes, activation) from moving the bar or lifting it
    /// out of its z-order.
    fn on_pos_changing(&self, pos: &mut WINDOWPOS) {
        let Ok(ui) = self.ui.try_lock() else {
            return;
        };
        if ui.positioning {
            return;
        }
        let Some((req, rect)) = ui.applied.clone() else {
            return;
        };
        let (fullscreen, raised) = (ui.fullscreen, ui.raised);
        drop(ui);
        if !pos.flags.contains(SWP_NOMOVE) {
            pos.x = rect.left;
            pos.y = rect.top;
        }
        if !pos.flags.contains(SWP_NOSIZE) {
            pos.cx = rect.width();
            pos.cy = rect.height();
        }
        let keep_low =
            (fullscreen && req.hide_on_fullscreen) || (req.z_order == ZOrder::Bottom && !raised);
        if keep_low && !pos.flags.contains(SWP_NOZORDER) {
            pos.hwndInsertAfter = HWND_BOTTOM;
        }
    }

    fn on_deactivated(&self) {
        if self.ui(|u| std::mem::replace(&mut u.raised, false)) {
            self.restack();
        }
    }

    fn appbar_notify(&self, message: u32) {
        if self.registered.load(Ordering::SeqCst) {
            let mut abd = self.appbar_data();
            // SAFETY: as in `register`.
            unsafe { SHAppBarMessage(message, &mut abd) };
        }
    }
}

/// Where the bar goes in the z-order (SPEC §5.3).
fn insert_after(req: &DockRequest, fullscreen: bool, raised: bool) -> HWND {
    if fullscreen && req.hide_on_fullscreen {
        return HWND_BOTTOM;
    }
    match req.z_order {
        ZOrder::Topmost => HWND_TOPMOST,
        ZOrder::Normal => HWND_NOTOPMOST,
        ZOrder::Bottom if raised => HWND_TOP,
        ZOrder::Bottom => HWND_BOTTOM,
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `data` is the `Arc<Shared>` leaked in `attach`; it stays alive until WM_NCDESTROY.
    let shared = unsafe { &*(data as *const Shared) };
    let m = messages();
    if msg == m.reposition {
        if wparam.0 == RAISE {
            shared.raise_now();
        } else {
            shared.reposition();
        }
        return LRESULT(0);
    }
    if msg == m.callback {
        shared.on_notification(wparam.0 as u32, lparam);
        return LRESULT(0);
    }
    if msg == m.taskbar_created {
        // Explorer restarted and forgot every appbar (SPEC §5.2 step 5).
        tracing::info!("taskbar re-created; registering again");
        shared.unregister();
        shared.post(0);
    }
    match msg {
        WM_ACTIVATE => {
            shared.appbar_notify(ABM_ACTIVATE);
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                shared.on_deactivated();
            }
        }
        WM_WINDOWPOSCHANGING if lparam.0 != 0 => {
            // SAFETY: for this message lParam points to a WINDOWPOS owned by the caller.
            shared.on_pos_changing(unsafe { &mut *(lparam.0 as *mut WINDOWPOS) });
        }
        WM_WINDOWPOSCHANGED => shared.appbar_notify(ABM_WINDOWPOSCHANGED),
        WM_DISPLAYCHANGE | WM_DPICHANGED => shared.post(0),
        WM_SETTINGCHANGE if wparam.0 == SPI_SETWORKAREA.0 as usize => shared.post(0),
        WM_NCDESTROY => {
            shared.unregister();
            // SAFETY: undoes `attach`: remove the subclass, then release its reference.
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
                drop(Arc::from_raw(data as *const Shared));
                return DefSubclassProc(hwnd, msg, wparam, lparam);
            }
        }
        _ => {}
    }
    // SAFETY: forwards to the next procedure in the chain (Tauri's).
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

pub struct WinAppBar {
    shared: Arc<Shared>,
}

impl AppBar for WinAppBar {
    fn configure(&self, request: DockRequest) {
        *lock(&self.shared.request) = Some(request);
        self.shared.post(0);
    }

    fn refresh(&self) {
        self.shared.post(0);
    }

    fn raise(&self) {
        self.shared.post(RAISE);
    }

    fn release(&self) {
        self.shared.unregister();
    }
}

/// Subclasses the bar window. Must run on the window's UI thread.
pub fn attach(
    window: &tauri::WebviewWindow,
    sink: AppBarSink,
) -> Result<Arc<dyn AppBar>, PlatformError> {
    let hwnd = window.hwnd()?;
    let m = messages();
    let shared = Arc::new(Shared {
        hwnd: hwnd.0 as isize,
        registered: AtomicBool::new(false),
        request: Mutex::new(None),
        ui: Mutex::new(UiState::default()),
        sink,
    });
    // SAFETY: all calls target our own window on its UI thread.
    unsafe {
        // A tool window has no Alt+Tab entry (skip_taskbar already removed the taskbar button).
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_TOOLWINDOW.0 as isize);
        // Let Explorer's TaskbarCreated through even if Infodock runs elevated.
        let _ = ChangeWindowMessageFilterEx(hwnd, m.taskbar_created, MSGFLT_ALLOW, None);
        let raw = Arc::into_raw(Arc::clone(&shared));
        if !SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, raw as usize).as_bool() {
            drop(Arc::from_raw(raw));
            return Err(PlatformError::Os("SetWindowSubclass failed".into()));
        }
    }
    Ok(Arc::new(WinAppBar { shared }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(z_order: ZOrder, hide_on_fullscreen: bool) -> DockRequest {
        DockRequest {
            visible: true,
            edge: Edge::Bottom,
            mode: BarMode::Docked,
            monitor: "primary".into(),
            height: 28,
            reserve_space: true,
            z_order,
            hide_on_fullscreen,
        }
    }

    #[test]
    fn z_order_follows_settings_and_fullscreen() {
        assert_eq!(
            insert_after(&req(ZOrder::Topmost, true), false, false),
            HWND_TOPMOST
        );
        assert_eq!(
            insert_after(&req(ZOrder::Normal, true), false, false),
            HWND_NOTOPMOST
        );
        assert_eq!(
            insert_after(&req(ZOrder::Bottom, true), false, false),
            HWND_BOTTOM
        );
        assert_eq!(
            insert_after(&req(ZOrder::Bottom, true), false, true),
            HWND_TOP
        );
        assert_eq!(
            insert_after(&req(ZOrder::Topmost, true), true, false),
            HWND_BOTTOM
        );
        assert_eq!(
            insert_after(&req(ZOrder::Topmost, false), true, false),
            HWND_TOPMOST
        );
    }
}
