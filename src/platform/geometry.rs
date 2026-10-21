//! Pure placement geometry shared by every platform: docking strips, logical→physical sizes and
//! monitor selection (SPEC §5.1–5.2). No OS calls, fully unit-tested.

use serde::{Deserialize, Serialize};

use crate::config::model::{Edge, FloatingRect, PRIMARY_MONITOR};

/// A rectangle in physical pixels, right/bottom exclusive (like Win32 `RECT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
    pub const fn width(&self) -> i32 {
        self.right - self.left
    }
    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// A monitor as reported by the platform (SPEC §4.6 `monitors()`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    /// Stable selection key stored in `settings.bar.monitor`: `<device>|<name>|<width>x<height>`.
    pub key: String,
    /// OS device name, e.g. `\\.\DISPLAY2`.
    pub device: String,
    /// Friendly name, e.g. `DELL U2720Q`.
    pub name: String,
    pub bounds: Rect,
    pub work_area: Rect,
    /// Scale factor (1.0 = 96 DPI).
    pub scale: f64,
    pub primary: bool,
}

impl MonitorInfo {
    pub fn new(
        device: String,
        name: String,
        bounds: Rect,
        work_area: Rect,
        scale: f64,
        primary: bool,
    ) -> Self {
        let key = monitor_key(&device, &name, bounds.width(), bounds.height());
        Self {
            key,
            device,
            name,
            bounds,
            work_area,
            scale,
            primary,
        }
    }
}

fn monitor_key(device: &str, name: &str, width: i32, height: i32) -> String {
    format!("{device}|{name}|{width}x{height}")
}

/// Picks the monitor for `selection` (`"primary"` or a key from [`MonitorInfo::key`]).
///
/// Order: device and name match → name and resolution match (device names change when monitors
/// are re-plugged) → device matches → primary → first. An absent monitor therefore falls back to
/// the primary one while the setting is kept for when it returns (SPEC §5.1).
pub fn select_monitor<'a>(monitors: &'a [MonitorInfo], selection: &str) -> Option<&'a MonitorInfo> {
    let primary = || {
        monitors
            .iter()
            .find(|m| m.primary)
            .or_else(|| monitors.first())
    };
    if selection == PRIMARY_MONITOR {
        return primary();
    }
    let (device, name, size) = parse_key(selection);
    let size_matches = |m: &MonitorInfo| {
        size.is_some_and(|(w, h)| m.bounds.width() == w && m.bounds.height() == h)
    };
    let named = |m: &MonitorInfo| name.is_some_and(|n| !n.is_empty() && m.name == n);
    monitors
        .iter()
        .find(|m| m.device == device && named(m))
        .or_else(|| monitors.iter().find(|m| named(m) && size_matches(m)))
        .or_else(|| monitors.iter().find(|m| m.device == device))
        .or_else(primary)
}

/// Splits `<device>|<name>|<w>x<h>`. Accepts a bare device name; a name may itself contain `|`.
fn parse_key(key: &str) -> (&str, Option<&str>, Option<(i32, i32)>) {
    let Some((device, rest)) = key.split_once('|') else {
        return (key, None, None);
    };
    match rest.rsplit_once('|') {
        Some((name, size)) => {
            let size = size
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
            (device, Some(name), size)
        }
        None => (device, Some(rest), None),
    }
}

/// Logical (DIP) length to physical pixels.
pub fn to_physical(logical: u32, scale: f64) -> i32 {
    // Heights are at most 40 and scales at most ~5, so the cast cannot overflow.
    (f64::from(logical) * scale).round() as i32
}

/// The strip of `height` pixels along `edge` of `area`.
pub fn dock_strip(area: Rect, edge: Edge, height: i32) -> Rect {
    match edge {
        Edge::Top => Rect::new(area.left, area.top, area.right, area.top + height),
        Edge::Bottom => Rect::new(area.left, area.bottom - height, area.right, area.bottom),
    }
}

/// After the shell adjusted a proposed rect (`ABM_QUERYPOS`), restores the bar's height measured
/// from the edge it is docked to.
pub fn keep_height(rect: Rect, edge: Edge, height: i32) -> Rect {
    match edge {
        Edge::Top => Rect::new(rect.left, rect.top, rect.right, rect.top + height),
        Edge::Bottom => Rect::new(rect.left, rect.bottom - height, rect.right, rect.bottom),
    }
}

/// A floating bar's width until the user sets one, logical px (SPEC §5.1).
pub const DEFAULT_FLOATING_WIDTH: u32 = 800;
/// Its default gap to the work area's edge, logical px.
const FLOATING_MARGIN: u32 = 12;

