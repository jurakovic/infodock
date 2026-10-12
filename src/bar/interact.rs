//! Pointer rules on the bar (SPEC §6.1): presses, clicks and Lock. Reorder drags (§6.2) arrive
//! in M1 S8; until then a press that moves too far is simply not a click.

use crate::config::model::{ClicksWhenLocked, Zone};
use crate::platform::PointF;

/// Moving farther than this (logical px) from the press starts a drag instead of a click.
pub const DRAG_THRESHOLD: f32 = 4.0;

/// Whether a click reaches the sensor (SPEC §6.1): always while unlocked; while locked as
/// `clicksWhenLocked` says.
pub fn clicks_allowed(locked: bool, rule: ClicksWhenLocked, alt: bool) -> bool {
    !locked || rule == ClicksWhenLocked::Always || alt
}

/// The zone under `x` when no sensor is (SPEC §6.2, §6.6): by thirds of the bar.
pub fn zone_at(x: f32, bar_width: f32) -> Zone {
    if x < bar_width / 3.0 {
        Zone::Left
    } else if x < bar_width * 2.0 / 3.0 {
        Zone::Center
    } else {
        Zone::Right
    }
}

/// Where a new sensor goes in `zone` when not inserted next to a sensor (SPEC §6.6): at the
/// end, except in the right zone, where it goes first (next to the centre).
pub fn insertion_index(zone: Zone, len: usize) -> usize {
    match zone {
        Zone::Left | Zone::Center => len,
        Zone::Right => 0,
    }
}

/// A left press on a sensor, until the button is released.
#[derive(Debug, Clone, PartialEq)]
pub struct Press {
    pub id: String,
    pub at: PointF,
    /// The pointer moved past [`DRAG_THRESHOLD`]: releasing is no click.
    pub moved: bool,
}

impl Press {
    pub fn new(id: String, at: PointF) -> Self {
        Self {
            id,
            at,
            moved: false,
        }
    }

    /// Notes a pointer move; returns whether it crossed the threshold just now.
    pub fn moved_to(&mut self, at: PointF) -> bool {
        if self.moved {
            return false;
        }
        let (dx, dy) = (at.x - self.at.x, at.y - self.at.y);
        self.moved = dx * dx + dy * dy > DRAG_THRESHOLD * DRAG_THRESHOLD;
        self.moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_rules() {
        use ClicksWhenLocked::{AltOnly, Always};
        assert!(clicks_allowed(false, AltOnly, false));
        assert!(clicks_allowed(true, Always, false));
        assert!(!clicks_allowed(true, AltOnly, false));
        assert!(clicks_allowed(true, AltOnly, true));
    }

    #[test]
    fn zones_by_thirds() {
        assert_eq!(zone_at(0.0, 900.0), Zone::Left);
        assert_eq!(zone_at(299.9, 900.0), Zone::Left);
        assert_eq!(zone_at(300.0, 900.0), Zone::Center);
        assert_eq!(zone_at(600.0, 900.0), Zone::Right);
        assert_eq!(insertion_index(Zone::Left, 3), 3);
        assert_eq!(insertion_index(Zone::Center, 1), 1);
        assert_eq!(insertion_index(Zone::Right, 2), 0);
    }

    #[test]
    fn a_press_becomes_a_drag_beyond_four_px() {
        let mut press = Press::new("s_a".into(), PointF::new(10.0, 10.0));
        assert!(!press.moved_to(PointF::new(13.0, 12.0)), "3.6 px");
        assert!(!press.moved);
        assert!(press.moved_to(PointF::new(14.5, 10.0)));
        assert!(!press.moved_to(PointF::new(30.0, 10.0)), "only once");
        assert!(press.moved);
    }
}
