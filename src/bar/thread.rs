//! The bar logic on the bar thread (SPEC §4.1, §4.4): applies the config and the OS appearance,
//! keeps the sensors, their wake-up timer and layout, draws the bar, and handles the pointer:
//! clicks, context menus and the hover hint (§6, §7.1).

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::anim::{self, Easing, Fade};
use super::hint::{self, HintBody, HintFrame, Hover, Step};
use super::interact::{self, DropTarget, Press};
use super::menus::{self, MenuCommand};
use super::paint::paint_bar;
use super::placement::Anchor;
use super::sensors::{BarSensors, Env};
use super::theme::Theme;
use super::tray::{self, TrayCommand};
use super::{BarHost, Command};
use crate::config::Config;
use crate::config::model::Zone;
use crate::config::model::{
    BarMode, Edge, FloatingRect, Layout, Material, SensorInstance, Settings,
};
use crate::i18n::{t, t_args};
use crate::platform::geometry::DEFAULT_FLOATING_WIDTH;
use crate::platform::{
    BarGeometry, BarHandler, BarWindow, Canvas, Modifiers, OsAppearance, Platform, PointF,
    PointerButton, Rect, RectF, SizeF, TextEngine, TimerId, UiEvent,
};
use crate::sampler::{Delivery, TopicRequest};
use crate::sensors::format::Numbers;
use crate::sensors::ops::index_after_removal;
use crate::sensors::registry::registry;
use crate::sensors::{ClickResult, LayoutCtx, State};

/// How often Esc is checked during a drag.
const ESC_POLL: Duration = Duration::from_millis(50);
/// The hover highlight fades in and out over this long (SPEC §5.5).
const HIGHLIGHT: Duration = Duration::from_millis(300);
/// The bar rises into place, or slides out, over this long (SPEC §5.5).
const RISE: Duration = Duration::from_millis(300);

/// A reorder drag in progress.
struct Drag {
    id: String,
    /// Where the sensor is.
    source: (Zone, usize),
    /// Where it would go; none until the pointer moved.
    target: Option<DropTarget>,
}

/// A zone as Settings routes name it.
fn zone_id(zone: Zone) -> &'static str {
    match zone {
        Zone::Left => "left",
        Zone::Center => "center",
        Zone::Right => "right",
    }
}

pub(super) struct BarLogic {
    settings: Settings,
    layout: Layout,
    state: BTreeMap<String, State>,
    /// Entries shown instead of the saved ones while Settings previews them, by id.
    previews: HashMap<String, SensorInstance>,
    /// The left button is down on a sensor.
    press: Option<Press>,
    /// A reorder drag runs (SPEC §6.2).
    drag: Option<Drag>,
    /// The left button is down on the background of a floating bar, which moves it (§5.1).
    moving: Option<Moving>,
    /// When the hover hint shows and hides.
    hover: Hover,
    /// Where the bar is; hints are placed from it.
    geometry: Option<BarGeometry>,
    sensors: BarSensors,
    text: Rc<dyn TextEngine>,
    appearance: OsAppearance,
    theme: Theme,
    /// The material last applied to the window; `None` until the first application.
    material: Option<Material>,
    /// Whether the tray icon is shown; `None` until first applied.
    tray_shown: Option<bool>,
    /// The topics each sensor was last subscribed to (SPEC §4.4).
    subscribed: HashMap<String, Vec<TopicRequest>>,
    /// Transitions animate (the setting, or the OS for `auto`; SPEC §5.5).
    animate: bool,
    /// How the locale of `settings.dateLocale` writes numbers (SPEC §8.6), read once and again
    /// when the setting or the OS regional format changes.
    numbers: Numbers,
    /// The pointer is over the bar.
    inside: bool,
    /// The sensor under the pointer, told when the pointer enters and leaves it.
    hovered: Option<String>,
    /// The hover highlight (SPEC §5.5), 0–1.
    highlight: Fade,
    /// How far the bar has risen into place, 0–1: its content's opacity, and how far it is
    /// off its place (SPEC §5.5).
    shown: Fade,
    /// The next placement starts the rise (start-up, and after sliding out).
    rise_pending: bool,
    /// Sliding out before an edge change; redocks when done.
    sliding_out: bool,
    /// The offset last given to [`BarWindow::slide`]; `None` after a new placement, which puts
    /// the window in its place.
    slid: Option<i32>,
    platform: Arc<dyn Platform>,
    commands: Receiver<Command>,
    host: Arc<dyn BarHost>,
}

/// A press on the background of a floating bar; past the threshold it moves the window.
struct Moving {
    press: Press,
    /// Where the window is after the last move, physical px.
    rect: Option<Rect>,
}

/// The subscription changes from `old` to `new` (by sensor id): new or changed topic lists, the
/// lists of sensors `created` anew (a re-created sensor keeps its id and topics but starts
/// without data, and subscribing sends it the newest samples, SPEC §4.4), and an empty list for
/// sensors that no longer subscribe. Sorted by id.
fn subscription_changes(
    old: &HashMap<String, Vec<TopicRequest>>,
    new: &HashMap<String, Vec<TopicRequest>>,
    created: &[String],
) -> Vec<(String, Vec<TopicRequest>)> {
    let mut changes: Vec<(String, Vec<TopicRequest>)> = new
        .iter()
        .filter(|(id, topics)| old.get(*id) != Some(*topics) || created.contains(*id))
        .map(|(id, topics)| (id.clone(), topics.clone()))
        .chain(
            old.keys()
                .filter(|id| !new.contains_key(*id))
                .map(|id| (id.clone(), Vec::new())),
        )
        .collect();
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    changes
}

impl BarLogic {
    pub(super) fn new(
        config: Config,
        platform: Arc<dyn Platform>,
        commands: Receiver<Command>,
        host: Arc<dyn BarHost>,
    ) -> Self {
        let appearance = platform.appearance();
        let animate = anim::enabled(config.settings.bar.animations, appearance.animations);
        let numbers = platform.locale(config.settings.date_locale).numbers();
        Self {
            theme: Theme::resolve(&config.settings, appearance),
            settings: config.settings,
            layout: config.layout,
            state: config.state,
            previews: HashMap::new(),
            press: None,
            drag: None,
            moving: None,
            hover: Hover::default(),
            geometry: None,
            sensors: BarSensors::default(),
            text: crate::platform::text_engine(),
            appearance,
            material: None,
            tray_shown: None,
            subscribed: HashMap::new(),
            animate,
            numbers,
            inside: false,
            hovered: None,
            highlight: Fade::at(0.0, Instant::now()),
            // Hidden until it rises, if it does.
            shown: Fade::at(if animate { 0.0 } else { 1.0 }, Instant::now()),
            rise_pending: true,
            sliding_out: false,
            slid: None,
            platform,
            commands,
            host,
        }
    }

    /// Shows or hides the tray icon as configured (SPEC §6.5).
    fn refresh_tray(&mut self, window: &dyn BarWindow) {
        let show = self.settings.show_tray_icon;
        if self.tray_shown != Some(show) {
            let tooltip = crate::i18n::t("tray.tooltip");
            window.set_tray_icon(show.then_some(tooltip.as_str()));
            self.tray_shown = Some(show);
        }
    }

    fn tray_menu(&mut self, window: &dyn BarWindow) {
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        let entries = tray::menu(&self.settings, self.host.bar_visible());
        let Some(command) = window.show_menu(&entries).and_then(TrayCommand::from_id) else {
            return;
        };
        match command {
            TrayCommand::ToggleBar => self.host.toggle_bar(),
            TrayCommand::ToggleLock => self
                .host
                .change_settings(Box::new(|s| s.locked = !s.locked)),
            TrayCommand::TogglePopups => {
                self.host
                    .change_settings(Box::new(|s| s.popups_enabled = !s.popups_enabled));
            }
            TrayCommand::OpenSettings => self.host.open_settings("general"),
            TrayCommand::Quit => self.host.quit(),
        }
    }

