//! The bar's shadow (SPEC §5.5): a soft band beside the bar on the side facing the screen centre,
//! shown in the shadow band's window like a skin's band (§5.1) when the skin has none.

use crate::platform::Bitmap;

/// How far the shadow reaches from the bar, in logical px.
const DEPTH: f32 = 6.0;
/// Its alpha next to the bar; it falls to nothing with the square of the distance.
const ALPHA: f32 = 0.18;

/// The shadow for a bar `width` physical px wide at `scale`, as wide as the bar and [`DEPTH`]
/// logical px deep in whole physical px. `above`: it goes above the bar (one at the bottom of
/// the screen), so its last row touches the bar; otherwise its first row does.
pub fn band(width: u32, scale: f32, above: bool) -> Option<Bitmap> {
    let depth = (DEPTH * scale).round().max(1.0) as u32;
    let mut pixels = Vec::with_capacity(width as usize * depth as usize * 4);
    for row in 0..depth {
        // Counted from the bar.
        let from_bar = if above { depth - 1 - row } else { row };
        let alpha = alpha(from_bar, depth);
        // Black, premultiplied: only the alpha is set.
        for _ in 0..width {
            pixels.extend_from_slice(&[0, 0, 0, alpha]);
        }
    }
    Bitmap::from_premultiplied(width, depth, pixels)
}

/// The alpha of row `from_bar` (0 touches the bar) of a shadow `depth` rows deep.
fn alpha(from_bar: u32, depth: u32) -> u8 {
    let reach = 1.0 - from_bar as f32 / depth as f32;
    (ALPHA * reach * reach * 255.0).round() as u8
}

/// The shadow as last drawn, kept while the bar's width, scale and side stay.
#[derive(Default)]
pub struct ShadowCache {
    drawn: Option<(u32, f32, bool, Option<Bitmap>)>,
}

impl ShadowCache {
    /// The shadow for these, drawn again only when one of them changed.
    pub fn get(&mut self, width: u32, scale: f32, above: bool) -> Option<Bitmap> {
        let fresh =
            matches!(&self.drawn, Some((w, s, a, _)) if (*w, *s, *a) == (width, scale, above));
        if !fresh {
            self.drawn = Some((width, scale, above, band(width, scale, above)));
        }
        self.drawn.as_ref().and_then(|(.., b)| b.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The alpha of each row, top-down, in the first column.
    fn column(bitmap: &Bitmap) -> Vec<u8> {
        let stride = bitmap.width() as usize * 4;
        (0..bitmap.height() as usize)
            .map(|row| bitmap.pixels()[row * stride + 3])
            .collect()
    }

    #[test]
    fn six_px_deep_fading_away_from_the_bar() {
        // Below a top bar: the first row touches the bar.
        let below = band(4, 1.0, false).expect("a band");
        assert_eq!((below.width(), below.height()), (4, 6));
        assert_eq!(column(&below), [46, 32, 20, 11, 5, 1]);
        // Black: every colour channel is 0, and every column is the same.
        assert!(below.pixels().chunks(4).all(|px| px[..3] == [0, 0, 0]));
        let stride = 16;
        for row in below.pixels().chunks(stride) {
            assert!(row.chunks(4).all(|px| px == &row[..4]));
        }
        // Above a bottom bar: the same rows, the last one touching the bar.
        let above = band(4, 1.0, true).expect("a band");
        let mut rows = column(&above);
        rows.reverse();
        assert_eq!(rows, column(&below));
    }

    #[test]
    fn depth_follows_the_scale_in_whole_px() {
        assert_eq!(band(10, 1.5, true).expect("a band").height(), 9);
        assert_eq!(band(10, 1.25, true).expect("a band").height(), 8);
        assert_eq!(band(10, 2.0, true).expect("a band").height(), 12);
        // Still fading from 18 % at the bar to nothing.
        let column = column(&band(1, 2.0, false).expect("a band"));
        assert_eq!(column[0], 46);
        assert!(column.windows(2).all(|w| w[0] > w[1] || w[1] <= 1));
        assert!(column[11] <= 1);
    }

    #[test]
    fn no_band_for_a_bar_without_width() {
        assert_eq!(band(0, 1.0, true), None);
    }

    #[test]
    fn the_cache_keeps_the_shadow_while_nothing_changes() {
        let mut cache = ShadowCache::default();
        let first = cache.get(100, 1.0, true).expect("a band");
        let again = cache.get(100, 1.0, true).expect("a band");
        assert!(std::sync::Arc::ptr_eq(
            first.shared_pixels(),
            again.shared_pixels()
        ));
        let wider = cache.get(120, 1.0, true).expect("a band");
        assert_eq!(wider.width(), 120);
        let other_side = cache.get(120, 1.0, false).expect("a band");
        assert!(!std::sync::Arc::ptr_eq(
            wider.shared_pixels(),
            other_side.shared_pixels()
        ));
    }
}
