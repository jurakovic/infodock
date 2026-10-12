//! Bitmaps (SPEC §4.6): decoded images as premultiplied RGBA in memory. They are plain data, so
//! worker threads can decode them (icons, images) and hand them to the bar thread. Raster files
//! are decoded by the platform (WIC on Windows); SVG is rasterised with `resvg` on every OS.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use super::canvas::Color;

/// Larger files are not read.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Larger results are refused, whatever the source says.
const MAX_SIDE: u32 = 8192;

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("could not read the image: {0}")]
    Io(#[from] std::io::Error),
    #[error("the image format is not supported")]
    Unsupported,
    #[error("the image is too large")]
    TooLarge,
    #[error("the image is damaged: {0}")]
    Invalid(String),
}

/// An image in physical px: premultiplied RGBA rows, top-down, without padding. Cheap to clone;
/// clones share the pixels.
#[derive(Clone, PartialEq)]
pub struct Bitmap {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl Bitmap {
    /// Wraps premultiplied RGBA; `None` if the buffer does not match the size.
    pub fn from_premultiplied(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        (width > 0 && height > 0 && pixels.len() == width as usize * height as usize * 4).then(
            || Self {
                width,
                height,
                pixels: pixels.into(),
            },
        )
    }

    /// Wraps straight (not premultiplied) RGBA, e.g. from a shell icon.
    pub fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Option<Self> {
        for px in rgba.as_chunks_mut::<4>().0 {
            let a = u32::from(px[3]);
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * a + 127) / 255) as u8;
            }
        }
        Self::from_premultiplied(width, height, rgba)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Premultiplied RGBA.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The pixels shared by this bitmap and its clones, for caches keyed by identity.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn shared_pixels(&self) -> &Arc<[u8]> {
        &self.pixels
    }

    /// Straight (not premultiplied) RGBA, e.g. for PNG files.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = self.pixels.to_vec();
        for px in rgba.as_chunks_mut::<4>().0 {
            let a = u32::from(px[3]);
            for c in &mut px[..3] {
                // Fully transparent pixels become transparent black.
                *c = (u32::from(*c) * 255 + a / 2)
                    .checked_div(a)
                    .map_or(0, |v| v.min(255)) as u8;
            }
        }
        rgba
    }

    /// The bitmap tinted with `tint` (SPEC §12.3): each pixel becomes its luminance
    /// `L = (0.299 R + 0.587 G + 0.114 B) / 255` times the tint colour; alpha is kept. Because
    /// the formula is linear in R, G and B, it applies to premultiplied pixels unchanged.
    pub fn tinted(&self, tint: Color) -> Self {
        let mut pixels = self.pixels.to_vec();
        for px in pixels.as_chunks_mut::<4>().0 {
            let l =
                (0.299 * f32::from(px[0]) + 0.587 * f32::from(px[1]) + 0.114 * f32::from(px[2]))
                    / 255.0;
            for (c, t) in px[..3].iter_mut().zip([tint.r, tint.g, tint.b]) {
                *c = (l * t.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        Self {
            pixels: pixels.into(),
            ..*self
        }
    }
}

impl fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bitmap({}×{})", self.width, self.height)
    }
}

/// The image formats Infodock reads (SPEC §4.6, §9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Ico,
    Svg,
}

impl ImageFormat {
    /// Recognises a format by its first bytes. Only these formats reach a decoder, so codecs
    /// installed by other software never see Infodock's files.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        const BOM: &[u8] = b"\xEF\xBB\xBF";
        let format = match bytes {
            [0x89, b'P', b'N', b'G', ..] => Self::Png,
            [0xFF, 0xD8, 0xFF, ..] => Self::Jpeg,
            [b'G', b'I', b'F', b'8', ..] => Self::Gif,
            [b'B', b'M', ..] => Self::Bmp,
            [0, 0, 1, 0, ..] => Self::Ico,
            // Compressed SVG (.svgz).
            [0x1F, 0x8B, ..] => Self::Svg,
            _ => {
                let text = bytes.strip_prefix(BOM).unwrap_or(bytes);
                let start = text.iter().position(|b| !b.is_ascii_whitespace())?;
                if text[start] != b'<' {
                    return None;
                }
                Self::Svg
            }
        };
        Some(format)
    }
}

/// Reads and decodes an image file; see [`decode_image`].
pub fn load_image(path: &Path, height: Option<u32>) -> Result<Bitmap, ImageError> {
    if std::fs::metadata(path)?.len() > MAX_FILE_BYTES {
        return Err(ImageError::TooLarge);
    }
    decode_image(&std::fs::read(path)?, height)
}

/// Decodes PNG, JPEG, GIF (first frame), BMP, ICO or SVG. With `height`, the result is scaled
/// to that many physical px keeping its aspect ratio (SVG is rasterised at that size; an icon
/// file picks its closest frame); without, it has the image's own size.
pub fn decode_image(bytes: &[u8], height: Option<u32>) -> Result<Bitmap, ImageError> {
    if height == Some(0) {
        return Err(ImageError::Invalid("zero height".into()));
    }
    match ImageFormat::sniff(bytes).ok_or(ImageError::Unsupported)? {
        ImageFormat::Svg => render_svg(bytes, height),
        format => super::decode_raster(bytes, format, height),
    }
}

/// The size of a `width`×`height` image scaled to `target` px tall, or `None` if it is too
/// large either way.
pub(crate) fn scaled_size(width: u32, height: u32, target: Option<u32>) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || width > MAX_SIDE * 8 || height > MAX_SIDE * 8 {
        return None;
    }
    let size = match target {
        None => (width, height),
        Some(h) => {
            let w = (f64::from(width) * f64::from(h) / f64::from(height)).round();
            (w.max(1.0) as u32, h)
        }
    };
    (size.0 <= MAX_SIDE && size.1 <= MAX_SIDE).then_some(size)
}

