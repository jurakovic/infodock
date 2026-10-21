//! Pixel work on decoded skin tiles (SPEC §12.4): cutting the bar part from the shadow band,
//! mirroring, scaling and tiling. Premultiplied RGBA, like [`Bitmap`], so scaling never bleeds
//! the colour of transparent pixels into their neighbours.

use crate::platform::Bitmap;

/// `count` rows of `bitmap` from row `top`; `None` if they are not all there.
pub fn rows(bitmap: &Bitmap, top: u32, count: u32) -> Option<Bitmap> {
    if top.checked_add(count)? > bitmap.height() {
        return None;
    }
    let stride = bitmap.width() as usize * 4;
    let start = top as usize * stride;
    let pixels = bitmap.pixels()[start..start + count as usize * stride].to_vec();
    Bitmap::from_premultiplied(bitmap.width(), count, pixels)
}

/// `bitmap` upside down.
pub fn flipped(bitmap: &Bitmap) -> Bitmap {
    let stride = bitmap.width() as usize * 4;
    let pixels: Vec<u8> = bitmap
        .pixels()
        .chunks_exact(stride)
        .rev()
        .flatten()
        .copied()
        .collect();
    Bitmap::from_premultiplied(bitmap.width(), bitmap.height(), pixels)
        .unwrap_or_else(|| bitmap.clone())
}

/// `bitmap` scaled to `width` × `height` with a linear filter, widened when shrinking so every
/// source pixel counts.
pub fn resized(bitmap: &Bitmap, width: u32, height: u32) -> Option<Bitmap> {
    if (width, height) == (bitmap.width(), bitmap.height()) {
        return Some(bitmap.clone());
    }
    if width == 0 || height == 0 {
        return None;
    }
    let (sw, sh) = (bitmap.width() as usize, bitmap.height() as usize);
    let src: Vec<f32> = bitmap.pixels().iter().map(|&v| f32::from(v)).collect();
    // Along each row, then along each column.
    let across = weights(sw, width as usize);
    let mut wide = vec![0.0; width as usize * sh * 4];
    for y in 0..sh {
        for (x, (start, ws)) in across.iter().enumerate() {
            for (k, w) in ws.iter().enumerate() {
                let s = (y * sw + start + k) * 4;
                let d = (y * width as usize + x) * 4;
                for c in 0..4 {
                    wide[d + c] += src[s + c] * w;
                }
            }
        }
    }
    let down = weights(sh, height as usize);
    let mut out = vec![0.0; width as usize * height as usize * 4];
    for (y, (start, ws)) in down.iter().enumerate() {
        for (k, w) in ws.iter().enumerate() {
            let s = (start + k) * width as usize * 4;
            let d = y * width as usize * 4;
            for i in 0..width as usize * 4 {
                out[d + i] += wide[s + i] * w;
            }
        }
    }
    let pixels = out
        .iter()
        .map(|v| v.round().clamp(0.0, 255.0) as u8)
        .collect();
    Bitmap::from_premultiplied(width, height, pixels)
}

/// For each destination pixel along an axis: the first source pixel and the weights of it and
/// the ones after it. A triangle filter, as wide as a source pixel when enlarging and as a
/// destination pixel when shrinking.
fn weights(src: usize, dst: usize) -> Vec<(usize, Vec<f32>)> {
    let ratio = src as f32 / dst as f32;
    let support = ratio.max(1.0);
    (0..dst)
        .map(|i| {
            let centre = (i as f32 + 0.5) * ratio;
            let start = (centre - support).floor().max(0.0) as usize;
            let end = ((centre + support).ceil() as usize).min(src);
            let mut ws: Vec<f32> = (start..end)
                .map(|j| (1.0 - ((j as f32 + 0.5) - centre).abs() / support).max(0.0))
                .collect();
            let sum: f32 = ws.iter().sum();
            // The nearest source pixel always counts, so the sum is never 0.
            ws.iter_mut().for_each(|w| *w /= sum);
            (start, ws)
        })
        .collect()
}

