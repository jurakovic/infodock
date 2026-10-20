//! The bar thread and the native bar window (SPEC §4.1, §4.6): window class, window procedure,
//! message loop, painting and material. The bar logic sees the window through [`BarWindow`] and
//! receives its events through [`BarHandler`].
//!
//! Transparency: the window keeps its redirection surface and enables per-pixel alpha with
//! `DwmEnableBlurBehindWindow` and an empty blur region (the same setup the M0 webview bar had
//! through tao), so a DWM material set by `window-vibrancy` shows through wherever the bar is
//! drawn translucent.
//!
//! Re-entrancy: Win32 calls made by the handler can dispatch messages to this window again
//! (modal loops such as menus run their own message pumps). Events that arrive while the handler
//! is busy are queued and delivered when it returns; a paint during that time is postponed.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::size_of;
use std::num::NonZeroIsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateRectRgn, DeleteObject, EndPaint, InvalidateRect, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW, IDC_ARROW, KillTimer,
    LoadCursorW, MA_NOACTIVATE, MSG, MSGFLT_ALLOW, PBT_APMPOWERSTATUSCHANGE,
    PBT_APMRESUMEAUTOMATIC, PostMessageW, PostQuitMessage, RegisterClassExW, SWP_HIDEWINDOW,
    SetTimer, SetWindowLongPtrW, TranslateMessage, WINDOWPOS, WM_APP, WM_CAPTURECHANGED, WM_CLOSE,
    WM_DEVICECHANGE, WM_DPICHANGED, WM_DWMCOLORIZATIONCOLORCHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WM_POWERBROADCAST, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETTINGCHANGE,
    WM_SIZE, WM_THEMECHANGED, WM_TIMECHANGE, WM_TIMER, WM_WINDOWPOSCHANGED, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::appbar::{Shared as AppBarState, WinAppBar};
use super::band::BandWindow;
use super::dialog;
use super::droptarget;
use super::hint::HintWindow;
use super::instance::activate_message;
use super::menu;
use super::popup::PopupWindow;
use super::render::Renderer;
use super::tray::{Tray, WM_TRAY, taskbar_created};
use crate::config::model::Material;
use crate::i18n::t;
use crate::platform::{
    AppBarEvent, Band, BarHandler, BarThread, BarWindow, Canvas, DragEvent, DropEffect,
    HandlerFactory, MenuEntry, Modifiers, PlatformError, PointF, PointerButton, Rect, SizeF,
    TimerId, TrayIcon, UiEvent, UiWaker,
};

pub(super) const CLASS_NAME: PCWSTR = w!("Infodock.Bar");
/// Private: [`UiWaker::wake`] was called.
const WM_WAKE: u32 = WM_APP + 1;
/// Window timer ids: the bar logic's timers ([`BarWindow::set_timer`]), the hint's fade and
/// drawing again after a failed frame.
const WAKE_TIMER: usize = 1;
const HINT_TIMER: usize = 2;
const DRAG_TIMER: usize = 3;
const FADE_TIMER: usize = 4;
const FRAME_TIMER: usize = 5;
const RETRY_TIMER: usize = 6;
const POPUP_TIMER: usize = 7;
const POPUP_FADE_TIMER: usize = 8;
/// A failed frame (e.g. a lost render target) is drawn again after this long: soon, but not at
/// once, so a failure that persists does not keep the bar thread busy.
const RETRY_MS: u32 = 250;

fn timer_id(id: TimerId) -> usize {
    match id {
        TimerId::Wake => WAKE_TIMER,
        TimerId::Hint => HINT_TIMER,
        TimerId::Drag => DRAG_TIMER,
        TimerId::Frame => FRAME_TIMER,
        TimerId::Popup => POPUP_TIMER,
    }
}

