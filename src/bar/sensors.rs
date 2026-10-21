//! The sensors on the bar (SPEC §4.4): one instance per layout entry, measured, placed and drawn
//! on the bar thread. An entry whose type or props change gets a new instance; a sensor that
//! reports a change is re-measured, and the zones are placed again only if a width changed.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use super::layout::{self, Zones};
use super::theme::Theme;
use crate::config::model::{Layout, SensorInstance, Settings, Zone};
use crate::platform::{Brush, Canvas, Capabilities, Color, Platform, RectF, Shape, TextEngine};
use crate::sampler::{ChangeSink, Delivery, Topic, TopicRequest};
use crate::sensors::drop::{DragInfo, DropAction, Dropped};
use crate::sensors::format::Numbers;
use crate::sensors::registry::Registry;
use crate::sensors::{
    ClickResult, DrawCtx, HintContent, IconSource, LayoutCtx, MenuId, MenuItem, Props, Sensor,
    SensorCtx, State,
};

/// A pressed sensor is drawn 40 % fainter (SPEC §6.1).
const PRESSED_OPACITY: f32 = 0.6;
/// The highlight behind a sensor whose popup is open (SPEC §7.2): the text colour this faint,
/// reaching this far beyond the sensor, with these corners.
const ACTIVE_ALPHA: f32 = 0.16;
const ACTIVE_REACH: f32 = 3.0;
const ACTIVE_RADIUS: f32 = 4.0;
/// The target of a drop has its background darkened (SPEC §9.19).
const DROP_TARGET: Color = Color::rgba(0, 0, 0, 0.78);
/// A broadcast sample this much before a sensor's interval is over still counts as on time,
/// at most (samples come on a grid, give or take a few ms).
const EARLY: Duration = Duration::from_millis(500);

struct Placed {
    entry: SensorInstance,
    sensor: Box<dyn Sensor>,
    /// What it subscribes to, asked once when it is created (SPEC §4.4).
    topics: Vec<TopicRequest>,
    /// When it last took a sample of each topic.
    taken: Vec<(Topic, Instant)>,
    /// Logical px, from the last measurement, and how narrow it can be (SPEC §5.4).
    measured: f32,
    min: f32,
    /// What the layout gives it, logical px: `measured`, or less when the zones do not fit.
    width: f32,
    /// The width it was last fitted to; measuring fits it to `measured`.
    fitted: f32,
    /// Logical px from the bar's left edge.
    x: f32,
    /// Needs measuring.
    dirty: bool,
}

