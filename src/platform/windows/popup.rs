//! The popup window (SPEC §4.1, §7.2): layered (`UpdateLayeredWindow`, per-pixel alpha),
//! topmost, a tool window owned by the bar, and focusable, so it reads the keyboard. Created on
//! first use and reused. Its input goes to the bar logic as [`UiEvent::Popup`]; it hides itself
//! and says so when it loses the activation (a click elsewhere, Alt+F4). A popup that appears
//! fades in over 120 ms, stepped by the bar window's timer.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, ScreenToClient};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
    VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW, HTCLIENT,
    HWND_TOPMOST, IDC_ARROW, IDC_SIZENS, IDC_SIZEWE, LoadCursorW, RegisterClassExW, SC_KEYMENU,
    SPI_GETWHEELSCROLLLINES, SW_HIDE, SW_SHOW, SWP_NOMOVE, SWP_NOSIZE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetCursor, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, SystemParametersInfoW, ULW_ALPHA, UpdateLayeredWindow, WA_INACTIVE,
    WM_ACTIVATE, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SETCURSOR, WM_SYSCOMMAND, WM_SYSKEYDOWN, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use super::hint::FADE;
use super::render::Renderer;
use crate::platform::{
    Canvas, Key, Modifiers, PointF, PointerButton, PopupCursor, PopupInput, Rect, UiEvent,
};

const CLASS_NAME: PCWSTR = w!("Infodock.Popup");
/// One wheel notch (`WHEEL_DELTA`).
const NOTCH: f32 = 120.0;
/// `WHEEL_PAGESCROLL`: the user scrolls by pages; taken as this many lines.
const PAGE_LINES: f32 = 10.0;

fn blend(alpha: u8) -> BLENDFUNCTION {
    BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: AC_SRC_ALPHA as u8,
    }
}

/// What the window procedure needs; leaked, like the window, which lives as long as the
/// process.
struct Shared {
    /// Delivers to the bar logic, on the bar thread.
    sink: Box<dyn Fn(UiEvent)>,
    visible: Cell<bool>,
    /// Physical px per logical px of the shown content.
    scale: Cell<f32>,
    /// `TrackMouseEvent` asked for `WM_MOUSELEAVE`.
    tracking_leave: Cell<bool>,
    /// The pointer's shape over the window.
    cursor: Cell<PopupCursor>,
}

impl Shared {
    fn send(&self, input: PopupInput) {
        (self.sink)(UiEvent::Popup(input));
    }

    /// Client px in `lparam` as logical px.
    fn point(&self, lparam: LPARAM) -> PointF {
        let x = (lparam.0 & 0xFFFF) as u16 as i16;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16;
        let scale = self.scale.get().max(0.1);
        PointF::new(f32::from(x) / scale, f32::from(y) / scale)
    }

    /// Hides the window after it lost the activation or was asked to close, and says so.
    fn deactivated(&self, hwnd: HWND) {
        if self.visible.replace(false) {
            // SAFETY: plain call on our own window.
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            (self.sink)(UiEvent::PopupDeactivated);
        }
    }