    fn clicks_allowed(&self, alt: bool) -> bool {
        let s = &self.settings;
        interact::clicks_allowed(s.locked, s.clicks_when_locked, alt)
    }

    /// A left press on a sensor dims it until release, if it can lead to a click or a drag
    /// (SPEC §6.1).
    fn pointer_down(&mut self, window: &dyn BarWindow, at: PointF, modifiers: Modifiers) {
        // Any click closes the hint (SPEC §7.1).
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        let Some(hit) = self.sensors.hit(at.x) else {
            if self.settings.bar.mode == BarMode::Floating && !self.settings.locked {
                window.capture_pointer(true);
                self.moving = Some(Moving {
                    press: Press::new(String::new(), at),
                    rect: None,
                });
            }
            return;
        };
        if !self.clicks_allowed(modifiers.alt) {
            return;
        }
        window.capture_pointer(true);
        if self.sensors.set_pressed(Some(&hit.entry.id)) {
            window.invalidate();
        }
        self.press = Some(Press::new(hit.entry.id, at));
    }

    fn pointer_moved(&mut self, window: &dyn BarWindow, at: PointF) {
        if !self.inside {
            self.inside = true;
            self.refresh_highlight(window);
        }
        if let Some(moving) = &mut self.moving {
            moving.press.moved_to(at);
            if moving.press.moved {
                // The window follows, so the pointer stays at the pressed point inside it.
                let scale = self.geometry.as_ref().map_or(1.0, |g| g.scale);
                let dx = (f64::from(at.x - moving.press.at.x) * scale).round() as i32;
                let dy = (f64::from(at.y - moving.press.at.y) * scale).round() as i32;
                if (dx, dy) != (0, 0)
                    && let Some(rect) = window.move_by(dx, dy)
                {
                    moving.rect = Some(rect);
                }
            }
            return;
        }
        if let Some(drag) = &mut self.drag {
            let target = interact::drop_target(
                at.x,
                &self.sensors.spans(),
                self.sensors.bar_width(),
                self.theme.pad,
                self.theme.gap,
            );
            if drag.target != Some(target) {
                drag.target = Some(target);
                window.invalidate();
            }
            return;
        }
        if let Some(press) = &mut self.press {
            // Past the threshold an unlocked press becomes a reorder drag; a locked one just
            // stops being a click.
            if press.moved_to(at) {
                let id = press.id.clone();
                match self.sensors.find(&id).filter(|_| !self.settings.locked) {
                    Some(source) => self.start_drag(window, id, source, at),
                    None => {
                        self.sensors.set_pressed(None);
                        window.invalidate();
                    }
                }
            }
            return;
        }
        let over = self.sensors.hit(at.x).map(|h| h.entry.id);
        self.set_hovered(window, over.clone());
        let over = over.filter(|_| self.settings.popups_enabled);
        let popups = &self.settings.popups;
        let step = self.hover.over(
            over.as_deref(),
            Duration::from_millis(u64::from(popups.show_delay_ms)),
            Duration::from_millis(u64::from(popups.hide_delay_ms)),
        );
        self.apply_hint(window, step);
    }

    /// Starts dragging `id` from `source`; the sensor stays dimmed in place and a placer shows
    /// where it would go (SPEC §6.2). Hints stay closed meanwhile.
    fn start_drag(
        &mut self,
        window: &dyn BarWindow,
        id: String,
        source: (Zone, usize),
        at: PointF,
    ) {
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        self.drag = Some(Drag {
            id,
            source,
            target: None,
        });
        window.set_timer(TimerId::Drag, Some(ESC_POLL));
        self.pointer_moved(window, at);
    }

    /// Ends a drag without moving anything (Esc, lost capture, a menu).
    fn cancel_drag(&mut self, window: &dyn BarWindow) {
        if self.drag.take().is_some() || self.press.is_some() {
            self.press = None;
            window.set_timer(TimerId::Drag, None);
            window.capture_pointer(false);
            self.sensors.set_pressed(None);
            window.invalidate();
        }
    }

    fn pointer_left(&mut self, window: &dyn BarWindow) {
        let hide = Duration::from_millis(u64::from(self.settings.popups.hide_delay_ms));
        let step = self.hover.leave(hide);
        self.apply_hint(window, step);
        self.inside = false;
        self.refresh_highlight(window);
        self.set_hovered(window, None);
    }

