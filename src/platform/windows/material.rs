//! The material behind the bar's translucent pixels (SPEC §12.2).
//!
//! Acrylic is the accent policy (`SetWindowCompositionAttribute` with
//! `ACCENT_ENABLE_ACRYLICBLURBEHIND`, Windows 10 1803 and later), as taskbars use it. Windows
//! 11's system backdrop (`DWMWA_SYSTEMBACKDROP_TYPE`) draws its solid fallback while its window
//! is inactive, and the bar never activates (§5.3), so its Acrylic showed as an opaque grey.
//! Mica exists only as a system backdrop (Windows 11 22H2 and later): the bar's window
//! procedure draws its frame as active whenever asked (`WM_NCACTIVATE`, `appbar.rs`), and
//! applying Mica asks once, since a window that never activates is never asked. Mica is the
//! wallpaper, blurred and tinted light or dark by the window's dark-mode attribute
//! (`DWMWA_USE_IMMERSIVE_DARK_MODE`), so the window takes the bar's darkness first.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_NCACTIVATE};
use windows::core::{BOOL, s, w};

use crate::platform::Material;

/// `WCA_ACCENT_POLICY`.
const ACCENT_POLICY: u32 = 19;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
/// The acrylic's tint, `0xAABBGGRR`: next to none, as the bar draws its own (§12.2); a zero
/// alpha turns the acrylic off on some builds.
const UNTINTED: u32 = 0x0100_0000;

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

/// Applies `material` behind the window `hwnd`, after removing the one before, in its dark look
/// when `dark` (the bar's background is dark); the error says why it is not available.
pub(super) fn apply(hwnd: HWND, material: Material, dark: bool) -> Result<(), String> {
    // Windows before 10 20H1 lacks the attribute; Mica, which uses it, is newer still.
    if let Err(e) = dark_mode(hwnd, dark) {
        tracing::debug!("the bar's dark mode attribute is not available: {e}");
    }
    // Either way may have been applied before; one that Windows lacks fails harmlessly.
    let _ = accent(hwnd, ACCENT_DISABLED);
    let _ = backdrop(hwnd, DWMSBT_NONE);
    match material {
        Material::None => Ok(()),
        Material::Acrylic => accent(hwnd, ACCENT_ENABLE_ACRYLICBLURBEHIND),
        Material::Mica => {
            backdrop(hwnd, DWMSBT_MAINWINDOW)?;
            // SAFETY: a plain message to our own window.
            unsafe { SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0))) };
            Ok(())
        }
    }
}

/// Sets the accent policy's `state`.
fn accent(hwnd: HWND, state: u32) -> Result<(), String> {
    // SAFETY: user32 is loaded in every process with windows; the name is a static C string.
    let export = unsafe { GetModuleHandleW(w!("user32.dll")) }
        .ok()
        .and_then(|user32| unsafe { GetProcAddress(user32, s!("SetWindowCompositionAttribute")) })
        .ok_or("SetWindowCompositionAttribute is not available")?;
    // SAFETY: the export has had this signature since Windows 7.
    let set: SetWindowCompositionAttribute = unsafe { std::mem::transmute_copy(&export) };
    let mut policy = AccentPolicy {
        state,
        flags: 0,
        gradient: UNTINTED,
        animation: 0,
    };
    let mut data = CompositionData {
        attribute: ACCENT_POLICY,
        data: (&raw mut policy).cast(),
        size: size_of::<AccentPolicy>(),
    };
    // SAFETY: `data` points to the policy, which outlives the call.
    if unsafe { set(hwnd, &mut data) }.as_bool() {
        Ok(())
    } else {
        Err("SetWindowCompositionAttribute failed".into())
    }
}

/// Sets the window's dark-mode attribute, which picks Mica's light or dark look.
fn dark_mode(hwnd: HWND, dark: bool) -> Result<(), String> {
    let value = BOOL::from(dark);
    // SAFETY: the attribute takes a BOOL, passed with its size.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const value).cast(),
            size_of::<BOOL>() as u32,
        )
    }
    .map_err(|e| e.message())
}

