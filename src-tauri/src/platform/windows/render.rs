//! The bar renderer (SPEC §4.6): Direct2D with a software render target, drawing into a
//! premultiplied 32-bit DIB section. Software rendering keeps the GPU driver out of the process;
//! a bar-sized frame is cheap to rasterise on the CPU. The caller presents the DIB: the bar
//! copies it to its DWM-composited window, layered popups pass it to `UpdateLayeredWindow`.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1CreateFactory,
    ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::core::Result;

use crate::platform::{Canvas, Color, RectF};

/// A memory DC with a top-down 32-bit DIB selected into it.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}

impl Surface {
    fn new(width: i32, height: i32) -> Result<Self> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative: top-down rows.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: plain GDI object creation; every object is released in `Drop`.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(bitmap) => bitmap,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(e);
                }
            };
            let previous = SelectObject(dc, bitmap.into());
            Ok(Self {
                dc,
                bitmap,
                previous,
                width,
                height,
            })
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: undoes `new` in reverse order.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Device-dependent resources; re-created together when Direct2D asks for it.
struct Target {
    target: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
}

impl Target {
    fn new(factory: &ID2D1Factory) -> Result<Self> {
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 0.0,
            dpiY: 0.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        // SAFETY: plain COM calls with initialised arguments.
        unsafe {
            let target = factory.CreateDCRenderTarget(&properties)?;
            let brush = target.CreateSolidColorBrush(&color_f(Color::TRANSPARENT), None)?;
            Ok(Self { target, brush })
        }
    }
}

pub struct Renderer {
    factory: ID2D1Factory,
    target: Option<Target>,
    surface: Option<Surface>,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        // SAFETY: plain factory creation; the factory is used on this thread only.
        let factory: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        Ok(Self {
            factory,
            target: None,
            surface: None,
        })
    }

    /// Draws a frame of `width`×`height` physical px at `scale` and returns the memory DC that
    /// holds it (valid until the next call). `paint` draws in logical px on a cleared canvas.
    pub fn draw(
        &mut self,
        width: i32,
        height: i32,
        scale: f32,
        paint: impl FnOnce(&mut dyn Canvas),
    ) -> Result<HDC> {
        let (width, height) = (width.max(1), height.max(1));
        if self
            .surface
            .as_ref()
            .is_some_and(|s| s.width != width || s.height != height)
        {
            self.surface = None;
        }
        let surface: &Surface = match &mut self.surface {
            Some(surface) => surface,
            empty => empty.insert(Surface::new(width, height)?),
        };
        let target: &Target = match &mut self.target {
            Some(target) => target,
            empty => empty.insert(Target::new(&self.factory)?),
        };
        let rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let dpi = 96.0 * scale;
        // SAFETY: the DC and the target stay alive for the whole frame.
        let result = unsafe {
            target.target.BindDC(surface.dc, &rect)?;
            target.target.SetDpi(dpi, dpi);
            target.target.BeginDraw();
            target.target.Clear(Some(&color_f(Color::TRANSPARENT)));
            paint(&mut D2dCanvas {
                target: &target.target,
                brush: &target.brush,
            });
            target.target.EndDraw(None, None)
        };
        match result {
            Ok(()) => Ok(surface.dc),
            Err(e) => {
                if e.code() == D2DERR_RECREATE_TARGET {
                    self.target = None;
                }
                Err(e)
            }
        }
    }
}

fn color_f(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

fn rect_f(r: RectF) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

struct D2dCanvas<'a> {
    target: &'a ID2D1DCRenderTarget,
    brush: &'a ID2D1SolidColorBrush,
}

impl Canvas for D2dCanvas<'_> {
    fn fill_rect(&mut self, rect: RectF, color: Color) {
        // SAFETY: called between BeginDraw and EndDraw on the target that owns the brush.
        unsafe {
            self.brush.SetColor(&color_f(color));
            self.target.FillRectangle(&rect_f(rect), self.brush);
        }
    }
}