/// Where a floating bar goes (SPEC §5.1): at `floating` (x and y physical screen px, width
/// logical px), kept inside the work area of the monitor it is on; without a position, centred
/// near `edge` of the selected monitor. Returns the rect and its monitor.
pub fn floating_rect<'a>(
    monitors: &'a [MonitorInfo],
    selection: &str,
    floating: Option<FloatingRect>,
    edge: Edge,
    height: u32,
) -> Option<(Rect, &'a MonitorInfo)> {
    let monitor = match floating {
        Some(f) => monitor_for(monitors, f, height)?,
        None => select_monitor(monitors, selection)?,
    };
    let work = monitor.work_area;
    let width = floating.map_or(DEFAULT_FLOATING_WIDTH, |f| f.width);
    let w = to_physical(width, monitor.scale).min(work.width());
    let h = to_physical(height, monitor.scale).min(work.height());
    let (x, y) = match floating {
        Some(f) => (f.x, f.y),
        None => {
            let margin = to_physical(FLOATING_MARGIN, monitor.scale);
            let y = match edge {
                Edge::Top => work.top + margin,
                Edge::Bottom => work.bottom - h - margin,
            };
            (work.left + (work.width() - w) / 2, y)
        }
    };
    let x = x.clamp(work.left, work.right - w);
    let y = y.clamp(work.top, work.bottom - h);
    Some((Rect::new(x, y, x + w, y + h), monitor))
}

/// The monitor a floating bar at `f` belongs to: the one holding the largest share of it (its
/// size depends on each monitor's scale, so shares are compared, not areas), else the one
/// nearest to its top-left corner. In `i64`, since a hand-edited position may be anything.
fn monitor_for(monitors: &[MonitorInfo], f: FloatingRect, height: u32) -> Option<&MonitorInfo> {
    let (x, y) = (i64::from(f.x), i64::from(f.y));
    let share = |m: &MonitorInfo| {
        let w = i64::from(to_physical(f.width, m.scale)).max(1);
        let h = i64::from(to_physical(height, m.scale)).max(1);
        let b = m.bounds;
        let across = (x + w).min(i64::from(b.right)) - x.max(i64::from(b.left));
        let down = (y + h).min(i64::from(b.bottom)) - y.max(i64::from(b.top));
        (across.max(0) * down.max(0)) as f64 / (w * h) as f64
    };
    let mut best: Option<(&MonitorInfo, f64)> = None;
    for m in monitors {
        let s = share(m);
        if s > 0.0 && best.is_none_or(|(_, b)| s > b) {
            best = Some((m, s));
        }
    }
    let distance = |m: &MonitorInfo| {
        let b = m.bounds;
        let dx = (i64::from(b.left) - x)
            .max(x - i64::from(b.right - 1))
            .max(0);
        let dy = (i64::from(b.top) - y)
            .max(y - i64::from(b.bottom - 1))
            .max(0);
        dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
    };
    best.map(|(m, _)| m)
        .or_else(|| monitors.iter().min_by_key(|m| distance(m)))
}

