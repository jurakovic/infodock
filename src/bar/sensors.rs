//! The sensors on the bar (SPEC §4.4): one instance per layout entry, measured, placed and drawn
//! on the bar thread. An entry whose type or props change gets a new instance; a sensor that
//! reports a change is re-measured, and the zones are placed again only if a width changed.

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use super::layout::{self, Zones};
use super::theme::Theme;
use crate::config::model::{Layout, SensorInstance, Settings, Zone};
use crate::platform::{Canvas, Capabilities, Platform, RectF};
use crate::sensors::registry::Registry;
use crate::sensors::{ClickResult, DrawCtx, HintContent, LayoutCtx, Sensor, SensorCtx, State};

/// A pressed sensor is drawn 40 % fainter (SPEC §6.1).
const PRESSED_OPACITY: f32 = 0.6;

struct Placed {
    entry: SensorInstance,
    sensor: Box<dyn Sensor>,
    /// Logical px, from the last measurement.
    width: f32,
    /// Logical px from the bar's left edge.
    x: f32,
    /// Needs measuring.
    dirty: bool,
}

/// What a sensor's context is built from.
pub(super) struct Env<'a> {
    pub settings: &'a Settings,
    pub theme: &'a Theme,
    pub platform: &'a dyn Platform,
    /// Alt is held.
    pub alt: bool,
}

impl Env<'_> {
    fn ctx<'a>(&'a self, id: &'a str) -> SensorCtx<'a> {
        SensorCtx {
            id,
            settings: self.settings,
            theme: self.theme,
            locked: self.settings.locked,
            alt: self.alt,
            platform: self.platform,
        }
    }
}

/// A sensor under the pointer.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Hit {
    pub zone: Zone,
    pub index: usize,
    pub entry: SensorInstance,
}

const ZONES: [Zone; 3] = [Zone::Left, Zone::Center, Zone::Right];

#[derive(Default)]
pub(super) struct BarSensors {
    /// Left, centre, right; each in visual order.
    zones: [Vec<Placed>; 3],
    /// Drawn dimmed while the pointer presses it (SPEC §6.1).
    pressed: Option<String>,
    /// The zones must be placed again.
    relayout: bool,
    /// The bar width and scale of the last layout.
    bar_width: f32,
    scale: f32,
}

impl BarSensors {
    /// Brings the instances in line with `layout`: entries with the same id, type and props
    /// keep their sensor, the others get a new one. Returns whether anything changed.
    pub fn sync(
        &mut self,
        layout: &Layout,
        state: &BTreeMap<String, State>,
        caps: &Capabilities,
        registry: &Registry,
    ) -> bool {
        let before: Vec<Vec<String>> = self.ids();
        let mut old: HashMap<String, Placed> = self
            .zones
            .iter_mut()
            .flat_map(std::mem::take)
            .map(|p| (p.entry.id.clone(), p))
            .collect();
        let mut created = false;
        let entries = [&layout.left, &layout.center, &layout.right];
        for (zone, entries) in self.zones.iter_mut().zip(entries) {
            for entry in entries {
                let placed = match old.remove(&entry.id) {
                    Some(p) if p.entry.kind == entry.kind && p.entry.props == entry.props => p,
                    _ => {
                        created = true;
                        let empty = State::new();
                        let state = state.get(&entry.id).unwrap_or(&empty);
                        Placed {
                            entry: entry.clone(),
                            sensor: registry.create(entry, state, caps),
                            width: 0.0,
                            x: 0.0,
                            dirty: true,
                        }
                    }
                };
                zone.push(placed);
            }
        }
        let changed = created || self.ids() != before;
        self.relayout |= changed;
        changed
    }

    fn ids(&self) -> Vec<Vec<String>> {
        self.zones
            .iter()
            .map(|z| z.iter().map(|p| p.entry.id.clone()).collect())
            .collect()
    }

