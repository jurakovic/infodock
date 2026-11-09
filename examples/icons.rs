//! Renders the raster artwork from its SVG sources (SPEC Appendix C). Run it from the repository
//! after changing one of the SVGs, and commit what it writes:
//!
//! ```text
//! cargo run --example icons
//! ```
//!
//! * `icons/icon.ico` and the PNGs beside it: the app icon. Each frame keeps a transparent margin
//!   of size / 16 whole pixels (rounded down) and draws the tile, the SVG's square from 16 to 240,
//!   into the rest, so no edge of the tile falls between pixels; 16–24 px take
//!   `app-icon-small.svg`.
//! * `icons/tray-dark.ico`, `icons/tray-light.ico`: the tray icons for a dark and a light taskbar.
//! * `icons/installer/header.bmp`, `sidebar.bmp`: the installer's images, 24-bit as NSIS takes
//!   them.
//!
//! The SVGs are drawn with resvg, as the app draws its own, at up to 16 times the size and then
//! averaged down, so each pixel takes its exact coverage (tiny-skia antialiases in quarter
//! pixels). The tests check that the committed files are what the SVGs give.

use std::error::Error;
use std::path::{Path, PathBuf};

use resvg::tiny_skia::{Pixmap, Rect, Transform};
use resvg::usvg;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The app icon's `.ico` frames (Appendix C).
const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];
/// The largest frame drawn from `app-icon-small.svg`.
const SMALL_ART_UP_TO: u32 = 24;
/// The app icon as PNG: the Settings window's icon (`ui/settings.slint`), the bar specimen's
/// raster image (`bar/paint.rs`) and the bundler's sizes.
const PNGS: [(&str, u32); 4] = [
    ("32x32.png", 32),
    ("128x128.png", 128),
    ("128x128@2x.png", 256),
    ("icon.png", 512),
];
/// The tray icon's frames: the small icon at 100–200 % (Appendix C).
const TRAY_SIZES: [u32; 4] = [16, 20, 24, 32];
const TRAY_VARIANTS: [&str; 2] = ["dark", "light"];
const INSTALLER_IMAGES: [&str; 2] = ["header", "sidebar"];
/// The supersampled canvas is at most this many px on its longer side.
const MAX_CANVAS: u32 = 1024;

