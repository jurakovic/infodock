//! The classic bar's background (SPEC §12.3–12.4): a skin drawn into a bitmap of the bar's
//! physical size, which the bar draws with the background opacity, so the opacity applies to the
//! skin only. Built-in skins are drawn from the base colour; skin packs and the custom image are
//! decoded, cut, tinted, scaled to the bar's height and tiled across it, and a pack's shadow band
//! is drawn the same way for the side facing the screen centre. The result is kept until the
//! skin, its colours, the bar's size or its edge change.

mod builtin;
pub mod pack;
mod raster;

pub use builtin::BuiltIn;

use std::path::{Path, PathBuf};

use crate::config::model::Edge;
use crate::platform::{Bitmap, Color, SizeF};
use pack::{Pack, PackError, TileLayout};

/// The skin used when the configured one is not on this machine (SPEC §12.4).
pub const FALLBACK: BuiltIn = BuiltIn::Gloss;
/// `classic.skin` for the user's own image, `classic.customSkin`.
pub const CUSTOM: &str = "custom";

/// The classic bar's skin as the settings choose it (SPEC §10.3).
#[derive(Debug, Clone, PartialEq)]
pub struct SkinSpec {
    /// `classic.skin`: a built-in skin's id, [`CUSTOM`] or a pack's folder name.
    pub id: String,
    /// `classic.customSkin`.
    pub custom_image: String,
    /// The built-in skins' base colour: `background.color`, or the theme's; opaque.
    pub base: Color,
    /// What packs and the custom image are tinted with (SPEC §12.3): the background colour the
    /// user set in custom colour mode. With the theme's colour they keep their own.
    pub tint: Option<Color>,
}

/// What a skin is drawn for: the bar's size in physical px, its scale and the edge it is
/// placed on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub edge: Edge,
}

impl Target {
    /// A bar of `size` logical px at `scale`, in the whole physical px its window has.
    pub fn of(size: SizeF, scale: f32, edge: Edge) -> Self {
        Self {
            width: (size.width * scale).round() as u32,
            height: (size.height * scale).round() as u32,
            scale,
            edge,
        }
    }
}

/// A drawn skin.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    /// The bar's background, at the bar's size.
    pub bar: Bitmap,
    /// A pack's shadow band, as wide as the bar, for the side facing the screen centre: its
    /// first row touches a top bar, its last row a bottom bar.
    pub band: Option<Bitmap>,
}

/// The skin `spec` chooses, drawn for `target` with packs from `dir`; `None` for an empty bar.
/// A pack that is not there or cannot be used gives Gloss, a custom image Flat (SPEC §12.4).
pub fn render(dir: &Path, spec: &SkinSpec, target: Target) -> Option<Rendered> {
    render_with(dir, spec, target, &platform_decode)
}

/// Decodes a pack's tile or the custom image with the platform (WIC on Windows).
fn platform_decode(bytes: &[u8]) -> Option<Bitmap> {
    crate::platform::decode_image(bytes, None).ok()
}

/// [`render`] with this image decoder.
fn render_with(
    dir: &Path,
    spec: &SkinSpec,
    target: Target,
    decode: &dyn Fn(&[u8]) -> Option<Bitmap>,
) -> Option<Rendered> {
    let built_in = |skin: BuiltIn| {
        skin.render(spec.base, target)
            .map(|bar| Rendered { bar, band: None })
    };
    if let Some(skin) = BuiltIn::from_id(&spec.id) {
        return built_in(skin);
    }
    if spec.id == CUSTOM {
        if spec.custom_image.is_empty() {
            return built_in(BuiltIn::Flat);
        }
        return match draw_custom(spec, target, decode) {
            Ok(rendered) => rendered,
            Err(e) => {
                tracing::warn!("custom skin {:?}: {}", spec.custom_image, e.message());
                built_in(BuiltIn::Flat)
            }
        };
    }
    let drawn = match pack::load(dir, &spec.id) {
        None => {
            tracing::info!("skin {:?} is not installed; drawing Gloss", spec.id);
            None
        }
        Some(Err(e)) => {
            tracing::warn!("skin pack {:?}: {}", spec.id, e.message());
            None
        }
        Some(Ok(pack)) => draw_pack(&pack, spec, target, decode)
            .map_err(|e| tracing::warn!("skin pack {:?}: {}", spec.id, e.message()))
            .ok(),
    };
    match drawn {
        Some(rendered) => rendered,
        None => built_in(FALLBACK),
    }
}