/// Sets the system backdrop (Windows 11 22H2 and later).
fn backdrop(hwnd: HWND, kind: DWM_SYSTEMBACKDROP_TYPE) -> Result<(), String> {
    // SAFETY: the attribute takes a DWM_SYSTEMBACKDROP_TYPE, passed with its size.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&raw const kind).cast(),
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    }
    .map_err(|e| e.message())
}

#[cfg(test)]
mod tests {
    //! Puts a window set up as the bar is (per-pixel alpha, never active), drawn fully
    //! transparent, over a red one, and reads the screen behind it with each material.

    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{COLORREF, HINSTANCE, LRESULT, RECT};
    use windows::Win32::Graphics::Dwm::{
        DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DWMSBT_TRANSIENTWINDOW,
        DwmEnableBlurBehindWindow,
    };
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, BitBlt, CAPTUREBLT, CreateCompatibleDC,
        CreateDIBSection, CreateRectRgn, CreateSolidBrush, DIB_RGB_COLORS, DeleteDC, DeleteObject,
        GetDC, GetStockObject, HBRUSH, ROP_CODE, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE,
        PeekMessageW, RegisterClassExW, TranslateMessage, WINDOW_EX_STYLE, WNDCLASSEXW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
    };
    use windows::core::PCWSTR;

    use super::*;
    use crate::platform::Platform;

    /// Whether the test window keeps its frame active, as the bar's does.
    static KEEP_ACTIVE: AtomicBool = AtomicBool::new(false);

    extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let wparam = if msg == WM_NCACTIVATE && KEEP_ACTIVE.load(Ordering::SeqCst) {
            WPARAM(1)
        } else {
            wparam
        };
        // SAFETY: default processing.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn window(class: PCWSTR, brush: HBRUSH, rect: RECT) -> HWND {
        // SAFETY: plain class registration and window creation for the test.
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None).expect("module").into();
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc),
                hInstance: instance,
                hbrBackground: brush,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WINDOW_EX_STYLE(0),
                class,
                PCWSTR::null(),
                WS_POPUP | WS_VISIBLE,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(instance),
                None,
            )
            .expect("window")
        }
    }

    /// Handles this thread's messages for `time`, so the windows paint and DWM composes.
    fn pump(time: Duration) {
        let until = Instant::now() + time;
        while Instant::now() < until {
            let mut msg = MSG::default();
            // SAFETY: plain message loop calls.
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    /// The screen in `rect` as straight RGBA rows.
    fn capture(rect: RECT) -> Vec<u8> {
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: plain GDI calls; every object is released before returning.
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).expect("DIB");
            let previous = SelectObject(dc, bitmap.into());
            BitBlt(
                dc,
                0,
                0,
                width,
                height,
                Some(screen),
                rect.left,
                rect.top,
                ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0),
            )
            .expect("BitBlt");
            let len = (width * height * 4) as usize;
            let bgra = std::slice::from_raw_parts(bits.cast::<u8>(), len);
            let rgba = bgra
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[2], p[1], p[0], 255])
                .collect();
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            rgba
        }
    }

    /// The mean colour of `rgba`.
    fn mean(rgba: &[u8]) -> [u8; 3] {
        let n = (rgba.len() / 4).max(1) as u64;
        let pixels = rgba.as_chunks::<4>().0;
        let sum = |c: usize| pixels.iter().map(|p| u64::from(p[c])).sum::<u64>() / n;
        [sum(0) as u8, sum(1) as u8, sum(2) as u8]
    }

    /// Writes `target/screenshots/material*.{txt,png}` for a person to compare. CI's runner (no
    /// GPU) draws no material although transparency effects are on: the system backdrop shows
    /// its solid fallback, the accent policy black. So only the clear bar is asserted; on real
    /// hardware Acrylic shows the red blurred (`cargo test material -- --ignored --nocapture`).
    #[test]
    #[ignore = "needs a Windows desktop session"]
    fn the_material_shows_what_is_behind_the_bar() {
        let transparency = super::super::WindowsPlatform.appearance().transparency;
        let red = RECT {
            left: 200,
            top: 200,
            right: 600,
            bottom: 400,
        };
        let inside = RECT {
            left: 260,
            top: 260,
            right: 540,
            bottom: 340,
        };
        // SAFETY: plain GDI object creation for the test.
        let (red_brush, black) = unsafe {
            (
                CreateSolidBrush(COLORREF(0x0000_00FF)),
                HBRUSH(GetStockObject(BLACK_BRUSH).0),
            )
        };
        let behind = window(w!("InfodockTestRed"), red_brush, red);
        let bar = window(w!("InfodockTestBar"), black, inside);
        // As the bar window (`window.rs`): per-pixel alpha, so black GDI pixels are clear.
        // SAFETY: plain calls on the test's window; the region is deleted after use.
        unsafe {
            let region = CreateRectRgn(0, 0, -1, -1);
            let blur = DWM_BLURBEHIND {
                dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                fEnable: true.into(),
                hRgnBlur: region,
                fTransitionOnMaximized: false.into(),
            };
            DwmEnableBlurBehindWindow(bar, &blur).expect("blur behind");
            let _ = DeleteObject(region.into());
        }

        type Setup = fn(HWND) -> Result<(), String>;
        let variants: [(&str, Setup); 7] = [
            ("none", |h| apply(h, Material::None, true)),
            ("acrylic", |h| apply(h, Material::Acrylic, true)),
            ("mica-dark", |h| apply(h, Material::Mica, true)),
            ("mica-light", |h| apply(h, Material::Mica, false)),
            ("acrylic-backdrop", |h| backdrop(h, DWMSBT_TRANSIENTWINDOW)),
            ("acrylic-backdrop-active", |h| {
                backdrop(h, DWMSBT_TRANSIENTWINDOW)?;
                KEEP_ACTIVE.store(true, Ordering::SeqCst);
                // SAFETY: a plain message to the test's window.
                unsafe { SendMessageW(h, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0))) };
                Ok(())
            }),
            ("mica-inactive", |h| backdrop(h, DWMSBT_MAINWINDOW)),
        ];
        let mut report = format!("transparency effects: {transparency}\n");
        let mut seen = Vec::new();
        for (name, setup) in variants {
            KEEP_ACTIVE.store(matches!(name, "mica-dark" | "mica-light"), Ordering::SeqCst);
            let _ = apply(bar, Material::None, true);
            // SAFETY: a plain message to the test's window: an inactive frame to start from.
            unsafe { SendMessageW(bar, WM_NCACTIVATE, Some(WPARAM(0)), Some(LPARAM(0))) };
            let applied = setup(bar);
            pump(Duration::from_millis(700));
            let rgba = capture(inside);
            let colour = mean(&rgba);
            let _ = writeln!(report, "{name}: {applied:?} -> {colour:?}");
            crate::test_util::write_png(
                &crate::test_util::screenshot_path(&format!("material-{name}")),
                (inside.right - inside.left) as u32,
                (inside.bottom - inside.top) as u32,
                &rgba,
            );
            seen.push((name, colour));
        }
        let _ = std::fs::write(
            crate::test_util::screenshot_path("material").with_extension("txt"),
            &report,
        );
        // SAFETY: the test's own windows.
        unsafe {
            let _ = DestroyWindow(bar);
            let _ = DestroyWindow(behind);
        }
        eprintln!("{report}");

        let colour = |name| seen.iter().find(|(n, _)| *n == name).map(|(_, c)| *c);
        let [r, g, b] = colour("none").expect("none");
        assert!(
            r > 200 && g < 60 && b < 60,
            "a clear bar shows the red: {report}"
        );
    }
}