    fn handle(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
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
                self.send(PopupInput::PointerMove(self.point(lparam)));
            }
            WM_MOUSELEAVE => {
                self.tracking_leave.set(false);
                self.send(PopupInput::PointerLeave);
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP
            | WM_MBUTTONUP => {
                let button = match msg {
                    WM_LBUTTONDOWN | WM_LBUTTONUP => PointerButton::Left,
                    WM_RBUTTONDOWN | WM_RBUTTONUP => PointerButton::Right,
                    _ => PointerButton::Middle,
                };
                let (at, modifiers) = (self.point(lparam), modifiers());
                self.send(
                    if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
                        PopupInput::PointerDown {
                            button,
                            at,
                            modifiers,
                        }
                    } else {
                        PopupInput::PointerUp {
                            button,
                            at,
                            modifiers,
                        }
                    },
                );
            }
            WM_MOUSEWHEEL => {
                let delta = f32::from((wparam.0 >> 16) as u16 as i16);
                // The position comes in screen px.
                let mut at = POINT {
                    x: i32::from((lparam.0 & 0xFFFF) as u16 as i16),
                    y: i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16),
                };
                // SAFETY: converts a point for our own window.
                let _ = unsafe { ScreenToClient(hwnd, &mut at) };
                let scale = self.scale.get().max(0.1);
                self.send(PopupInput::Wheel {
                    lines: delta / NOTCH * wheel_lines(),
                    at: PointF::new(at.x as f32 / scale, at.y as f32 / scale),
                });
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let key = key(VIRTUAL_KEY(wparam.0 as u16))?;
                self.send(PopupInput::Key {
                    key,
                    modifiers: modifiers(),
                });
            }
            WM_CHAR => {
                let c = char::from_u32(wparam.0 as u32).filter(|c| !c.is_control())?;
                self.send(PopupInput::Char(c));
            }
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
                self.deactivated(hwnd);
                return None;
            }
            // Alt+F4 hides it; the window stays for the next popup.
            WM_CLOSE => self.deactivated(hwnd),
            // Handled: TRUE, so nothing sets the class cursor over it.
            WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                set_cursor(self.cursor.get());
                return Some(LRESULT(1));
            }
            WM_CAPTURECHANGED if lparam.0 != hwnd.0 as isize => {
                self.send(PopupInput::CaptureLost);
                return None;
            }
            // Alt alone does not start menu mode in a window without a menu.
            WM_SYSCOMMAND if (wparam.0 & 0xFFF0) as u32 == SC_KEYMENU => {}
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

/// Shows `cursor` now; also for the bar window.
pub(super) fn set_cursor(cursor: PopupCursor) {
    let id = match cursor {
        PopupCursor::Arrow => IDC_ARROW,
        PopupCursor::ResizeWidth => IDC_SIZEWE,
        PopupCursor::ResizeHeight => IDC_SIZENS,
    };
    // SAFETY: loads and sets a system cursor.
    unsafe {
        if let Ok(cursor) = LoadCursorW(None, id) {
            SetCursor(Some(cursor));
        }
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

/// The keys popups use (SPEC §9.19).
fn key(vk: VIRTUAL_KEY) -> Option<Key> {
    Some(match vk {
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_RETURN => Key::Enter,
        VK_ESCAPE => Key::Escape,
        VK_BACK => Key::Backspace,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_PRIOR => Key::PageUp,
        VK_NEXT => Key::PageDown,
        _ => return None,
    })
}

/// The lines one wheel notch scrolls (Settings ▸ Mouse), 3 where it cannot be read.
fn wheel_lines() -> f32 {
    let mut lines: u32 = 3;
    // SAFETY: SPI_GETWHEELSCROLLLINES writes a UINT to the pointer.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some((&raw mut lines).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    match (read.is_ok(), lines) {
        (false, _) => 3.0,
        (true, u32::MAX) => PAGE_LINES,
        (true, n) => n as f32,
    }
}

unsafe extern "system" fn popup_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE && lparam.0 != 0 {
        // SAFETY: lpCreateParams is the leaked `Shared` passed to CreateWindowExW.
        unsafe {
            let shared = (*(lparam.0 as *const CREATESTRUCTW)).lpCreateParams;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, shared as isize);
        }
    }
    // SAFETY: GWLP_USERDATA is 0 or the leaked `Shared`, which is never freed.
    let shared = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Shared).as_ref() };
    let handled = shared.and_then(|shared| {
        // A panic must not cross the FFI boundary; the panic hook has logged it.
        catch_unwind(AssertUnwindSafe(|| {
            shared.handle(hwnd, msg, wparam, lparam)
        }))
        .ok()
        .flatten()
    });
    // SAFETY: default processing for our own window.
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
}

pub(super) struct PopupWindow {
    hwnd: HWND,
    shared: &'static Shared,
    renderer: RefCell<Renderer>,
    /// When the current fade-in started.
    fading_since: Cell<Option<Instant>>,
}