fn draw_pack(
    pack: &Pack,
    spec: &SkinSpec,
    target: Target,
    decode: &dyn Fn(&[u8]) -> Option<Bitmap>,
) -> Result<Option<Rendered>, PackError> {
    let (name, layout) = pack.tile(target.edge);
    // Checked again: the files may have changed since the pack was listed.
    let bytes = pack::check_file(&pack.dir, &name, pack.tile_height(), pack.shadow_band)?;
    let tile = decode(&bytes).ok_or(PackError::Format(name))?;
    Ok(draw_tile(
        &tile,
        pack.shadow_band,
        layout,
        spec.tint,
        target,
    ))
}

/// The custom image: tiled like a pack's tile without a band (SPEC §12.4).
fn draw_custom(
    spec: &SkinSpec,
    target: Target,
    decode: &dyn Fn(&[u8]) -> Option<Bitmap>,
) -> Result<Option<Rendered>, PackError> {
    let name = &spec.custom_image;
    let bytes = pack::read_image(Path::new(name), name)?;
    pack::check_image(name, &bytes, None)?;
    let tile = decode(&bytes).ok_or_else(|| PackError::Format(name.clone()))?;
    let layout = TileLayout {
        band_first: true,
        mirror_band: false,
    };
    Ok(draw_tile(&tile, 0, layout, spec.tint, target))
}

/// Cuts `tile` into its bar part and its `band` rows, tints both, scales them (the bar part to
/// the bar's height, the band by the bar's scale; widths by the scale, never stretched) and
/// tiles them across the bar.
fn draw_tile(
    tile: &Bitmap,
    band: u32,
    layout: TileLayout,
    tint: Option<Color>,
    target: Target,
) -> Option<Rendered> {
    let bar_rows = tile.height().checked_sub(band).filter(|r| *r > 0)?;
    let (bar_top, band_top) = if layout.band_first {
        (band, 0)
    } else {
        (0, bar_rows)
    };
    let tinted = |b: Bitmap| match tint {
        Some(t) => b.tinted(t),
        None => b,
    };
    let tile_width = ((tile.width() as f32 * target.scale).round() as u32).max(1);
    let bar = tinted(raster::rows(tile, bar_top, bar_rows)?);
    let bar = raster::tiled(
        &raster::resized(&bar, tile_width, target.height)?,
        target.width,
    )?;
    let band = (band > 0)
        .then(|| raster::rows(tile, band_top, band))
        .flatten()
        .map(|b| {
            if layout.mirror_band {
                raster::flipped(&b)
            } else {
                b
            }
        })
        .and_then(|b| {
            let height = ((band as f32 * target.scale).round() as u32).max(1);
            raster::tiled(
                &raster::resized(&tinted(b), tile_width, height)?,
                target.width,
            )
        });
    Some(Rendered { bar, band })
}

/// The bar's drawn skin, kept between frames.
pub struct SkinCache {
    /// The skins folder (SPEC §11.1).
    dir: PathBuf,
    decode: fn(&[u8]) -> Option<Bitmap>,
    drawn: Option<(SkinSpec, Target, Option<Rendered>)>,
}

