//! Where a hint or popup appears (SPEC §7.3). Pure; all values in logical screen px (physical
//! px divided by the monitor's scale).
//!
//! ```text
//! margin = 7
//! x = anchor.centerX − w/2, clamped to [M.left + margin, M.right − margin − w]
//! y = barRect.top − h (bar at the bottom: above it) or barRect.bottom (at the top: below it)
//! pointerX = clamp(anchor.centerX − x − pointerW/2, radius + 1, w − radius − 1 − pointerW)
//! ```

use crate::config::model::Edge;
use crate::platform::{RectF, SizeF};

/// Distance kept from the monitor's sides.
const MARGIN: f32 = 7.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The top-left corner of the content (bubble and pointer).
    pub x: f32,
    pub y: f32,
    /// The pointer triangle's left edge, from the content's left edge.
    pub pointer_x: f32,
}

pub struct Anchor {
    /// The sensor.
    pub rect: RectF,
    pub bar: RectF,
    pub monitor: RectF,
    pub edge: Edge,
}

/// Places content of `size` (bubble and pointer) for `anchor`; the pointer is `pointer_width`
/// wide and keeps clear of corners of `radius`.
pub fn place(anchor: &Anchor, size: SizeF, pointer_width: f32, radius: f32) -> Placement {
    let center = (anchor.rect.left + anchor.rect.right) / 2.0;
    let min_x = anchor.monitor.left + MARGIN;
    let max_x = anchor.monitor.right - MARGIN - size.width;
    // `max` last: on a monitor narrower than the content, keep its left edge visible.
    let x = (center - size.width / 2.0).min(max_x).max(min_x);
    let y = match anchor.edge {
        Edge::Bottom => anchor.bar.top - size.height,
        Edge::Top => anchor.bar.bottom,
    };
    let low = radius + 1.0;
    let high = size.width - radius - 1.0 - pointer_width;
    let pointer_x = (center - x - pointer_width / 2.0).min(high).max(low);
    Placement { x, y, pointer_x }
}

/// Places popup content like [`place`], but with its left edge at the sensor's left edge, or
/// its right edge at the sensor's right edge when that would overflow the monitor (SPEC §9.19).
pub fn place_left_edge(anchor: &Anchor, size: SizeF, pointer_width: f32, radius: f32) -> Placement {
    let center = (anchor.rect.left + anchor.rect.right) / 2.0;
    let min_x = anchor.monitor.left + MARGIN;
    let max_x = anchor.monitor.right - MARGIN - size.width;
    let x = if anchor.rect.left <= max_x {
        anchor.rect.left
    } else {
        anchor.rect.right - size.width
    };
    let x = x.min(max_x).max(min_x);
    let low = radius + 1.0;
    let high = size.width - radius - 1.0 - pointer_width;
    Placement {
        x,
        y: place(anchor, size, pointer_width, radius).y,
        pointer_x: (center - x - pointer_width / 2.0).min(high).max(low),
    }
}

/// Placement with the right edge at the sensor's right edge (a [`place_left_edge`] popup
/// being resized while it sits so), kept on the monitor.
pub fn place_right_edge(
    anchor: &Anchor,
    size: SizeF,
    pointer_width: f32,
    radius: f32,
) -> Placement {
    let center = (anchor.rect.left + anchor.rect.right) / 2.0;
    let min_x = anchor.monitor.left + MARGIN;
    let max_x = anchor.monitor.right - MARGIN - size.width;
    let x = (anchor.rect.right - size.width).min(max_x).max(min_x);
    let low = radius + 1.0;
    let high = size.width - radius - 1.0 - pointer_width;
    Placement {
        x,
        y: place(anchor, size, pointer_width, radius).y,
        pointer_x: (center - x - pointer_width / 2.0).min(high).max(low),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(left: f32, right: f32, edge: Edge) -> Anchor {
        let bar = match edge {
            Edge::Bottom => RectF::new(0.0, 1052.0, 1920.0, 1080.0),
            Edge::Top => RectF::new(0.0, 0.0, 1920.0, 28.0),
        };
        Anchor {
            rect: RectF::new(left, bar.top, right, bar.bottom),
            bar,
            monitor: RectF::new(0.0, 0.0, 1920.0, 1080.0),
            edge,
        }
    }

    const SIZE: SizeF = SizeF::new(100.0, 47.0);

    #[test]
    fn centred_above_a_bottom_bar() {
        let p = place(&anchor(900.0, 960.0, Edge::Bottom), SIZE, 13.0, 8.0);
        // Centre 930 → x = 880; the pointer's centre is over the sensor's.
        assert_eq!(
            p,
            Placement {
                x: 880.0,
                y: 1005.0,
                pointer_x: 43.5
            }
        );
    }

    #[test]
    fn below_a_top_bar() {
        let p = place(&anchor(900.0, 960.0, Edge::Top), SIZE, 13.0, 8.0);
        assert_eq!(p.y, 28.0);
    }

    #[test]
    fn popups_can_line_up_with_the_sensors_left_edge() {
        let p = place_left_edge(&anchor(900.0, 960.0, Edge::Bottom), SIZE, 13.0, 8.0);
        assert_eq!((p.x, p.y), (900.0, 1005.0));
        assert_eq!(p.pointer_x, 23.5, "pointing at the sensor's centre");
        // Near the right edge: its right edge at the sensor's.
        let p = place_left_edge(&anchor(1850.0, 1880.0, Edge::Bottom), SIZE, 13.0, 8.0);
        assert_eq!(p.x, 1780.0);
        // Wider than the room either way: kept on the monitor.
        let p = place_left_edge(&anchor(1900.0, 1913.0, Edge::Top), SIZE, 13.0, 8.0);
        assert_eq!((p.x, p.y), (1813.0, 28.0));
    }

    #[test]
    fn kept_on_screen_with_the_pointer_out_of_the_corner() {
        // A sensor at the left edge: the content stops 7 px from the monitor's edge.
        let p = place(&anchor(7.0, 20.0, Edge::Bottom), SIZE, 13.0, 8.0);
        assert_eq!(p.x, 7.0);
        // centre 13.5 − 7 − 6.5 = 0 → at least radius + 1.
        assert_eq!(p.pointer_x, 9.0);

        let p = place(&anchor(1900.0, 1913.0, Edge::Bottom), SIZE, 13.0, 8.0);
        assert_eq!(p.x, 1813.0);
        // At most w − radius − 1 − pointerW = 78.
        assert_eq!(p.pointer_x, 78.0);
    }
}