/// Per-window state, reached from the window procedure through `GWLP_USERDATA`. The bar window
/// lives as long as the process, so its state is never freed.
struct WindowState {
    hwnd: Cell<isize>,
    handler: RefCell<Option<Box<dyn BarHandler>>>,
    appbar: OnceCell<Arc<AppBarState>>,
    tray: OnceCell<Arc<Tray>>,
    /// AppBar events raised while handling a message; delivered to the handler afterwards.
    appbar_events: Arc<Mutex<Vec<AppBarEvent>>>,
    /// Events that arrived while the handler was busy.
    deferred: RefCell<VecDeque<UiEvent>>,
    /// A paint arrived while the handler was busy.
    repaint: Cell<bool>,
    renderer: RefCell<Option<Renderer>>,
    /// A wake message is queued and not yet handled.
    wake_queued: Arc<AtomicBool>,
    /// Rendering failed once; later failures are logged at debug level.
    render_failed: Cell<bool>,
    /// `TrackMouseEvent` asked for `WM_MOUSELEAVE`.
    tracking_leave: Cell<bool>,
    /// Created on first use; `Err` once creating it failed.
    hint: OnceCell<std::result::Result<HintWindow, ()>>,
    /// The same for the popup window.
    popup: OnceCell<std::result::Result<PopupWindow, ()>>,
    /// The skin's shadow band; created when a skin first has one.
    band: OnceCell<std::result::Result<BandWindow, ()>>,
}

