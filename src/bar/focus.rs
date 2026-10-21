//! The keyboard on the bar (SPEC §14.4), the pure part: which sensor Tab moves the focus ring
//! to, where the focus goes when its sensor is removed, and where the ring is drawn.

use crate::platform::{RectF, SizeF};

/// How far the ring lies inside the bar, logical px; its stroke is centred on that line.
const RING_INSET: f32 = 1.5;
/// The ring's stroke and corners, logical px.
pub(super) const RING_WIDTH: f32 = 2.0;
pub(super) const RING_RADIUS: f32 = 3.0;

/// A move of the focus ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Move {
    /// Tab, or Right.
    Next,
    /// Shift+Tab, or Left.
    Previous,
    /// Home.
    First,
    /// End.
    Last,
}

/// The sensor the focus moves to from `current` among `order` (the focusable sensors in visual
/// order); Tab past the last sensor wraps to the first and Shift+Tab before the first to the
/// last. From no sensor (the bar itself), Tab goes to the first and Shift+Tab to the last.
/// `None` on an empty bar.
pub(super) fn step(order: &[String], current: Option<&str>, to: Move) -> Option<String> {
    let last = order.len().checked_sub(1)?;
    let at = current.and_then(|id| order.iter().position(|o| o == id));
    let index = match (to, at) {
        (Move::First, _) | (Move::Next, None) => 0,
        (Move::Last, _) | (Move::Previous, None) => last,
        (Move::Next, Some(i)) => {
            if i == last {
                0
            } else {
                i + 1
            }
        }
        (Move::Previous, Some(i)) => i.checked_sub(1).unwrap_or(last),
    };
    order.get(index).cloned()
}

/// Where the focus is after the layout changed from `old` to `new` (both in focus order) while
/// `current` had it: still on it if it remains; else on the sensor that followed it and
/// remains, which took its place; else on the nearest one before it; else on the bar itself.
pub(super) fn after_change(old: &[String], new: &[String], current: &str) -> Option<String> {
    if new.iter().any(|id| id == current) {
        return Some(current.to_owned());
    }
    let at = old.iter().position(|id| id == current)?;
    let remains = |id: &&String| new.contains(id);
    old[at + 1..]
        .iter()
        .find(remains)
        .or_else(|| old[..at].iter().rev().find(remains))
        .cloned()
}

/// The focus ring's rectangle on a bar of `size`, logical px: around the sensor at `place`
/// (its x and width), or around the whole bar when no sensor has the focus. It reaches a
/// little beyond the sensor, into the gap, so it does not cover the sensor's edges.
pub(super) fn ring(place: Option<(f32, f32)>, size: SizeF, gap: f32) -> RectF {
    let (top, bottom) = (RING_INSET, size.height - RING_INSET);
    match place {
        Some((x, width)) => {
            let reach = (gap / 2.0).min(3.0);
            RectF::new(
                (x - reach).max(RING_INSET),
                top,
                (x + width + reach).min(size.width - RING_INSET),
                bottom,
            )
        }
        None => RectF::new(RING_INSET, top, size.width - RING_INSET, bottom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn tab_goes_through_the_sensors_and_wraps() {
        let order = ids(&["a", "b", "c"]);
        let next = |from| step(&order, from, Move::Next);
        let previous = |from| step(&order, from, Move::Previous);
        assert_eq!(next(None).as_deref(), Some("a"), "from the bar: the first");
        assert_eq!(next(Some("a")).as_deref(), Some("b"));
        assert_eq!(
            next(Some("c")).as_deref(),
            Some("a"),
            "past the last: wraps"
        );
        assert_eq!(
            previous(None).as_deref(),
            Some("c"),
            "from the bar: the last"
        );
        assert_eq!(previous(Some("b")).as_deref(), Some("a"));
        assert_eq!(
            previous(Some("a")).as_deref(),
            Some("c"),
            "before the first: wraps"
        );
        assert_eq!(step(&order, Some("b"), Move::First).as_deref(), Some("a"));
        assert_eq!(step(&order, Some("b"), Move::Last).as_deref(), Some("c"));
        // A sensor that is no longer there counts as none.
        assert_eq!(next(Some("gone")).as_deref(), Some("a"));
        assert_eq!(step(&[], None, Move::Next), None, "an empty bar");
        assert_eq!(step(&[], Some("a"), Move::Previous), None);
    }

    #[test]
    fn a_removed_sensors_focus_goes_to_the_one_that_took_its_place() {
        let old = ids(&["a", "b", "c"]);
        assert_eq!(after_change(&old, &old, "b").as_deref(), Some("b"));
        assert_eq!(
            after_change(&old, &ids(&["c", "a", "b"]), "b").as_deref(),
            Some("b"),
            "moved: it keeps the focus"
        );
        assert_eq!(
            after_change(&old, &ids(&["a", "c"]), "b").as_deref(),
            Some("c")
        );
        assert_eq!(
            after_change(&old, &ids(&["a"]), "b").as_deref(),
            Some("a"),
            "nothing after it: the one before"
        );
        assert_eq!(
            after_change(&old, &ids(&["a", "b"]), "c").as_deref(),
            Some("b")
        );
        assert_eq!(after_change(&old, &[], "b"), None, "the bar emptied");
        assert_eq!(
            after_change(&old, &ids(&["x"]), "b"),
            None,
            "nothing it knew remains"
        );
    }

    #[test]
    fn the_ring_surrounds_the_sensor_or_the_bar() {
        let size = SizeF::new(1000.0, 28.0);
        assert_eq!(
            ring(Some((100.0, 50.0)), size, 8.0),
            RectF::new(97.0, 1.5, 153.0, 26.5),
            "into the gap, at most 3 px"
        );
        assert_eq!(
            ring(Some((100.0, 50.0)), size, 2.0),
            RectF::new(99.0, 1.5, 151.0, 26.5),
            "half a narrow gap"
        );
        assert_eq!(
            ring(Some((0.0, 40.0)), size, 8.0),
            RectF::new(1.5, 1.5, 43.0, 26.5),
            "kept inside the bar"
        );
        assert_eq!(ring(None, size, 8.0), RectF::new(1.5, 1.5, 998.5, 26.5));
    }
}