    fn all_mut(&mut self) -> impl Iterator<Item = &mut Placed> {
        self.zones.iter_mut().flatten()
    }

    /// Every sensor measures again, e.g. after the theme changed.
    pub fn invalidate_all(&mut self) {
        for p in self.all_mut() {
            p.dirty = true;
        }
    }

    /// Measures the sensors that need it and places the zones if a width, the bar width or the
    /// scale changed.
    pub fn update_layout(&mut self, cx: &LayoutCtx, bar_width: f32) {
        if cx.scale != self.scale {
            // Images are decoded at their physical size.
            self.invalidate_all();
            self.scale = cx.scale;
        }
        let mut resized = false;
        for p in self.all_mut().filter(|p| p.dirty) {
            let width = p.sensor.measure(cx).max(0.0);
            resized |= width != p.width;
            p.width = width;
            p.dirty = false;
        }
        if resized || self.relayout || bar_width != self.bar_width {
            let widths: Zones = self
                .zones
                .each_ref()
                .map(|z| z.iter().map(|p| p.width).collect());
            let xs = layout::place(&widths, bar_width, cx.theme.pad, cx.theme.gap);
            for (zone, xs) in self.zones.iter_mut().zip(xs) {
                for (p, x) in zone.iter_mut().zip(xs) {
                    p.x = x;
                }
            }
            self.bar_width = bar_width;
            self.relayout = false;
        }
    }

    /// Draws every sensor in its place: left, right, then the centre on top (SPEC §5.4). A
    /// pressed sensor is dimmed by 40 % (§6.1).
    pub fn draw(&self, canvas: &mut dyn Canvas, theme: &Theme, bar_height: f32) {
        for zone in [0, 2, 1] {
            for p in &self.zones[zone] {
                let rect = RectF::new(p.x, 0.0, p.x + p.width, bar_height);
                let pressed = self.pressed.as_deref() == Some(p.entry.id.as_str());
                if pressed {
                    canvas.push_opacity(PRESSED_OPACITY);
                }
                p.sensor.draw(canvas, &DrawCtx { theme, rect });
                if pressed {
                    canvas.pop();
                }
            }
        }
    }

    /// The sensor at `x`; the centre zone first, since it is drawn on top.
    pub fn hit(&self, x: f32) -> Option<Hit> {
        [1, 0, 2].into_iter().find_map(|zone| {
            let index = self.zones[zone]
                .iter()
                .position(|p| x >= p.x && x < p.x + p.width)?;
            Some(Hit {
                zone: ZONES[zone],
                index,
                entry: self.zones[zone][index].entry.clone(),
            })
        })
    }

    /// How many sensors `zone` has.
    pub fn len(&self, zone: Zone) -> usize {
        self.zones[zone as usize].len()
    }

    /// The bar width of the last layout, in logical px.
    pub fn bar_width(&self) -> f32 {
        self.bar_width
    }

    /// Dims sensor `id` (or none); returns whether that changed.
    pub fn set_pressed(&mut self, id: Option<&str>) -> bool {
        if self.pressed.as_deref() == id {
            return false;
        }
        self.pressed = id.map(str::to_owned);
        true
    }

    /// Each zone's sensors as `(x, width)`, logical px, in visual order.
    pub fn spans(&self) -> [Vec<(f32, f32)>; 3] {
        self.zones
            .each_ref()
            .map(|z| z.iter().map(|p| (p.x, p.width)).collect())
    }

    /// The zone and index of sensor `id`.
    pub fn find(&self, id: &str) -> Option<(Zone, usize)> {
        ZONES.into_iter().find_map(|zone| {
            let index = self.zones[zone as usize]
                .iter()
                .position(|p| p.entry.id == id)?;
            Some((zone, index))
        })
    }

    /// Where sensor `id` is: its x and width, logical px.
    pub fn place_of(&self, id: &str) -> Option<(f32, f32)> {
        self.zones
            .iter()
            .flatten()
            .find(|p| p.entry.id == id)
            .map(|p| (p.x, p.width))
    }