impl WindowState {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd.get() as *mut c_void)
    }

    fn handle(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let hwnd = self.hwnd();
        if msg == WM_TRAY {
            if let Some(event) = self.tray.get().and_then(|tray| tray.event(lparam)) {
                self.dispatch(event);
            }
            return LRESULT(0);
        }
        if msg == taskbar_created()
            && let Some(tray) = self.tray.get()
        {
            tray.restore();
        }
        let handled = self
            .appbar
            .get()
            .and_then(|appbar| appbar.handle(msg, wparam, lparam));
        let events: Vec<AppBarEvent> = std::mem::take(
            &mut *self
                .appbar_events
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for event in events {
            self.dispatch(match event {
                AppBarEvent::Placed(geometry) => UiEvent::Placed(geometry),
                AppBarEvent::Fullscreen(on) => UiEvent::Fullscreen(on),
            });
        }
        if let Some(result) = handled {
            return result;
        }
        if msg == activate_message() {
            self.dispatch(UiEvent::Activate);
            return LRESULT(0);
        }
        match msg {
            WM_PAINT => {
                self.paint();
                return LRESULT(0);
            }
            WM_ERASEBKGND => return LRESULT(1),
            // Clicks must not take focus from the user's window (SPEC §5.3).
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_WAKE => {
                self.wake_queued.store(false, Ordering::SeqCst);
                self.dispatch(UiEvent::Wake);
                return LRESULT(0);
            }
            WM_CLOSE => {
                self.dispatch(UiEvent::CloseRequested);
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                if !self.tracking_leave.replace(true) {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: `track` is valid for the call.
                    if unsafe { TrackMouseEvent(&mut track) }.is_err() {
                        self.tracking_leave.set(false);
                    }
                }
                self.dispatch(UiEvent::PointerMove(self.point(lparam)));
                return LRESULT(0);
            }
            WM_MOUSELEAVE => {
                self.tracking_leave.set(false);
                self.dispatch(UiEvent::PointerLeave);
                return LRESULT(0);
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP
            | WM_MBUTTONUP => {
                let button = match msg {
                    WM_LBUTTONDOWN | WM_LBUTTONUP => PointerButton::Left,
                    WM_RBUTTONDOWN | WM_RBUTTONUP => PointerButton::Right,
                    _ => PointerButton::Middle,
                };
                let (at, modifiers) = (self.point(lparam), modifiers());
                let down = matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN);
                self.dispatch(if down {
                    UiEvent::PointerDown {
                        button,
                        at,
                        modifiers,
                    }
                } else {
                    UiEvent::PointerUp {
                        button,
                        at,
                        modifiers,
                    }
                });
                // Handled here, so no WM_CONTEXTMENU follows the right button.
                return LRESULT(0);
            }
            WM_TIMER
                if matches!(
                    wparam.0,
                    WAKE_TIMER | HINT_TIMER | DRAG_TIMER | FRAME_TIMER | POPUP_TIMER
                ) =>
            {
                // One-shot: the handler sets the next one.
                // SAFETY: plain call on our own window.
                let _ = unsafe { KillTimer(Some(hwnd), wparam.0) };
                let id = match wparam.0 {
                    WAKE_TIMER => TimerId::Wake,
                    HINT_TIMER => TimerId::Hint,
                    DRAG_TIMER => TimerId::Drag,
                    POPUP_TIMER => TimerId::Popup,
                    _ => TimerId::Frame,
                };
                self.dispatch(UiEvent::Timer(id));
                return LRESULT(0);
            }
            WM_CAPTURECHANGED if lparam.0 != hwnd.0 as isize => {
                self.dispatch(UiEvent::CaptureLost);
            }
            WM_TIMECHANGE => self.dispatch(UiEvent::TimeChanged),
            // Volumes are broadcast to every top-level window.
            WM_DEVICECHANGE if is_volume_change(wparam, lparam) => {
                self.dispatch(UiEvent::DevicesChanged);
            }
            // Sent on every resume, also when no user input follows (unlike the "suspend" one).
            WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => {
                self.dispatch(UiEvent::Resumed);
            }
            // AC plugged in or out, or the charge moved (SPEC §9.14).
            WM_POWERBROADCAST if wparam.0 == PBT_APMPOWERSTATUSCHANGE as usize => {
                self.dispatch(UiEvent::PowerChanged);
            }
            WM_TIMER if wparam.0 == RETRY_TIMER => {
                // SAFETY: plain call on our own window.
                let _ = unsafe { KillTimer(Some(hwnd), RETRY_TIMER) };
                self.invalidate();
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == POPUP_FADE_TIMER => {
                let going_on = matches!(self.popup.get(), Some(Ok(popup)) if popup.fade_step());
                if !going_on {
                    // SAFETY: plain call on our own window.
                    let _ = unsafe { KillTimer(Some(hwnd), POPUP_FADE_TIMER) };
                }
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == FADE_TIMER => {
                let going_on = matches!(self.hint.get(), Some(Ok(hint)) if hint.fade_step());
                if !going_on {
                    // SAFETY: plain call on our own window.
                    let _ = unsafe { KillTimer(Some(hwnd), FADE_TIMER) };
                }
                return LRESULT(0);
            }
            WM_SIZE | WM_DPICHANGED => self.invalidate(),
            // Moved, sized, shown or hidden, or restacked: the shadow band follows.
            WM_WINDOWPOSCHANGED => {
                // SAFETY: for this message lParam points to the change's WINDOWPOS.
                let hiding = lparam.0 != 0
                    && unsafe { &*(lparam.0 as *const WINDOWPOS) }
                        .flags
                        .contains(SWP_HIDEWINDOW);
                self.follow_band(hiding);
            }
            WM_SETTINGCHANGE if is_appearance_change(wparam, lparam) => {
                self.dispatch(UiEvent::AppearanceChanged);
            }
            WM_SETTINGCHANGE if setting_area(lparam).as_deref() == Some("intl") => {
                self.dispatch(UiEvent::LocaleChanged);
            }
            WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED => {
                self.dispatch(UiEvent::AppearanceChanged);
            }
            WM_NCDESTROY => {
                if let Some(tray) = self.tray.get() {
                    tray.hide();
                }
                // SAFETY: detaches this state from the dying window; it is intentionally leaked.
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    PostQuitMessage(0);
                }
            }
            _ => {}
        }
        // SAFETY: default processing for our own window.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Delivers `event` now, or after the handler call in progress returns.
    fn dispatch(&self, event: UiEvent) {
        let Ok(mut slot) = self.handler.try_borrow_mut() else {
            self.deferred.borrow_mut().push_back(event);
            return;
        };
        let Some(handler) = slot.as_mut() else {
            return;
        };
        handler.on_event(self, event);
        self.drain(handler.as_mut());
        drop(slot);
        if self.repaint.take() {
            self.invalidate();
        }
    }

    /// Delivers the events that arrived while the handler was busy.
    fn drain(&self, handler: &mut dyn BarHandler) {
        loop {
            // Separate statement: the queue must not stay borrowed while the handler runs.
            let next = self.deferred.borrow_mut().pop_front();
            let Some(event) = next else {
                break;
            };
            handler.on_event(self, event);
        }
    }

    /// A drag from another app (SPEC §6.3): the handler answers with the effect at once; while
    /// it is busy (a menu is open), nothing can be dropped.
    fn drag(&self, event: DragEvent) -> DropEffect {
        let Ok(mut slot) = self.handler.try_borrow_mut() else {
            return DropEffect::None;
        };
        let Some(handler) = slot.as_mut() else {
            return DropEffect::None;
        };
        let effect = handler.on_drag(self, event);
        self.drain(handler.as_mut());
        drop(slot);
        if self.repaint.take() {
            self.invalidate();
        }
        effect
    }

    fn paint(&self) {
        let hwnd = self.hwnd();
        let mut ps = PAINTSTRUCT::default();
        // SAFETY: BeginPaint/EndPaint pair on our own window; the DC is used only in between.
        let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
        match self.handler.try_borrow_mut() {
            Ok(mut slot) => {
                if let Some(handler) = slot.as_mut() {
                    self.render(hdc, handler.as_mut());
                }
            }
            Err(_) => self.repaint.set(true),
        }
        // SAFETY: ends the paint started above.
        unsafe {
            let _ = EndPaint(hwnd, &ps);
        }
    }

    fn render(&self, hdc: windows::Win32::Graphics::Gdi::HDC, handler: &mut dyn BarHandler) {
        let hwnd = self.hwnd();
        let mut client = RECT::default();
        // SAFETY: reads our own window's client rect.
        if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
            return;
        }
        let (width, height) = (client.right - client.left, client.bottom - client.top);
        if width <= 0 || height <= 0 {
            return;
        }
        // SAFETY: plain query on our own window.
        let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
        let mut renderer = self.renderer.borrow_mut();
        if renderer.is_none() {
            match Renderer::new() {
                Ok(r) => *renderer = Some(r),
                Err(e) => {
                    self.report_render_error(&e);
                    return self.retry_render();
                }
            }
        }
        let Some(renderer) = renderer.as_mut() else {
            return;
        };
        let size = SizeF {
            width: width as f32 / scale,
            height: height as f32 / scale,
        };
        match renderer.draw(width, height, scale, |canvas| handler.paint(canvas, size)) {
            Ok(frame) => {
                // SAFETY: copies the finished frame, alpha included, into the window's surface.
                let copied =
                    unsafe { BitBlt(hdc, 0, 0, width, height, Some(frame), 0, 0, SRCCOPY) };
                if let Err(e) = copied {
                    self.report_render_error(&e);
                    self.retry_render();
                }
                self.show_band(handler.band());
            }
            Err(e) => {
                self.report_render_error(&e);
                // A lost render target is re-created on the next frame.
                self.retry_render();
            }
        }
    }

    /// Shows the skin's shadow band beside the bar, or none (SPEC §5.1).
    fn show_band(&self, band: Option<Band>) {
        if band.is_none() && self.band.get().is_none() {
            return;
        }
        let window = self.band.get_or_init(|| {
            BandWindow::new(self.hwnd()).map_err(|e| {
                tracing::warn!("the shadow band window could not be created: {e}");
            })
        });
        if let Ok(window) = window {
            window.set(band, self.away());
        }
    }

    /// Puts the shadow band beside the bar where it is now; `hiding`: the bar is being hidden.
    fn follow_band(&self, hiding: bool) {
        if let Some(Ok(band)) = self.band.get() {
            band.follow(hiding || self.away());
        }
    }

    /// The bar is shown off its place, rising or sliding out (SPEC §5.5); its band waits.
    fn away(&self) -> bool {
        self.appbar.get().is_some_and(|appbar| appbar.offset() != 0)
    }

    /// Draws the bar again shortly after a failed frame; nothing else may ask for one.
    fn retry_render(&self) {
        // SAFETY: plain call on our own window; a timer with the same id is replaced.
        unsafe { SetTimer(Some(self.hwnd()), RETRY_TIMER, RETRY_MS, None) };
    }

    /// The pointer position in `lparam` (client px) in logical px.
    fn point(&self, lparam: LPARAM) -> PointF {
        // SAFETY: plain query on our own window.
        let scale = unsafe { GetDpiForWindow(self.hwnd()) }.max(96) as f32 / 96.0;
        // Signed 16-bit coordinates: negative while captured left of or above the window.
        let x = (lparam.0 & 0xFFFF) as u16 as i16;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16;
        PointF::new(f32::from(x) / scale, f32::from(y) / scale)
    }

    fn report_render_error(&self, e: &windows::core::Error) {
        if self.render_failed.replace(true) {
            tracing::debug!("bar rendering failed: {e}");
        } else {
            tracing::warn!("bar rendering failed: {e}");
        }
    }
}

impl BarWindow for WindowState {
    fn invalidate(&self) {
        // SAFETY: plain call on our own window.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd()), None, false);
        }
    }

    fn set_material(&self, material: Material) {
        let Some(handle) = NonZeroIsize::new(self.hwnd.get()).map(BarHandle) else {
            return;
        };
        let _ = window_vibrancy::clear_acrylic(&handle);
        let _ = window_vibrancy::clear_mica(&handle);
        // The bar draws its own tint (SPEC §12.2), so the backdrop itself stays untinted.
        let result = match material {
            Material::None => Ok(()),
            Material::Acrylic => window_vibrancy::apply_acrylic(&handle, Some((0, 0, 0, 0))),
            Material::Mica => window_vibrancy::apply_mica(&handle, None),
        };
        match result {
            Ok(()) => tracing::debug!("window material: {material:?}"),
            Err(e) => tracing::warn!("window material {material:?} not available: {e}"),
        }
    }

    fn set_tray_icon(&self, tooltip: Option<&str>) {
        if let Some(tray) = self.tray.get() {
            match tooltip {
                Some(tooltip) => tray.show(tooltip),
                None => tray.hide(),
            }
        }
    }

    fn show_menu(&self, entries: &[MenuEntry]) -> Option<u32> {
        menu::show(self.hwnd(), entries)
    }

    fn capture_pointer(&self, capture: bool) {
        // SAFETY: plain calls on our own window.
        unsafe {
            if capture {
                SetCapture(self.hwnd());
            } else {
                let _ = ReleaseCapture();
            }
        }
    }

    fn move_by(&self, dx: i32, dy: i32) -> Option<Rect> {
        self.appbar.get()?.move_by(dx, dy)
    }

    fn slide(&self, dy: i32) {
        if let Some(appbar) = self.appbar.get() {
            appbar.slide(dy);
        }
    }

    fn escape_down(&self) -> bool {
        // SAFETY: plain key-state query; the high bit is set while the key is down.
        unsafe { GetAsyncKeyState(i32::from(VK_ESCAPE.0)) < 0 }
    }

    fn confirm(&self, message: &str, action: &str) -> bool {
        dialog::confirm(self.hwnd(), &t("app.name"), message, action)
    }

    fn set_timer(&self, id: TimerId, delay: Option<Duration>) {
        let (hwnd, id) = (self.hwnd(), timer_id(id));
        match delay {
            // SetTimer replaces a timer with the same id; USER clamps tiny delays to ~10 ms.
            // SAFETY: plain calls on our own window.
            Some(delay) => unsafe {
                let ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX).max(1);
                if SetTimer(Some(hwnd), id, ms, None) == 0 {
                    tracing::warn!("timer {id} could not be set");
                }
            },
            // SAFETY: plain call on our own window; a missing timer is no error worth noting.
            None => unsafe {
                let _ = KillTimer(Some(hwnd), id);
            },
        }
    }

    fn show_hint(&self, rect: Rect, scale: f32, paint: &mut dyn FnMut(&mut dyn Canvas)) {
        let hint = self.hint.get_or_init(|| {
            HintWindow::new(self.hwnd()).map_err(|e| {
                tracing::warn!("the hint window could not be created: {e}");
            })
        });
        let Ok(hint) = hint else {
            return;
        };
        match hint.show(rect, scale, paint) {
            Ok(true) => {
                // About 60 steps a second while it fades in.
                // SAFETY: plain call on our own window.
                unsafe { SetTimer(Some(self.hwnd()), FADE_TIMER, 16, None) };
            }
            Ok(false) => {}
            Err(e) => tracing::debug!("the hint could not be shown: {e}"),
        }
    }

    fn hide_hint(&self) {
        if let Some(Ok(hint)) = self.hint.get() {
            hint.hide();
        }
    }

    fn show_popup(&self, rect: Rect, scale: f32, paint: &mut dyn FnMut(&mut dyn Canvas)) {
        let popup = self.popup.get_or_init(|| {
            // SAFETY: the state is leaked when the bar window is created and never freed.
            let state: &'static WindowState = unsafe { &*std::ptr::from_ref(self) };
            let sink = Box::new(move |event| state.dispatch(event));
            PopupWindow::new(self.hwnd(), sink).map_err(|e| {
                tracing::warn!("the popup window could not be created: {e}");
            })
        });
        let Ok(popup) = popup else {
            return;
        };
        match popup.show(rect, scale, paint) {
            Ok(true) => {
                // About 60 steps a second while it fades in.
                // SAFETY: plain call on our own window.
                unsafe { SetTimer(Some(self.hwnd()), POPUP_FADE_TIMER, 16, None) };
            }
            Ok(false) => {}
            Err(e) => tracing::debug!("the popup could not be shown: {e}"),
        }
    }

    fn hide_popup(&self) {
        if let Some(Ok(popup)) = self.popup.get() {
            popup.hide();
        }
    }

    fn set_popup_cursor(&self, cursor: crate::platform::PopupCursor) {
        if let Some(Ok(popup)) = self.popup.get() {
            popup.set_cursor(cursor);
        }
    }

    fn capture_popup_pointer(&self, capture: bool) {
        if let Some(Ok(popup)) = self.popup.get() {
            popup.capture(capture);
        }
    }
}