    /// Tells the sensors the pointer entered or left (e.g. the Spacer's divider fades in).
    fn set_hovered(&mut self, window: &dyn BarWindow, id: Option<String>) {
        if self.hovered == id {
            return;
        }
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: false,
            animate: self.animate,
        };
        let now = Instant::now();
        let mut dirty = false;
        if let Some(old) = &self.hovered {
            dirty |= self.sensors.on_hover(old, false, now, &env);
        }
        if let Some(new) = &id {
            dirty |= self.sensors.on_hover(new, true, now, &env);
        }
        self.hovered = id;
        if dirty {
            window.invalidate();
        }
        self.arm_timer(window);
    }

    /// Draws the next frame of the bar's animations and asks for another while one runs.
    fn frame(&mut self, window: &dyn BarWindow, now: Instant) {
        window.invalidate();
        // A finished fade rests at its end, whatever the clock says when painting.
        for fade in [&mut self.shown, &mut self.highlight] {
            if !fade.running(now) {
                *fade = Fade::at(fade.target(), now);
            }
        }
        if let Some(g) = &self.geometry {
            // A docked bar slides from its edge; a floating one only fades.
            let off = 1.0 - self.shown.value(now);
            let dy = if self.settings.bar.mode == BarMode::Docked {
                let dy = (off * g.rect.height() as f32).round() as i32;
                if g.edge == Edge::Top { -dy } else { dy }
            } else {
                0
            };
            // Highlight-only frames leave the window where it is.
            if self.slid != Some(dy) {
                self.slid = Some(dy);
                window.slide(dy);
            }
        }
        if self.sliding_out && !self.shown.running(now) {
            self.sliding_out = false;
            self.rise_pending = true;
            self.host.redock();
            // Already placed at the edge it docks to (the edge changed back meanwhile, or a
            // placement came during the slide): no new placement follows, so it rises now.
            let edge = self.settings.bar.edge;
            if self.geometry.as_ref().is_some_and(|g| g.edge == edge) {
                self.rise(window);
            }
        }
        if self.highlight.running(now) || self.shown.running(now) {
            window.set_timer(TimerId::Frame, Some(anim::FRAME));
        }
    }

    /// Starts the rise into place after a placement (SPEC §5.5).
    fn rise(&mut self, window: &dyn BarWindow) {
        if !std::mem::take(&mut self.rise_pending) || self.sliding_out {
            return;
        }
        let now = Instant::now();
        if self.animate {
            self.shown = Fade::at(0.0, now);
            self.shown.go(1.0, RISE, Easing::Out, now);
        } else {
            self.shown = Fade::at(1.0, now);
        }
        self.frame(window, now);
    }

    /// Takes a new config; returns whether the sensors need syncing.
    fn apply_config(&mut self, window: &dyn BarWindow, config: Config) -> bool {
        let (old, new) = (&self.settings.bar, &config.settings.bar);
        if old.edge != new.edge || config.settings.sensor_spacing != self.settings.sensor_spacing {
            window.invalidate();
        }
        // A docked bar changing edges slides out toward the old one first (SPEC §5.5); the
        // controller left the redocking to us.
        if old.edge != new.edge && old.mode == BarMode::Docked && new.mode == BarMode::Docked {
            if self.animate && self.geometry.is_some() && self.host.bar_visible() {
                if !self.sliding_out {
                    let step = self.hover.dismiss();
                    self.apply_hint(window, step);
                    self.sliding_out = true;
                    self.shown.go(0.0, RISE, Easing::In, Instant::now());
                    window.set_timer(TimerId::Frame, Some(anim::FRAME));
                }
            } else {
                self.host.redock();
            }
        }
        let layout_changed = config.layout != self.layout || config.state != self.state;
        let locale_changed = config.settings.date_locale != self.settings.date_locale;
        self.settings = config.settings;
        if locale_changed {
            self.locale_changed(window);
        }
        self.layout = config.layout;
        self.state = config.state;
        layout_changed
    }

    /// Fades the hover highlight in while the pointer is over the bar and the setting is on,
    /// and out otherwise (SPEC §5.5).
    fn refresh_highlight(&mut self, window: &dyn BarWindow) {
        let on = self.inside && self.settings.bar.highlight_on_hover;
        let (to, easing) = if on {
            (1.0, Easing::In)
        } else {
            (0.0, Easing::Out)
        };
        if self.highlight.target() == to {
            return;
        }
        let full = if self.animate {
            HIGHLIGHT
        } else {
            Duration::ZERO
        };
        self.highlight.go(to, full, easing, Instant::now());
        window.invalidate();
        window.set_timer(TimerId::Frame, Some(anim::FRAME));
    }

    /// Carries out what the hover logic decided.
    fn apply_hint(&mut self, window: &dyn BarWindow, step: Step) {
        match step {
            Step::Idle => {}
            Step::Show(id) => {
                if !self.show_hint(window, &id) {
                    self.hover.nothing_shown();
                    window.hide_hint();
                }
            }
            Step::Hide => {
                window.set_timer(TimerId::Hint, None);
                window.hide_hint();
            }
            Step::Wait(delay) => window.set_timer(TimerId::Hint, Some(delay)),
            Step::StopWaiting => window.set_timer(TimerId::Hint, None),
        }
    }

    /// Shows (or updates) sensor `id`'s hint above or below it (SPEC §7.1, §7.3); `false` if
    /// it has none.
    fn show_hint(&self, window: &dyn BarWindow, id: &str) -> bool {
        let Some(geometry) = &self.geometry else {
            return false;
        };
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: false,
            animate: self.animate,
        };
        let (Some(content), Some((x, width))) =
            (self.sensors.hint(id, &env), self.sensors.place_of(id))
        else {
            return false;
        };
        let Some(body) = HintBody::new(content, self.text.as_ref(), &self.theme) else {
            return false;
        };
        let scale = geometry.scale as f32;
        let logical = |r: Rect| {
            RectF::new(
                r.left as f32 / scale,
                r.top as f32 / scale,
                r.right as f32 / scale,
                r.bottom as f32 / scale,
            )
        };
        let bar = logical(geometry.rect);
        let anchor = Anchor {
            rect: RectF::new(bar.left + x, bar.top, bar.left + x + width, bar.bottom),
            bar,
            monitor: logical(geometry.monitor.bounds),
            edge: geometry.edge,
        };
        let frame = HintFrame::new(body.size(), &anchor, &self.theme);
        let physical = |v: f32| (v * scale).round() as i32;
        let rect = Rect::new(
            physical(frame.window.left),
            physical(frame.window.top),
            physical(frame.window.right),
            physical(frame.window.bottom),
        );
        let theme = &self.theme;
        window.show_hint(rect, scale, &mut |canvas| {
            hint::paint(canvas, &frame, &body, theme);
        });
        true
    }

    /// Redraws a shown hint, whose sensor or theme may have changed (SPEC §7.1: live).
    fn refresh_hint(&mut self, window: &dyn BarWindow) {
        if !self.settings.popups_enabled {
            let step = self.hover.dismiss();
            self.apply_hint(window, step);
            return;
        }
        if let Some(id) = self.hover.shown().map(str::to_owned) {
            // The hint is placed by its sensor, which may have moved: place the sensors now
            // rather than at the next paint.
            self.sensors.refresh_layout(&self.theme, self.text.as_ref());
            if !self.show_hint(window, &id) {
                self.hover.nothing_shown();
                window.hide_hint();
            }
        }
    }

    /// After samples or wake-ups changed what sensors show: the open hint is rebuilt only when
    /// its own sensor changed or moved, so an open calendar is not laid out again for every
    /// CPU sample.
    fn refresh_changed_hint(&mut self, window: &dyn BarWindow) {
        let Some(id) = self.hover.shown().map(str::to_owned) else {
            return;
        };
        let changed = self.sensors.is_dirty(&id);
        let before = self.sensors.place_of(&id);
        self.sensors.refresh_layout(&self.theme, self.text.as_ref());
        if changed || self.sensors.place_of(&id) != before {
            self.refresh_hint(window);
        }
    }

    /// Releasing over the pressed sensor without having moved is a click.
    /// Stores where a moved floating bar is now (SPEC §5.1).
    fn end_moving(&mut self, window: &dyn BarWindow) {
        let Some(moving) = self.moving.take() else {
            return;
        };
        window.capture_pointer(false);
        if let Some(rect) = moving.rect {
            self.host.change_settings(Box::new(move |s| {
                let width = s.bar.floating.map_or(DEFAULT_FLOATING_WIDTH, |f| f.width);
                s.bar.floating = Some(FloatingRect {
                    x: rect.left,
                    y: rect.top,
                    width,
                });
            }));
        }
    }

    fn pointer_up(&mut self, window: &dyn BarWindow, at: PointF, modifiers: Modifiers) {
        if self.moving.is_some() {
            self.end_moving(window);
            return;
        }
        if let Some(drag) = self.drag.take() {
            self.press = None;
            window.set_timer(TimerId::Drag, None);
            window.capture_pointer(false);
            self.sensors.set_pressed(None);
            window.invalidate();
            if let Some(target) = drag.target
                && !interact::is_no_move(drag.source, &target)
            {
                let index = index_after_removal(drag.source, target.zone, target.insertion);
                self.host.move_sensor(&drag.id, target.zone, index);
            }
            return;
        }
        let Some(press) = self.press.take() else {
            return;
        };
        window.capture_pointer(false);
        if self.sensors.set_pressed(None) {
            window.invalidate();
        }
        let over = self
            .sensors
            .hit(at.x)
            .is_some_and(|h| h.entry.id == press.id);
        if press.moved || !over || !self.clicks_allowed(modifiers.alt) {
            return;
        }
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: modifiers.alt,
            animate: self.animate,
        };
        if self.sensors.click(&press.id, &env) == ClickResult::Handled {
            window.invalidate();
        }
    }

    /// The bar or sensor menu at `at` (SPEC §6.4), and what its command does.
    fn context_menu(&mut self, window: &dyn BarWindow, at: PointF) {
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        self.cancel_drag(window);
        let caps = self.platform.capabilities();
        let hit = self.sensors.hit(at.x);
        let entries = match &hit {
            Some(h) => {
                let editable = registry().editable(&h.entry, &caps);
                menus::sensor_menu(&self.settings, registry(), &caps, editable)
            }
            None => menus::bar_menu(&self.settings, registry(), &caps),
        };
        let Some(command) = window.show_menu(&entries).and_then(MenuCommand::from_id) else {
            return;
        };
        // New sensors go after the sensor, or to the zone under the pointer (SPEC §6.6).
        let (zone, index) = match &hit {
            Some(h) => (h.zone, h.index + 1),
            None => {
                let zone = interact::zone_at(at.x, self.sensors.bar_width());
                (
                    zone,
                    interact::insertion_index(zone, self.sensors.len(zone)),
                )
            }
        };
        let target = hit.as_ref().map(|h| &h.entry);
        match command {
            MenuCommand::NewSensor => self.host.open_settings(&match target {
                Some(entry) => format!("new?target={}", entry.id),
                None => format!("new?zone={}&index={index}", zone_id(zone)),
            }),
            MenuCommand::Add(i) => {
                let types = menus::addable_types(registry(), &caps);
                if let Some(ty) = types.get(i)
                    && let Some(id) = self.host.add_sensor(ty.id(), zone, index)
                {
                    self.host.open_settings(&format!("sensor/{id}"));
                }
            }
            MenuCommand::Properties => {
                if let Some(entry) = target {
                    self.host.open_settings(&format!("sensor/{}", entry.id));
                }
            }
            MenuCommand::Remove => {
                if let Some(entry) = target {
                    let name = registry().type_name(&entry.kind);
                    let question = t_args("menu.remove.confirm", &[("name", &name)]);
                    if window.confirm(&question, &t("menu.remove.action")) {
                        self.host.remove_sensor(&entry.id);
                    }
                }
            }
            MenuCommand::LayoutManager => self.host.open_settings("layout"),
            // Greyed out until their milestones (M4, M5).
            MenuCommand::ImportLayout
            | MenuCommand::ExportLayout
            | MenuCommand::ImportLegacy
            | MenuCommand::Help => {}
            MenuCommand::Settings => self.host.open_settings("general"),
            MenuCommand::ToggleLock => self
                .host
                .change_settings(Box::new(|s| s.locked = !s.locked)),
            MenuCommand::TogglePopups => {
                self.host
                    .change_settings(Box::new(|s| s.popups_enabled = !s.popups_enabled));
            }
            MenuCommand::Quit => self.host.quit(),
        }
    }

    /// Re-resolves the theme; applies a changed material and repaints if anything changed.
    fn refresh_theme(&mut self, window: &dyn BarWindow) {
        self.animate = anim::enabled(self.settings.bar.animations, self.appearance.animations);
        self.refresh_highlight(window);
        let theme = Theme::resolve(&self.settings, self.appearance);
        if self.material != Some(theme.material) {
            window.set_material(theme.material);
            self.material = Some(theme.material);
        }
        if theme != self.theme {
            self.theme = theme;
            self.sensors.invalidate_all();
            window.invalidate();
            self.refresh_hint(window);
        }
    }

    /// Creates sensors for new or changed layout entries, showing previews where there are.
    fn sync_sensors(&mut self, window: &dyn BarWindow) {
        let caps = self.platform.capabilities();
        let mut shown = self.layout.clone();
        for entry in shown.all_mut() {
            if let Some(preview) = self.previews.get(&entry.id) {
                *entry = preview.clone();
            }
        }
        if self.sensors.sync(&shown, &self.state, &caps, registry()) {
            // New clocks read the time now, so the next frame does not show them empty.
            let env = Env {
                settings: &self.settings,
                theme: &self.theme,
                platform: self.platform.as_ref(),
                alt: false,
                animate: self.animate,
            };
            self.sensors.wake(Instant::now(), &env);
            self.store_states();
            window.invalidate();
            self.refresh_hint(window);
        }
        let subscriptions = self.sensors.subscriptions();
        let created = self.sensors.take_created();
        for (id, topics) in subscription_changes(&self.subscribed, &subscriptions, &created) {
            self.host.subscribe(&id, topics);
        }
        self.subscribed = subscriptions;
        self.arm_timer(window);
    }

    /// Hands a sample to its sensors and redraws what changed.
    fn deliver(&mut self, window: &dyn BarWindow, delivery: &Delivery) {
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: false,
            animate: self.animate,
        };
        if self.sensors.deliver(delivery, &env, Instant::now()) {
            window.invalidate();
            self.refresh_changed_hint(window);
        }
        self.store_states();
    }

    /// The locale for dates and numbers may have changed (the setting, or the OS regional
    /// format): numbers are read again, and clocks re-read the locale as on a clock change.
    fn locale_changed(&mut self, window: &dyn BarWindow) {
        self.numbers = self.platform.locale(self.settings.date_locale).numbers();
        window.invalidate();
        self.clock_changed(window);
    }

    /// The wall clock jumped: clocks read it again and move their wake-ups.
    fn clock_changed(&mut self, window: &dyn BarWindow) {
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: false,
            animate: self.animate,
        };
        if self.sensors.clock_changed(Instant::now(), &env) {
            window.invalidate();
            self.refresh_hint(window);
        }
        self.arm_timer(window);
    }

    /// Hands the state sensors want stored to the host (SPEC §11.2).
    fn store_states(&mut self) {
        for (id, state) in self.sensors.take_states() {
            self.host.set_sensor_state(&id, state);
        }
    }

    /// Sets the window's timer to the earliest wake-up any sensor asked for (SPEC §4.4).
    fn arm_timer(&self, window: &dyn BarWindow) {
        let now = Instant::now();
        window.set_timer(
            TimerId::Wake,
            self.sensors
                .next_wake(now)
                .map(|at| at.saturating_duration_since(now)),
        );
    }

    fn on_timer(&mut self, window: &dyn BarWindow, id: TimerId) {
        match id {
            TimerId::Wake => {
                let env = Env {
                    settings: &self.settings,
                    theme: &self.theme,
                    platform: self.platform.as_ref(),
                    alt: false,
                    animate: self.animate,
                };
                if self.sensors.wake(Instant::now(), &env) {
                    window.invalidate();
                    self.refresh_changed_hint(window);
                }
                self.store_states();
                self.arm_timer(window);
            }
            TimerId::Hint => {
                let step = self.hover.timer();
                self.apply_hint(window, step);
            }
            // Esc cancels a drag; the bar has no focus, so it is polled (SPEC §6.2).
            TimerId::Drag if self.drag.is_some() => {
                if window.escape_down() {
                    self.cancel_drag(window);
                } else {
                    window.set_timer(TimerId::Drag, Some(ESC_POLL));
                }
            }
            TimerId::Drag => {}
            TimerId::Frame => self.frame(window, Instant::now()),
        }
    }
}