    /// Sensor `id`'s hint.
    pub fn hint(&self, id: &str, env: &Env) -> Option<HintContent> {
        let p = self.zones.iter().flatten().find(|p| p.entry.id == id)?;
        p.sensor.hint(&env.ctx(id))
    }

    /// Clicks sensor `id`; returns whether it changed what it shows.
    pub fn click(&mut self, id: &str, env: &Env) -> ClickResult {
        let Some(p) = self.all_mut().find(|p| p.entry.id == id) else {
            return ClickResult::None;
        };
        p.sensor.click(&env.ctx(id))
    }

    /// The earliest wake-up any sensor asked for.
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        self.zones
            .iter()
            .flatten()
            .filter_map(|p| p.sensor.wake_at(now))
            .min()
    }

    /// Wakes the sensors that are due; returns whether any changed what it shows.
    pub fn wake(&mut self, now: Instant, env: &Env) -> bool {
        let mut changed = false;
        for p in self.all_mut() {
            if p.sensor.wake_at(now).is_some_and(|at| at <= now) {
                let dirty = p.sensor.on_wake(now, &env.ctx(&p.entry.id));
                p.dirty |= dirty;
                changed |= dirty;
            }
        }
        changed
    }

    /// Each sensor's id and place, in drawing order; for tests.
    #[cfg(test)]
    fn places(&self) -> Vec<(String, f32, f32)> {
        self.zones
            .iter()
            .flatten()
            .map(|p| (p.entry.id.clone(), p.x, p.width))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::config::model::Config;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sensors::registry::registry;
    use crate::sensors::test_support::theme;

    fn entry(id: &str, kind: &str, props: serde_json::Value) -> SensorInstance {
        SensorInstance {
            id: id.into(),
            kind: kind.into(),
            props: props.as_object().cloned().unwrap_or_default(),
        }
    }

    fn layout_ctx<'a>(theme: &'a Theme, scale: f32) -> LayoutCtx<'a> {
        LayoutCtx {
            theme,
            text: &FixedMetrics,
            scale,
        }
    }

    #[test]
    fn lays_out_and_draws_the_first_run_layout() {
        let theme = theme();
        let config = Config::first_run("1.0.0");
        let mut sensors = BarSensors::default();
        assert!(sensors.sync(
            &config.layout,
            &config.state,
            &Capabilities::default(),
            registry()
        ));
        sensors.update_layout(&layout_ctx(&theme, 1.0), 800.0);
        let places: Vec<(f32, f32)> = sensors.places().iter().map(|p| (p.1, p.2)).collect();
        // Placeholders (M2 types): label × 5 px + 8; the logo is 27 px wide.
        assert_eq!(
            places,
            [
                (7.0, 53.0),   // cpu-graph
                (70.0, 68.0),  // memory-meter
                (148.0, 73.0), // network-graph
                (382.0, 27.0), // logo: round((800 − 37) / 2)
                (727.0, 28.0), // date
                (765.0, 28.0)  // time, ending 7 px from the right
            ]
        );
        let mut canvas = RecordingCanvas::new(1.0);
        sensors.draw(&mut canvas, &theme, 28.0);
        let dump = canvas.dump();
        let order: Vec<&str> = dump
            .lines()
            .filter_map(|l| {
                l.split('"')
                    .nth(1)
                    .or(l.starts_with("bitmap").then_some("logo"))
            })
            .collect();
        assert_eq!(
            order,
            [
                "cpu-graph",
                "memory-meter",
                "network-graph",
                "date",
                "time",
                "logo"
            ],
            "the centre zone is drawn last"
        );
    }

    #[test]
    fn keeps_unchanged_sensors_and_recreates_changed_ones() {
        let theme = theme();
        let caps = Capabilities::default();
        let mut layout = Layout {
            left: vec![
                entry("a", "text", json!({ "text": "One" })),
                entry("b", "spacer", json!({})),
            ],
            ..Layout::default()
        };
        let mut sensors = BarSensors::default();
        sensors.sync(&layout, &BTreeMap::new(), &caps, registry());
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        assert!(!sensors.sync(&layout, &BTreeMap::new(), &caps, registry()));

        // A new text: re-created, re-measured, and the spacer moves.
        layout.left[0].props.insert("text".into(), json!("Longer"));
        assert!(sensors.sync(&layout, &BTreeMap::new(), &caps, registry()));
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        assert_eq!(sensors.places()[1], ("b".into(), 56.0, 9.0));

        // Moving a sensor to another zone is a change too.
        let b = layout.left.remove(1);
        layout.right.push(b);
        assert!(sensors.sync(&layout, &BTreeMap::new(), &caps, registry()));
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        assert_eq!(sensors.places()[1], ("b".into(), 384.0, 9.0));

        layout.right.clear();
        assert!(sensors.sync(&layout, &BTreeMap::new(), &caps, registry()));
        assert_eq!(sensors.places().len(), 1);
    }

    /// Counts its wake-ups; due every 10 s from `start`.
    struct Ticker {
        start: Instant,
        wakes: Rc<Cell<u32>>,
    }

    impl Sensor for Ticker {
        fn measure(&mut self, _cx: &LayoutCtx) -> f32 {
            10.0 + self.wakes.get() as f32
        }

        fn draw(&self, _canvas: &mut dyn Canvas, _cx: &DrawCtx) {}

        fn wake_at(&self, _now: Instant) -> Option<Instant> {
            Some(self.start + Duration::from_secs(10 * u64::from(self.wakes.get() + 1)))
        }

        fn on_wake(&mut self, _now: Instant, _cx: &SensorCtx) -> bool {
            self.wakes.set(self.wakes.get() + 1);
            true
        }
    }

    #[test]
    fn wakes_due_sensors_and_remeasures_them() {
        let theme = theme();
        let settings = Settings::default();
        let platform = crate::platform::current();
        let env = Env {
            settings: &settings,
            theme: &theme,
            platform: platform.as_ref(),
            alt: false,
        };
        let start = Instant::now();
        let wakes = Rc::new(Cell::new(0));
        let mut sensors = BarSensors::default();
        sensors.zones[0].push(Placed {
            entry: entry("t", "ticker", json!({})),
            sensor: Box::new(Ticker {
                start,
                wakes: Rc::clone(&wakes),
            }),
            width: 0.0,
            x: 0.0,
            dirty: true,
        });
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        assert_eq!(sensors.places()[0].2, 10.0);

        let first = start + Duration::from_secs(10);
        assert_eq!(sensors.next_wake(start), Some(first));
        assert!(
            !sensors.wake(first - Duration::from_millis(5), &env),
            "not due yet"
        );
        assert!(sensors.wake(first, &env));
        assert_eq!(wakes.get(), 1);
        assert_eq!(
            sensors.next_wake(first),
            Some(start + Duration::from_secs(20))
        );
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        assert_eq!(sensors.places()[0].2, 11.0, "re-measured after waking");
    }

    #[test]
    fn a_new_scale_remeasures_everything() {
        let theme = theme();
        let layout = Layout {
            center: vec![entry("logo", "image", json!({}))],
            ..Layout::default()
        };
        let mut sensors = BarSensors::default();
        sensors.sync(
            &layout,
            &BTreeMap::new(),
            &Capabilities::default(),
            registry(),
        );
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        sensors.update_layout(&layout_ctx(&theme, 1.5), 400.0);
        let mut canvas = RecordingCanvas::new(1.5);
        sensors.draw(&mut canvas, &theme, 28.0);
        assert!(
            canvas.dump().starts_with("bitmap 41×41"),
            "{}",
            canvas.dump()
        );
    }
}