/// The bar window as a raw window handle, for `window-vibrancy`.
struct BarHandle(NonZeroIsize);

impl HasWindowHandle for BarHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(self.0));
        // SAFETY: the bar window lives as long as the process.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// The modifier keys held now.
fn modifiers() -> Modifiers {
    // SAFETY: plain key-state queries; the high bit is set while the key is down.
    let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
    Modifiers {
        alt: down(VK_MENU),
        ctrl: down(VK_CONTROL),
        shift: down(VK_SHIFT),
    }
}

/// Whether a `WM_DEVICECHANGE` says a volume arrived or went (`DBT_DEVICEARRIVAL`,
/// `DBT_DEVICEREMOVECOMPLETE` with a `DEV_BROADCAST_VOLUME`; a disc counts as arriving).
fn is_volume_change(wparam: WPARAM, lparam: LPARAM) -> bool {
    const DBT_DEVICEARRIVAL: usize = 0x8000;
    const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;
    const DBT_DEVTYP_VOLUME: u32 = 2;
    /// `DEV_BROADCAST_HDR`.
    #[repr(C)]
    struct Header {
        size: u32,
        device_type: u32,
        reserved: u32,
    }
    if !matches!(wparam.0, DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE) || lparam.0 == 0 {
        return false;
    }
    // SAFETY: with these events, lparam points to a DEV_BROADCAST_HDR.
    let header = unsafe { &*(lparam.0 as *const Header) };
    header.device_type == DBT_DEVTYP_VOLUME
}

