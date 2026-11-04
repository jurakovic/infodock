//! Zone layout (SPEC §5.4): where each sensor sits, from the widths the sensors measured.
//!
//! ```text
//! left:   x = pad; for s in left: place(s, x); x += width(s) + gap
//! center: total = Σ(width(s) + gap); x = round((barWidth − total) / 2); then as left
//! right:  x = barWidth − pad; for s in reverse(right): x −= width(s); place(s, x); x −= gap
//! ```
//!
//! The centre total includes one trailing gap, as the spec says. A centre zone that would meet
//! the left or right zone moves towards the side with room, as far as it needs. When the zones
//! do not fit side by side, [`fit`] narrows the sensors that can shrink (text with an
//! ellipsis), the centre zone first, then the left, then the right; what still does not fit
//! overlaps, and the centre zone is drawn on top.

/// Sensor widths (input) or x positions (output) per zone: left, centre, right; each in visual
/// left-to-right order. Logical px.
pub type Zones = [Vec<f32>; 3];

/// The width each sensor gets (SPEC §5.4): the width it measured while the zones fit the bar
/// side by side, with a gap between zones and the padding at the ends (the centre zone then
/// moves off the centre as far as it needs, [`place`]). Otherwise the sensors that can shrink
/// (a minimum below their width) give way, down to their minimums, until the zones fit: the
/// centre zone first, then the left, then the right, its sensors in proportion to what they can
/// give and in whole px.
pub fn fit(widths: &Zones, mins: &Zones, bar_width: f32, pad: f32, gap: f32) -> Zones {
    let mut out = widths.clone();
    let over = needed(&out, pad, gap) - bar_width;
    let rest = shrink(&mut out[1], &mins[1], over);
    let rest = shrink(&mut out[0], &mins[0], rest);
    shrink(&mut out[2], &mins[2], rest);
    out
}

/// A zone's width: its sensors with a gap between them.
fn span(zone: &[f32], gap: f32) -> f32 {
    match zone.len() {
        0 => 0.0,
        n => zone.iter().sum::<f32>() + gap * (n - 1) as f32,
    }
}

