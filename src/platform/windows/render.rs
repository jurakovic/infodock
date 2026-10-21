//! The bar renderer (SPEC §4.6): Direct2D with a software render target, drawing into a
//! premultiplied 32-bit DIB section, with DirectWrite text in grayscale antialiasing (the
//! background is transparent). Software rendering keeps the GPU driver out of the process; a
//! bar-sized frame is cheap to rasterise on the CPU. The caller presents the DIB: the bar copies
//! it to its DWM-composited window, layered popups pass it to `UpdateLayeredWindow`, and
//! [`render_offscreen`] copies it into a [`Bitmap`].

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{ManuallyDrop, size_of};
use std::sync::{Arc, Weak};

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_EXTEND_MODE_CLAMP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_GAMMA_2_2,
    D2D1_LAYER_OPTIONS_NONE, D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap,
    ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1Geometry, ID2D1Layer, ID2D1SolidColorBrush,
    ID2D1StrokeStyle,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::core::Result;
use windows_numerics::{Matrix3x2, Vector2};

use super::text::DwLayout;
use crate::platform::{
    Bitmap, Brush, Canvas, Color, PlatformError, PointF, RectF, Shape, TextLayout,
};

/// Bitmaps uploaded to a render target are kept for this many distinct bitmaps.
const BITMAP_CACHE: usize = 64;

/// A memory DC with a top-down 32-bit DIB selected into it.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    /// The DIB's pixels: premultiplied BGRA, top-down rows of `width` pixels.
    bits: *const u8,
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
                bits: bits.cast(),
                width,
                height,
            })
        }
    }

    /// The frame as premultiplied RGBA.
    fn to_bitmap(&self) -> Option<Bitmap> {
        let len = self.width as usize * self.height as usize * 4;
        // SAFETY: the DIB holds `len` bytes while the surface lives; GDI has finished writing
        // to it (the caller flushed).
        let bgra = unsafe { std::slice::from_raw_parts(self.bits, len) };
        Bitmap::from_premultiplied(self.width as u32, self.height as u32, swap_red_blue(bgra))
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

/// A memory DC holding a copy of a bitmap, e.g. for `UpdateLayeredWindow`.
pub(super) struct BitmapDc(Surface);

impl BitmapDc {
    pub(super) fn new(bitmap: &Bitmap) -> Result<Self> {
        let surface = Surface::new(bitmap.width() as i32, bitmap.height() as i32)?;
        let bgra = swap_red_blue(bitmap.pixels());
        // SAFETY: the DIB holds width × height × 4 bytes, as many as the bitmap; nothing else
        // writes to it.
        unsafe {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), surface.bits.cast_mut(), bgra.len());
        }
        Ok(Self(surface))
    }

    pub(super) fn dc(&self) -> HDC {
        self.0.dc
    }
}

/// RGBA ↔ BGRA.
fn swap_red_blue(pixels: &[u8]) -> Vec<u8> {
    let mut out = pixels.to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    out
}

/// A bitmap uploaded to the render target, while its pixels are alive.
struct CachedBitmap {
    pixels: Weak<[u8]>,
    bitmap: ID2D1Bitmap,
}

/// Device-dependent resources; re-created together when Direct2D asks for it.
struct Target {
    target: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    bitmaps: RefCell<Vec<CachedBitmap>>,
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
            // ClearType needs an opaque background; the bar's is translucent.
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = target.CreateSolidColorBrush(&color_f(Color::TRANSPARENT), None)?;
            Ok(Self {
                target,
                brush,
                bitmaps: RefCell::new(Vec::new()),
            })
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
        target
            .bitmaps
            .borrow_mut()
            .retain(|cached| cached.pixels.strong_count() > 0);
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
            let mut canvas = D2dCanvas {
                factory: &self.factory,
                target,
                scale,
                stack: Vec::new(),
            };
            paint(&mut canvas);
            canvas.pop_all();
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

/// Renders `width`×`height` physical px at `scale` into a bitmap, e.g. for previews and the
/// screenshots CI publishes (SPEC §12.4, §16).
pub fn render_offscreen(
    width: u32,
    height: u32,
    scale: f32,
    paint: &mut dyn FnMut(&mut dyn Canvas),
) -> std::result::Result<Bitmap, PlatformError> {
    let os = |e: windows::core::Error| PlatformError::Os(format!("rendering failed: {e}"));
    let (width, height) = (
        i32::try_from(width).map_err(|_| PlatformError::Os("too wide".into()))?,
        i32::try_from(height).map_err(|_| PlatformError::Os("too tall".into()))?,
    );
    let mut renderer = Renderer::new().map_err(os)?;
    renderer
        .draw(width, height, scale, |canvas| paint(canvas))
        .map_err(os)?;
    // SAFETY: waits for GDI to finish writing to the DIB.
    let _ = unsafe { GdiFlush() };
    renderer
        .surface
        .as_ref()
        .and_then(Surface::to_bitmap)
        .ok_or_else(|| PlatformError::Os("no frame was rendered".into()))
}

/// A renderer kept across frames, as the bar window keeps its own: what drawing a frame costs,
/// for the budget tests (SPEC §14.1).
#[cfg(test)]
pub struct FrameRenderer(Renderer);

#[cfg(test)]
impl FrameRenderer {
    pub fn new() -> std::result::Result<Self, PlatformError> {
        Renderer::new()
            .map(Self)
            .map_err(|e| PlatformError::Os(format!("no renderer: {e}")))
    }