impl BarHandler for BarLogic {
    fn on_event(&mut self, window: &dyn BarWindow, event: UiEvent) {
        match event {
            UiEvent::Started => {
                self.refresh_theme(window);
                self.refresh_tray(window);
                self.sync_sensors(window);
            }
            UiEvent::Wake => {
                let mut layout_changed = false;
                let mut configured = false;
                while let Ok(command) = self.commands.try_recv() {
                    match command {
                        Command::Config(config) => {
                            configured = true;
                            layout_changed |= self.apply_config(window, *config);
                        }
                        Command::Preview(sensor) => {
                            self.previews.insert(sensor.id.clone(), *sensor);
                            layout_changed = true;
                        }
                        Command::EndPreview(id) => {
                            layout_changed |= self.previews.remove(&id).is_some();
                        }
                        Command::Sample(delivery) => self.deliver(window, &delivery),
                    }
                }
                // Samples alone change neither the theme nor the tray.
                if configured {
                    self.refresh_theme(window);
                    self.refresh_tray(window);
                }
                if layout_changed {
                    self.sync_sensors(window);
                }
            }
            UiEvent::Timer(id) => self.on_timer(window, id),
            UiEvent::PointerDown {
                button: PointerButton::Left,
                at,
                modifiers,
            } => self.pointer_down(window, at, modifiers),
            UiEvent::PointerMove(at) => self.pointer_moved(window, at),
            UiEvent::PointerUp {
                button: PointerButton::Left,
                at,
                modifiers,
            } => self.pointer_up(window, at, modifiers),
            UiEvent::PointerUp {
                button: PointerButton::Right,
                at,
                ..
            } => self.context_menu(window, at),
            UiEvent::PointerLeave => self.pointer_left(window),
            UiEvent::CaptureLost => {
                self.cancel_drag(window);
                self.end_moving(window);
            }
            // Middle clicks are reserved (SPEC §6.1).
            UiEvent::PointerDown { .. } | UiEvent::PointerUp { .. } => {}
            UiEvent::AppearanceChanged => {
                let appearance = self.platform.appearance();
                if appearance != self.appearance {
                    tracing::info!("OS appearance changed: {appearance:?}");
                    self.appearance = appearance;
                    self.refresh_theme(window);
                }
            }
            UiEvent::Placed(geometry) => {
                self.geometry = Some(geometry);
                self.slid = None;
                window.invalidate();
                self.rise(window);
                self.refresh_hint(window);
            }
            // Full-screen apps hide the hint (SPEC §5.3); popups arrive in M3.
            UiEvent::Fullscreen(true) => {
                let step = self.hover.dismiss();
                self.apply_hint(window, step);
            }
            UiEvent::Fullscreen(false) => {}
            UiEvent::CloseRequested => self.host.quit(),
            UiEvent::TimeChanged => self.clock_changed(window),
            UiEvent::LocaleChanged => self.locale_changed(window),
            UiEvent::Resumed => {
                self.host.resumed();
                self.clock_changed(window);
            }
            UiEvent::Activate => self.host.activate(),
            UiEvent::TrayClick => self.host.toggle_bar(),
            UiEvent::TrayMenu => self.tray_menu(window),
        }
    }