/// The width the zones need side by side: the padding at both ends and a gap between zones.
fn needed(zones: &Zones, pad: f32, gap: f32) -> f32 {
    let shown = zones.iter().filter(|z| !z.is_empty());
    let (count, width) = shown.fold((0, 0.0), |(n, w), z| (n + 1, w + span(z, gap)));
    2.0 * pad + width + gap * (count - 1).max(0) as f32
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

/// The x position of every sensor. The centre zone is centred where it has room; where it would
/// meet the left or the right zone, it moves towards the other side as far as it needs, and
/// with no room either way it sits midway between them, over both (SPEC §5.4).
pub fn place(widths: &Zones, bar_width: f32, pad: f32, gap: f32) -> Zones {
    let [left, center, right] = widths;
    let total: f32 = center.iter().map(|w| w + gap).sum();
    let centred = ((bar_width - total) / 2.0).round();
    // Where the centre zone may start: after the left zone and before the right one.
    let earliest = if left.is_empty() {
        pad
    } else {
        pad + span(left, gap) + gap
    };
    let end = if right.is_empty() {
        bar_width - pad
    } else {
        bar_width - pad - span(right, gap) - gap
    };
    let latest = end - span(center, gap);
    let (earliest, latest) = (earliest.ceil(), latest.floor());
    let center_x = if earliest <= latest {
        centred.clamp(earliest, latest)
    } else {
        ((earliest + latest) / 2.0).round()
    };

    let mut x = pad;
    let left = left
        .iter()
        .map(|w| {
            let at = x;
            x += w + gap;
            at
        })
        .collect();

    let mut x = center_x;
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
    fn the_centre_moves_before_anything_gives_way() {
        // Side by side they need 7 + 100 + 10 + 200 + 10 + 60 + 7 = 394 of 400: nothing is cut.
        // Centred (at 95) the centre would meet the left, which ends at 107; it moves right to
        // start after the left's gap, at 117, and still ends before the right's gap (323).
        let widths = [vec![100.0], vec![200.0], vec![60.0]];
        let min = mins(&widths, [&[30.0], &[30.0], &[30.0]]);
        let fitted = fit(&widths, &min, 400.0, 7.0, 10.0);
        assert_eq!(fitted, widths);
        let [left, center, right] = place(&fitted, 400.0, 7.0, 10.0);
        assert_eq!((left[0] + 100.0 + 10.0, center[0]), (117.0, 117.0));
        assert!(center[0] + 200.0 + 10.0 <= right[0]);
        // The other way round, it moves left.
        let mirrored = [vec![60.0], vec![200.0], vec![100.0]];
        let [_, center, right] = place(&mirrored, 400.0, 7.0, 10.0);
        assert_eq!(center[0] + 200.0 + 10.0, right[0]);
        // With room on both sides it stays centred.
        let [_, center, _] = place(&[vec![60.0], vec![200.0], vec![60.0]], 400.0, 7.0, 10.0);
        assert_eq!(center[0], 95.0);
    }

    #[test]
    fn the_centre_gives_way_first() {
        // 54 px too wide: the centre gives them all and fits exactly between the others.
        let widths = [vec![100.0], vec![260.0], vec![60.0]];
        let min = mins(&widths, [&[30.0], &[30.0], &[30.0]]);
        let fitted = fit(&widths, &min, 400.0, 7.0, 10.0);
        assert_eq!(fitted, [vec![100.0], vec![206.0], vec![60.0]]);
        let [left, center, right] = place(&fitted, 400.0, 7.0, 10.0);
        assert_eq!(left[0] + 100.0 + 10.0, center[0]);
        assert_eq!(center[0] + 206.0 + 10.0, right[0]);
    }

    #[test]
    fn then_the_left_and_the_right() {
        // 94 px too wide: the centre goes to its minimum (40 px given), then the left gives the
        // other 54, all from its first sensor (the second cannot shrink); the right keeps its
        // width.
        let widths = [vec![100.0, 50.0], vec![80.0], vec![120.0]];
        let min = mins(&widths, [&[30.0, 0.0], &[40.0], &[20.0]]);
        let fitted = fit(&widths, &min, 300.0, 7.0, 10.0);
        assert_eq!(fitted, [vec![46.0, 50.0], vec![40.0], vec![120.0]]);
        let [left, center, right] = place(&fitted, 300.0, 7.0, 10.0);
        assert_eq!(left[1] + 50.0 + 10.0, center[0]);
        assert_eq!(center[0] + 40.0 + 10.0, right[0]);
        // Narrower still: the left is at its minimum and the right gives way.
        let fitted = fit(&widths, &min, 260.0, 7.0, 10.0);
        assert_eq!(fitted, [vec![30.0, 50.0], vec![40.0], vec![96.0]]);
    }

    /// The zones overlap: a gap is missing between neighbours, or a zone leaves the padding.
    fn overlaps(widths: &Zones, bar_width: f32) -> bool {
        let [left, center, right] = place(widths, bar_width, 7.0, 10.0);
        let ends = |xs: &[f32], ws: &[f32]| xs.last().zip(ws.last()).map(|(x, w)| x + w);
        let mut edges = Vec::new();
        for (xs, ws) in [
            (&left, &widths[0]),
            (&center, &widths[1]),
            (&right, &widths[2]),
        ] {
            if let (Some(&start), Some(end)) = (xs.first(), ends(xs, ws)) {
                edges.push((start, end));
            }
        }
        edges.windows(2).any(|p| p[0].1 + 10.0 > p[1].0 + 0.01)
            || edges.first().is_some_and(|e| e.0 < 7.0)
            || edges.last().is_some_and(|e| e.1 > bar_width - 7.0 + 0.01)
    }

    #[test]
    fn nothing_overlaps_until_every_text_is_at_its_minimum() {
        // The narrow floating bar of the manual check: a graph with its rates on the left
        // (fixed), an icon in the centre (fixed), a text on the right (down to 20 px).
        let widths = [vec![120.0], vec![20.0], vec![80.0]];
        let min = mins(&widths, [&[0.0], &[0.0], &[20.0]]);
        // At their minimums they need 7 + 120 + 10 + 20 + 10 + 20 + 7 = 194 px.
        for bar in 194..=320 {
            let bar = bar as f32;
            let fitted = fit(&widths, &min, bar, 7.0, 10.0);
            assert!(!overlaps(&fitted, bar), "{bar} px: {fitted:?}");
            // The text is cut only where the zones cannot sit side by side whole (254 px).
            assert_eq!(fitted[2][0] == 80.0, bar >= 254.0, "{bar} px: {fitted:?}");
        }
        // Below that, every text is at its minimum and the centre sits over both sides.
        let fitted = fit(&widths, &min, 180.0, 7.0, 10.0);
        assert_eq!(fitted, [vec![120.0], vec![20.0], vec![20.0]]);
        assert!(overlaps(&fitted, 180.0));
        let [_, center, _] = place(&fitted, 180.0, 7.0, 10.0);
        // Midway between where it may start (137) and end (123 + 20).
        assert_eq!(center[0], 130.0);
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
