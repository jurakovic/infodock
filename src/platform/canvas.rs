//! Drawing types shared by the bar logic and the platform renderers (SPEC §4.6). Everything is
//! in logical px; renderers map it to physical px with the window's scale.

use super::bitmap::Bitmap;
use super::text::TextLayout;

/// A colour with straight (not premultiplied) alpha, each channel 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0.0);
    pub const BLACK: Self = Self::rgba(0, 0, 0, 1.0);
    pub const WHITE: Self = Self::rgba(255, 255, 255, 1.0);

    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a,
        }
    }

    /// Parses `#RRGGBB` (any case).
    pub fn from_hex(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self::rgba(channel(0)?, channel(2)?, channel(4)?, 1.0))
    }

    /// `#RRGGBB`, or `#RRGGBB/AA%` when not opaque; for logs and draw-call snapshots.
    pub fn to_hex(self) -> String {
        let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let rgb = format!(
            "#{:02X}{:02X}{:02X}",
            channel(self.r),
            channel(self.g),
            channel(self.b)
        );
        if self.a >= 1.0 {
            rgb
        } else {
            format!("{rgb}/{}%", (self.a.clamp(0.0, 1.0) * 100.0).round())
        }
    }

    /// The same colour with alpha `a` (0–1).
    pub fn with_alpha(self, a: f32) -> Self {
        Self {
            a: a.clamp(0.0, 1.0),
            ..self
        }
    }

    /// The same colour with its alpha multiplied by `opacity` (0–1).
    pub fn faded(self, opacity: f32) -> Self {
        self.with_alpha(self.a * opacity)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PointF {
    pub x: f32,
    pub y: f32,
}

impl PointF {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SizeF {
    pub width: f32,
    pub height: f32,
}

impl SizeF {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn from_size(size: SizeF) -> Self {
        Self::new(0.0, 0.0, size.width, size.height)
    }

    /// The rectangle at `origin` with `size`.
    pub fn at(origin: PointF, size: SizeF) -> Self {
        Self::new(
            origin.x,
            origin.y,
            origin.x + size.width,
            origin.y + size.height,
        )
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    /// Shrunk by `d` on every side (grown for negative `d`).
    pub fn inset(&self, d: f32) -> Self {
        Self::new(self.left + d, self.top + d, self.right - d, self.bottom - d)
    }
}

/// A colour stop of a gradient; `offset` runs from 0 (start) to 1 (end).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// What shapes and lines are painted with.
#[derive(Debug, Clone, PartialEq)]
pub enum Brush {
    Solid(Color),
    /// Colours change along the line from `start` to `end`; beyond its ends the end colours
    /// continue.
    Linear {
        start: PointF,
        end: PointF,
        stops: Vec<GradientStop>,
    },
}

impl From<Color> for Brush {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Rect(RectF),
    RoundedRect {
        rect: RectF,
        radius: f32,
    },
    /// A closed polygon.
    Polygon(Vec<PointF>),
}

/// Drawing operations, in logical px (SPEC §4.6). Implemented by Direct2D on Windows and by the
/// recording canvas in tests and on other OSes.
pub trait Canvas {
    /// Physical px per logical px.
    fn scale(&self) -> f32;
    fn fill(&mut self, shape: &Shape, brush: &Brush);
    /// Outlines `shape` with a line `width` px wide, centred on its edge.
    fn stroke(&mut self, shape: &Shape, brush: &Brush, width: f32);
    fn draw_line(&mut self, from: PointF, to: PointF, brush: &Brush, width: f32);
    /// Draws `layout` with the top-left corner of its box at `origin`.
    fn draw_text(&mut self, layout: &TextLayout, origin: PointF, color: Color);
    /// Draws `bitmap` scaled into `dest` with `opacity` (0–1).
    fn draw_bitmap(&mut self, bitmap: &Bitmap, dest: RectF, opacity: f32);
    /// Restricts drawing to `rect` until the matching [`Canvas::pop`].
    fn push_clip(&mut self, rect: RectF);
    /// Composes everything drawn until the matching [`Canvas::pop`] as one layer with `opacity`
    /// (0–1), e.g. for fades.
    fn push_opacity(&mut self, opacity: f32);
    /// Ends the latest clip or opacity layer. Layers still open when a frame ends are closed.
    fn pop(&mut self);

    fn fill_rect(&mut self, rect: RectF, color: Color) {
        self.fill(&Shape::Rect(rect), &Brush::Solid(color));
    }

    /// `value` rounded to whole physical px, for crisp edges and text baselines.
    fn snap(&self, value: f32) -> f32 {
        let scale = self.scale();
        (value * scale).round() / scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colours() {
        let c = Color::from_hex("#1c1C1F").expect("valid colour");
        assert_eq!(c, Color::rgba(0x1C, 0x1C, 0x1F, 1.0));
        assert_eq!(Color::from_hex("1C1C1F"), None);
        assert_eq!(Color::from_hex("#1C1C1"), None);
        assert_eq!(Color::from_hex("#GG0000"), None);
    }

    #[test]
    fn formats_hex_colours() {
        assert_eq!(Color::rgba(0x1C, 0x1C, 0x1F, 1.0).to_hex(), "#1C1C1F");
        assert_eq!(Color::WHITE.with_alpha(0.08).to_hex(), "#FFFFFF/8%");
    }

    #[test]
    fn alpha_is_clamped() {
        assert_eq!(Color::rgba(1, 2, 3, 1.0).with_alpha(1.5).a, 1.0);
        assert_eq!(Color::rgba(1, 2, 3, 1.0).with_alpha(-1.0).a, 0.0);
        assert_eq!(Color::WHITE.with_alpha(0.5).faded(0.5).a, 0.25);
    }

    #[test]
    fn rect_helpers() {
        let r = RectF::at(PointF::new(2.0, 3.0), SizeF::new(10.0, 4.0));
        assert_eq!(r, RectF::new(2.0, 3.0, 12.0, 7.0));
        assert_eq!((r.width(), r.height()), (10.0, 4.0));
        assert_eq!(r.inset(1.0), RectF::new(3.0, 4.0, 11.0, 6.0));
    }
}
