//! Zone layout (SPEC §5.4): where each sensor sits, from the widths the sensors measured.
//!
//! ```text
//! left:   x = pad; for s in left: place(s, x); x += width(s) + gap
//! center: total = Σ(width(s) + gap); x = round((barWidth − total) / 2); then as left
//! right:  x = barWidth − pad; for s in reverse(right): x −= width(s); place(s, x); x −= gap
//! ```
//!
//! The centre total includes one trailing gap, as the spec says. When the zones do not fit
//! the bar with a gap between them, [`fit`] narrows the sensors that can shrink (text with an
//! ellipsis), the centre zone first, then the left, then the right; what still does not fit
//! overlaps, and the centre zone is drawn on top.

/// Sensor widths (input) or x positions (output) per zone: left, centre, right; each in visual
/// left-to-right order. Logical px.
pub type Zones = [Vec<f32>; 3];

/// The width each sensor gets (SPEC §5.4): the width it measured while the zones fit the bar,
/// with a gap between zones and the padding at the ends. Otherwise the sensors that can shrink
/// (a minimum below their width) give way, down to their minimums: the centre zone first, then
/// the left, then the right, each as far as its own overlap needs, its sensors in proportion to
/// what they can give and in whole px.
pub fn fit(widths: &Zones, mins: &Zones, bar_width: f32, pad: f32, gap: f32) -> Zones {
    let span = |zone: &[f32]| match zone.len() {
        0 => 0.0,
        n => zone.iter().sum::<f32>() + gap * (n - 1) as f32,
    };
    let mut out = widths.clone();
    if out[1].is_empty() {
        let between = if out[0].is_empty() || out[2].is_empty() {
            0.0
        } else {
            gap
        };
        let over = 2.0 * pad + span(&out[0]) + between + span(&out[2]) - bar_width;
        let rest = shrink(&mut out[0], &mins[0], over);
        shrink(&mut out[2], &mins[2], rest);
        return out;
    }
    // Where the centre zone may start and end.
    let first = |out: &Zones| {
        if out[0].is_empty() {
            pad
        } else {
            pad + span(&out[0]) + gap
        }
    };
    let last = |out: &Zones| {
        if out[2].is_empty() {
            bar_width - pad
        } else {
            bar_width - pad - span(&out[2]) - gap
        }
    };
    // It starts at (bar − centre − gap) / 2, since its total has a trailing gap.
    let room = (bar_width - gap - 2.0 * first(&out)).min(2.0 * last(&out) - bar_width + gap);
    let over = span(&out[1]) - room;
    shrink(&mut out[1], &mins[1], over);
    let center = span(&out[1]);
    let start = (bar_width - center - gap) / 2.0;
    let over = first(&out) - start;
    shrink(&mut out[0], &mins[0], over);
    let over = start + center - last(&out);
    shrink(&mut out[2], &mins[2], over);
    out
}

