//! Zone layout (SPEC §5.4): where each sensor sits, from the widths the sensors measured.
//!
//! ```text
//! left:   x = pad; for s in left: place(s, x); x += width(s) + gap
//! center: total = Σ(width(s) + gap); x = round((barWidth − total) / 2); then as left
//! right:  x = barWidth − pad; for s in reverse(right): x −= width(s); place(s, x); x −= gap
//! ```
//!
//! The centre total includes one trailing gap, as the spec says. Zones may overlap when the
//! content is wider than the bar; the centre zone is drawn on top. Shrinking text with an
//! ellipsis in that case (a SHOULD of §5.4) is not done yet.

/// Sensor widths (input) or x positions (output) per zone: left, centre, right; each in visual
/// left-to-right order. Logical px.
pub type Zones = [Vec<f32>; 3];

/// The x position of every sensor.
pub fn place(widths: &Zones, bar_width: f32, pad: f32, gap: f32) -> Zones {
    let [left, center, right] = widths;

    let mut x = pad;
    let left = left
        .iter()
        .map(|w| {
            let at = x;
            x += w + gap;
            at
        })
        .collect();

    let total: f32 = center.iter().map(|w| w + gap).sum();
    let mut x = ((bar_width - total) / 2.0).round();
    let center = center
        .iter()
        .map(|w| {
            let at = x;
            x += w + gap;
            at
        })
        .collect();

    let mut x = bar_width - pad;
    let mut right: Vec<f32> = right
        .iter()
        .rev()
        .map(|w| {
            x -= w;
            let at = x;
            x -= gap;
            at
        })
        .collect();
    right.reverse();

    [left, center, right]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_each_zone() {
        let widths = [vec![50.0, 30.0], vec![27.0], vec![40.0, 20.0]];
        let [left, center, right] = place(&widths, 1000.0, 7.0, 10.0);
        assert_eq!(left, [7.0, 67.0]);
        // (1000 − (27 + 10)) / 2 = 481.5 → 482.
        assert_eq!(center, [482.0]);
        // From the right edge: 1000 − 7 − 20 = 973; 973 − 10 − 40 = 923.
        assert_eq!(right, [923.0, 973.0]);
    }

    #[test]
    fn empty_zones() {
        let [left, center, right] = place(&[vec![], vec![], vec![]], 500.0, 7.0, 10.0);
        assert!(left.is_empty() && center.is_empty() && right.is_empty());
    }

    #[test]
    fn centre_includes_one_trailing_gap() {
        let [_, center, _] = place(&[vec![], vec![10.0, 10.0], vec![]], 100.0, 7.0, 4.0);
        // total = 28 → x = 36, then 50.
        assert_eq!(center, [36.0, 50.0]);
    }
}
