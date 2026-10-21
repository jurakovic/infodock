//! Resizing a floating bar by its ends (SPEC §5.1): which end the pointer is at, and where the
//! bar is while one is dragged.

use crate::platform::geometry::{Rect, to_physical};

/// The pointer within this many logical px of a floating bar's left or right end resizes it.
pub const GRIP: f32 = 6.0;
/// A floating bar's width, logical px: the range of the *Width* setting (SPEC §10.2).
const MIN_WIDTH: u32 = 200;
pub const MAX_WIDTH: u32 = 3840;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Left,
    Right,
}

/// The end of a bar `width` logical px wide that the pointer `x` logical px from its left edge
/// is at, if any.
pub fn end_at(x: f32, width: f32) -> Option<End> {
    if x < GRIP {
        Some(End::Left)
    } else if x >= width - GRIP {
        Some(End::Right)
    } else {
        None
    }
}

/// One end being dragged: the dragged end keeps its distance to the pointer, the other stays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resize {
    pub end: End,
    /// The bar when the drag started, physical screen px.
    start: Rect,
    /// How far the pointer was from the dragged end, physical px.
    grab: i32,
}

impl Resize {
    /// A drag of `end` of the bar at `start`, pressed at screen x `at` (physical px).
    pub fn new(end: End, start: Rect, at: i32) -> Self {
        let edge = match end {
            End::Left => start.left,
            End::Right => start.right,
        };
        Self {
            end,
            start,
            grab: at - edge,
        }
    }

    /// Where the bar is with the pointer at screen x `at` (physical px), and its width in
    /// logical px: within [`MIN_WIDTH`] and [`MAX_WIDTH`] at `scale`, and within `work`, the
    /// work area of its monitor. Its top and height stay.
    pub fn rect(&self, at: i32, scale: f64, work: Rect) -> (Rect, u32) {
        let min = to_physical(MIN_WIDTH, scale).min(work.width());
        let max = to_physical(MAX_WIDTH, scale);
        let s = self.start;
        let (left, right) = match self.end {
            End::Left => {
                let latest = s.right - min;
                let earliest = (s.right - max).max(work.left).min(latest);
                ((at - self.grab).clamp(earliest, latest), s.right)
            }
            End::Right => {
                let earliest = s.left + min;
                let latest = (s.left + max).min(work.right).max(earliest);
                (s.left, (at - self.grab).clamp(earliest, latest))
            }
        };
        let width = (f64::from(right - left) / scale).round() as u32;
        (
            Rect::new(left, s.top, right, s.bottom),
            width.clamp(MIN_WIDTH, MAX_WIDTH),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ends_are_six_px_wide() {
        assert_eq!(end_at(0.0, 800.0), Some(End::Left));
        assert_eq!(end_at(5.9, 800.0), Some(End::Left));
        assert_eq!(end_at(6.0, 800.0), None);
        assert_eq!(end_at(400.0, 800.0), None);
        assert_eq!(end_at(793.9, 800.0), None);
        assert_eq!(end_at(794.0, 800.0), Some(End::Right));
        assert_eq!(end_at(799.0, 800.0), Some(End::Right));
    }

    const WORK: Rect = Rect::new(0, 0, 1920, 1040);

    #[test]
    fn the_dragged_end_follows_the_pointer() {
        let start = Rect::new(500, 900, 1300, 928);
        // Pressed 3 px inside the right end.
        let drag = Resize::new(End::Right, start, 1297);
        assert_eq!(
            drag.rect(1397, 1.0, WORK),
            (Rect::new(500, 900, 1400, 928), 900)
        );
        assert_eq!(
            drag.rect(1097, 1.0, WORK),
            (Rect::new(500, 900, 1100, 928), 600)
        );
        // The left end: the right one stays.
        let drag = Resize::new(End::Left, start, 502);
        assert_eq!(
            drag.rect(302, 1.0, WORK),
            (Rect::new(300, 900, 1300, 928), 1000)
        );
        // At 150 %, the width in logical px: 1200 physical px are 800.
        let drag = Resize::new(End::Right, start, 1300);
        assert_eq!(
            drag.rect(1700, 1.5, WORK),
            (Rect::new(500, 900, 1700, 928), 800)
        );
    }

    #[test]
    fn the_width_stays_within_its_range_and_the_work_area() {
        let start = Rect::new(500, 900, 1300, 928);
        let drag = Resize::new(End::Right, start, 1300);
        // Not narrower than 200 px, nor past the work area's right edge.
        assert_eq!(
            drag.rect(0, 1.0, WORK),
            (Rect::new(500, 900, 700, 928), 200)
        );
        assert_eq!(
            drag.rect(5000, 1.0, WORK),
            (Rect::new(500, 900, 1920, 928), 1420)
        );
        let drag = Resize::new(End::Left, start, 500);
        assert_eq!(
            drag.rect(-300, 1.0, WORK),
            (Rect::new(0, 900, 1300, 928), 1300)
        );
        assert_eq!(
            drag.rect(1250, 1.0, WORK),
            (Rect::new(1100, 900, 1300, 928), 200)
        );
        // Not wider than 3840 px, on a wide monitor.
        let wide = Rect::new(0, 0, 7680, 2160);
        let drag = Resize::new(End::Right, Rect::new(100, 0, 900, 28), 900);
        assert_eq!(
            drag.rect(7000, 1.0, wide),
            (Rect::new(100, 0, 3940, 28), 3840)
        );
        // A monitor narrower than the minimum keeps the bar as wide as it is.
        let narrow = Rect::new(0, 0, 150, 600);
        let drag = Resize::new(End::Right, Rect::new(0, 0, 150, 28), 150);
        assert_eq!(drag.rect(10, 1.0, narrow).0, Rect::new(0, 0, 150, 28));
    }
}