impl SkinCache {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            decode: platform_decode,
            drawn: None,
        }
    }

    /// With another image decoder, for tests on every OS.
    #[cfg(test)]
    pub fn with_decoder(dir: PathBuf, decode: fn(&[u8]) -> Option<Bitmap>) -> Self {
        Self {
            decode,
            ..Self::new(dir)
        }
    }

    /// The skin for `spec` and `target`, drawn again only when either changed.
    pub fn get(&mut self, spec: &SkinSpec, target: Target) -> Option<&Rendered> {
        let fresh = matches!(&self.drawn, Some((s, t, _)) if s == spec && *t == target);
        if !fresh {
            let rendered = render_with(&self.dir, spec, target, &self.decode);
            self.drawn = Some((spec.clone(), target, rendered));
        }
        self.drawn.as_ref().and_then(|(_, _, r)| r.as_ref())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use super::*;
    pub(crate) use pack::tests::pack;
    use pack::tests::png;

    const BASE: Color = Color::rgba(0x2A, 0x2A, 0x2E, 1.0);

    fn spec(id: &str) -> SkinSpec {
        SkinSpec {
            id: id.into(),
            custom_image: String::new(),
            base: BASE,
            tint: None,
        }
    }

    fn target(width: u32, height: u32, scale: f32, edge: Edge) -> Target {
        Target {
            width,
            height,
            scale,
            edge,
        }
    }

    fn built_in(skin: BuiltIn, t: Target) -> Option<Rendered> {
        skin.render(BASE, t).map(|bar| Rendered { bar, band: None })
    }

    /// Decodes the PNGs the tests write, as the platform would.
    pub(crate) fn decode_png(bytes: &[u8]) -> Option<Bitmap> {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .ok()?;
        let mut buf = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        buf.truncate(info.line_size * info.height as usize);
        Bitmap::from_rgba(info.width, info.height, buf)
    }

    /// Writes rows of opaque grey, `values[y]` for row `y`, `width` px wide.
    pub(crate) fn rows_png(path: &Path, width: u32, values: &[u8]) {
        let rgba: Vec<u8> = values
            .iter()
            .flat_map(|&v| [v, v, v, 255].repeat(width as usize))
            .collect();
        crate::test_util::write_png(path, width, values.len() as u32, &rgba);
    }

    /// The red channel of column 0, top to bottom.
    fn column(b: &Bitmap) -> Vec<u8> {
        (0..b.height())
            .map(|y| b.pixels()[(y * b.width() * 4) as usize])
            .collect()
    }

    #[test]
    fn target_is_the_window_in_physical_px() {
        let t = Target::of(SizeF::new(800.0, 25.0), 1.5, Edge::Top);
        assert_eq!((t.width, t.height), (1200, 38));
    }

    #[test]
    fn built_in_skins_and_fallbacks() {
        let dir = tempfile::tempdir().expect("temp dir");
        let t = target(20, 25, 1.0, Edge::Bottom);
        let draw = |s: &SkinSpec| render_with(dir.path(), s, t, &decode_png);
        assert_eq!(draw(&spec("gloss")), built_in(BuiltIn::Gloss, t));
        assert_eq!(draw(&spec("carbon")), built_in(BuiltIn::Carbon, t));
        // A pack that is not installed, or not valid: Gloss.
        assert_eq!(draw(&spec("my-pack")), built_in(BuiltIn::Gloss, t));
        pack(dir.path(), "broken", "{");
        assert_eq!(draw(&spec("broken")), built_in(BuiltIn::Gloss, t));
        // The custom image without a file, or a missing one: Flat.
        assert_eq!(draw(&spec(CUSTOM)), built_in(BuiltIn::Flat, t));
        let missing = SkinSpec {
            custom_image: dir.path().join("nope.png").display().to_string(),
            ..spec(CUSTOM)
        };
        assert_eq!(draw(&missing), built_in(BuiltIn::Flat, t));
        // An image the decoder cannot read: Gloss for a pack.
        let folder = pack(dir.path(), "odd", r#"{ "image": "t.png" }"#);
        png(&folder.join("t.png"), 4, 25, [1, 2, 3]);
        assert_eq!(
            render_with(dir.path(), &spec("odd"), t, &|_| None),
            built_in(BuiltIn::Gloss, t)
        );
    }

    #[test]
    fn pack_is_cut_scaled_and_tiled() {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = pack(
            dir.path(),
            "stripes",
            r#"{ "image": "t.png", "height": 4, "shadowBand": 2 }"#,
        );
        // Band rows 10, 20 on top; bar rows 100, 100, 200, 200.
        rows_png(&folder.join("t.png"), 3, &[10, 20, 100, 100, 200, 200]);
        let bottom = render_with(
            dir.path(),
            &spec("stripes"),
            target(7, 4, 1.0, Edge::Bottom),
            &decode_png,
        )
        .expect("drawn");
        assert_eq!((bottom.bar.width(), bottom.bar.height()), (7, 4));
        assert_eq!(column(&bottom.bar), [100, 100, 200, 200]);
        let band = bottom.band.expect("a band");
        assert_eq!((band.width(), band.height()), (7, 2));
        // Above a bottom bar the band is as drawn: its last row touches the bar.
        assert_eq!(column(&band), [10, 20]);

        // A top bar with the shared image: the bar part as is, the band mirrored below it.
        let top = render_with(
            dir.path(),
            &spec("stripes"),
            target(7, 4, 1.0, Edge::Top),
            &decode_png,
        )
        .expect("drawn");
        assert_eq!(column(&top.bar), [100, 100, 200, 200]);
        assert_eq!(column(&top.band.expect("a band")), [20, 10]);

        // At 200 % with the bar at 6 px (tiles drawn for 4): the bar part is scaled to 12 px,
        // the band keeps its height at that scale, tiles are twice as wide.
        let large = render_with(
            dir.path(),
            &spec("stripes"),
            target(13, 12, 2.0, Edge::Bottom),
            &decode_png,
        )
        .expect("drawn");
        assert_eq!(large.bar.height(), 12);
        let col = column(&large.bar);
        assert_eq!((col[0], col[11]), (100, 200));
        assert_eq!(large.band.expect("a band").height(), 4);
    }

    #[test]
    fn edge_tiles_are_used_for_their_edge() {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = pack(
            dir.path(),
            "two",
            r#"{ "imageTop": "top.png", "imageBottom": "bottom.png", "height": 2, "shadowBand": 1 }"#,
        );
        // The top tile has its band at the bottom.
        rows_png(&folder.join("top.png"), 2, &[50, 60, 5]);
        rows_png(&folder.join("bottom.png"), 2, &[7, 70, 80]);
        let draw = |edge| {
            render_with(
                dir.path(),
                &spec("two"),
                target(4, 2, 1.0, edge),
                &decode_png,
            )
            .expect("drawn")
        };
        let top = draw(Edge::Top);
        assert_eq!(column(&top.bar), [50, 60]);
        assert_eq!(column(&top.band.expect("band")), [5]);
        let bottom = draw(Edge::Bottom);
        assert_eq!(column(&bottom.bar), [70, 80]);
        assert_eq!(column(&bottom.band.expect("band")), [7]);
    }

    #[test]
    fn packs_and_the_custom_image_take_the_tint() {
        let dir = tempfile::tempdir().expect("temp dir");
        let image = dir.path().join("wood.png");
        rows_png(&image, 5, &[255; 10]);
        let custom = SkinSpec {
            custom_image: image.display().to_string(),
            tint: Some(Color::rgba(0x3A, 0x6E, 0xA5, 1.0)),
            ..spec(CUSTOM)
        };
        let t = target(12, 25, 1.0, Edge::Bottom);
        let drawn = render_with(dir.path(), &custom, t, &decode_png).expect("drawn");
        // White tinted is the tint; no band; scaled from 10 rows to the bar's 25.
        assert_eq!(&drawn.bar.pixels()[..4], &[0x3A, 0x6E, 0xA5, 255]);
        assert_eq!((drawn.bar.width(), drawn.bar.height()), (12, 25));
        assert_eq!(drawn.band, None);
        // Without a tint the image keeps its colours.
        let plain = SkinSpec {
            tint: None,
            ..custom
        };
        let drawn = render_with(dir.path(), &plain, t, &decode_png).expect("drawn");
        assert_eq!(&drawn.bar.pixels()[..4], &[255, 255, 255, 255]);
        // The custom image is checked like a pack's tiles.
        fs::write(&image, b"GIF89a").expect("written");
        assert_eq!(
            render_with(dir.path(), &plain, t, &decode_png),
            built_in(BuiltIn::Flat, t)
        );
    }

    /// Packs decode through the platform: WIC on Windows (SPEC §16).
    #[cfg(windows)]
    #[test]
    #[ignore = "decodes with WIC; run with --ignored"]
    fn packs_decode_with_the_platform() {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = pack(
            dir.path(),
            "stripes",
            r#"{ "image": "t.png", "height": 4, "shadowBand": 2 }"#,
        );
        rows_png(&folder.join("t.png"), 3, &[10, 20, 100, 100, 200, 200]);
        let t = target(7, 4, 1.0, Edge::Bottom);
        let drawn = render(dir.path(), &spec("stripes"), t).expect("drawn");
        assert_eq!(column(&drawn.bar), [100, 100, 200, 200]);
        assert_eq!(column(&drawn.band.expect("a band")), [10, 20]);
    }

    #[test]
    fn cache_draws_again_only_after_a_change() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut cache = SkinCache::new(dir.path().into());
        let t = target(20, 25, 1.0, Edge::Bottom);
        let first = cache.get(&spec("gloss"), t).cloned().expect("drawn");
        let again = cache.get(&spec("gloss"), t).cloned().expect("drawn");
        // The very same pixels, so the renderer's upload cache keeps its copy.
        assert!(std::sync::Arc::ptr_eq(
            first.bar.shared_pixels(),
            again.bar.shared_pixels()
        ));
        let wider = Target { width: 30, ..t };
        let gloss = cache.get(&spec("gloss"), wider).map(|r| r.bar.width());
        assert_eq!(gloss, Some(30));
        let flat = cache.get(&spec("flat"), wider).cloned();
        assert_eq!(flat, built_in(BuiltIn::Flat, wider));
        let empty = Target { width: 0, ..t };
        assert!(cache.get(&spec("flat"), empty).is_none());
    }
}