    fn paint(&mut self, canvas: &mut dyn Canvas, size: SizeF) {
        let cx = LayoutCtx {
            theme: &self.theme,
            text: self.text.as_ref(),
            scale: canvas.scale(),
            numbers: self.numbers,
        };
        self.sensors.update_layout(&cx, size.width);
        let now = Instant::now();
        // The content fades in while the bar rises (SPEC §5.5).
        let opacity = self.shown.value(now);
        if opacity < 1.0 {
            canvas.push_opacity(opacity);
        }
        let highlight = self.highlight.value(now);
        // The edge it is placed on: the nearer one for a floating bar, the old one while it
        // slides out (SPEC §12.2).
        let edge = self
            .geometry
            .as_ref()
            .map_or(self.settings.bar.edge, |g| g.edge);
        paint_bar(canvas, size, &self.theme, edge, highlight);
        self.sensors.draw(canvas, &self.theme, size.height);
        // The placer: a 2 px line in the accent colour where the dragged sensor would go
        // (SPEC §6.2, §12.2).
        if let Some(target) = self.drag.as_ref().and_then(|d| d.target) {
            let x = canvas.snap(target.placer_x - 1.0);
            canvas.fill_rect(
                RectF::new(x, 2.0, x + 2.0, size.height - 2.0),
                self.theme.accent,
            );
        }
        if opacity < 1.0 {
            canvas.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::Mutex;
    use std::sync::mpsc;

    use super::*;
    use crate::config::model::ClicksWhenLocked;
    use crate::platform::recording::RecordingCanvas;
    use crate::platform::{MenuEntry, PointerButton};

    /// Records what the bar asked of the window; the menu returns `choose` once.
    #[derive(Default)]
    struct FakeWindow {
        choose: Cell<Option<u32>>,
        menus: RefCell<Vec<Vec<MenuEntry>>>,
        captured: Cell<bool>,
        confirmed: RefCell<Vec<String>>,
        hint_timer: Cell<Option<std::time::Duration>>,
        /// Frame timers requested.
        frames: Cell<u32>,
        /// Where a floating bar is; `None` when docked.
        position: Cell<Option<Rect>>,
        /// The rise animation's offsets.
        slides: RefCell<Vec<i32>>,
        escape: Cell<bool>,
        /// The shown hint: where, and what it draws.
        hint: RefCell<Option<(Rect, String)>>,
    }

    impl BarWindow for FakeWindow {
        fn invalidate(&self) {}
        fn set_material(&self, _material: Material) {}
        fn set_tray_icon(&self, _tooltip: Option<&str>) {}
        fn show_menu(&self, entries: &[MenuEntry]) -> Option<u32> {
            self.menus.borrow_mut().push(entries.to_vec());
            self.choose.take()
        }
        fn set_timer(&self, id: TimerId, delay: Option<std::time::Duration>) {
            match id {
                TimerId::Hint => self.hint_timer.set(delay),
                TimerId::Frame => self.frames.set(self.frames.get() + 1),
                _ => {}
            }
        }
        fn show_hint(&self, rect: Rect, scale: f32, paint: &mut dyn FnMut(&mut dyn Canvas)) {
            let mut canvas = RecordingCanvas::new(scale);
            paint(&mut canvas);
            *self.hint.borrow_mut() = Some((rect, canvas.dump()));
        }
        fn hide_hint(&self) {
            *self.hint.borrow_mut() = None;
        }
        fn capture_pointer(&self, capture: bool) {
            self.captured.set(capture);
        }
        fn slide(&self, dy: i32) {
            self.slides.borrow_mut().push(dy);
        }
        fn move_by(&self, dx: i32, dy: i32) -> Option<Rect> {
            let mut rect = self.position.get()?;
            rect = Rect::new(
                rect.left + dx,
                rect.top + dy,
                rect.right + dx,
                rect.bottom + dy,
            );
            self.position.set(Some(rect));
            Some(rect)
        }
        fn escape_down(&self) -> bool {
            self.escape.get()
        }
        fn confirm(&self, message: &str, _action: &str) -> bool {
            self.confirmed.borrow_mut().push(message.to_owned());
            true
        }
    }

    #[derive(Default)]
    struct FakeHost {
        calls: Mutex<Vec<String>>,
        /// What `change_settings` changes.
        settings: Mutex<Settings>,
    }

    impl FakeHost {
        fn log(&self, call: String) {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    impl BarHost for FakeHost {
        fn quit(&self) {
            self.log("quit".into());
        }
        fn activate(&self) {}
        fn bar_visible(&self) -> bool {
            true
        }
        fn toggle_bar(&self) {}
        fn open_settings(&self, route: &str) {
            self.log(format!("open {route}"));
        }
        fn change_settings(&self, change: Box<dyn FnOnce(&mut Settings) + Send>) {
            self.log("change settings".into());
            change(&mut self.settings.lock().unwrap_or_else(|e| e.into_inner()));
        }
        fn add_sensor(&self, kind: &str, zone: Zone, index: usize) -> Option<String> {
            self.log(format!("add {kind} {} {index}", zone_id(zone)));
            Some("s_new".into())
        }
        fn remove_sensor(&self, id: &str) {
            self.log(format!("remove {id}"));
        }
        fn move_sensor(&self, id: &str, zone: Zone, index: usize) {
            self.log(format!("move {id} {} {index}", zone_id(zone)));
        }
        fn subscribe(&self, id: &str, topics: Vec<TopicRequest>) {
            self.log(format!("subscribe {id} {topics:?}"));
        }
        fn redock(&self) {
            self.log("redock".into());
        }
        fn resumed(&self) {
            self.log("resumed".into());
        }
        fn set_sensor_state(&self, id: &str, state: crate::sensors::State) {
            self.log(format!("state {id} {}", serde_json::Value::Object(state)));
        }
    }

    const WIDTH: f32 = 1200.0;

    /// The bar logic with the first-run layout, laid out on a 1200 px bar, in place.
    fn bar(settings: Settings) -> (BarLogic, FakeWindow, Arc<FakeHost>, Config) {
        let (mut logic, window, host, config) = rising_bar(settings);
        logic.frame(&window, Instant::now() + Duration::from_secs(1));
        window.frames.set(0);
        window.slides.borrow_mut().clear();
        // The start's subscriptions (the CPU graph) are not what tests look for.
        host.calls.lock().unwrap_or_else(|e| e.into_inner()).clear();
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        (logic, window, host, config)
    }

    /// The bar logic just placed, so it has started to rise (with animations).
    fn rising_bar(settings: Settings) -> (BarLogic, FakeWindow, Arc<FakeHost>, Config) {
        let mut config = Config::first_run("1.0.0");
        config.settings = settings;
        let host = Arc::new(FakeHost::default());
        let (_tx, rx) = mpsc::channel();
        let mut logic = BarLogic::new(
            config.clone(),
            crate::platform::current(),
            rx,
            Arc::clone(&host) as Arc<dyn BarHost>,
        );
        // The same text metrics on every OS, readable by the recording canvas.
        logic.text = Rc::new(crate::platform::recording::FixedMetrics);
        let window = FakeWindow::default();
        logic.on_event(&window, UiEvent::Started);
        // A 1200 × 1080 monitor with the bar at the bottom.
        let monitor = crate::platform::MonitorInfo::new(
            r"\\.\DISPLAY1".into(),
            "Test".into(),
            Rect::new(0, 0, 1200, 1080),
            Rect::new(0, 0, 1200, 1052),
            1.0,
            true,
        );
        logic.on_event(
            &window,
            UiEvent::Placed(BarGeometry {
                rect: Rect::new(0, 1052, 1200, 1080),
                edge: crate::config::model::Edge::Bottom,
                monitor,
                scale: 1.0,
            }),
        );
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        (logic, window, host, config)
    }

    /// The middle of the logo, the only sensor in the centre.
    const LOGO: PointF = PointF::new(WIDTH / 2.0, 14.0);

    fn press(logic: &mut BarLogic, window: &FakeWindow, at: PointF, alt: bool) {
        let modifiers = Modifiers {
            alt,
            ..Modifiers::default()
        };
        logic.on_event(
            window,
            UiEvent::PointerDown {
                button: PointerButton::Left,
                at,
                modifiers,
            },
        );
    }

    fn release(logic: &mut BarLogic, window: &FakeWindow, at: PointF) {
        logic.on_event(
            window,
            UiEvent::PointerUp {
                button: PointerButton::Left,
                at,
                modifiers: Modifiers::default(),
            },
        );
    }

    fn right_click(logic: &mut BarLogic, window: &FakeWindow, at: PointF, choose: MenuCommand) {
        window.choose.set(Some(choose.id()));
        logic.on_event(
            window,
            UiEvent::PointerUp {
                button: PointerButton::Right,
                at,
                modifiers: Modifiers::default(),
            },
        );
    }

    /// Whether the last frame drew a dimmed sensor.
    fn dimmed(logic: &mut BarLogic) -> bool {
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        canvas.dump().contains("opacity 0.6")
    }

    #[test]
    fn subscriptions_change_only_where_topics_differ() {
        use crate::sampler::Topic;
        let uptime = |secs| TopicRequest {
            topic: Topic::Uptime,
            interval: Duration::from_secs(secs),
        };
        let map = |list: &[(&str, u64)]| -> HashMap<String, Vec<TopicRequest>> {
            list.iter()
                .map(|(id, secs)| ((*id).to_owned(), vec![uptime(*secs)]))
                .collect()
        };
        let old = map(&[("a", 60), ("b", 60), ("c", 60)]);
        let new = map(&[("a", 60), ("b", 30), ("d", 60)]);
        assert_eq!(
            subscription_changes(&old, &new, &[]),
            [
                ("b".to_owned(), vec![uptime(30)]),
                ("c".to_owned(), Vec::new()),
                ("d".to_owned(), vec![uptime(60)]),
            ]
        );
        assert!(subscription_changes(&new, &new, &[]).is_empty());
        // A re-created sensor subscribes again, to be sent the newest samples.
        assert_eq!(
            subscription_changes(&new, &new, &["a".to_owned(), "c".to_owned()]),
            [("a".to_owned(), vec![uptime(60)])]
        );
    }

    #[test]
    fn highlight_fades_in_while_the_pointer_is_over_the_bar() {
        let mut settings = Settings::default();
        settings.bar.highlight_on_hover = true;
        let (mut logic, window, _, _) = bar(settings);
        let empty = PointF::new(300.0, 14.0);
        logic.on_event(&window, UiEvent::PointerMove(empty));
        assert_eq!(logic.highlight.target(), 1.0);
        assert_eq!(window.frames.get(), 1);
        // Moving within the bar starts nothing new.
        logic.on_event(&window, UiEvent::PointerMove(LOGO));
        assert_eq!(window.frames.get(), 1);
        logic.on_event(&window, UiEvent::PointerLeave);
        assert_eq!(logic.highlight.target(), 0.0);
        assert_eq!(window.frames.get(), 2);

        // Off by default.
        let (mut logic, window, _, _) = bar(Settings::default());
        logic.on_event(&window, UiEvent::PointerMove(empty));
        assert_eq!(logic.highlight.target(), 0.0);
        assert_eq!(window.frames.get(), 0);
    }

    #[test]
    fn rises_into_place_and_slides_out_before_changing_edges() {
        // On, so the OS "Animation effects" setting (off on CI runners) does not matter.
        let mut settings = Settings::default();
        settings.bar.animations = crate::config::model::Animations::On;
        let (mut logic, window, host, config) = rising_bar(settings);
        // Placed: 28 px below its place, transparent, then rising.
        assert_eq!(window.slides.borrow().first(), Some(&28));
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        let dump = canvas.dump();
        assert!(dump.starts_with("opacity 0"), "{dump}");
        let done = Instant::now() + RISE;
        logic.frame(&window, done);
        assert_eq!(window.slides.borrow().last(), Some(&0));
        // Frames that move nothing (e.g. for the hover highlight) leave the window where it is.
        let slides = window.slides.borrow().len();
        logic.frame(&window, done + Duration::from_millis(16));
        assert_eq!(window.slides.borrow().len(), slides);
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        assert!(!canvas.dump().starts_with("opacity"));

        // To the top: it slides out downwards first, and only then is redocked.
        let mut top = config.clone();
        top.settings.bar.edge = Edge::Top;
        logic.apply_config(&window, top);
        assert!(!host.calls().contains(&"redock".to_owned()));
        // Still at the bottom: the hairline stays on its top side.
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        assert!(
            canvas.dump().contains("fill rect 0,0 1200×1 "),
            "{}",
            canvas.dump()
        );
        logic.frame(&window, Instant::now() + RISE);
        assert_eq!(window.slides.borrow().last(), Some(&28));
        assert!(host.calls().contains(&"redock".to_owned()));
        // Placed at the top: it rises from above.
        window.slides.borrow_mut().clear();
        let mut geometry = logic.geometry.clone().expect("placed");
        geometry.edge = Edge::Top;
        geometry.rect = Rect::new(0, 0, 1200, 28);
        logic.on_event(&window, UiEvent::Placed(geometry));
        assert_eq!(window.slides.borrow().first(), Some(&-28));
    }

    #[test]
    fn an_edge_changed_back_during_the_slide_out_rises_again() {
        let mut settings = Settings::default();
        settings.bar.animations = crate::config::model::Animations::On;
        let (mut logic, window, host, config) = rising_bar(settings);
        logic.frame(&window, Instant::now() + RISE);
        // To the top and back before the slide-out ends: the placement stays at the bottom,
        // so no new one is reported.
        let mut top = config.clone();
        top.settings.bar.edge = Edge::Top;
        logic.apply_config(&window, top);
        logic.apply_config(&window, config);
        logic.frame(&window, Instant::now() + RISE);
        assert!(host.calls().contains(&"redock".to_owned()));
        assert_eq!(logic.shown.target(), 1.0, "it rises");
        logic.frame(&window, Instant::now() + 2 * RISE);
        assert_eq!(window.slides.borrow().last(), Some(&0));
    }

    #[test]
    fn a_sensors_new_state_is_stored() {
        use crate::sampler::{Delivery, Sample, Topic, UptimeSample};
        let (mut logic, window, host, mut config) = bar(Settings::default());
        let uptime = SensorInstance {
            id: "s_up".into(),
            kind: "uptime".into(),
            props: crate::sensors::Props::new(),
        };
        config.layout.left.push(uptime);
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        let sample = Sample::Uptime(UptimeSample {
            boot_unix_ms: 0,
            uptime_secs: 700,
            read_at: Instant::now(),
        });
        logic.deliver(
            &window,
            &Delivery {
                topic: Topic::Uptime,
                sample: Arc::new(sample),
                to: None,
            },
        );
        assert!(
            host.calls()
                .contains(&r#"state s_up {"longestSecs":700}"#.to_owned()),
            "{:?}",
            host.calls()
        );
    }

    #[test]
    fn a_resume_reaches_the_sampler() {
        let (mut logic, window, host, _) = bar(Settings::default());
        logic.on_event(&window, UiEvent::TimeChanged);
        assert!(!host.calls().contains(&"resumed".to_owned()));
        logic.on_event(&window, UiEvent::Resumed);
        assert!(host.calls().contains(&"resumed".to_owned()));
    }

    #[test]
    fn without_animations_the_bar_is_simply_there() {
        let mut settings = Settings::default();
        settings.bar.animations = crate::config::model::Animations::Off;
        let (mut logic, window, host, config) = rising_bar(settings);
        assert_eq!(*window.slides.borrow(), [0]);
        let mut top = config;
        top.settings.bar.edge = Edge::Top;
        logic.apply_config(&window, top);
        assert!(host.calls().contains(&"redock".to_owned()));
    }

    #[test]
    fn a_floating_bar_moves_by_its_background_while_unlocked() {
        let mut settings = Settings::default();
        settings.bar.mode = BarMode::Floating;
        let (mut logic, window, host, _) = bar(settings.clone());
        let start = Rect::new(100, 500, 1300, 528);
        window.position.set(Some(start));
        let empty = PointF::new(300.0, 14.0);
        press(&mut logic, &window, empty, false);
        assert!(window.captured.get());
        // Within the threshold nothing moves.
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(302.0, 14.0)));
        assert_eq!(window.position.get(), Some(start));
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(320.0, 4.0)));
        assert_eq!(window.position.get(), Some(Rect::new(120, 490, 1320, 518)));
        // The window followed, so the pointer is back at the pressed point inside it.
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(305.0, 14.0)));
        assert_eq!(window.position.get(), Some(Rect::new(125, 490, 1325, 518)));
        release(&mut logic, &window, PointF::new(300.0, 14.0));
        assert!(!window.captured.get());
        let stored = host
            .settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bar
            .floating;
        assert_eq!(
            stored,
            Some(FloatingRect {
                x: 125,
                y: 490,
                width: DEFAULT_FLOATING_WIDTH
            })
        );