fn render_svg(bytes: &[u8], height: Option<u32>) -> Result<Bitmap, ImageError> {
    use resvg::{tiny_skia, usvg};

    let mut options = usvg::Options::default();
    // Images referenced by path are not loaded; embedded (data URL) images are.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree =
        usvg::Tree::from_data(bytes, &options).map_err(|e| ImageError::Invalid(e.to_string()))?;
    let size = tree.size();
    let (own_width, own_height) = (size.width().ceil() as u32, size.height().ceil() as u32);
    let (width, height) = scaled_size(own_width, own_height, height).ok_or(ImageError::TooLarge)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(ImageError::TooLarge)?;
    let transform = tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Bitmap::from_premultiplied(width, height, pixmap.take()).ok_or(ImageError::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8">
        <rect width="8" height="8" fill="#FF0000"/></svg>"##;

    fn pixel(b: &Bitmap, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * b.width() + x) * 4) as usize;
        b.pixels()[i..i + 4].try_into().expect("4 bytes")
    }

    #[test]
    fn premultiplies_and_back() {
        let b = Bitmap::from_rgba(2, 1, vec![255, 128, 0, 128, 10, 20, 30, 0]).expect("valid");
        assert_eq!(pixel(&b, 0, 0), [128, 64, 0, 128]);
        assert_eq!(pixel(&b, 1, 0), [0, 0, 0, 0]);
        assert_eq!(&b.to_rgba()[..4], &[255, 128, 0, 128]);
        assert!(Bitmap::from_rgba(2, 2, vec![0; 8]).is_none());
        assert!(Bitmap::from_rgba(0, 0, Vec::new()).is_none());
    }

    #[test]
    fn tint_uses_luminance_and_keeps_alpha() {
        let white = Bitmap::from_rgba(1, 1, vec![255, 255, 255, 255]).expect("valid");
        let tint = Color::rgba(0x40, 0x80, 0xFF, 1.0);
        assert_eq!(pixel(&white.tinted(tint), 0, 0), [0x40, 0x80, 0xFF, 255]);

        let red = Bitmap::from_rgba(1, 1, vec![255, 0, 0, 255]).expect("valid");
        // L = 0.299 → 0.299 × 255.
        assert_eq!(pixel(&red.tinted(Color::WHITE), 0, 0), [76, 76, 76, 255]);

        // Half-transparent white stays half-transparent, premultiplied.
        let half = Bitmap::from_rgba(1, 1, vec![255, 255, 255, 128]).expect("valid");
        assert_eq!(
            pixel(&half.tinted(Color::WHITE), 0, 0),
            [128, 128, 128, 128]
        );
    }

    #[test]
    fn sniffs_formats() {
        assert_eq!(ImageFormat::sniff(b"\x89PNG\r\n"), Some(ImageFormat::Png));
        assert_eq!(
            ImageFormat::sniff(b"\xFF\xD8\xFF\xE0"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::sniff(b"GIF89a"), Some(ImageFormat::Gif));
        assert_eq!(ImageFormat::sniff(b"BM...."), Some(ImageFormat::Bmp));
        assert_eq!(ImageFormat::sniff(b"\0\0\x01\0"), Some(ImageFormat::Ico));
        assert_eq!(
            ImageFormat::sniff(b"\xEF\xBB\xBF  <?xml"),
            Some(ImageFormat::Svg)
        );
        assert_eq!(ImageFormat::sniff(b"\x1F\x8B"), Some(ImageFormat::Svg));
        assert_eq!(ImageFormat::sniff(b"RIFF....WEBP"), None);
        assert_eq!(ImageFormat::sniff(b""), None);
        assert!(matches!(
            decode_image(b"hello", None),
            Err(ImageError::Unsupported)
        ));
    }

    #[test]
    fn rasterises_svg_at_the_requested_height() {
        let own = decode_image(SQUARE.as_bytes(), None).expect("decoded");
        assert_eq!((own.width(), own.height()), (16, 8));
        assert_eq!(pixel(&own, 2, 2), [255, 0, 0, 255]);
        assert_eq!(pixel(&own, 12, 2), [0, 0, 0, 0]);

        let big = decode_image(SQUARE.as_bytes(), Some(24)).expect("decoded");
        assert_eq!((big.width(), big.height()), (48, 24));
        assert_eq!(pixel(&big, 20, 20), [255, 0, 0, 255]);
        assert_eq!(pixel(&big, 30, 20), [0, 0, 0, 0]);

        assert!(matches!(
            decode_image(b"<svg", None),
            Err(ImageError::Invalid(_))
        ));
        assert!(matches!(
            decode_image(SQUARE.as_bytes(), Some(100_000)),
            Err(ImageError::TooLarge)
        ));
    }

    #[test]
    fn scaled_sizes() {
        assert_eq!(scaled_size(100, 50, None), Some((100, 50)));
        assert_eq!(scaled_size(100, 50, Some(27)), Some((54, 27)));
        assert_eq!(scaled_size(1, 1000, Some(10)), Some((1, 10)));
        assert_eq!(scaled_size(0, 10, None), None);
        assert_eq!(scaled_size(100_000, 10, Some(10)), None);
        assert_eq!(scaled_size(9000, 10, None), None);
    }

    #[test]
    fn decodes_the_app_icon() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/app-icon.svg");
        let icon = load_image(&path, Some(32)).expect("decoded");
        assert_eq!(icon.height(), 32);
        assert!(
            icon.pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|px| px[3] == 255)
        );
    }
}
