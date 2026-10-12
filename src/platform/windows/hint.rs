//! The hint window (SPEC §4.1, §7.1): layered (`UpdateLayeredWindow`, per-pixel alpha),
//! topmost, click-through, never activated, owned by the bar window. Created on first use and
//! reused. A new hint fades in over 120 ms; the bar window's timer drives the fade.

use std::cell::{Cell, RefCell};
use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HWND_TOPMOST, RegisterClassExW, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use super::render::Renderer;
use crate::platform::{Canvas, Rect};

const CLASS_NAME: PCWSTR = w!("Infodock.Hint");
/// How long a new hint takes to fade in (SPEC §7.3).
pub(super) const FADE: Duration = Duration::from_millis(120);

fn blend(alpha: u8) -> BLENDFUNCTION {
    BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: AC_SRC_ALPHA as u8,
    }
}

/// The hint window only shows what it is given; everything goes to the default procedure.
unsafe extern "system" fn hint_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: default processing for our own window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub(super) struct HintWindow {
    hwnd: HWND,
    renderer: RefCell<Renderer>,
    visible: Cell<bool>,
    /// When the current fade-in started.
    fading_since: Cell<Option<Instant>>,
}

impl HintWindow {
    /// Creates the (hidden) hint window, owned by the bar window.
    pub(super) fn new(owner: HWND) -> Result<Self> {
        // SAFETY: plain class registration and window creation; the window is never destroyed
        // before the process ends.
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(hint_proc),
                hInstance: instance,
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            // A second registration fails harmlessly.
            RegisterClassExW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
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
                None,
            )?;
            Ok(Self {
                hwnd,
                renderer: RefCell::new(Renderer::new()?),
                visible: Cell::new(false),
                fading_since: Cell::new(None),
            })
        }
    }

    /// Renders and shows the hint at `rect` (physical screen px). Returns whether a fade-in
    /// started (the caller then steps it with [`Self::fade_step`]).
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
        let appearing = !self.visible.get();
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
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
                self.visible.set(true);
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
        if !self.visible.get() {
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

    pub(super) fn hide(&self) {
        if self.visible.replace(false) {
            self.fading_since.set(None);
            // SAFETY: plain call on our own window.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }
}
