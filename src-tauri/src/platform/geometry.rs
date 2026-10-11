//! Pure placement geometry shared by every platform: docking strips, logical→physical sizes and
//! monitor selection (SPEC §5.1–5.2). No OS calls, fully unit-tested.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config::model::{Edge, PRIMARY_MONITOR};

/// A rectangle in physical pixels, right/bottom exclusive (like Win32 `RECT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
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
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

pub fn monitor_key(device: &str, name: &str, width: i32, height: i32) -> String {
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

    #[test]
    fn no_primary_flag_uses_first() {
        let ms = [mon("D1", "A", 100, 100, false)];
        assert_eq!(select_monitor(&ms, "primary").unwrap().device, "D1");
    }
}