    /// Draws a frame of `width`×`height` physical px at `scale` and waits until its pixels are
    /// written, as before the bar copies them to its window.
    pub fn draw(
        &mut self,
        width: u32,
        height: u32,
        scale: f32,
        paint: &mut dyn FnMut(&mut dyn Canvas),
    ) -> std::result::Result<(), PlatformError> {
        let size = |n: u32| i32::try_from(n).map_err(|_| PlatformError::Os("too large".into()));
        self.0
            .draw(size(width)?, size(height)?, scale, |canvas| paint(canvas))
            .map_err(|e| PlatformError::Os(format!("rendering failed: {e}")))?;
        // SAFETY: waits for GDI to finish writing to the DIB.
        let _ = unsafe { GdiFlush() };
        Ok(())
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

fn point(p: PointF) -> Vector2 {
    Vector2 { X: p.x, Y: p.y }
}

/// What [`Canvas::pop`] ends.
enum Pushed {
    Clip,
    Layer,
}

struct D2dCanvas<'a> {
    factory: &'a ID2D1Factory,
    target: &'a Target,
    scale: f32,
    stack: Vec<Pushed>,
}

impl D2dCanvas<'_> {
    /// A Direct2D brush for `brush`; `None` if it cannot be created.
    fn brush(&self, brush: &Brush) -> Option<ID2D1Brush> {
        match brush {
            Brush::Solid(color) => {
                // SAFETY: plain setter on the target's own brush.
                unsafe { self.target.brush.SetColor(&color_f(*color)) };
                Some(self.target.brush.clone().into())
            }
            Brush::Linear { start, end, stops } => {
                let stops: Vec<D2D1_GRADIENT_STOP> = stops
                    .iter()
                    .map(|s| D2D1_GRADIENT_STOP {
                        position: s.offset,
                        color: color_f(s.color),
                    })
                    .collect();
                let properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: point(*start),
                    endPoint: point(*end),
                };
                // SAFETY: plain resource creation on the target in use.
                let created = unsafe {
                    self.target
                        .target
                        .CreateGradientStopCollection(
                            &stops,
                            D2D1_GAMMA_2_2,
                            D2D1_EXTEND_MODE_CLAMP,
                        )
                        .and_then(|collection| {
                            self.target.target.CreateLinearGradientBrush(
                                &properties,
                                None,
                                &collection,
                            )
                        })
                };
                created.ok().map(Into::into)
            }
        }
    }

    fn polygon(&self, points: &[PointF]) -> Option<ID2D1Geometry> {
        let (first, rest) = points.split_first()?;
        let rest: Vec<Vector2> = rest.iter().copied().map(point).collect();
        // SAFETY: plain geometry construction; the sink is closed before use.
        let created = unsafe {
            self.factory.CreatePathGeometry().and_then(|geometry| {
                let sink = geometry.Open()?;
                sink.BeginFigure(point(*first), D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLines(&rest);
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                sink.Close()?;
                Ok(geometry)
            })
        };
        created.ok().map(Into::into)
    }

    /// The Direct2D bitmap for `bitmap`, uploaded on first use.
    fn upload(&self, bitmap: &Bitmap) -> Option<ID2D1Bitmap> {
        let pixels = bitmap.shared_pixels();
        let mut cache = self.target.bitmaps.borrow_mut();
        if let Some(cached) = cache
            .iter()
            .find(|c| std::ptr::addr_eq(c.pixels.as_ptr(), Arc::as_ptr(pixels)))
        {
            return Some(cached.bitmap.clone());
        }
        let bgra = swap_red_blue(bitmap.pixels());
        let properties = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let size = D2D_SIZE_U {
            width: bitmap.width(),
            height: bitmap.height(),
        };
        // SAFETY: `bgra` holds `height` rows of `width × 4` bytes for the duration of the call.
        let uploaded = unsafe {
            self.target.target.CreateBitmap(
                size,
                Some(bgra.as_ptr().cast()),
                bitmap.width() * 4,
                &properties,
            )
        }
        .ok()?;
        if cache.len() >= BITMAP_CACHE {
            cache.remove(0);
        }
        cache.push(CachedBitmap {
            pixels: Arc::downgrade(pixels),
            bitmap: uploaded.clone(),
        });
        Some(uploaded)
    }

    fn pop_all(&mut self) {
        while !self.stack.is_empty() {
            self.pop();
        }
    }
}

impl Canvas for D2dCanvas<'_> {
    fn scale(&self) -> f32 {
        self.scale
    }