/// Takes `amount` logical px from `widths`, each giving in proportion to how far it is above
/// its minimum and keeping whole px; returns what they could not give.
fn shrink(widths: &mut [f32], mins: &[f32], amount: f32) -> f32 {
    if amount <= 0.0 {
        return 0.0;
    }
    let room: f32 = widths.iter().zip(mins).map(|(w, m)| (w - m).max(0.0)).sum();
    if room <= 0.0 {
        return amount;
    }
    let share = (amount / room).min(1.0);
    let before: f32 = widths.iter().sum();
    for (width, &min) in widths.iter_mut().zip(mins) {
        if *width > min {
            *width = (*width - (*width - min) * share).floor().max(min);
        }
    }
    (amount - (before - widths.iter().sum::<f32>())).max(0.0)
}

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

    /// Each sensor's minimum: `0` for those that cannot shrink.
    fn mins(widths: &Zones, mins: [&[f32]; 3]) -> Zones {
        std::array::from_fn(|z| {
            widths[z]
                .iter()
                .zip(mins[z])
                .map(|(&w, &m)| if m == 0.0 { w } else { m })
                .collect()
        })
    }

    #[test]
    fn zones_that_fit_keep_their_widths() {
        let widths = [vec![100.0], vec![50.0], vec![100.0]];
        let min = mins(&widths, [&[20.0], &[20.0], &[20.0]]);
        assert_eq!(fit(&widths, &min, 400.0, 7.0, 10.0), widths);
        // Nothing can shrink: they overlap as they are.
        let crowded = [vec![300.0], vec![200.0], vec![300.0]];
        let fixed = crowded.clone();
        assert_eq!(fit(&crowded, &fixed, 400.0, 7.0, 10.0), crowded);
    }

    #[test]
    fn the_centre_gives_way_first() {
        // Left ends at 107; the right starts at 333. The centre, centred with its trailing
        // gap, may be 400 − 10 − 2 × 117 = 156 px wide before it reaches the left's gap.
        let widths = [vec![100.0], vec![200.0], vec![60.0]];
        let min = mins(&widths, [&[30.0], &[30.0], &[30.0]]);
        let fitted = fit(&widths, &min, 400.0, 7.0, 10.0);
        assert_eq!(fitted, [vec![100.0], vec![156.0], vec![60.0]]);
        let [left, center, _] = place(&fitted, 400.0, 7.0, 10.0);
        assert_eq!((left[0] + 100.0 + 10.0, center[0]), (117.0, 117.0));
    }

    #[test]
    fn then_the_left_and_the_right() {
        // The centre goes to its minimum (40) and still meets both sides: it starts at 125, so
        // the left gives 52 px, all from its first sensor (the second cannot shrink), and the
        // right 2 px.
        let widths = [vec![100.0, 50.0], vec![80.0], vec![120.0]];
        let min = mins(&widths, [&[30.0, 0.0], &[40.0], &[20.0]]);
        let fitted = fit(&widths, &min, 300.0, 7.0, 10.0);
        assert_eq!(fitted, [vec![48.0, 50.0], vec![40.0], vec![118.0]]);
        let [left, center, right] = place(&fitted, 300.0, 7.0, 10.0);
        assert_eq!(left[1] + 50.0 + 10.0, center[0]);
        assert_eq!(center[0] + 40.0 + 10.0, right[0]);
    }

    #[test]
    fn without_a_centre_the_left_gives_way_before_the_right() {
        let widths = [vec![100.0], vec![], vec![100.0]];
        let min = mins(&widths, [&[30.0], &[], &[30.0]]);
        // 7 + 100 + 10 + 100 + 7 is 24 px too wide for 200.
        assert_eq!(
            fit(&widths, &min, 200.0, 7.0, 10.0),
            [vec![76.0], vec![], vec![100.0]]
        );
        // The left reaches its minimum; the right gives the rest.
        let widths = [vec![60.0], vec![], vec![150.0]];
        let min = mins(&widths, [&[50.0], &[], &[30.0]]);
        assert_eq!(
            fit(&widths, &min, 200.0, 7.0, 10.0),
            [vec![50.0], vec![], vec![126.0]]
        );
        // One zone alone keeps within the padding.
        let widths = [vec![], vec![], vec![250.0]];
        let min = mins(&widths, [&[], &[], &[30.0]]);
        assert_eq!(
            fit(&widths, &min, 200.0, 7.0, 10.0),
            [vec![], vec![], vec![186.0]]
        );
    }

    #[test]
    fn a_zone_shares_what_it_gives() {
        // 50 px from sensors that can give 80 and 20: 40 and 10.
        let mut widths = [100.0, 60.0, 30.0];
        assert_eq!(shrink(&mut widths, &[20.0, 40.0, 30.0], 50.0), 0.0);
        assert_eq!(widths, [60.0, 50.0, 30.0]);
        // More than they can give: all at their minimums, the rest returned.
        assert_eq!(shrink(&mut widths, &[20.0, 40.0, 30.0], 70.0), 20.0);
        assert_eq!(widths, [20.0, 40.0, 30.0]);
        // Whole px: 10 px from 25.5 and 25.5 with minimums 10 gives 20 and 20.
        let mut widths = [25.5, 25.5];
        assert_eq!(shrink(&mut widths, &[10.0, 10.0], 10.0), 0.0);
        assert_eq!(widths, [20.0, 20.0]);
    }

    #[test]
    fn centre_includes_one_trailing_gap() {
        let [_, center, _] = place(&[vec![], vec![10.0, 10.0], vec![]], 100.0, 7.0, 4.0);
        // total = 28 → x = 36, then 50.
        assert_eq!(center, [36.0, 50.0]);
    }
}