impl Placed {
    fn new(entry: SensorInstance, sensor: Box<dyn Sensor>) -> Self {
        Self {
            entry,
            topics: sensor.topics(),
            sensor,
            taken: Vec::new(),
            measured: 0.0,
            min: 0.0,
            width: 0.0,
            fitted: 0.0,
            x: 0.0,
            dirty: true,
        }
    }
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
    /// Where sensors report what their actions changed (SPEC §4.4).
    pub changes: &'a ChangeSink,
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
            changes: self.changes,
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
    /// Drawn with a highlight behind it while its popup is open (SPEC §7.2).
    active: Option<String>,
    /// Drawn on a dark background while a drag from another app would drop onto it (§6.3).
    drop_target: Option<String>,
    /// The zones must be placed again.
    relayout: bool,
    /// Ids of the sensors created since [`Self::take_created`] was last called.
    created: Vec<String>,
    /// The bar width, scale, numbers and the theme's padding and gap of the last layout.
    bar_width: f32,
    scale: f32,
    numbers: Numbers,
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
        let mut reconfigured = false;
        let entries = [&layout.left, &layout.center, &layout.right];
        for (zone, entries) in self.zones.iter_mut().zip(entries) {
            for entry in entries {
                // Same props: kept. Changed props: kept if the sensor takes them in place.
                let kept = old
                    .remove(&entry.id)
                    .filter(|p| p.entry.kind == entry.kind)
                    .and_then(|mut p| {
                        if p.entry.props == entry.props {
                            return Some(p);
                        }
                        if !registry.reconfigure(entry, p.sensor.as_mut(), caps) {
                            return None;
                        }
                        reconfigured = true;
                        p.entry = entry.clone();
                        p.topics = p.sensor.topics();
                        p.dirty = true;
                        Some(p)
                    });
                let placed = kept.unwrap_or_else(|| {
                    created = true;
                    self.created.push(entry.id.clone());
                    let empty = State::new();
                    let state = state.get(&entry.id).unwrap_or(&empty);
                    Placed::new(entry.clone(), registry.create(entry, state, caps))
                });
                zone.push(placed);
            }
        }
        let changed = created || reconfigured || self.ids() != before;
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
    /// scale or the spacing (Sensor spacing, SPEC §10.4) changed; sensors that must give way
    /// for the zones to fit the bar are narrowed (§5.4).
    pub fn update_layout(&mut self, cx: &LayoutCtx, bar_width: f32) {
        if cx.scale != self.scale || cx.numbers != self.numbers {
            // Images are decoded at their physical size; numbers are written as the locale
            // writes them.
            self.invalidate_all();
            self.scale = cx.scale;
            self.numbers = cx.numbers;
        }
        let mut resized = false;
        for p in self.all_mut().filter(|p| p.dirty) {
            let measured = p.sensor.measure(cx).max(0.0);
            let min = p
                .sensor
                .min_width()
                .map_or(measured, |m| m.clamp(0.0, measured));
            resized |= measured != p.measured || min != p.min;
            p.measured = measured;
            p.min = min;
            p.fitted = measured;
            p.dirty = false;
        }
        let spacing = (cx.theme.pad, cx.theme.gap);
        if resized || self.relayout || bar_width != self.bar_width || spacing != self.spacing {
            let (pad, gap) = spacing;
            let of = |f: fn(&Placed) -> f32| -> Zones {
                self.zones.each_ref().map(|z| z.iter().map(f).collect())
            };
            let widths = layout::fit(&of(|p| p.measured), &of(|p| p.min), bar_width, pad, gap);
            let xs = layout::place(&widths, bar_width, pad, gap);
            for ((zone, widths), xs) in self.zones.iter_mut().zip(widths).zip(xs) {
                for ((p, width), x) in zone.iter_mut().zip(widths).zip(xs) {
                    p.width = width;
                    p.x = x;
                }
            }
            self.bar_width = bar_width;
            self.spacing = spacing;
            self.relayout = false;
        }
        for p in self.all_mut().filter(|p| p.fitted != p.width) {
            p.sensor.fit(p.width, cx);
            p.fitted = p.width;
        }
    }

