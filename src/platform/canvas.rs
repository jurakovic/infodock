//! Drawing types shared by the bar logic and the platform renderers (SPEC §4.6). Everything is
//! in logical px; renderers map it to physical px with the window's scale.

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

    /// The same colour with alpha `a` (0–1).
    pub fn with_alpha(self, a: f32) -> Self {
        Self {
            a: a.clamp(0.0, 1.0),
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SizeF {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
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
}

/// Drawing operations. Implemented by Direct2D on Windows; the API grows with the slices that
/// need it (text, bitmaps and shapes arrive in M1 S2).
pub trait Canvas {
    fn fill_rect(&mut self, rect: RectF, color: Color);
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
    fn alpha_is_clamped() {
        assert_eq!(Color::rgba(1, 2, 3, 1.0).with_alpha(1.5).a, 1.0);
        assert_eq!(Color::rgba(1, 2, 3, 1.0).with_alpha(-1.0).a, 0.0);
    }
}