/// `tile` repeated across `width` px, starting at the left edge; never stretched.
pub fn tiled(tile: &Bitmap, width: u32) -> Option<Bitmap> {
    let stride = tile.width() as usize * 4;
    let mut pixels = Vec::with_capacity(width as usize * tile.height() as usize * 4);
    for row in tile.pixels().chunks_exact(stride) {
        pixels.extend(row.iter().cycle().take(width as usize * 4));
    }
    Bitmap::from_premultiplied(width, tile.height(), pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitmap whose pixel at `x`, `y` is opaque grey `f(x, y)`.
    fn grey(width: u32, height: u32, f: impl Fn(u32, u32) -> u8) -> Bitmap {
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let v = f(x, y);
                [v, v, v, 255]
            })
            .collect();
        Bitmap::from_premultiplied(width, height, pixels).expect("valid")
    }

    fn values(bitmap: &Bitmap) -> Vec<u8> {
        bitmap
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| px[0])
            .collect()
    }

    #[test]
    fn cuts_rows() {
        let b = grey(2, 4, |x, y| (y * 10 + x) as u8);
        assert_eq!(values(&rows(&b, 1, 2).expect("there")), [10, 11, 20, 21]);
        assert!(rows(&b, 3, 2).is_none());
        assert!(rows(&b, 0, 0).is_none(), "an empty bitmap is no bitmap");
    }

    #[test]
    fn flips() {
        let b = grey(2, 3, |x, y| (y * 10 + x) as u8);
        assert_eq!(values(&flipped(&b)), [20, 21, 10, 11, 0, 1]);
    }

    #[test]
    fn tiles_without_stretching() {
        let b = grey(3, 2, |x, y| (y * 10 + x) as u8);
        let t = tiled(&b, 7).expect("tiled");
        assert_eq!((t.width(), t.height()), (7, 2));
        assert_eq!(
            values(&t),
            [0, 1, 2, 0, 1, 2, 0, 10, 11, 12, 10, 11, 12, 10]
        );
        // Narrower than the tile: its left part.
        assert_eq!(values(&tiled(&b, 2).expect("tiled")), [0, 1, 10, 11]);
    }

    #[test]
    fn resizing_keeps_flat_areas_and_blends_edges() {
        // The same size: the same pixels.
        let b = grey(4, 4, |_, y| if y < 2 { 0 } else { 200 });
        assert_eq!(resized(&b, 4, 4), Some(b.clone()));
        // A flat colour stays flat at any size.
        let flat = grey(5, 25, |_, _| 77);
        for (w, h) in [(5, 22), (5, 38), (8, 40), (3, 10)] {
            let r = resized(&flat, w, h).expect("resized");
            assert_eq!((r.width(), r.height()), (w, h));
            assert!(values(&r).iter().all(|&v| v == 77), "{w}×{h}");
        }
        // Twice as tall: the top and bottom rows keep their colour, the middle blends.
        let tall = resized(&b, 4, 8).expect("resized");
        let column: Vec<u8> = (0..8).map(|y| values(&tall)[y * 4]).collect();
        assert_eq!(column[0], 0);
        assert_eq!(column[7], 200);
        assert!(column[3] > 0 && column[3] < 100 && column[4] > 100 && column[4] < 200);
        // Half as tall: each row weighs the two rows it covers most, their neighbours less:
        // (100 × 0.75 + 200 × 0.25) / 1.75 and (100 × 0.25 + 200 × 1.5) / 1.75.
        let short =
            resized(&grey(1, 4, |_, y| [0, 100, 200, 200][y as usize]), 1, 2).expect("resized");
        assert_eq!(values(&short), [71, 186]);
        assert!(resized(&b, 0, 4).is_none());
    }

    #[test]
    fn resizing_keeps_alpha_premultiplied() {
        // Opaque white beside transparent black: the edge is half-transparent white, not grey.
        let pixels = [255, 255, 255, 255, 0, 0, 0, 0].repeat(2);
        let b = Bitmap::from_premultiplied(2, 2, pixels).expect("valid");
        let r = resized(&b, 1, 2).expect("resized");
        assert!(
            r.pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| *px == [128, 128, 128, 128])
        );
    }
}