impl PopupWindow {
    /// Creates the (hidden) popup window, owned by the bar window; its input goes to `sink`.
    pub(super) fn new(owner: HWND, sink: Box<dyn Fn(UiEvent)>) -> Result<Self> {
        let shared: &'static Shared = Box::leak(Box::new(Shared {
            sink,
            visible: Cell::new(false),
            scale: Cell::new(1.0),
            tracking_leave: Cell::new(false),
            cursor: Cell::new(PopupCursor::Arrow),
        }));
        // SAFETY: plain class registration and window creation; the window and `shared` live
        // until the process ends.
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(popup_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            // A second registration fails harmlessly.
            RegisterClassExW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                Some(owner),
                None,
                Some(instance),
                Some(std::ptr::from_ref(shared).cast::<c_void>()),
            )?;
            Ok(Self {
                hwnd,
                shared,
                renderer: RefCell::new(Renderer::new()?),
                fading_since: Cell::new(None),
            })
        }
    }

    /// Renders and shows the popup at `rect` (physical screen px), activated. Returns whether
    /// a fade-in started (the caller then steps it with [`Self::fade_step`]).
    pub(super) fn show(
        &self,
        rect: Rect,
        scale: f32,
        paint: &mut dyn FnMut(&mut dyn Canvas),
    ) -> Result<bool> {
        let (width, height) = (rect.width().max(1), rect.height().max(1));
        let dc = self
            .renderer
            .borrow_mut()
            .draw(width, height, scale, |canvas| paint(canvas))?;
        self.shared.scale.set(scale);
        let appearing = !self.shared.visible.get();
        let alpha = if appearing { 0 } else { self.alpha() };
        // SAFETY: plain calls on our own window; the frame's DC stays valid until the next draw.
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&POINT {
                    x: rect.left,
                    y: rect.top,
                }),
                Some(&SIZE {
                    cx: width,
                    cy: height,
                }),
                Some(dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend(alpha)),
                ULW_ALPHA,
            )?;
            if appearing {
                // Visible first: the activation changes below must not look like a blur.
                self.shared.visible.set(true);
                let _ = ShowWindow(self.hwnd, SW_SHOW);
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE,
                );
                // The click on the bar was this process's input, so it may take the
                // foreground, and with it the keyboard.
                if !SetForegroundWindow(self.hwnd).as_bool() {
                    tracing::debug!("the popup could not take the foreground");
                }
                let _ = SetFocus(Some(self.hwnd));
                self.fading_since.set(Some(Instant::now()));
            }
        }
        Ok(appearing)
    }

    /// The opacity the fade has reached.
    fn alpha(&self) -> u8 {
        self.fading_since.get().map_or(255, |since| {
            let progress = since.elapsed().as_secs_f32() / FADE.as_secs_f32();
            (progress.min(1.0) * 255.0) as u8
        })
    }

    /// Advances the fade-in; returns whether it goes on.
    pub(super) fn fade_step(&self) -> bool {
        if !self.shared.visible.get() {
            return false;
        }
        let alpha = self.alpha();
        // SAFETY: changes only the window's opacity; position, size and content stay.
        let _ = unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                None,
                None,
                None,
                None,
                COLORREF(0),
                Some(&blend(alpha)),
                ULW_ALPHA,
            )
        };
        if alpha == 255 {
            self.fading_since.set(None);
        }
        alpha < 255
    }

    /// Sets the pointer's shape over the window, at once if the pointer is there.
    pub(super) fn set_cursor(&self, cursor: PopupCursor) {
        if self.shared.cursor.replace(cursor) != cursor && self.shared.tracking_leave.get() {
            set_cursor(cursor);
        }
    }

    /// Captures the pointer for the window, or releases it.
    pub(super) fn capture(&self, capture: bool) {
        // SAFETY: plain calls on our own window.
        unsafe {
            if capture {
                SetCapture(self.hwnd);
            } else if GetCapture() == self.hwnd {
                let _ = ReleaseCapture();
            }
        }
    }

    /// Hides the popup; no deactivation is reported.
    pub(super) fn hide(&self) {
        if self.shared.visible.replace(false) {
            self.fading_since.set(None);
            self.shared.tracking_leave.set(false);
            // SAFETY: plain call on our own window.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }
}