    /// Measures and places what changed now, with the scale and bar width of the last layout,
    /// rather than at the next paint; e.g. before a hint is placed next to its sensor.
    pub fn refresh_layout(&mut self, theme: &Theme, text: &dyn TextEngine, icons: &dyn IconSource) {
        if self.scale > 0.0 {
            let cx = LayoutCtx {
                theme,
                text,
                scale: self.scale,
                numbers: self.numbers,
                icons,
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
                if self.drop_target.as_deref() == Some(p.entry.id.as_str()) {
                    let x = (
                        canvas.snap(rect.left - ACTIVE_REACH),
                        canvas.snap(rect.right + ACTIVE_REACH),
                    );
                    canvas.fill(
                        &Shape::RoundedRect {
                            rect: RectF::new(x.0, 2.0, x.1, bar_height - 2.0),
                            radius: ACTIVE_RADIUS,
                        },
                        &Brush::Solid(DROP_TARGET),
                    );
                }
                if self.active.as_deref() == Some(p.entry.id.as_str()) {
                    let x = (
                        canvas.snap(rect.left - ACTIVE_REACH),
                        canvas.snap(rect.right + ACTIVE_REACH),
                    );
                    canvas.fill(
                        &Shape::RoundedRect {
                            rect: RectF::new(x.0, 2.0, x.1, bar_height - 2.0),
                            radius: ACTIVE_RADIUS,
                        },
                        &Brush::Solid(theme.sensor_text.with_alpha(ACTIVE_ALPHA)),
                    );
                }
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

    /// Highlights sensor `id` (or none) as the one whose popup is open; returns whether that
    /// changed.
    pub fn set_active(&mut self, id: Option<&str>) -> bool {
        if self.active.as_deref() == id {
            return false;
        }
        self.active = id.map(str::to_owned);
        true
    }

    /// Darkens sensor `id` (or none) as the target of a drop; returns whether that changed.
    pub fn set_drop_target(&mut self, id: Option<&str>) -> bool {
        if self.drop_target.as_deref() == id {
            return false;
        }
        self.drop_target = id.map(str::to_owned);
        true
    }

    /// What dropping `items` onto sensor `id` would do; `None` when it does not take them.
    pub fn drop_action(
        &self,
        id: &str,
        items: &[Dropped],
        drag: DragInfo,
        env: &Env,
    ) -> Option<DropAction> {
        let p = self.zones.iter().flatten().find(|p| p.entry.id == id)?;
        p.sensor.drop_action(items, drag, &env.ctx(id))
    }

    /// Drops `items` onto sensor `id`.
    pub fn drop_items(&mut self, id: &str, items: &[Dropped], drag: DragInfo, env: &Env) {
        if let Some(p) = self.all_mut().find(|p| p.entry.id == id) {
            p.sensor.drop_items(items, drag, &env.ctx(id));
        }
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

    /// Whether sensor `id` changed what it shows since it was last measured.
    pub fn is_dirty(&self, id: &str) -> bool {
        self.zones
            .iter()
            .flatten()
            .any(|p| p.entry.id == id && p.dirty)
    }

    /// Sensor `id` as [`Self::hit`] finds it.
    pub fn get(&self, id: &str) -> Option<Hit> {
        let (zone, index) = self.find(id)?;
        Some(Hit {
            zone,
            index,
            entry: self.zones[zone as usize][index].entry.clone(),
        })
    }

    /// The ids of the sensors the keyboard focus moves between (SPEC §14.4), in visual order:
    /// the zones from left to right, each in its order.
    pub fn focus_order(&self) -> Vec<String> {
        self.zones
            .iter()
            .flatten()
            .filter(|p| p.sensor.focusable())
            .map(|p| p.entry.id.clone())
            .collect()
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

    /// Clicks sensor `id`.
    pub fn click(&mut self, id: &str, env: &Env) -> ClickResult {
        let Some(p) = self.all_mut().find(|p| p.entry.id == id) else {
            return ClickResult::None;
        };
        let result = p.sensor.click(&env.ctx(id));
        // What it shows may have changed, e.g. a Battery switching to the time left.
        p.dirty |= result == ClickResult::Handled;
        result
    }

    /// Sensor `id`'s own menu items (SPEC §6.4).
    pub fn menu(&self, id: &str, env: &Env) -> Vec<MenuItem> {
        self.zones
            .iter()
            .flatten()
            .find(|p| p.entry.id == id)
            .map(|p| p.sensor.menu(&env.ctx(id)))
            .unwrap_or_default()
    }

    /// Hands sensor `id` the menu item it offered and the user chose; returns whether what it
    /// shows changed.
    pub fn on_menu(&mut self, id: &str, item: MenuId, env: &Env) -> bool {
        let Some(p) = self.all_mut().find(|p| p.entry.id == id) else {
            return false;
        };
        let dirty = p.sensor.on_menu(item, &env.ctx(id));
        p.dirty |= dirty;
        dirty
    }

    /// Tells sensor `id` the user agreed to its click's question; returns whether what it
    /// shows changed.
    pub fn confirmed(&mut self, id: &str, env: &Env) -> bool {
        let Some(p) = self.all_mut().find(|p| p.entry.id == id) else {
            return false;
        };
        let dirty = p.sensor.confirmed(&env.ctx(id));
        p.dirty |= dirty;
        dirty
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

    /// The state sensors want stored (SPEC §11.2), by id.
    pub fn take_states(&mut self) -> Vec<(String, State)> {
        self.all_mut()
            .filter_map(|p| p.sensor.take_state().map(|s| (p.entry.id.clone(), s)))
            .collect()
    }

    /// The props sensors changed themselves and want saved (SPEC §9.14), by id.
    pub fn take_props(&mut self) -> Vec<(String, Props)> {
        self.all_mut()
            .filter_map(|p| p.sensor.take_props().map(|c| (p.entry.id.clone(), c)))
            .collect()
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

    /// Hands a sample that arrived `now` to the sensors it is for; returns whether any
    /// changed what it shows. A topic is sampled at the shortest interval any subscriber asked
    /// for, so a broadcast reaches a sensor only once its own interval has passed since the
    /// last sample it took (a graph keeps its time scale); a catch-up always does (SPEC §4.4).
    pub fn deliver(&mut self, delivery: &Delivery, env: &Env, now: Instant) -> bool {
        let mut changed = false;
        for p in self.all_mut() {
            let addressed = delivery.to.as_ref().is_none_or(|to| *to == p.entry.id);
            let Some(request) = p.topics.iter().find(|r| r.topic == delivery.topic) else {
                continue;
            };
            let last = p.taken.iter_mut().find(|(t, _)| *t == delivery.topic);
            let early = request.interval.mul_f32(0.25).min(EARLY);
            let on_time = delivery.to.is_some()
                || delivery.pushed
                || last.as_ref().is_none_or(|(_, at)| {
                    now.saturating_duration_since(*at) + early >= request.interval
                });
            if !addressed || !on_time {
                continue;
            }
            match last {
                Some((_, at)) => *at = now,
                None => p.taken.push((delivery.topic.clone(), now)),
            }
            let cx = env.ctx(&p.entry.id);
            let dirty = p.sensor.on_sample(&delivery.topic, &delivery.sample, &cx);
            p.dirty |= dirty;
            changed |= dirty;
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
            numbers: Default::default(),
            icons: &crate::sensors::NoIcons,
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
        // Fixed metrics: 5 px per character at 10 px; the logo is 27 px wide. Date and Time
        // show nothing until their first wake-up reads the clock.
        assert_eq!(
            places,
            [
                (7.0, 75.0),    // CPU Graph: 50 px plot, 5 px gap, "100%"
                (92.0, 28.0),   // Memory Meter: the bar, the gap and "100%"
                (130.0, 100.0), // Network Graph: 50 px plot, 5 px gap, "1023 MB/s"
                (382.0, 27.0),  // logo: round((800 − 37) / 2)
                (783.0, 0.0),   // date
                (793.0, 0.0)    // time, ending 7 px from the right
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
            ["CPU", "", "RAM", "", "", "", "logo"],
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

    #[test]
    fn text_gives_way_while_the_zones_do_not_fit_the_bar() {
        let theme = theme();
        let layout = Layout {
            left: vec![entry("a", "text", json!({ "text": "Left side" }))],
            center: vec![entry(
                "b",
                "text",
                json!({ "text": "A rather long centre text" }),
            )],
            right: vec![entry("c", "spacer", json!({ "width": 40 }))],
        };
        let mut sensors = BarSensors::default();
        sensors.sync(
            &layout,
            &BTreeMap::new(),
            &Capabilities::default(),
            registry(),
        );
        let centre = |sensors: &BarSensors| {
            let mut canvas = RecordingCanvas::new(1.0);
            sensors.draw(&mut canvas, &theme, 28.0);
            let dump = canvas.dump();
            dump.lines().last().unwrap_or_default().to_owned()
        };
        // 13 px text: 6.5 px a character. On 200 px the centre may start at 76, after the left
        // and its gap, so it gets 200 − 10 − 2 × 76 = 38 px of its 163.
        sensors.update_layout(&layout_ctx(&theme, 1.0), 200.0);
        assert_eq!(
            sensors.places(),
            [
                ("a".into(), 7.0, 59.0),
                ("b".into(), 76.0, 38.0),
                ("c".into(), 153.0, 40.0)
            ]
        );
        assert!(centre(&sensors).contains("\"A ra…\" "), "cut");
        // Measured again, it stays cut.
        sensors.invalidate_all();
        sensors.update_layout(&layout_ctx(&theme, 1.0), 200.0);
        assert!(centre(&sensors).contains("\"A ra…\" "));
        // With room, it is whole again.
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        assert_eq!(sensors.places()[1], ("b".into(), 114.0, 163.0));
        assert!(centre(&sensors).contains("\"A rather long centre text\""));
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
            changes: &crate::sampler::ChangeSink::NONE,
        };
        let start = Instant::now();
        let mut sensors = BarSensors::default();
        for (zone, id) in [(0, "a"), (2, "b")] {
            sensors.zones[zone].push(Placed {
                dirty: false,
                ..Placed::new(
                    entry(id, "ticker", json!({})),
                    Box::new(Ticker {
                        start,
                        wakes: Rc::new(Cell::new(0)),
                    }),
                )
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
            changes: &crate::sampler::ChangeSink::NONE,
        };
        let start = Instant::now();
        let wakes = Rc::new(Cell::new(0));
        let mut sensors = BarSensors::default();
        sensors.zones[0].push(Placed::new(
            entry("t", "ticker", json!({})),
            Box::new(Ticker {
                start,
                wakes: Rc::clone(&wakes),
            }),
        ));
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

    /// Offers *Count* (and a greyed-out *Never*); counts how often it was chosen.
    struct Counter(u32);

    impl Sensor for Counter {
        fn measure(&mut self, _cx: &LayoutCtx) -> f32 {
            10.0
        }

        fn draw(&self, _canvas: &mut dyn Canvas, _cx: &DrawCtx) {}

        fn menu(&self, _cx: &SensorCtx) -> Vec<MenuItem> {
            vec![
                MenuItem::new(7, format!("Count ({})", self.0)),
                MenuItem::new(8, "Never").enabled(false),
            ]
        }

        fn on_menu(&mut self, item: MenuId, _cx: &SensorCtx) -> bool {
            self.0 += u32::from(item == 7);
            item == 7
        }
    }

    #[test]
    fn menu_items_come_from_the_sensor_and_go_back_to_it() {
        let theme = theme();
        let settings = Settings::default();
        let platform = crate::platform::current();
        let env = Env {
            settings: &settings,
            theme: &theme,
            platform: platform.as_ref(),
            alt: false,
            animate: true,
            changes: &crate::sampler::ChangeSink::NONE,
        };
        let mut sensors = BarSensors::default();
        sensors.zones[0].push(Placed {
            dirty: false,
            ..Placed::new(entry("c", "counter", json!({})), Box::new(Counter(0)))
        });
        assert_eq!(sensors.menu("c", &env)[0].label, "Count (0)");
        assert!(sensors.menu("nobody", &env).is_empty());
        assert!(sensors.on_menu("c", 7, &env));
        assert!(sensors.is_dirty("c"), "measured again");
        assert!(!sensors.on_menu("nobody", 7, &env));
        assert_eq!(sensors.menu("c", &env)[0].label, "Count (1)");
    }

    #[test]
    fn a_sensor_saves_its_own_props_and_takes_them_back_in_place() {
        let theme = theme();
        let settings = Settings::default();
        let platform = crate::platform::current();
        let env = Env {
            settings: &settings,
            theme: &theme,
            platform: platform.as_ref(),
            alt: false,
            animate: true,
            changes: &crate::sampler::ChangeSink::NONE,
        };
        let caps = Capabilities {
            battery: true,
            ..Capabilities::default()
        };
        let mut layout = Layout {
            left: vec![entry("b", "battery", json!({}))],
            ..Layout::default()
        };
        let mut sensors = BarSensors::default();
        sensors.sync(&layout, &BTreeMap::new(), &caps, registry());
        assert_eq!(sensors.take_created(), ["b"]);
        let on_battery = crate::sampler::BatterySample {
            present: true,
            name: String::new(),
            percent: 73.0,
            ac_online: false,
            state: crate::platform::BatteryState::Discharging,
            time_to_empty: None,
            time_to_full: None,
        };
        let delivery = Delivery {
            topic: Topic::Battery(0),
            sample: std::sync::Arc::new(Sample::Battery(on_battery)),
            to: None,
            pushed: false,
        };
        sensors.deliver(&delivery, &env, Instant::now());
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        // A click switches to the time left and asks for it to be saved.
        assert_eq!(sensors.click("b", &env), ClickResult::Handled);
        assert!(sensors.is_dirty("b"), "measured again");
        let saved = sensors.take_props();
        assert_eq!(
            saved,
            [(
                "b".to_owned(),
                json!({ "onBatteryShow": "time" })
                    .as_object()
                    .cloned()
                    .unwrap_or_default()
            )]
        );
        assert!(sensors.take_props().is_empty(), "taken once");
        // The saved props come back with the config: the same sensor takes them.
        layout.left[0].props = saved[0].1.clone();
        assert!(sensors.sync(&layout, &BTreeMap::new(), &caps, registry()));
        assert!(sensors.take_created().is_empty(), "not created anew");
        // Another battery: created anew.
        layout.left[0].props = json!({ "battery": 1 })
            .as_object()
            .cloned()
            .unwrap_or_default();
        sensors.sync(&layout, &BTreeMap::new(), &caps, registry());
        assert_eq!(sensors.take_created(), ["b"]);
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
            let Sample::Uptime(uptime) = sample else {
                return false;
            };
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
            changes: &crate::sampler::ChangeSink::NONE,
        };
        let mut sensors = BarSensors::default();
        for id in ["u1", "u2"] {
            sensors.zones[0].push(Placed::new(
                entry(id, "uptime", json!({})),
                Box::new(Uptime { minutes: 0 }),
            ));
        }
        sensors.zones[2].push(Placed::new(
            entry("t", "ticker", json!({})),
            Box::new(Ticker {
                start: Instant::now(),
                wakes: Rc::new(Cell::new(0)),
            }),
        ));
        let mut ids: Vec<_> = sensors.subscriptions().into_keys().collect();
        ids.sort();
        assert_eq!(ids, ["u1", "u2"]);
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);

        let delivery = |secs, to: Option<&str>| Delivery {
            topic: Topic::Uptime,
            sample: std::sync::Arc::new(Sample::Uptime(crate::sampler::UptimeSample {
                boot_unix_ms: 0,
                uptime_secs: secs,
                read_at: Instant::now(),
            })),
            to: to.map(str::to_owned),
            pushed: false,
        };
        // A catch-up reaches only its subscriber.
        let t0 = Instant::now();
        let minute = Duration::from_secs(60);
        assert!(sensors.deliver(&delivery(120, Some("u2")), &env, t0));
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        let widths: Vec<f32> = sensors.places().iter().map(|p| p.2).collect();
        assert_eq!(widths, [10.0, 12.0, 10.0]);
        // Every subscriber gets a broadcast; one whose minute is unchanged stays clean.
        assert!(sensors.deliver(&delivery(150, None), &env, t0 + minute));
        assert!(!sensors.deliver(&delivery(170, None), &env, t0 + 2 * minute));
        sensors.update_layout(&layout_ctx(&theme, 1.0), 400.0);
        let widths: Vec<f32> = sensors.places().iter().map(|p| p.2).collect();
        assert_eq!(widths, [12.0, 12.0, 10.0]);
        // Samples sooner than a sensor's own interval pass it by (the topic may be read
        // faster for another sensor); one a little early is on time.
        let soon = t0 + 2 * minute + Duration::from_secs(30);
        assert!(
            !sensors.deliver(&delivery(300, None), &env, soon),
            "not taken"
        );
        let nearly = t0 + 3 * minute - Duration::from_millis(400);
        assert!(sensors.deliver(&delivery(300, None), &env, nearly));
        // A reading pushed by a change is news for everyone, however soon it comes.
        let pushed = Delivery {
            pushed: true,
            ..delivery(420, None)
        };
        assert!(sensors.deliver(&pushed, &env, nearly + Duration::from_secs(1)));
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