fn main() -> Result<()> {
    for (path, bytes) in outputs()? {
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn icons() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("icons")
}

/// Every file the script writes, with its contents.
fn outputs() -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let icons = icons();
    let mut files = Vec::new();
    let large = load(&icons.join("app-icon.svg"))?;
    let small = load(&icons.join("app-icon-small.svg"))?;
    let frames = ICO_SIZES
        .iter()
        .map(|&size| {
            app_icon(
                if size <= SMALL_ART_UP_TO {
                    &small
                } else {
                    &large
                },
                size,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    files.push((icons.join("icon.ico"), ico(&frames)?));
    for (name, size) in PNGS {
        files.push((icons.join(name), app_icon(&large, size)?.encode_png()?));
    }
    for variant in TRAY_VARIANTS {
        let tree = load(&icons.join(format!("tray-{variant}.svg")))?;
        let frames = TRAY_SIZES
            .iter()
            .map(|&size| whole(&tree, size, size))
            .collect::<Result<Vec<_>>>()?;
        files.push((icons.join(format!("tray-{variant}.ico")), ico(&frames)?));
    }
    for name in INSTALLER_IMAGES {
        let installer = icons.join("installer");
        let tree = load(&installer.join(format!("{name}.svg")))?;
        let size = tree.size().to_int_size();
        let image = whole(&tree, size.width(), size.height())?;
        files.push((installer.join(format!("{name}.bmp")), bmp(&image)));
    }
    Ok(files)
}

/// Parses an SVG; `<image>` paths in it are relative to its folder.
fn load(path: &Path) -> Result<usvg::Tree> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let options = usvg::Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        ..usvg::Options::default()
    };
    Ok(usvg::Tree::from_data(&data, &options)?)
}

/// The app icon's frame: the tile inside a margin of `size / 16` whole px.
fn app_icon(tree: &usvg::Tree, size: u32) -> Result<Pixmap> {
    let margin = (size / 16) as f32;
    let inner = size as f32 - 2.0 * margin;
    let tile = rect(16.0, 16.0, 224.0, 224.0)?;
    render(tree, tile, rect(margin, margin, inner, inner)?, size, size)
}

/// The whole SVG at `width` × `height` px.
fn whole(tree: &usvg::Tree, width: u32, height: u32) -> Result<Pixmap> {
    let size = tree.size();
    let area = rect(0.0, 0.0, size.width(), size.height())?;
    render(
        tree,
        area,
        rect(0.0, 0.0, width as f32, height as f32)?,
        width,
        height,
    )
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Result<Rect> {
    Rect::from_xywh(x, y, width, height).ok_or_else(|| "an empty rectangle".into())
}

/// Draws `area` of the SVG (in its units) into `dest` (px) of a transparent `width` × `height`
/// image, supersampled.
fn render(tree: &usvg::Tree, area: Rect, dest: Rect, width: u32, height: u32) -> Result<Pixmap> {
    let factor = (MAX_CANVAS / width.max(height)).clamp(1, 16);
    let f = factor as f32;
    let (sx, sy) = (
        dest.width() / area.width() * f,
        dest.height() / area.height() * f,
    );
    let transform = Transform::from_row(
        sx,
        0.0,
        0.0,
        sy,
        dest.x() * f - area.x() * sx,
        dest.y() * f - area.y() * sy,
    );
    let mut canvas = Pixmap::new(width * factor, height * factor).ok_or("an empty image")?;
    resvg::render(tree, transform, &mut canvas.as_mut());
    downsample(&canvas, factor)
}

/// Averages each `factor` × `factor` block. The pixels are premultiplied, so this gives each
/// pixel's coverage and its colour together.
fn downsample(canvas: &Pixmap, factor: u32) -> Result<Pixmap> {
    let (width, height) = (canvas.width() / factor, canvas.height() / factor);
    let mut out = Pixmap::new(width, height).ok_or("an empty image")?;
    let source = canvas.data();
    let count = factor * factor;
    let stride = canvas.width() as usize * 4;
    for (i, pixel) in out.data_mut().chunks_exact_mut(4).enumerate() {
        let (x, y) = (i as u32 % width, i as u32 / width);
        let mut sum = [0u32; 4];
        for dy in 0..factor {
            let row = (y * factor + dy) as usize * stride;
            let start = row + (x * factor) as usize * 4;
            for block in source[start..start + factor as usize * 4].chunks_exact(4) {
                for (total, &channel) in sum.iter_mut().zip(block) {
                    *total += u32::from(channel);
                }
            }
        }
        for (channel, total) in pixel.iter_mut().zip(sum) {
            *channel = ((total + count / 2) / count) as u8;
        }
    }
    Ok(out)
}

/// An `.ico` with a 32-bit PNG per frame, as Windows reads them since Vista.
fn ico(frames: &[Pixmap]) -> Result<Vec<u8>> {
    let pngs = frames
        .iter()
        .map(Pixmap::encode_png)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let count = u16::try_from(frames.len())?;
    let mut out = Vec::new();
    // ICONDIR: reserved, type 1 (icon), the number of images.
    for value in [0, 1, count] {
        out.extend(value.to_le_bytes());
    }
    let mut offset = 6 + 16 * frames.len();
    for (frame, png) in frames.iter().zip(&pngs) {
        // ICONDIRENTRY: 0 stands for 256 px.
        let side = |px: u32| u8::try_from(px).unwrap_or(0);
        out.extend([side(frame.width()), side(frame.height()), 0, 0]);
        // Planes, bits per pixel.
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend(u32::try_from(png.len())?.to_le_bytes());
        out.extend(u32::try_from(offset)?.to_le_bytes());
        offset += png.len();
    }
    for png in pngs {
        out.extend(png);
    }
    Ok(out)
}

/// A 24-bit bottom-up BMP at 96 DPI, over white.
fn bmp(image: &Pixmap) -> Vec<u8> {
    const HEADERS: u32 = 14 + 40;
    // 96 DPI in px per metre.
    const DPI_96: u32 = 3780;
    let (width, height) = (image.width(), image.height());
    let row = (width * 3).div_ceil(4) * 4;
    let pixels = row * height;
    let mut out = Vec::with_capacity((HEADERS + pixels) as usize);
    // BITMAPFILEHEADER.
    out.extend(b"BM");
    out.extend((HEADERS + pixels).to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(HEADERS.to_le_bytes());
    // BITMAPINFOHEADER: size, width, height (positive: bottom-up), planes, 24 bits, BI_RGB.
    for value in [40, width, height] {
        out.extend(value.to_le_bytes());
    }
    out.extend(1u16.to_le_bytes());
    out.extend(24u16.to_le_bytes());
    for value in [0, pixels, DPI_96, DPI_96, 0, 0] {
        out.extend(value.to_le_bytes());
    }
    for y in (0..height).rev() {
        let start = out.len();
        for x in 0..width {
            if let Some(pixel) = image.pixel(x, y) {
                // Premultiplied, so over white is the colour plus what the alpha leaves.
                let over = |channel: u8| channel.saturating_add(255 - pixel.alpha());
                out.extend([over(pixel.blue()), over(pixel.green()), over(pixel.red())]);
            }
        }
        out.resize(start + row as usize, 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The images in an `.ico`, decoded.
    fn frames(ico: &[u8]) -> Vec<Pixmap> {
        let word = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]) as usize;
        let dword = |at: usize| {
            u32::from_le_bytes([ico[at], ico[at + 1], ico[at + 2], ico[at + 3]]) as usize
        };
        assert_eq!((word(0), word(2)), (0, 1), "an icon");
        (0..word(4))
            .map(|i| {
                let entry = 6 + 16 * i;
                let (size, offset) = (dword(entry + 8), dword(entry + 12));
                Pixmap::decode_png(&ico[offset..offset + size]).expect("a PNG frame")
            })
            .collect()
    }

    fn sizes(frames: &[Pixmap]) -> Vec<u32> {
        frames.iter().map(Pixmap::width).collect()
    }

    /// The rows and columns that hold anything: (left, top, right, bottom), exclusive.
    fn drawn(image: &Pixmap) -> (u32, u32, u32, u32) {
        let mut bounds = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..image.height() {
            for x in 0..image.width() {
                if image.pixel(x, y).is_some_and(|p| p.alpha() > 0) {
                    bounds = (
                        bounds.0.min(x),
                        bounds.1.min(y),
                        bounds.2.max(x + 1),
                        bounds.3.max(y + 1),
                    );
                }
            }
        }
        bounds
    }

    fn read(path: &Path) -> Vec<u8> {
        std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn the_app_icon_has_every_size_with_a_whole_pixel_margin() {
        let frames = frames(&read(&icons().join("icon.ico")));
        assert_eq!(sizes(&frames), ICO_SIZES);
        for frame in &frames {
            let (size, margin) = (frame.width(), frame.width() / 16);
            assert_eq!(frame.height(), size);
            assert_eq!(drawn(frame), (margin, margin, size - margin, size - margin));
            // The tile is opaque up to its edge: its corner pixel is whole.
            let corner = frame.pixel(margin, margin).map(|p| p.alpha());
            assert_eq!(corner, Some(255), "{size} px");
        }
    }

    #[test]
    fn the_tray_icons_have_the_small_sizes() {
        for variant in TRAY_VARIANTS {
            let frames = frames(&read(&icons().join(format!("tray-{variant}.ico"))));
            assert_eq!(sizes(&frames), TRAY_SIZES, "{variant}");
        }
    }

    /// Each committed file holds what the SVGs give, to one level per channel (PNG and BMP
    /// round their values): an SVG changed without running the script fails here.
    #[test]
    fn the_committed_files_are_rendered_from_the_svgs() {
        for (path, expected) in outputs().expect("the SVGs render") {
            let committed = read(&path);
            let name = path.display();
            match path.extension().and_then(|e| e.to_str()) {
                Some("ico") => {
                    let (now, then) = (frames(&expected), frames(&committed));
                    assert_eq!(sizes(&now), sizes(&then), "{name}");
                    for (a, b) in now.iter().zip(&then) {
                        assert!(close(a.data(), b.data()), "{name}, {} px", a.width());
                    }
                }
                Some("png") => {
                    let then = Pixmap::decode_png(&committed).expect("a PNG");
                    let now = Pixmap::decode_png(&expected).expect("a PNG");
                    assert_eq!(now.width(), then.width(), "{name}");
                    assert!(close(now.data(), then.data()), "{name}");
                }
                _ => {
                    assert_eq!(expected.len(), committed.len(), "{name}");
                    assert!(close(&expected, &committed), "{name}");
                }
            }
        }
    }

    fn close(a: &[u8], b: &[u8]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 1)
    }

    /// CI's screenshots (`target/screenshots`): each tray icon's frames on its taskbar's colour,
    /// as they are and four times as large, and the installer's images side by side.
    #[test]
    #[ignore = "writes target/screenshots"]
    fn render_artwork_sheets() {
        use resvg::tiny_skia::{Color, FilterQuality, PixmapPaint};
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screenshots");
        std::fs::create_dir_all(&out).expect("the screenshots folder");
        let paste = |sheet: &mut Pixmap, image: &Pixmap, x: u32, y: u32, zoom: f32| {
            let paint = PixmapPaint {
                quality: FilterQuality::Nearest,
                ..PixmapPaint::default()
            };
            let at = Transform::from_row(zoom, 0.0, 0.0, zoom, x as f32, y as f32);
            sheet.draw_pixmap(0, 0, image.as_ref(), &paint, at, None);
        };
        // Windows 11's taskbar colours, without Mica.
        for (variant, taskbar) in [("dark", (0x20, 0x20, 0x20)), ("light", (0xEE, 0xEE, 0xEE))] {
            let frames = frames(&read(&icons().join(format!("tray-{variant}.ico"))));
            let width = 12 + frames.iter().map(|f| f.width() * 4 + 12).sum::<u32>();
            let mut sheet = Pixmap::new(width, 12 + 32 + 12 + 32 * 4 + 12).expect("a sheet");
            sheet.fill(Color::from_rgba8(taskbar.0, taskbar.1, taskbar.2, 255));
            let mut x = 12;
            for frame in &frames {
                paste(&mut sheet, frame, x, 12, 1.0);
                paste(&mut sheet, frame, x, 56, 4.0);
                x += frame.width() * 4 + 12;
            }
            let path = out.join(format!("tray-{variant}.png"));
            sheet.save_png(&path).expect("the sheet saved");
        }
        let installer = icons().join("installer");
        let images = INSTALLER_IMAGES.map(|name| {
            let tree = load(&installer.join(format!("{name}.svg"))).expect("the SVG");
            let size = tree.size().to_int_size();
            whole(&tree, size.width(), size.height()).expect("rendered")
        });
        let [header, sidebar] = &images;
        let mut sheet = Pixmap::new(sidebar.width() + header.width() + 36, sidebar.height() + 24)
            .expect("a sheet");
        sheet.fill(Color::from_rgba8(0xF0, 0xF0, 0xF0, 255));
        paste(&mut sheet, sidebar, 12, 12, 1.0);
        paste(&mut sheet, header, sidebar.width() + 24, 12, 1.0);
        sheet
            .save_png(out.join("installer.png"))
            .expect("the sheet saved");
    }

    #[test]
    fn the_installer_images_have_the_sizes_nsis_takes() {
        let installer = icons().join("installer");
        for (name, size) in [("header", (150, 57)), ("sidebar", (164, 314))] {
            let bytes = read(&installer.join(format!("{name}.bmp")));
            let dword = |at: usize| {
                u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
            };
            assert_eq!(&bytes[..2], b"BM");
            assert_eq!((dword(18), dword(22)), size, "{name}");
            assert_eq!(u16::from_le_bytes([bytes[28], bytes[29]]), 24, "{name}");
        }
    }
}