/// `WM_SETTINGCHANGE` for light/dark mode, accent colour or transparency effects.
/// A `WM_SETTINGCHANGE` that can change the bar's look: the colour set (dark mode, accent,
/// transparency) or *Animation effects*, which `animations: auto` follows (SPEC §5.5).
fn is_appearance_change(wparam: WPARAM, lparam: LPARAM) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SPI_SETCLIENTAREAANIMATION;
    if wparam.0 == SPI_SETCLIENTAREAANIMATION.0 as usize {
        return true;
    }
    setting_area(lparam).as_deref() == Some("ImmersiveColorSet")
}

/// What a `WM_SETTINGCHANGE` is about, e.g. `ImmersiveColorSet` or `intl` (the regional
/// format); `None` when it does not say.
fn setting_area(lparam: LPARAM) -> Option<String> {
    if lparam.0 == 0 {
        return None;
    }
    // SAFETY: for WM_SETTINGCHANGE a non-null lParam points to a NUL-terminated string.
    unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.ok()
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE && lparam.0 != 0 {
        // SAFETY: for WM_NCCREATE lParam points to the CREATESTRUCTW whose lpCreateParams is
        // the `WindowState` passed to CreateWindowExW.
        unsafe {
            let state = (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams as *const WindowState;
            (*state).hwnd.set(hwnd.0 as isize);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
    }
    // SAFETY: GWLP_USERDATA is either 0 or the leaked `WindowState` set above.
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowState;
    if state.is_null() {
        // SAFETY: default processing before the state is attached or after it was detached.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the state is never freed while the window exists.
    let state = unsafe { &*state };
    // A panic must not cross the FFI boundary (it would abort); the panic hook has logged it.
    match catch_unwind(AssertUnwindSafe(|| state.handle(msg, wparam, lparam))) {
        Ok(result) => result,
        // SAFETY: default processing for the message whose handling panicked.
        Err(_) => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn os_error(what: &str, e: windows::core::Error) -> PlatformError {
    PlatformError::Os(format!("{what}: {e}"))
}

/// Creates the bar window (hidden) on the calling thread.
fn create_window() -> Result<&'static WindowState, PlatformError> {
    // SAFETY: plain module and class registration calls.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| os_error("GetModuleHandleW", e))?
        .into();
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        // SAFETY: loads a system cursor.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised; a second registration fails harmlessly.
    unsafe { RegisterClassExW(&class) };

    let state: &'static WindowState = Box::leak(Box::new(WindowState {
        hwnd: Cell::new(0),
        handler: RefCell::new(None),
        appbar: OnceCell::new(),
        tray: OnceCell::new(),
        appbar_events: Arc::new(Mutex::new(Vec::new())),
        deferred: RefCell::new(VecDeque::new()),
        repaint: Cell::new(false),
        renderer: RefCell::new(None),
        wake_queued: Arc::new(AtomicBool::new(false)),
        render_failed: Cell::new(false),
        tracking_leave: Cell::new(false),
        hint: OnceCell::new(),
        popup: OnceCell::new(),
        band: OnceCell::new(),
    }));
    let title = HSTRING::from(t("app.name"));
    // SAFETY: creates our own window; `state` outlives it.
    let hwnd = unsafe {
        CreateWindowExW(
            // No taskbar button or Alt+Tab entry; clicks do not activate it (SPEC §4.1).
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS_NAME,
            &title,
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            Some(std::ptr::from_ref(state).cast()),
        )
    }
    .map_err(|e| os_error("CreateWindowExW", e))?;

    // Per-pixel alpha in the redirection surface: an empty blur region blurs nothing.
    // SAFETY: plain calls on our own window; the region is deleted after use.
    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);
        let blur = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: true.into(),
            hRgnBlur: region,
            fTransitionOnMaximized: false.into(),
        };
        if let Err(e) = DwmEnableBlurBehindWindow(hwnd, &blur) {
            tracing::warn!("per-pixel alpha unavailable: {e}");
        }
        let _ = DeleteObject(region.into());
    }

    // A second launch may run with different integrity; let its activation through (UIPI).
    // SAFETY: plain call on our own window.
    unsafe {
        let _ = ChangeWindowMessageFilterEx(hwnd, activate_message(), MSGFLT_ALLOW, None);
    }

    let events = Arc::clone(&state.appbar_events);
    let sink = Box::new(move |event| {
        events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    });
    let _ = state.appbar.set(AppBarState::new(hwnd, sink));
    let _ = state.tray.set(Arc::new(Tray::new(hwnd)));
    Ok(state)
}