    fn fill(&mut self, shape: &Shape, brush: &Brush) {
        let Some(brush) = self.brush(brush) else {
            return;
        };
        let target = &self.target.target;
        // SAFETY: drawing calls between BeginDraw and EndDraw with resources of this target.
        unsafe {
            match shape {
                Shape::Rect(rect) => target.FillRectangle(&rect_f(*rect), &brush),
                Shape::RoundedRect { rect, radius } => target.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect_f(*rect),
                        radiusX: *radius,
                        radiusY: *radius,
                    },
                    &brush,
                ),
                Shape::Polygon(points) => {
                    if let Some(geometry) = self.polygon(points) {
                        target.FillGeometry(&geometry, &brush, None::<&ID2D1Brush>);
                    }
                }
            }
        }
    }

    fn stroke(&mut self, shape: &Shape, brush: &Brush, width: f32) {
        let Some(brush) = self.brush(brush) else {
            return;
        };
        let target = &self.target.target;
        let plain = None::<&ID2D1StrokeStyle>;
        // SAFETY: drawing calls between BeginDraw and EndDraw with resources of this target.
        unsafe {
            match shape {
                Shape::Rect(rect) => target.DrawRectangle(&rect_f(*rect), &brush, width, plain),
                Shape::RoundedRect { rect, radius } => target.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect_f(*rect),
                        radiusX: *radius,
                        radiusY: *radius,
                    },
                    &brush,
                    width,
                    plain,
                ),
                Shape::Polygon(points) => {
                    if let Some(geometry) = self.polygon(points) {
                        target.DrawGeometry(&geometry, &brush, width, plain);
                    }
                }
            }
        }
    }

    fn draw_line(&mut self, from: PointF, to: PointF, brush: &Brush, width: f32) {
        if let Some(brush) = self.brush(brush) {
            // SAFETY: a drawing call between BeginDraw and EndDraw.
            unsafe {
                self.target.target.DrawLine(
                    point(from),
                    point(to),
                    &brush,
                    width,
                    None::<&ID2D1StrokeStyle>,
                );
            }
        }
    }

    fn draw_text(&mut self, layout: &TextLayout, origin: PointF, color: Color) {
        let Some(DwLayout(layout)) = layout.inner::<DwLayout>() else {
            return;
        };
        // SAFETY: drawing calls between BeginDraw and EndDraw with this target's brush.
        unsafe {
            self.target.brush.SetColor(&color_f(color));
            self.target.target.DrawTextLayout(
                point(origin),
                layout,
                &self.target.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    fn draw_bitmap(&mut self, bitmap: &Bitmap, dest: RectF, opacity: f32) {
        if let Some(uploaded) = self.upload(bitmap) {
            // SAFETY: a drawing call between BeginDraw and EndDraw with a bitmap of this target.
            unsafe {
                self.target.target.DrawBitmap(
                    &uploaded,
                    Some(&rect_f(dest)),
                    opacity.clamp(0.0, 1.0),
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
        }
    }

    fn push_clip(&mut self, rect: RectF) {
        // SAFETY: balanced by `pop`, or by `pop_all` at the end of the frame.
        unsafe {
            self.target
                .target
                .PushAxisAlignedClip(&rect_f(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
        self.stack.push(Pushed::Clip);
    }

    fn push_opacity(&mut self, opacity: f32) {
        let parameters = D2D1_LAYER_PARAMETERS {
            contentBounds: D2D_RECT_F {
                left: f32::MIN,
                top: f32::MIN,
                right: f32::MAX,
                bottom: f32::MAX,
            },
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: opacity.clamp(0.0, 1.0),
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        // SAFETY: balanced by `pop`, or by `pop_all` at the end of the frame. Without a layer
        // object Direct2D manages one itself (Windows 8 and later).
        unsafe {
            self.target
                .target
                .PushLayer(&parameters, None::<&ID2D1Layer>)
        };
        self.stack.push(Pushed::Layer);
    }

    fn pop(&mut self) {
        // SAFETY: undoes the matching push.
        unsafe {
            match self.stack.pop() {
                Some(Pushed::Clip) => self.target.target.PopAxisAlignedClip(),
                Some(Pushed::Layer) => self.target.target.PopLayer(),
                None => {}
            }
        }
    }
}