/// The monitor edge nearer to the middle of `rect`: where hints of a floating bar open away
/// from (SPEC §7.3).
pub fn nearer_edge(rect: Rect, monitor: &MonitorInfo) -> Edge {
    let middle = (rect.top + rect.bottom) / 2;
    if middle - monitor.bounds.top < monitor.bounds.bottom - middle {
        Edge::Top
    } else {
        Edge::Bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(device: &str, name: &str, w: i32, h: i32, primary: bool) -> MonitorInfo {
        let bounds = Rect::new(0, 0, w, h);
        MonitorInfo::new(device.into(), name.into(), bounds, bounds, 1.0, primary)
    }

    #[test]
    fn dock_strip_top_and_bottom() {
        let area = Rect::new(0, 0, 1920, 1040);
        assert_eq!(dock_strip(area, Edge::Top, 28), Rect::new(0, 0, 1920, 28));
        assert_eq!(
            dock_strip(area, Edge::Bottom, 28),
            Rect::new(0, 1012, 1920, 1040)
        );
        let second = Rect::new(1920, -200, 4480, 1240);
        assert_eq!(
            dock_strip(second, Edge::Top, 42),
            Rect::new(1920, -200, 4480, -158)
        );
    }

    #[test]
    fn keep_height_measures_from_docked_edge() {
        // The shell moved our bottom bar up above a 48 px taskbar and shrank it.
        let adjusted = Rect::new(0, 1012, 1920, 1032);
        assert_eq!(
            keep_height(adjusted, Edge::Bottom, 28),
            Rect::new(0, 1004, 1920, 1032)
        );
        let top = Rect::new(0, 48, 1920, 60);
        assert_eq!(keep_height(top, Edge::Top, 28), Rect::new(0, 48, 1920, 76));
    }

    #[test]
    fn physical_size_rounds() {
        assert_eq!(to_physical(28, 1.0), 28);
        assert_eq!(to_physical(28, 1.5), 42);
        assert_eq!(to_physical(25, 1.25), 31);
        assert_eq!(to_physical(25, 1.75), 44);
    }

    #[test]
    fn key_round_trip() {
        let m = mon(r"\\.\DISPLAY2", "DELL U2720Q", 2560, 1440, false);
        assert_eq!(m.key, r"\\.\DISPLAY2|DELL U2720Q|2560x1440");
        assert_eq!(
            parse_key(&m.key),
            (r"\\.\DISPLAY2", Some("DELL U2720Q"), Some((2560, 1440)))
        );
        assert_eq!(parse_key("a|b|c|10x20"), ("a", Some("b|c"), Some((10, 20))));
        assert_eq!(parse_key(r"\\.\DISPLAY1"), (r"\\.\DISPLAY1", None, None));
    }

    #[test]
    fn selects_primary() {
        let ms = [
            mon("D1", "A", 100, 100, false),
            mon("D2", "B", 200, 200, true),
        ];
        assert_eq!(select_monitor(&ms, "primary").unwrap().device, "D2");
    }

    #[test]
    fn selects_by_device_and_name() {
        let ms = [
            mon("D1", "A", 100, 100, true),
            mon("D2", "B", 200, 200, false),
        ];
        assert_eq!(select_monitor(&ms, "D2|B|200x200").unwrap().device, "D2");
    }

    #[test]
    fn follows_monitor_whose_device_name_changed() {
        // "B" moved from DISPLAY2 to DISPLAY3; DISPLAY2 is now another monitor.
        let ms = [
            mon("D1", "A", 100, 100, true),
            mon("D2", "C", 300, 300, false),
            mon("D3", "B", 200, 200, false),
        ];
        assert_eq!(select_monitor(&ms, "D2|B|200x200").unwrap().device, "D3");
    }

    #[test]
    fn falls_back_to_device_then_primary() {
        let ms = [
            mon("D1", "A", 100, 100, true),
            mon("D2", "X", 300, 300, false),
        ];
        assert_eq!(select_monitor(&ms, "D2|B|200x200").unwrap().device, "D2");
        assert_eq!(select_monitor(&ms, "D9|Z|1x1").unwrap().device, "D1");
        assert_eq!(select_monitor(&ms, r"D2").unwrap().device, "D2");
        assert!(select_monitor(&[], "primary").is_none());
    }

    /// A 1920 × 1080 primary at 100 % and a 2560 × 1440 monitor at 150 % to its right.
    fn two_monitors() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo::new(
                "A".into(),
                "Left".into(),
                Rect::new(0, 0, 1920, 1080),
                Rect::new(0, 0, 1920, 1040),
                1.0,
                true,
            ),
            MonitorInfo::new(
                "B".into(),
                "Right".into(),
                Rect::new(1920, 0, 4480, 1440),
                Rect::new(1920, 0, 4480, 1440),
                1.5,
                false,
            ),
        ]
    }

    #[test]
    fn floating_bar_starts_centred_near_the_edge() {
        let monitors = two_monitors();
        let (rect, m) =
            floating_rect(&monitors, "primary", None, Edge::Bottom, 28).expect("placed");
        assert_eq!(m.device, "A");
        assert_eq!(rect, Rect::new(560, 1000, 1360, 1028));
        let (rect, _) = floating_rect(&monitors, "primary", None, Edge::Top, 28).expect("placed");
        assert_eq!(rect.top, 12);
    }

    #[test]
    fn floating_bar_keeps_its_place_inside_its_monitor() {
        let monitors = two_monitors();
        let at = |x, y, width| {
            let f = FloatingRect { x, y, width };
            floating_rect(&monitors, "primary", Some(f), Edge::Bottom, 28)
                .map(|(r, m)| (r, m.device.clone()))
        };
        // On the right monitor, at its scale: 600 × 1.5 wide, 28 × 1.5 tall.
        assert_eq!(
            at(2000, 100, 600),
            Some((Rect::new(2000, 100, 2900, 142), "B".into()))
        );
        // Partly off the work area: pulled back in.
        assert_eq!(
            at(1500, 1030, 600),
            Some((Rect::new(1320, 1012, 1920, 1040), "A".into()))
        );
        // Mostly on the right monitor, though its left end is on the left one: on the right.
        assert_eq!(
            at(1700, 1030, 600),
            Some((Rect::new(1920, 1030, 2820, 1072), "B".into()))
        );
        // A hand-edited position far off: the nearest monitor, without overflowing.
        assert_eq!(
            at(i32::MIN, i32::MIN, 300).map(|(_, m)| m),
            Some("A".into())
        );
        assert_eq!(at(i32::MAX, 0, 300).map(|(_, m)| m), Some("B".into()));
        // Off every monitor: onto the nearest one.
        assert_eq!(at(-500, 500, 300).map(|(r, _)| r.left), Some(0));
        // Wider than the monitor: as wide as its work area.
        assert_eq!(at(0, 0, 5000).map(|(r, _)| r.width()), Some(1920));

        let rect = Rect::new(2000, 100, 2900, 142);
        assert_eq!(nearer_edge(rect, &monitors[1]), Edge::Top);
        assert_eq!(
            nearer_edge(Rect::new(0, 1000, 800, 1028), &monitors[0]),
            Edge::Bottom
        );
    }

    #[test]
    fn no_primary_flag_uses_first() {
        let ms = [mon("D1", "A", 100, 100, false)];
        assert_eq!(select_monitor(&ms, "primary").unwrap().device, "D1");
    }
}
