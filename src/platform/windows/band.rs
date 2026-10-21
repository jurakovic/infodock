//! The shadow band window (SPEC §5.1, §5.5, §12.4): a skin's transparent shadow, or the bar's
//! own, beside the bar on the side facing the screen centre, outside the reserved work area.
//! Layered (`UpdateLayeredWindow`, per-pixel alpha), click-through, never activated, and owned
//! by the bar window, so it keeps to the bar's z-order. Created when a band is first shown; it
//! follows the bar's window: moved with it, and hidden while the bar is hidden or off its place
//! (rising or sliding out).

use std::cell::{Cell, RefCell};
use std::mem::size_of;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GW_HWNDPREV, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW,
    GetWindowRect, HWND_TOP, HWND_TOPMOST, IsWindowVisible, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use super::render::BitmapDc;
use crate::platform::Band;

const CLASS_NAME: PCWSTR = w!("Infodock.Band");

/// The band window only shows what it is given; everything goes to the default procedure.
unsafe extern "system" fn band_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: default processing for our own window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub(super) struct BandWindow {
    hwnd: HWND,
    /// The bar window.
    owner: HWND,
    band: RefCell<Option<Band>>,
    /// The band's pixels are in the window: drawn since the band last changed.
    drawn: Cell<bool>,
    /// Where the window was last put (screen px), so a frame that moves nothing moves nothing.
    at: Cell<Option<(i32, i32)>>,
    visible: Cell<bool>,
}

impl BandWindow {
    /// Creates the (hidden) band window, owned by the bar window.
    pub(super) fn new(owner: HWND) -> Result<Self> {
        // SAFETY: plain class registration and window creation; the window is never destroyed
        // before the process ends.
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(band_proc),
                hInstance: instance,
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            // A second registration fails harmlessly.
            RegisterClassExW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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
                owner,
                band: RefCell::new(None),
                drawn: Cell::new(false),
                at: Cell::new(None),
                visible: Cell::new(false),
            })
        }
    }

    /// Shows `band` beside the bar from now on, or none. `away`: the bar is off its place, or
    /// being hidden.
    pub(super) fn set(&self, band: Option<Band>, away: bool) {
        let same = match (&*self.band.borrow(), &band) {
            (Some(old), Some(new)) => old.same(new),
            (None, None) => true,
            _ => false,
        };
        if !same {
            *self.band.borrow_mut() = band;
            self.drawn.set(false);
        }
        self.follow(away);
    }

    /// Puts the band beside the bar window where it is now, or hides it while the bar is hidden
    /// or `away` (off its place, or being hidden).
    pub(super) fn follow(&self, away: bool) {
        // SAFETY: plain queries on the bar window.
        let bar = unsafe {
            let mut rect = Default::default();
            (IsWindowVisible(self.owner).as_bool() && GetWindowRect(self.owner, &mut rect).is_ok())
                .then_some(rect)
        };
        let band = self.band.borrow();
        let (Some(band), Some(bar), false) = (band.as_ref(), bar, away) else {
            self.hide();
            return;
        };
        let height = band.bitmap.height() as i32;
        let at = POINT {
            x: bar.left,
            y: if band.above {
                bar.top - height
            } else {
                bar.bottom
            },
        };
        let placed = if self.drawn.get() {
            if self.at.get() == Some((at.x, at.y)) {
                Ok(())
            } else {
                self.move_to(at)
            }
        } else {
            self.draw(band, at)
        };
        if let Err(e) = placed {
            tracing::debug!("the shadow band could not be shown: {e}");
            self.hide();
            return;
        }
        self.at.set(Some((at.x, at.y)));
        if !self.visible.replace(true) {
            // SAFETY: plain call on our own window.
            let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
        }
        self.restack();
    }

    /// Moves the window, keeping its size, z-order and content.
    fn move_to(&self, at: POINT) -> Result<()> {
        // SAFETY: moves our own window; size, z-order and content stay.
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                at.x,
                at.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
    }

    /// Puts the band's pixels in the window at `at`.
    fn draw(&self, band: &Band, at: POINT) -> Result<()> {
        let pixels = BitmapDc::new(&band.bitmap)?;
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // SAFETY: plain call on our own window; the DC lives until the call returns.
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&at),
                Some(&SIZE {
                    cx: band.bitmap.width() as i32,
                    cy: band.bitmap.height() as i32,
                }),
                Some(pixels.dc()),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )?;
        }
        self.drawn.set(true);
        Ok(())
    }

    /// Keeps the band right above the bar in the z-order. Owned windows stay above their owner
    /// anyway; this also covers the bar turning topmost or going to the bottom (a full-screen
    /// app, SPEC §5.3) while the band is shown.
    fn restack(&self) {
        // SAFETY: plain queries and a z-order change of our own window.
        unsafe {
            let above = GetWindow(self.owner, GW_HWNDPREV).ok();
            if above == Some(self.hwnd) {
                return;
            }
            let topmost =
                GetWindowLongPtrW(self.owner, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
            let insert_after = match above {
                Some(window) if !window.is_invalid() => window,
                _ if topmost => HWND_TOPMOST,
                _ => HWND_TOP,
            };
            let _ = SetWindowPos(
                self.hwnd,
                Some(insert_after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    fn hide(&self) {
        if self.visible.replace(false) {
            // SAFETY: plain call on our own window.
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
    }
}