fn run(make_handler: HandlerFactory, ready: mpsc::Sender<Result<BarThread, PlatformError>>) {
    // A single-threaded apartment with OLE, which drag & drop needs (SPEC §4.1). Never
    // uninitialised: the thread runs until the process ends.
    // SAFETY: first call on this thread.
    if let Err(e) = unsafe { OleInitialize(None) } {
        tracing::warn!("OLE is not available on the bar thread: {e}");
    }
    let state = match create_window() {
        Ok(state) => state,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let (Some(appbar), Some(tray)) = (state.appbar.get().cloned(), state.tray.get().cloned())
    else {
        let _ = ready.send(Err(PlatformError::Os("AppBar or tray missing".into())));
        return;
    };
    let hwnd = state.hwnd.get();
    let queued = Arc::clone(&state.wake_queued);
    let waker = UiWaker::new(move || {
        if !queued.swap(true, Ordering::SeqCst) {
            // SAFETY: posting to the bar window, which lives as long as the process.
            let posted = unsafe {
                PostMessageW(
                    Some(HWND(hwnd as *mut c_void)),
                    WM_WAKE,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
            if posted.is_err() {
                queued.store(false, Ordering::SeqCst);
            }
        }
    });
    *state.handler.borrow_mut() = Some(make_handler(waker.clone()));
    state.dispatch(UiEvent::Started);
    droptarget::register(state.hwnd(), Box::new(|event| state.drag(event)));
    let _ = ready.send(Ok(BarThread {
        appbar: Arc::new(WinAppBar { shared: appbar }),
        tray: tray as Arc<dyn TrayIcon>,
        waker,
    }));

    let mut msg = MSG::default();
    // SAFETY: the standard message loop for this thread's windows.
    unsafe {
        // 0 = WM_QUIT, -1 = error.
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    tracing::info!("bar thread finished");
}

pub fn start_bar_thread(make_handler: HandlerFactory) -> Result<BarThread, PlatformError> {
    let (ready, started) = mpsc::channel();
    std::thread::Builder::new()
        .name("bar".into())
        .spawn(move || run(make_handler, ready))
        .map_err(|e| PlatformError::Os(format!("could not start the bar thread: {e}")))?;
    started
        .recv()
        .map_err(|_| PlatformError::Os("the bar thread ended during start-up".into()))?
}
