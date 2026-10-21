//! Pointer rules on the bar (SPEC §6.1, §6.2): presses, clicks, and where a reorder drag drops.
//! The Lock rules for clicks are the config's (`Settings::clicks_allowed`).

use crate::config::model::Zone;
use crate::platform::PointF;

/// Moving farther than this (logical px) from the press starts a drag instead of a click.
const DRAG_THRESHOLD: f32 = 4.0;

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

/// Where a dragged sensor would land (SPEC §6.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropTarget {
    pub zone: Zone,
    /// The insertion point in `zone`, counted with the dragged sensor still in place.
    pub insertion: usize,
    /// The centre of the placer line, logical px.
    pub placer_x: f32,
}

/// The drop target for the pointer at `x`, given each zone's sensors as `(x, width)` in
/// visual order (SPEC §6.2): over a sensor, before it on its left half and after it on its
/// right half (the centre zone first, as it is drawn on top); elsewhere the zone by thirds, at
/// its insertion point.
pub fn drop_target(
    x: f32,
    zones: &[Vec<(f32, f32)>; 3],
    bar_width: f32,
    pad: f32,
    gap: f32,
) -> DropTarget {
    const ORDER: [Zone; 3] = [Zone::Center, Zone::Left, Zone::Right];
    for zone in ORDER {
        let sensors = &zones[zone as usize];
        if let Some(i) = sensors.iter().position(|&(sx, w)| x >= sx && x < sx + w) {
            let (sx, w) = sensors[i];
            return if x < sx + w / 2.0 {
                DropTarget {
                    zone,
                    insertion: i,
                    placer_x: sx - gap / 2.0,
                }
            } else {
                DropTarget {
                    zone,
                    insertion: i + 1,
                    placer_x: sx + w + gap / 2.0,
                }
            };
        }
    }
    let zone = zone_at(x, bar_width);
    let sensors = &zones[zone as usize];
    let after_last = sensors.last().map(|&(sx, w)| sx + w + gap / 2.0);
    let placer_x = match zone {
        Zone::Left => after_last.unwrap_or(pad),
        Zone::Center => after_last.unwrap_or(bar_width / 2.0),
        Zone::Right => sensors
            .first()
            .map_or(bar_width - pad, |&(sx, _)| sx - gap / 2.0),
    };
    DropTarget {
        zone,
        insertion: insertion_index(zone, sensors.len()),
        placer_x,
    }
}

/// Whether dropping at `target` leaves the sensor at `source` where it is.
pub fn is_no_move(source: (Zone, usize), target: &DropTarget) -> bool {
    source.0 == target.zone && (target.insertion == source.1 || target.insertion == source.1 + 1)
}

/// A left press on a sensor, until the button is released.
#[derive(Debug, Clone, PartialEq)]
pub struct Press {
    pub id: String,
    pub at: PointF,
    /// The pointer moved past [`DRAG_THRESHOLD`]: releasing is no click.
    pub moved: bool,
    /// It closed this sensor's popup, so releasing it does not open the popup again, however
    /// long it was held (SPEC §7.2).
    pub closed_popup: bool,
}

impl Press {
    pub fn new(id: String, at: PointF) -> Self {
        Self {
            id,
            at,
            moved: false,
            closed_popup: false,
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

    fn zones() -> [Vec<(f32, f32)>; 3] {
        [
            vec![(7.0, 50.0), (67.0, 30.0)],
            vec![(482.0, 27.0)],
            vec![(923.0, 40.0), (973.0, 20.0)],
        ]
    }

    fn target(x: f32) -> DropTarget {
        drop_target(x, &zones(), 1000.0, 7.0, 10.0)
    }

    #[test]
    fn over_a_sensor_by_halves() {
        assert_eq!(
            target(20.0),
            DropTarget {
                zone: Zone::Left,
                insertion: 0,
                placer_x: 2.0
            }
        );
        assert_eq!(
            target(40.0),
            DropTarget {
                zone: Zone::Left,
                insertion: 1,
                placer_x: 62.0
            }
        );
        assert_eq!(target(990.0).insertion, 2);
        assert_eq!(target(500.0).zone, Zone::Center);
    }

    #[test]
    fn over_empty_space_by_thirds() {
        // Left third, right of the sensors: after the last one.
        assert_eq!(
            target(200.0),
            DropTarget {
                zone: Zone::Left,
                insertion: 2,
                placer_x: 102.0
            }
        );
        // Centre third: after the centre's last sensor.
        assert_eq!(target(400.0).placer_x, 514.0);
        // Right third: before the right zone's first sensor.
        assert_eq!(
            target(800.0),
            DropTarget {
                zone: Zone::Right,
                insertion: 0,
                placer_x: 918.0
            }
        );
        // Empty zones: at the padding or the centre.
        let empty = [vec![], vec![], vec![]];
        assert_eq!(drop_target(10.0, &empty, 900.0, 7.0, 10.0).placer_x, 7.0);
        assert_eq!(drop_target(450.0, &empty, 900.0, 7.0, 10.0).placer_x, 450.0);
        assert_eq!(drop_target(890.0, &empty, 900.0, 7.0, 10.0).placer_x, 893.0);
    }

    #[test]
    fn dropping_next_to_itself_moves_nothing() {
        let source = (Zone::Left, 0);
        assert!(is_no_move(source, &target(20.0)));
        assert!(is_no_move(source, &target(40.0)));
        // Left of the second sensor is where it already is; right of it is a move.
        assert!(is_no_move(source, &target(80.0)));
        assert!(!is_no_move(source, &target(90.0)));
        assert!(!is_no_move(source, &target(500.0)));
    }
}
