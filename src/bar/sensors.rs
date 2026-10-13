//! The sensors on the bar (SPEC §4.4): one instance per layout entry, measured, placed and drawn
//! on the bar thread. An entry whose type or props change gets a new instance; a sensor that
//! reports a change is re-measured, and the zones are placed again only if a width changed.

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use super::layout::{self, Zones};
use super::theme::Theme;
use crate::config::model::{Layout, SensorInstance, Settings, Zone};
use crate::platform::{Canvas, Capabilities, Platform, RectF, TextEngine};
use crate::sampler::{Delivery, TopicRequest};
use crate::sensors::registry::Registry;
use crate::sensors::{ClickResult, DrawCtx, HintContent, LayoutCtx, Sensor, SensorCtx, State};

/// A pressed sensor is drawn 40 % fainter (SPEC §6.1).
const PRESSED_OPACITY: f32 = 0.6;

struct Placed {
    entry: SensorInstance,
    sensor: Box<dyn Sensor>,
    /// What it subscribes to, asked once when it is created (SPEC §4.4).
    topics: Vec<TopicRequest>,
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
    /// Transitions animate (SPEC §5.5).
    pub animate: bool,
}

impl Env<'_> {
    fn ctx<'a>(&'a self, id: &'a str) -> SensorCtx<'a> {
        SensorCtx {
            id,
            settings: self.settings,
            theme: self.theme,
            locked: self.settings.locked,
            alt: self.alt,
            animate: self.animate,
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
    /// Ids of the sensors created since [`Self::take_created`] was last called.
    created: Vec<String>,
    /// The bar width, scale and the theme's padding and gap of the last layout.
    bar_width: f32,
    scale: f32,
    spacing: (f32, f32),
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
                        self.created.push(entry.id.clone());
                        let empty = State::new();
                        let state = state.get(&entry.id).unwrap_or(&empty);
                        let sensor = registry.create(entry, state, caps);
                        Placed {
                            entry: entry.clone(),
                            topics: sensor.topics(),
                            sensor,
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

    /// The ids of the sensors created (or re-created) since the last call.
    pub fn take_created(&mut self) -> Vec<String> {
        std::mem::take(&mut self.created)
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

    /// Measures the sensors that need it and places the zones if a width, the bar width, the
    /// scale or the spacing (Sensor spacing, SPEC §10.4) changed.
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
        let spacing = (cx.theme.pad, cx.theme.gap);
        if resized || self.relayout || bar_width != self.bar_width || spacing != self.spacing {
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
            self.spacing = spacing;
            self.relayout = false;
        }
    }

    /// Measures and places what changed now, with the scale and bar width of the last layout,
    /// rather than at the next paint; e.g. before a hint is placed next to its sensor.
    pub fn refresh_layout(&mut self, theme: &Theme, text: &dyn TextEngine) {
        if self.scale > 0.0 {
            let cx = LayoutCtx {
                theme,
                text,
                scale: self.scale,
            };
            self.update_layout(&cx, self.bar_width);
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

    /// Tells every sensor the wall clock jumped (SPEC §9.4–9.5); returns whether what any of
    /// them shows changed.
    pub fn clock_changed(&mut self, now: Instant, env: &Env) -> bool {
        let mut changed = false;
        for p in self.all_mut() {
            let dirty = p.sensor.on_clock_change(now, &env.ctx(&p.entry.id));
            p.dirty |= dirty;
            changed |= dirty;
        }
        changed
    }

    /// Tells sensor `id` the pointer entered or left it; returns whether what it shows changed.
    pub fn on_hover(&mut self, id: &str, hovered: bool, now: Instant, env: &Env) -> bool {
        let Some(p) = self.all_mut().find(|p| p.entry.id == id) else {
            return false;
        };
        let dirty = p.sensor.on_hover(hovered, now, &env.ctx(id));
        p.dirty |= dirty;
        dirty
    }

    /// The topics of each sensor that has any (SPEC §4.4).
    pub fn subscriptions(&self) -> HashMap<String, Vec<TopicRequest>> {
        self.zones
            .iter()
            .flatten()
            .filter(|p| !p.topics.is_empty())
            .map(|p| (p.entry.id.clone(), p.topics.clone()))
            .collect()
    }

    /// Hands a sample to the sensors it is for; returns whether any changed what it shows.
    pub fn deliver(&mut self, delivery: &Delivery, env: &Env) -> bool {
        let mut changed = false;
        for p in self.all_mut() {
            let addressed = delivery.to.as_ref().is_none_or(|to| *to == p.entry.id);
            if addressed && p.topics.iter().any(|r| r.topic == delivery.topic) {
                let cx = env.ctx(&p.entry.id);
                let dirty = p.sensor.on_sample(&delivery.topic, &delivery.sample, &cx);
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
    use crate::sampler::{Sample, Topic};
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
        // Placeholders (M2 types): label × 5 px + 8; the logo is 27 px wide. Time shows
        // nothing until its first wake-up reads the clock.
        assert_eq!(
            places,
            [
                (7.0, 53.0),   // cpu-graph
                (70.0, 68.0),  // memory-meter
                (148.0, 73.0), // network-graph
                (382.0, 27.0), // logo: round((800 − 37) / 2)
                (755.0, 28.0), // date
                (793.0, 0.0)   // time, ending 7 px from the right
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
            ["cpu-graph", "memory-meter", "network-graph", "date", "logo"],
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
        assert_eq!(sensors.take_created(), ["a", "b"]);
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        assert!(!sensors.sync(&layout, &BTreeMap::new(), &caps, registry()));
        assert!(sensors.take_created().is_empty());

        // A new text: re-created, re-measured, and the spacer moves.
        layout.left[0].props.insert("text".into(), json!("Longer"));
        assert!(sensors.sync(&layout, &BTreeMap::new(), &caps, registry()));
        assert_eq!(sensors.take_created(), ["a"], "same id, new instance");
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

        /// Counts from the new time, as a clock re-reads it.
        fn on_clock_change(&mut self, now: Instant, _cx: &SensorCtx) -> bool {
            self.start = now;
            true
        }
    }

    #[test]
    fn a_clock_change_reaches_every_sensor() {
        let theme = theme();
        let settings = Settings::default();
        let platform = crate::platform::current();
        let env = Env {
            settings: &settings,
            theme: &theme,
            platform: platform.as_ref(),
            alt: false,
            animate: true,
        };
        let start = Instant::now();
        let mut sensors = BarSensors::default();
        for (zone, id) in [(0, "a"), (2, "b")] {
            sensors.zones[zone].push(Placed {
                entry: entry(id, "ticker", json!({})),
                sensor: Box::new(Ticker {
                    start,
                    wakes: Rc::new(Cell::new(0)),
                }),
                topics: Vec::new(),
                width: 0.0,
                x: 0.0,
                dirty: false,
            });
        }
        let later = start + Duration::from_secs(3);
        assert!(sensors.clock_changed(later, &env));
        assert_eq!(
            sensors.next_wake(later),
            Some(later + Duration::from_secs(10))
        );
        assert!(sensors.all_mut().all(|p| p.dirty), "both re-measure");
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
            animate: true,
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
            topics: Vec::new(),
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

    /// Shows the uptime in whole minutes, from samples.
    struct Uptime {
        minutes: u64,
    }

    impl Sensor for Uptime {
        fn measure(&mut self, _cx: &LayoutCtx) -> f32 {
            10.0 + self.minutes as f32
        }

        fn draw(&self, _canvas: &mut dyn Canvas, _cx: &DrawCtx) {}

        fn topics(&self) -> Vec<TopicRequest> {
            vec![TopicRequest {
                topic: Topic::Uptime,
                interval: Duration::from_secs(60),
            }]
        }

        fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
            let Sample::Uptime(uptime) = sample;
            let minutes = uptime.uptime_secs / 60;
            let changed = minutes != self.minutes;
            self.minutes = minutes;
            changed
        }
    }

    #[test]
    fn delivers_samples_to_the_sensors_that_subscribe() {
        let theme = theme();
        let settings = Settings::default();
        let platform = crate::platform::current();
        let env = Env {
            settings: &settings,
            theme: &theme,
            platform: platform.as_ref(),
            alt: false,
            animate: true,
        };
        let mut sensors = BarSensors::default();
        for id in ["u1", "u2"] {
            let sensor: Box<dyn Sensor> = Box::new(Uptime { minutes: 0 });
            sensors.zones[0].push(Placed {
                entry: entry(id, "uptime", json!({})),
                topics: sensor.topics(),
                sensor,
                width: 0.0,
                x: 0.0,
                dirty: true,
            });
        }
        sensors.zones[2].push(Placed {
            entry: entry("t", "ticker", json!({})),
            sensor: Box::new(Ticker {
                start: Instant::now(),
                wakes: Rc::new(Cell::new(0)),
            }),
            topics: Vec::new(),
            width: 0.0,
            x: 0.0,
            dirty: true,
        });
        let mut ids: Vec<_> = sensors.subscriptions().into_keys().collect();
        ids.sort();
        assert_eq!(ids, ["u1", "u2"]);
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);

        let delivery = |secs, to: Option<&str>| Delivery {
            topic: Topic::Uptime,
            sample: std::sync::Arc::new(Sample::Uptime(crate::sampler::UptimeSample {
                boot_unix_ms: 0,
                uptime_secs: secs,
            })),
            to: to.map(str::to_owned),
        };
        // A catch-up reaches only its subscriber.
        assert!(sensors.deliver(&delivery(120, Some("u2")), &env));
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        let widths: Vec<f32> = sensors.places().iter().map(|p| p.2).collect();
        assert_eq!(widths, [10.0, 12.0, 10.0]);
        // Every subscriber gets a broadcast; one whose minute is unchanged stays clean.
        assert!(sensors.deliver(&delivery(150, None), &env));
        assert!(!sensors.deliver(&delivery(170, None), &env));
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        let widths: Vec<f32> = sensors.places().iter().map(|p| p.2).collect();
        assert_eq!(widths, [12.0, 12.0, 10.0]);
    }

    #[test]
    fn a_new_spacing_places_the_sensors_again() {
        let mut theme = theme();
        let config = Config::first_run("1.0.0");
        let mut sensors = BarSensors::default();
        sensors.sync(
            &config.layout,
            &config.state,
            &Capabilities::default(),
            registry(),
        );
        sensors.update_layout(&layout_ctx(&theme, 1.0), 800.0);
        let xs =
            |sensors: &BarSensors| -> Vec<f32> { sensors.places().iter().map(|p| p.1).collect() };
        let before = xs(&sensors);
        // A theme change marks every sensor dirty; the widths stay, the gap does not.
        theme.gap += 4.0;
        sensors.invalidate_all();
        sensors.update_layout(&layout_ctx(&theme, 1.0), 800.0);
        let after = xs(&sensors);
        assert_eq!(after[1] - before[1], 4.0, "{before:?} → {after:?}");
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