        // Locked, or docked: the background does not move the bar.
        settings.locked = true;
        let (mut logic, window, _, _) = bar(settings);
        window.position.set(Some(start));
        press(&mut logic, &window, empty, false);
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(350.0, 14.0)));
        assert_eq!(window.position.get(), Some(start));
        let (mut logic, window, _, _) = bar(Settings::default());
        press(&mut logic, &window, empty, false);
        assert!(!window.captured.get());
    }

    #[test]
    fn a_press_dims_the_sensor_until_release() {
        let (mut logic, window, _, _) = bar(Settings::default());
        press(&mut logic, &window, LOGO, false);
        assert!(window.captured.get());
        assert!(dimmed(&mut logic));
        release(&mut logic, &window, LOGO);
        assert!(!window.captured.get());
        assert!(!dimmed(&mut logic));

        // Moving starts a drag: the sensor stays dimmed until it is dropped.
        press(&mut logic, &window, LOGO, false);
        logic.on_event(
            &window,
            UiEvent::PointerMove(PointF::new(LOGO.x + 5.0, 14.0)),
        );
        assert!(dimmed(&mut logic));
        release(&mut logic, &window, LOGO);
        assert!(!dimmed(&mut logic));
    }

    #[test]
    fn locked_alt_only_bar_ignores_presses_without_alt() {
        let settings = Settings {
            locked: true,
            clicks_when_locked: ClicksWhenLocked::AltOnly,
            ..Settings::default()
        };
        let (mut logic, window, _, _) = bar(settings);
        press(&mut logic, &window, LOGO, false);
        assert!(!window.captured.get());
        assert!(!dimmed(&mut logic));
        press(&mut logic, &window, LOGO, true);
        assert!(dimmed(&mut logic));
    }

    #[test]
    fn sensor_menu_commands() {
        let (mut logic, window, host, config) = bar(Settings::default());
        let logo = &config.layout.center[0].id;
        right_click(&mut logic, &window, LOGO, MenuCommand::Properties);
        right_click(&mut logic, &window, LOGO, MenuCommand::NewSensor);
        right_click(&mut logic, &window, LOGO, MenuCommand::Remove);
        assert_eq!(
            host.calls(),
            [
                format!("open sensor/{logo}"),
                format!("open new?target={logo}"),
                format!("remove {logo}"),
            ]
        );
        assert_eq!(*window.confirmed.borrow(), ["Remove sensor “Image”?"]);
        // The sensor menu has Properties; the logo has some, so it is enabled.
        let menus = window.menus.borrow();
        assert!(menus[0].iter().any(|e| matches!(
            e,
            MenuEntry::Item { id, enabled: true, .. } if *id == MenuCommand::Properties.id()
        )));
    }

    #[test]
    fn bar_menu_inserts_by_zone() {
        let (mut logic, window, host, _) = bar(Settings::default());
        // Left third, right of the three placeholders: appended.
        right_click(
            &mut logic,
            &window,
            PointF::new(350.0, 14.0),
            MenuCommand::NewSensor,
        );
        // Right third, left of Date and Time: first in the zone.
        right_click(
            &mut logic,
            &window,
            PointF::new(900.0, 14.0),
            MenuCommand::NewSensor,
        );
        right_click(
            &mut logic,
            &window,
            PointF::new(900.0, 14.0),
            MenuCommand::Settings,
        );
        right_click(
            &mut logic,
            &window,
            PointF::new(900.0, 14.0),
            MenuCommand::ToggleLock,
        );
        assert_eq!(
            host.calls(),
            [
                "open new?zone=left&index=3",
                "open new?zone=right&index=0",
                "open general",
                "change settings",
            ]
        );
    }

    #[test]
    fn submenu_adds_the_type_and_opens_its_properties() {
        let settings = Settings {
            new_sensor_menu: crate::config::model::NewSensorMenu::Submenu,
            ..Settings::default()
        };
        let (mut logic, window, host, _) = bar(settings);
        let types = menus::addable_types(registry(), &logic.platform.capabilities());
        let text = types
            .iter()
            .position(|t| t.id() == "text")
            .expect("text type");
        right_click(&mut logic, &window, LOGO, MenuCommand::Add(text));
        assert_eq!(host.calls(), ["add text center 1", "open sensor/s_new"]);
    }

    fn hover(logic: &mut BarLogic, window: &FakeWindow, at: PointF) {
        logic.on_event(window, UiEvent::PointerMove(at));
    }

    fn hint_timer(logic: &mut BarLogic, window: &FakeWindow) {
        logic.on_event(window, UiEvent::Timer(TimerId::Hint));
    }

    #[test]
    fn hint_shows_after_the_delay_above_the_sensor() {
        let (mut logic, window, _, _) = bar(Settings::default());
        hover(&mut logic, &window, LOGO);
        assert_eq!(
            window.hint_timer.get(),
            Some(Duration::from_millis(600)),
            "the show delay runs"
        );
        assert!(window.hint.borrow().is_none());
        hint_timer(&mut logic, &window);
        let hint = window.hint.borrow().clone().expect("shown");
        // The logo's popup text, above the bar, its pointer under the bubble.
        assert!(
            hint.1.contains(&format!("\"Infodock {}\"", crate::VERSION)),
            "{}",
            hint.1
        );
        assert!(hint.0.bottom <= 1052 + 6, "above the bar: {:?}", hint.0);
        assert!(
            hint.0.left < 600 && hint.0.right > 600,
            "centred on the logo: {:?}",
            hint.0
        );

        // Leaving: hidden after the hide delay.
        logic.on_event(&window, UiEvent::PointerLeave);
        assert_eq!(window.hint_timer.get(), Some(Duration::from_millis(100)));
        assert!(window.hint.borrow().is_some());
        hint_timer(&mut logic, &window);
        assert!(window.hint.borrow().is_none());
    }

    #[test]
    fn hint_closes_on_a_press_and_stays_closed() {
        let (mut logic, window, _, _) = bar(Settings::default());
        hover(&mut logic, &window, LOGO);
        hint_timer(&mut logic, &window);
        press(&mut logic, &window, LOGO, false);
        assert!(window.hint.borrow().is_none());
        release(&mut logic, &window, LOGO);
        hover(&mut logic, &window, PointF::new(LOGO.x + 1.0, 14.0));
        hint_timer(&mut logic, &window);
        assert!(
            window.hint.borrow().is_none(),
            "not again over the same sensor"
        );
    }

    #[test]
    fn no_hints_while_popups_are_off() {
        let settings = Settings {
            popups_enabled: false,
            ..Settings::default()
        };
        let (mut logic, window, _, _) = bar(settings);
        hover(&mut logic, &window, LOGO);
        hint_timer(&mut logic, &window);
        assert!(window.hint.borrow().is_none());
    }

    #[test]
    fn placeholders_explain_themselves() {
        let (mut logic, window, _, mut config) = bar(Settings::default());
        // A sensor of a type from a newer version, first on the left.
        let unknown = SensorInstance {
            id: "s_mail".into(),
            kind: "mail".into(),
            props: Default::default(),
        };
        config.layout.left.insert(0, unknown);
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        hover(&mut logic, &window, PointF::new(10.0, 14.0));
        hint_timer(&mut logic, &window);
        let hint = window.hint.borrow().clone().expect("shown");
        assert!(
            hint.1.contains("not available in this version"),
            "{}",
            hint.1
        );
        // Kept on screen: 7 px from the monitor's left edge, less the shadow margin.
        assert_eq!(hint.0.left, 1);
    }

    #[test]
    fn a_hint_is_rebuilt_only_when_its_sensor_changes() {
        use crate::sampler::{CpuSample, Sample, Topic};
        let (mut logic, window, _, _) = bar(Settings::default());
        hover(&mut logic, &window, LOGO);
        hint_timer(&mut logic, &window);
        assert!(window.hint.borrow().is_some(), "the logo's hint");
        *window.hint.borrow_mut() = None;
        // The CPU graph changes; the logo's hint is left as it is.
        let cpu = Sample::Cpu(CpuSample {
            total: 50.0,
            ..Default::default()
        });
        logic.deliver(
            &window,
            &Delivery {
                topic: Topic::Cpu,
                sample: Arc::new(cpu),
                to: None,
            },
        );
        assert!(window.hint.borrow().is_none(), "not shown again");
    }

    #[test]
    fn a_live_hint_follows_its_sensor() {
        let (mut logic, window, _, config) = bar(Settings::default());
        hover(&mut logic, &window, LOGO);
        hint_timer(&mut logic, &window);
        let before = window.hint.borrow().clone().expect("shown").0;
        // A sensor added in front of the logo moves it to the right.
        let mut config = config;
        let text = SensorInstance {
            id: "s_text".into(),
            kind: "text".into(),
            props: serde_json::json!({ "text": "Hello" })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        };
        config.layout.center.insert(0, text);
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        let after = window.hint.borrow().clone().expect("still shown").0;
        assert!(after.left > before.left, "{before:?} → {after:?}");
    }

    fn drag_to(logic: &mut BarLogic, window: &FakeWindow, from: PointF, to: PointF) -> bool {
        press(logic, window, from, false);
        logic.on_event(
            window,
            UiEvent::PointerMove(PointF::new(from.x + 6.0, from.y)),
        );
        logic.on_event(window, UiEvent::PointerMove(to));
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        let placer = canvas.dump().lines().any(|l| l.ends_with("#0078D4"));
        release(logic, window, to);
        placer
    }

    #[test]
    fn dragging_moves_a_sensor() {
        let (mut logic, window, host, config) = bar(Settings::default());
        let logo = config.layout.center[0].id.clone();
        // Drop the logo on the empty right part of the left third: after the left zone's last.
        assert!(
            drag_to(&mut logic, &window, LOGO, PointF::new(350.0, 14.0)),
            "placer drawn"
        );
        assert_eq!(host.calls(), [format!("move {logo} left 3")]);
        assert!(!window.captured.get());

        // The first placeholder onto the right half of the third: after it, removal counted.
        let cpu = config.layout.left[0].id.clone();
        let third_x = {
            let spans = logic.sensors.spans();
            spans[0][2].0 + spans[0][2].1 - 2.0
        };
        drag_to(
            &mut logic,
            &window,
            PointF::new(10.0, 14.0),
            PointF::new(third_x, 14.0),
        );
        assert_eq!(host.calls()[1], format!("move {cpu} left 2"));
    }

    #[test]
    fn dropping_in_place_or_pressing_esc_moves_nothing() {
        let (mut logic, window, host, _) = bar(Settings::default());
        drag_to(&mut logic, &window, LOGO, PointF::new(LOGO.x + 8.0, 14.0));
        assert!(host.calls().is_empty());

        press(&mut logic, &window, LOGO, false);
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(350.0, 14.0)));
        window.escape.set(true);
        logic.on_event(&window, UiEvent::Timer(TimerId::Drag));
        assert!(!window.captured.get(), "cancelled");
        release(&mut logic, &window, PointF::new(350.0, 14.0));
        assert!(host.calls().is_empty());
    }

    #[test]
    fn a_locked_bar_does_not_drag() {
        let settings = Settings {
            locked: true,
            ..Settings::default()
        };
        let (mut logic, window, host, _) = bar(settings);
        assert!(!drag_to(
            &mut logic,
            &window,
            LOGO,
            PointF::new(350.0, 14.0)
        ));
        assert!(host.calls().is_empty());
    }
}
