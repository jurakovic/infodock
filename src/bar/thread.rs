//! The bar logic on the bar thread (SPEC §4.1, §4.4): applies the config and the OS appearance,
//! keeps the sensors, their wake-up timer and layout, draws the bar, and handles the pointer
//! and the keyboard: clicks, context menus, the hover hint and the focus ring (§6, §7.1, §14.4).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::anim::{self, Easing, Fade};
use super::drop::{self, Dropped};
use super::focus::{self, Move};
use super::hint::{self, HintBody, HintFrame, Hover, Step};
use super::icons::IconLoader;
use super::interact::{self, DropTarget, Press};
use super::menus::{self, MenuCommand};
use super::paint::paint_bar;
use super::placement::Anchor;
use super::popup::{self, Open, PopupHost};
use super::resize::{self, Resize};
use super::sensors::{BarSensors, Env, Hit};
use super::shadow::ShadowCache;
use super::skins::{self, SkinCache};
use super::theme::Theme;
use super::tray::{self, TrayCommand};
use super::{BarHost, Command};
use crate::config::Config;
use crate::config::exchange::ImportFormat;
use crate::config::model::Zone;
use crate::config::model::{
    BarMode, Edge, FloatingRect, Layout, Material, SensorInstance, Settings,
};
use crate::i18n::{t, t_args};
use crate::platform::geometry::{DEFAULT_FLOATING_WIDTH, nearer_edge};
use crate::platform::{
    AccessibleAction, AccessibleBar, Band, BarGeometry, BarHandler, BarWindow, Brush, Canvas,
    DragEvent, DropEffect, Key, Modifiers, OsAppearance, Platform, PointF, PointerButton,
    PopupCursor, PopupInput, Rect, RectF, Shape, SizeF, TextEngine, TimerId, UiEvent, UiWaker,
};
use crate::sampler::{Change, ChangeSink, Delivery, TopicRequest};
use crate::sensors::drop::{DragInfo, DropAction};
use crate::sensors::format::Numbers;
use crate::sensors::ops::index_after_removal;
use crate::sensors::popup::{Popup, PopupAlign, PopupCtx, PopupReply};
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

/// A drag from another app over the bar (SPEC §6.3).
struct ExternalDrag {
    /// What the bar takes of the dragged items, in order.
    items: Vec<Dropped>,
    /// Where new sensors would go; none until the pointer is over the bar, and none over a
    /// sensor that takes the drop itself.
    target: Option<DropTarget>,
    /// The sensor under the pointer, the keys it was asked with, and what it would do
    /// (`None` when it leaves the drop to the bar). Asked again only when either changes.
    onto: Option<(String, DragInfo, Option<DropAction>)>,
}

/// The keyboard on the bar (SPEC §14.4), from the hotkey until Esc or the focus goes to
/// another app.
struct KeyFocus {
    /// The sensor with the focus ring; `None`: the bar itself.
    sensor: Option<String>,
    /// The bar window has the focus; while a popup or a confirmation of the bar's has it, the
    /// ring waits for it to come back.
    focused: bool,
}

/// `rect` in logical screen px as physical px at `scale`.
fn physical_rect(rect: RectF, scale: f32) -> Rect {
    let physical = |v: f32| (v * scale).round() as i32;
    Rect::new(
        physical(rect.left),
        physical(rect.top),
        physical(rect.right),
        physical(rect.bottom),
    )
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
    /// Another app drags something over the bar (SPEC §6.3).
    external: Option<ExternalDrag>,
    /// The left button is down on the background of a floating bar, which moves it (§5.1).
    moving: Option<Moving>,
    /// The left button is down on an end of a floating bar, which resizes it (§5.1).
    resizing: Option<Resizing>,
    /// The pointer's shape over the bar as last set.
    cursor: PopupCursor,
    /// When the hover hint shows and hides.
    hover: Hover,
    /// The open interactive popup (SPEC §7.2).
    popups: PopupHost,
    /// Wakes this thread from workers, e.g. a popup's.
    waker: UiWaker,
    /// Where the bar is; hints are placed from it.
    geometry: Option<BarGeometry>,
    sensors: BarSensors,
    text: Rc<dyn TextEngine>,
    /// Shell icons for the sensors, read on a worker thread.
    icons: IconLoader,
    appearance: OsAppearance,
    theme: Theme,
    /// The classic bar's skin as last drawn (SPEC §12.4).
    skin: SkinCache,
    /// The bar's own shadow as last drawn (SPEC §5.5).
    shadow: ShadowCache,
    /// The skin's shadow band or the bar's shadow as last drawn, for the platform to show beside
    /// the bar (SPEC §5.1).
    band: Option<Band>,
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
    /// The keyboard on the bar (SPEC §14.4); `None` unless the hotkey focused it.
    keys: Option<KeyFocus>,
    /// The size of the last frame, logical px.
    painted: SizeF,
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
    /// Reports what sensors' actions changed to the sampler, through the host (SPEC §4.4).
    changes: ChangeSink,
    commands: Receiver<Command>,
    host: Arc<dyn BarHost>,
}

/// A press on the background of a floating bar; past the threshold it moves the window.
struct Moving {
    press: Press,
    /// Where the window is after the last move, physical px.
    rect: Option<Rect>,
}

/// A press on an end of a floating bar, which follows the pointer at once.
struct Resizing {
    drag: Resize,
    /// The width after the last change, logical px; `None` until it changed.
    width: Option<u32>,
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
        waker: UiWaker,
        skins: PathBuf,
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
            external: None,
            moving: None,
            resizing: None,
            cursor: PopupCursor::Arrow,
            hover: Hover::default(),
            popups: PopupHost::default(),
            waker: waker.clone(),
            geometry: None,
            sensors: BarSensors::default(),
            text: crate::platform::text_engine(),
            icons: IconLoader::new(Arc::clone(&platform), waker),
            appearance,
            skin: SkinCache::new(skins),
            shadow: ShadowCache::default(),
            band: None,
            material: None,
            tray_shown: None,
            subscribed: HashMap::new(),
            animate,
            numbers,
            inside: false,
            hovered: None,
            keys: None,
            painted: SizeF::new(0.0, 0.0),
            highlight: Fade::at(0.0, Instant::now()),
            // Hidden until it rises, if it does.
            shown: Fade::at(if animate { 0.0 } else { 1.0 }, Instant::now()),
            rise_pending: true,
            sliding_out: false,
            slid: None,
            platform,
            changes: ChangeSink::new({
                let host = Arc::clone(&host);
                move |change| host.changed(change)
            }),
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
        let Some(command) = window
            .show_menu(&entries, None)
            .and_then(TrayCommand::from_id)
        else {
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
        // Any click closes the hint (SPEC §7.1) and an open popup, which keeps the focus since
        // the bar never takes it (§7.2).
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        let had_popup = self.popups.owner().map(str::to_owned);
        self.close_popup(window);
        if let (Some(end), Some(g)) = (self.resize_end(at), &self.geometry) {
            let x = g.rect.left + (f64::from(at.x) * g.scale).round() as i32;
            window.capture_pointer(true);
            self.resizing = Some(Resizing {
                drag: Resize::new(end, g.rect, x),
                width: None,
            });
            return;
        }
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
        let closed_popup = had_popup.as_deref() == Some(hit.entry.id.as_str());
        self.press = Some(Press {
            closed_popup,
            ..Press::new(hit.entry.id, at)
        });
    }

    /// The end of a floating, unlocked bar under `at`, by which it is resized (SPEC §5.1).
    fn resize_end(&self, at: PointF) -> Option<resize::End> {
        if self.settings.bar.mode != BarMode::Floating || self.settings.locked {
            return None;
        }
        let g = self.geometry.as_ref()?;
        resize::end_at(at.x, (f64::from(g.rect.width()) / g.scale) as f32)
    }

    fn set_cursor(&mut self, window: &dyn BarWindow, cursor: PopupCursor) {
        if self.cursor != cursor {
            self.cursor = cursor;
            window.set_cursor(cursor);
        }
    }

    fn pointer_moved(&mut self, window: &dyn BarWindow, at: PointF) {
        if !self.inside {
            self.inside = true;
            self.refresh_highlight(window);
        }
        let pressed = self.moving.is_some() || self.drag.is_some() || self.press.is_some();
        let resizes = self.resizing.is_some() || (!pressed && self.resize_end(at).is_some());
        let cursor = if resizes {
            PopupCursor::ResizeWidth
        } else {
            PopupCursor::Arrow
        };
        self.set_cursor(window, cursor);
        if let Some(resizing) = &mut self.resizing {
            // The dragged end follows the pointer, in screen px since the window moves with
            // the left end.
            if let Some(g) = self.geometry.as_mut() {
                let x = g.rect.left + (f64::from(at.x) * g.scale).round() as i32;
                let (rect, width) = resizing.drag.rect(x, g.scale, g.monitor.work_area);
                if rect != g.rect
                    && let Some(rect) = window.resize_to(rect, width)
                {
                    g.rect = rect;
                    resizing.width = Some(width);
                }
            }
            return;
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
                    // The nearer edge follows, so the shadow and hairline turn while it moves.
                    if let Some(g) = self.geometry.as_mut() {
                        g.rect = rect;
                        let edge = nearer_edge(rect, &g.monitor);
                        if edge != g.edge {
                            g.edge = edge;
                            window.invalidate();
                        }
                    }
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
        // No hints over an open popup; the hover state waits, so the sensor whose popup closes
        // stays without its hint while the pointer remains over it (SPEC §7.1).
        if self.popups.open.is_some() {
            return;
        }
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
        if self.resizing.is_none() {
            self.set_cursor(window, PopupCursor::Arrow);
        }
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
            changes: &self.changes,
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
        if old.edge != new.edge
            || old.shadow != new.shadow
            || config.settings.sensor_spacing != self.settings.sensor_spacing
        {
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

    /// Sensor `id`'s place on the screen, for placing its hint or popup (SPEC §7.3), in logical
    /// px, and the monitor's scale; `None` before the bar is placed or without that sensor.
    fn anchor(&self, id: &str) -> Option<(Anchor, f32)> {
        let geometry = self.geometry.as_ref()?;
        let (x, width) = self.sensors.place_of(id)?;
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
        Some((anchor, scale))
    }

    /// Shows (or updates) sensor `id`'s hint above or below it (SPEC §7.1, §7.3); `false` if
    /// it has none.
    fn show_hint(&self, window: &dyn BarWindow, id: &str) -> bool {
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: false,
            animate: self.animate,
            changes: &self.changes,
        };
        let (Some(content), Some((anchor, scale))) = (self.sensors.hint(id, &env), self.anchor(id))
        else {
            return false;
        };
        // What screen readers read of it: its text, or for a rich hint (a calendar) the
        // sensor's value (SPEC §14.4).
        let name = match &content {
            crate::sensors::HintContent::Text(text) => hint::spoken(text),
            crate::sensors::HintContent::Rich(_) => self.sensors.value(id, &env),
        };
        let Some(body) = HintBody::new(content, self.text.as_ref(), &self.theme) else {
            return false;
        };
        let frame = HintFrame::new(body.size(), &anchor, &self.theme);
        let rect = physical_rect(frame.window, scale);
        let theme = &self.theme;
        window.show_hint(rect, scale, &name, &mut |canvas| {
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
            self.sensors
                .refresh_layout(&self.theme, self.text.as_ref(), &self.icons);
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
        self.sensors
            .refresh_layout(&self.theme, self.text.as_ref(), &self.icons);
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

    /// Stores the width and place of a resized floating bar (SPEC §5.1); the *Width* setting
    /// follows.
    fn end_resizing(&mut self, window: &dyn BarWindow) {
        let Some(resizing) = self.resizing.take() else {
            return;
        };
        window.capture_pointer(false);
        if let (Some(width), Some(g)) = (resizing.width, &self.geometry) {
            let rect = g.rect;
            self.host.change_settings(Box::new(move |s| {
                s.bar.floating = Some(FloatingRect {
                    x: rect.left,
                    y: rect.top,
                    width,
                });
            }));
        }
    }

    fn pointer_up(&mut self, window: &dyn BarWindow, at: PointF, modifiers: Modifiers) {
        if self.resizing.is_some() {
            self.end_resizing(window);
            return;
        }
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
        // The press that closed this sensor's popup does not open it again, nor does a click
        // right after its popup closed by losing the focus (SPEC §7.2).
        if press.closed_popup || self.popups.guarded(&press.id, Instant::now()) {
            return;
        }
        self.click(window, &press.id, modifiers.alt);
    }

    /// Clicks sensor `id`, with the pointer or the keyboard (SPEC §6.1, §14.4).
    fn click(&mut self, window: &dyn BarWindow, id: &str, alt: bool) {
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt,
            animate: self.animate,
            changes: &self.changes,
        };
        match self.sensors.click(id, &env) {
            ClickResult::None => {}
            ClickResult::Handled => window.invalidate(),
            // Asked first (SPEC §9.21); the sensor acts only if the user agrees.
            ClickResult::Confirm(question) => {
                if window.confirm(&question.message, &question.action)
                    && self.sensors.confirmed(id, &env)
                {
                    window.invalidate();
                }
            }
            ClickResult::Popup(popup) => self.open_popup(window, id, popup),
        }
        // A click may change what a sensor shows for good (SPEC §9.14).
        self.store_states();
    }

    /// The bar or sensor menu at `at` (SPEC §6.4), and what its command does.
    fn context_menu(&mut self, window: &dyn BarWindow, at: PointF) {
        let hit = self.sensors.hit(at.x);
        // New sensors go after the sensor, or to the zone under the pointer (SPEC §6.6).
        let place = match &hit {
            Some(h) => (h.zone, h.index + 1),
            None => {
                let zone = interact::zone_at(at.x, self.sensors.bar_width());
                (
                    zone,
                    interact::insertion_index(zone, self.sensors.len(zone)),
                )
            }
        };
        self.menu(window, hit, place, None);
    }

    /// The sensor menu of `hit`, or the bar menu with new sensors going to `place` (zone and
    /// index), shown at the pointer or beside `near` (SPEC §6.4); and what its command does.
    fn menu(
        &mut self,
        window: &dyn BarWindow,
        hit: Option<Hit>,
        place: (Zone, usize),
        near: Option<Rect>,
    ) {
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        self.close_popup(window);
        self.cancel_drag(window);
        let caps = self.platform.capabilities();
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: false,
            animate: self.animate,
            changes: &self.changes,
        };
        let entries = match &hit {
            Some(h) => {
                let editable = registry().editable(&h.entry, &caps);
                let own = self.sensors.menu(&h.entry.id, &env);
                menus::sensor_menu(&self.settings, registry(), &caps, editable, &own)
            }
            None => menus::bar_menu(&self.settings, registry(), &caps),
        };
        let Some(command) = window
            .show_menu(&entries, near)
            .and_then(MenuCommand::from_id)
        else {
            return;
        };
        let (zone, index) = place;
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
            MenuCommand::ImportLayout => self.host.import_layout(ImportFormat::Json),
            MenuCommand::ExportLayout => self.host.export_layout(),
            MenuCommand::ImportLegacy => self.host.import_layout(ImportFormat::Legacy),
            // Greyed out until M5.
            MenuCommand::Help => {}
            MenuCommand::Settings => self.host.open_settings("general"),
            MenuCommand::ToggleLock => self
                .host
                .change_settings(Box::new(|s| s.locked = !s.locked)),
            MenuCommand::TogglePopups => {
                self.host
                    .change_settings(Box::new(|s| s.popups_enabled = !s.popups_enabled));
            }
            MenuCommand::Quit => self.host.quit(),
            MenuCommand::Sensor(item) => {
                if let Some(entry) = target {
                    let env = Env {
                        settings: &self.settings,
                        theme: &self.theme,
                        platform: self.platform.as_ref(),
                        alt: false,
                        animate: self.animate,
                        changes: &self.changes,
                    };
                    if self.sensors.on_menu(&entry.id, item, &env) {
                        window.invalidate();
                        self.refresh_changed_hint(window);
                    }
                }
            }
        }
    }

    /// Opens `popup` for sensor `id` next to it (SPEC §7.2): the hint closes, the sensor shows
    /// its active state, and the popup window takes the focus.
    fn open_popup(&mut self, window: &dyn BarWindow, id: &str, popup: Box<dyn Popup>) {
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        self.close_popup(window);
        self.popups.open = Some(Open {
            owner: id.to_owned(),
            popup,
            frame: None,
            room: SizeF::new(0.0, 0.0),
            monitor_height: 0.0,
            free_left: false,
            captured: false,
        });
        if self.sensors.set_active(Some(id)) {
            window.invalidate();
        }
        self.relayout_popup(window);
        self.arm_popup_timer(window);
    }

    /// Lays the open popup out, places it next to its sensor and shows it; closes it when its
    /// sensor is gone.
    fn relayout_popup(&mut self, window: &dyn BarWindow) {
        let Some(owner) = self.popups.owner().map(str::to_owned) else {
            return;
        };
        // Placed by its sensor, which may have moved.
        self.sensors
            .refresh_layout(&self.theme, self.text.as_ref(), &self.icons);
        let Some((anchor, scale)) = self.anchor(&owner) else {
            self.close_popup(window);
            return;
        };
        let mut room = popup::room(&anchor);
        let edge = anchor.edge;
        let Some(open) = self.popups.open.as_mut() else {
            return;
        };
        // A grip being dragged: no wider than its side has room for, so it does not flip.
        if open.captured && open.popup.align() == PopupAlign::LeftEdge {
            room.width = room.width.min(popup::room_beside(&anchor, open.free_left));
        }
        let cx = PopupCtx {
            theme: &self.theme,
            settings: &self.settings,
            text: self.text.as_ref(),
            scale,
            platform: &self.platform,
            icons: &self.icons.transient(),
            waker: &self.waker,
            room,
            monitor_height: anchor.monitor.height(),
            edge,
            free_left: open.free_left,
            animate: self.animate,
            now: Instant::now(),
        };
        let size = open.popup.layout(&cx);
        open.room = room;
        open.place(size, &anchor, &self.theme);
        self.render_popup(window);
    }

    /// Draws the open popup again where it is.
    fn render_popup(&self, window: &dyn BarWindow) {
        let (Some(open), Some(geometry)) = (&self.popups.open, &self.geometry) else {
            return;
        };
        let Some(frame) = open.frame else {
            return;
        };
        let scale = geometry.scale as f32;
        let theme = &self.theme;
        let name = Some(open.popup.name())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| t("app.name"));
        let rect = physical_rect(frame.window, scale);
        window.show_popup(rect, scale, &name, &mut |canvas| {
            popup::paint(canvas, open, theme);
        });
    }

    /// Closes the open popup, if any: its window hides, its sensor's active state ends, and
    /// the sensor ignores clicks for a moment (SPEC §7.2).
    fn close_popup(&mut self, window: &dyn BarWindow) {
        let Some(mut open) = self.popups.close(Instant::now()) else {
            return;
        };
        if open.captured {
            window.capture_popup_pointer(false);
        }
        window.hide_popup();
        window.set_timer(TimerId::Popup, None);
        if let Some(changes) = open.popup.take_props() {
            self.host.update_sensor(&open.owner, changes);
        }
        if self.sensors.set_active(None) {
            window.invalidate();
        }
    }

    /// Carries out what the popup asked for after an input, a wake-up or a poll.
    fn popup_reply(&mut self, window: &dyn BarWindow, reply: PopupReply) {
        if let Some(open) = self.popups.open.as_mut() {
            // The pointer's shape and capture follow the popup (a grip being dragged).
            window.set_popup_cursor(open.popup.cursor());
            let captures = open.popup.captures();
            if open.captured != captures {
                open.captured = captures;
                window.capture_popup_pointer(captures);
            }
            if let Some(changes) = open.popup.take_props() {
                let owner = open.owner.clone();
                self.host.update_sensor(&owner, changes);
            }
        }
        match reply {
            PopupReply::Nothing => {}
            PopupReply::Redraw => self.render_popup(window),
            PopupReply::Relayout => self.relayout_popup(window),
            PopupReply::Close => {
                self.close_popup(window);
                self.refocus(window);
            }
        }
        self.arm_popup_timer(window);
    }

    /// Runs `f` on the open popup with its context; `None` when none is open.
    fn with_popup(
        &mut self,
        f: impl FnOnce(&mut Open, &PopupCtx) -> PopupReply,
    ) -> Option<PopupReply> {
        let scale = self.geometry.as_ref().map_or(1.0, |g| g.scale as f32);
        let edge = self
            .geometry
            .as_ref()
            .map_or(self.settings.bar.edge, |g| g.edge);
        let open = self.popups.open.as_mut()?;
        let cx = PopupCtx {
            theme: &self.theme,
            settings: &self.settings,
            text: self.text.as_ref(),
            scale,
            platform: &self.platform,
            icons: &self.icons.transient(),
            waker: &self.waker,
            room: open.room,
            monitor_height: open.monitor_height,
            edge,
            free_left: open.free_left,
            animate: self.animate,
            now: Instant::now(),
        };
        Some(f(open, &cx))
    }

    /// Input in the popup window: Esc closes it (SPEC §7.2), the rest goes to the popup.
    fn popup_input(&mut self, window: &dyn BarWindow, input: PopupInput) {
        if matches!(
            input,
            PopupInput::Key {
                key: Key::Escape,
                ..
            }
        ) {
            self.close_popup(window);
            self.refocus(window);
            return;
        }
        let reply = self.with_popup(|open, cx| {
            let input = open.to_content(input);
            open.popup.input(&input, cx)
        });
        if let Some(reply) = reply {
            self.popup_reply(window, reply);
        }
    }

    /// Sets the popup timer to the open popup's next wake-up.
    fn arm_popup_timer(&self, window: &dyn BarWindow) {
        let now = Instant::now();
        let at = self.popups.open.as_ref().and_then(|o| o.popup.wake_at());
        window.set_timer(
            TimerId::Popup,
            at.map(|at| at.saturating_duration_since(now)),
        );
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
            if theme.skin.is_none() {
                // The modern bar draws no skin: the last one need not stay in memory.
                self.skin.forget();
            }
            self.theme = theme;
            self.sensors.invalidate_all();
            window.invalidate();
            self.refresh_hint(window);
            self.relayout_popup(window);
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
        let order = self.sensors.focus_order();
        if self.sensors.sync(&shown, &self.state, &caps, registry()) {
            // A removed sensor's focus goes to the one that took its place (SPEC §14.4).
            if let Some(keys) = self.keys.as_mut()
                && let Some(id) = keys.sensor.take()
            {
                keys.sensor = focus::after_change(&order, &self.sensors.focus_order(), &id);
            }
            // New clocks read the time now, so the next frame does not show them empty.
            let env = Env {
                settings: &self.settings,
                theme: &self.theme,
                platform: self.platform.as_ref(),
                alt: false,
                animate: self.animate,
                changes: &self.changes,
            };
            self.sensors.wake(Instant::now(), &env);
            self.store_states();
            window.invalidate();
            self.refresh_hint(window);
            // Its sensor may have moved, or gone.
            self.relayout_popup(window);
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
            changes: &self.changes,
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
            changes: &self.changes,
        };
        if self.sensors.clock_changed(Instant::now(), &env) {
            window.invalidate();
            self.refresh_hint(window);
        }
        self.arm_timer(window);
    }

    /// Hands the state and props sensors want stored to the host (SPEC §11.2, §9.14).
    fn store_states(&mut self) {
        for (id, state) in self.sensors.take_states() {
            self.host.set_sensor_state(&id, state);
        }
        for (id, changes) in self.sensors.take_props() {
            self.host.update_sensor(&id, changes);
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
                    changes: &self.changes,
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
            TimerId::Popup => {
                if let Some(reply) = self.with_popup(|open, cx| open.popup.on_wake(cx)) {
                    self.popup_reply(window, reply);
                }
            }
        }
    }
}

impl BarLogic {
    /// The bar was hidden: nothing may stay on screen pointing at a bar that is gone.
    fn hidden(&mut self, window: &dyn BarWindow) {
        self.leave_keyboard(window, true);
        self.cancel_drag(window);
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        self.close_popup(window);
    }

    /// Another app's drag enters the bar: the hint closes, and the items the bar can take are
    /// kept for the drop (SPEC §6.3).
    fn drag_enter(&mut self, window: &dyn BarWindow, items: &[crate::platform::DragItem]) {
        self.cancel_drag(window);
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        let items = items.iter().filter_map(drop::classify).collect();
        self.external = Some(ExternalDrag {
            items,
            target: None,
            onto: None,
        });
    }

    /// The pointer of another app's drag is at `at`: a sensor under it may take the drop
    /// itself (it is darkened and a floating hint says what would happen, SPEC §9.16, §9.19);
    /// otherwise the placer shows where new sensors would go. Returns the effect.
    fn drag_over(&mut self, window: &dyn BarWindow, at: PointF, drag: DragInfo) -> DropEffect {
        let target = interact::drop_target(
            at.x,
            &self.sensors.spans(),
            self.sensors.bar_width(),
            self.theme.pad,
            self.theme.gap,
        );
        let hit = self.sensors.hit(at.x).map(|h| h.entry.id);
        let Some(external) = self.external.as_mut() else {
            return DropEffect::None;
        };
        if external.items.is_empty() {
            return DropEffect::None;
        }
        let asked = external
            .onto
            .as_ref()
            .is_some_and(|(id, info, _)| Some(id) == hit.as_ref() && *info == drag);
        if !asked {
            let env = Env {
                settings: &self.settings,
                theme: &self.theme,
                platform: self.platform.as_ref(),
                alt: drag.modifiers.alt,
                animate: self.animate,
                changes: &self.changes,
            };
            external.onto = hit.map(|id| {
                let action = self.sensors.drop_action(&id, &external.items, drag, &env);
                (id, drag, action)
            });
        }
        let taken = external
            .onto
            .as_ref()
            .and_then(|(id, _, action)| Some((id.clone(), action.clone()?)));
        let placer = taken.is_none().then_some(target);
        let mut changed = external.target != placer;
        external.target = placer;
        changed |= self
            .sensors
            .set_drop_target(taken.as_ref().map(|(id, _)| id.as_str()));
        if changed {
            window.invalidate();
        }
        match taken {
            Some((id, action)) => {
                if !asked {
                    match &action.hint {
                        Some(text) => self.show_drop_hint(window, &id, text),
                        None => window.hide_hint(),
                    }
                }
                action.effect
            }
            None => {
                if !asked {
                    window.hide_hint();
                }
                drop::creation_effect(drag.allowed)
            }
        }
    }

    /// Shows the floating drop hint `text` above (or below) sensor `id` (SPEC §9.19).
    fn show_drop_hint(&self, window: &dyn BarWindow, id: &str, text: &str) {
        let Some((anchor, scale)) = self.anchor(id) else {
            return;
        };
        let content = crate::sensors::HintContent::Text(text.to_owned());
        let Some(body) = HintBody::new(content, self.text.as_ref(), &self.theme) else {
            return;
        };
        let frame = HintFrame::new(body.size(), &anchor, &self.theme);
        let rect = physical_rect(frame.window, scale);
        let theme = &self.theme;
        window.show_hint(rect, scale, &hint::spoken(text), &mut |canvas| {
            hint::paint(canvas, &frame, &body, theme);
        });
    }

    /// Another app's drag left, or dropped: the placer, the darkened sensor and the floating
    /// hint go.
    fn drag_end(&mut self, window: &dyn BarWindow) -> Option<ExternalDrag> {
        let external = self.external.take();
        let darkened = self.sensors.set_drop_target(None);
        if darkened || external.as_ref().is_some_and(|e| e.target.is_some()) {
            window.invalidate();
        }
        if external.as_ref().is_some_and(|e| e.onto.is_some()) {
            window.hide_hint();
        }
        external
    }

    /// Drops the dragged items: onto the sensor under the pointer if it takes them, otherwise
    /// one new sensor each, in order, where the placer was (SPEC §6.3), without Properties.
    fn drag_drop(&mut self, window: &dyn BarWindow, at: PointF, drag: DragInfo) -> DropEffect {
        let effect = self.drag_over(window, at, drag);
        let Some(external) = self.drag_end(window) else {
            return DropEffect::None;
        };
        if effect == DropEffect::None {
            return DropEffect::None;
        }
        if let Some((id, _, Some(_))) = &external.onto {
            let env = Env {
                settings: &self.settings,
                theme: &self.theme,
                platform: self.platform.as_ref(),
                alt: drag.modifiers.alt,
                animate: self.animate,
                changes: &self.changes,
            };
            self.sensors.drop_items(id, &external.items, drag, &env);
            return effect;
        }
        let Some(target) = external.target else {
            return DropEffect::None;
        };
        // Which paths are files and which folders is asked now, once.
        let platform = &self.platform;
        let sensors: Vec<_> = external
            .items
            .iter()
            .filter_map(|item| drop::new_sensor(item, |path| platform.path_kind(path)))
            .collect();
        if sensors.is_empty() {
            return DropEffect::None;
        }
        self.host
            .add_sensors(target.zone, target.insertion, sensors);
        effect
    }
}

impl BarLogic {
    /// The hotkey focuses the bar (SPEC §14.4): the window takes the keyboard focus and the
    /// ring goes around the bar itself, or stays on its sensor when the bar had the focus.
    fn focus_bar(&mut self, window: &dyn BarWindow) {
        if !self.host.bar_visible() {
            return;
        }
        let sensor = match &self.keys {
            Some(keys) if keys.focused => keys.sensor.clone(),
            _ => None,
        };
        self.keys = window.take_focus().then_some(KeyFocus {
            sensor,
            focused: true,
        });
        window.invalidate();
    }

    /// The sensor with the focus ring.
    fn focused_sensor(&self) -> Option<String> {
        self.keys.as_ref()?.sensor.clone()
    }

    /// A key on the focused bar: Tab and the arrows move the ring, Enter and Space click its
    /// sensor, Shift+F10 and the Menu key open its menu, Esc gives the focus back (§14.4).
    fn key(&mut self, window: &dyn BarWindow, key: Key, modifiers: Modifiers) {
        if !self.keys.as_ref().is_some_and(|k| k.focused) {
            return;
        }
        match key {
            Key::Tab if modifiers.shift => self.move_focus(window, Move::Previous),
            Key::Tab | Key::Right => self.move_focus(window, Move::Next),
            Key::Left => self.move_focus(window, Move::Previous),
            Key::Home => self.move_focus(window, Move::First),
            Key::End => self.move_focus(window, Move::Last),
            // A deliberate key, so the Lock rules for the pointer do not apply.
            Key::Enter | Key::Space => {
                if let Some(id) = self.focused_sensor() {
                    let step = self.hover.dismiss();
                    self.apply_hint(window, step);
                    self.click(window, &id, modifiers.alt);
                }
            }
            Key::Menu => self.keyboard_menu(window),
            Key::F10 if modifiers.shift => self.keyboard_menu(window),
            Key::Escape => self.leave_keyboard(window, true),
            _ => {}
        }
    }

    /// Moves the focus ring; the sensor's hint follows after the show delay, as on hover.
    fn move_focus(&mut self, window: &dyn BarWindow, to: Move) {
        let order = self.sensors.focus_order();
        let Some(keys) = self.keys.as_mut() else {
            return;
        };
        let next = focus::step(&order, keys.sensor.as_deref(), to);
        if next == keys.sensor {
            return;
        }
        keys.sensor = next;
        window.invalidate();
        if self.popups.open.is_none() {
            let id = self
                .focused_sensor()
                .filter(|_| self.settings.popups_enabled);
            let popups = &self.settings.popups;
            let step = self.hover.over(
                id.as_deref(),
                Duration::from_millis(u64::from(popups.show_delay_ms)),
                Duration::from_millis(u64::from(popups.hide_delay_ms)),
            );
            self.apply_hint(window, step);
        }
    }

    /// The focused sensor's menu, or the bar menu with no sensor focused, beside it (§6.4).
    fn keyboard_menu(&mut self, window: &dyn BarWindow) {
        let hit = self.focused_sensor().and_then(|id| self.sensors.get(&id));
        // The bar menu's new sensors go to the end of the left zone.
        let place = match &hit {
            Some(h) => (h.zone, h.index + 1),
            None => (
                Zone::Left,
                interact::insertion_index(Zone::Left, self.sensors.len(Zone::Left)),
            ),
        };
        let near = match &hit {
            Some(h) => self
                .anchor(&h.entry.id)
                .map(|(anchor, scale)| physical_rect(anchor.rect, scale)),
            None => self.geometry.as_ref().map(|g| g.rect),
        };
        self.menu(window, hit, place, near);
    }

    /// Ends the keyboard on the bar: the ring and its hint go, and with `restore` the focus
    /// goes back to the window that had it before the hotkey (Esc, hiding the bar).
    fn leave_keyboard(&mut self, window: &dyn BarWindow, restore: bool) {
        let Some(keys) = self.keys.take() else {
            return;
        };
        window.invalidate();
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
        if restore && keys.focused {
            window.restore_focus();
        }
    }

    /// The bar window lost the focus: to a popup or a confirmation of its own the ring waits
    /// for it to come back; anywhere else (a click elsewhere) the keyboard on the bar ends.
    fn focus_lost(&mut self, window: &dyn BarWindow, within_app: bool) {
        match self.keys.as_mut() {
            Some(keys) if within_app => {
                keys.focused = false;
                window.invalidate();
            }
            Some(_) => self.leave_keyboard(window, false),
            None => {}
        }
    }

    fn focus_gained(&mut self, window: &dyn BarWindow) {
        if let Some(keys) = self.keys.as_mut().filter(|k| !k.focused) {
            keys.focused = true;
            window.invalidate();
        }
    }

    /// What a screen reader asks of a sensor (SPEC §14.4): a click, its popup opened or
    /// closed, the focus ring moved to it. Deliberate, like a key, so the Lock rules for the
    /// pointer do not apply.
    fn accessible_action(&mut self, window: &dyn BarWindow, action: AccessibleAction) {
        let (AccessibleAction::Invoke(id)
        | AccessibleAction::Expand(id)
        | AccessibleAction::Collapse(id)
        | AccessibleAction::Focus(id)) = &action;
        if self.sensors.find(id).is_none() {
            return;
        }
        let open = self.popups.owner() == Some(id.as_str());
        match action {
            AccessibleAction::Invoke(id) => {
                let step = self.hover.dismiss();
                self.apply_hint(window, step);
                self.click(window, &id, false);
            }
            AccessibleAction::Expand(id) if !open => {
                let step = self.hover.dismiss();
                self.apply_hint(window, step);
                self.click(window, &id, false);
            }
            AccessibleAction::Collapse(_) if open => {
                self.close_popup(window);
                self.refocus(window);
            }
            AccessibleAction::Expand(_) | AccessibleAction::Collapse(_) => {}
            AccessibleAction::Focus(id) => {
                if !self.keys.as_ref().is_some_and(|k| k.focused) {
                    self.focus_bar(window);
                }
                if let Some(keys) = self.keys.as_mut() {
                    keys.sensor = Some(id);
                    window.invalidate();
                }
            }
        }
    }

    /// A popup opened from the keyboard closed by itself (Esc, an item chosen): the bar takes
    /// the focus back, with the ring where it was (SPEC §14.4).
    fn refocus(&mut self, window: &dyn BarWindow) {
        if self.keys.as_ref().is_some_and(|k| !k.focused) {
            if window.take_focus() {
                self.focus_gained(window);
            } else {
                self.leave_keyboard(window, false);
            }
        }
    }
}

impl BarHandler for BarLogic {
    fn on_drag(&mut self, window: &dyn BarWindow, event: DragEvent) -> DropEffect {
        match event {
            DragEvent::Enter {
                items,
                at,
                modifiers,
                allowed,
            } => {
                self.drag_enter(window, &items);
                self.drag_over(window, at, DragInfo { modifiers, allowed })
            }
            DragEvent::Over {
                at,
                modifiers,
                allowed,
            } => self.drag_over(window, at, DragInfo { modifiers, allowed }),
            DragEvent::Leave => {
                self.drag_end(window);
                DropEffect::None
            }
            DragEvent::Drop {
                at,
                modifiers,
                allowed,
            } => self.drag_drop(window, at, DragInfo { modifiers, allowed }),
        }
    }

    fn on_event(&mut self, window: &dyn BarWindow, event: UiEvent) {
        // A press on the bar after the focus went elsewhere ends what is left of the keyboard
        // on it, so the ring does not come back with a menu's activation.
        if matches!(event, UiEvent::PointerDown { .. })
            && self.keys.as_ref().is_some_and(|k| !k.focused)
        {
            self.keys = None;
        }
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
                        Command::Hidden => self.hidden(window),
                        Command::Focus => self.focus_bar(window),
                        Command::SkinsChanged => {
                            self.skin.forget();
                            window.invalidate();
                        }
                    }
                }
                // Icons that arrived: the sensors (and a popup) that asked for them measure again.
                if self.icons.receive() {
                    self.sensors.invalidate_all();
                    window.invalidate();
                    self.relayout_popup(window);
                }
                // Work from the popup's own threads, e.g. a folder listing.
                if let Some(reply) = self.with_popup(|open, cx| open.popup.poll(cx)) {
                    self.popup_reply(window, reply);
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
            UiEvent::Popup(input) => self.popup_input(window, input),
            UiEvent::PopupDeactivated => {
                // The focus went elsewhere, not back to the bar.
                if self.keys.as_ref().is_some_and(|k| !k.focused) {
                    self.leave_keyboard(window, false);
                }
                self.close_popup(window);
            }
            UiEvent::Key { key, modifiers } => self.key(window, key, modifiers),
            UiEvent::Accessible(action) => self.accessible_action(window, action),
            UiEvent::FocusGained => self.focus_gained(window),
            UiEvent::FocusLost { within_app } => self.focus_lost(window, within_app),
            UiEvent::CaptureLost => {
                self.cancel_drag(window);
                self.end_moving(window);
                self.end_resizing(window);
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
                self.relayout_popup(window);
            }
            // Full-screen apps close the hint and the popup (SPEC §5.3).
            UiEvent::Fullscreen(true) => {
                let step = self.hover.dismiss();
                self.apply_hint(window, step);
                self.close_popup(window);
            }
            UiEvent::Fullscreen(false) => {}
            UiEvent::CloseRequested => self.host.quit(),
            UiEvent::TimeChanged => self.clock_changed(window),
            UiEvent::LocaleChanged => self.locale_changed(window),
            UiEvent::Resumed => {
                self.host.resumed();
                self.clock_changed(window);
            }
            UiEvent::DevicesChanged => self.host.changed(Change::Devices),
            UiEvent::PowerChanged => self.host.changed(Change::Power),
            UiEvent::Activate => self.host.activate(),
            UiEvent::TrayClick => self.host.toggle_bar(),
            UiEvent::TrayMenu => self.tray_menu(window),
        }
    }

    fn paint(&mut self, canvas: &mut dyn Canvas, size: SizeF) {
        self.painted = size;
        let cx = LayoutCtx {
            theme: &self.theme,
            text: self.text.as_ref(),
            scale: canvas.scale(),
            numbers: self.numbers,
            icons: &self.icons,
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
        let target = skins::Target::of(size, canvas.scale(), edge);
        let skin = self
            .theme
            .skin
            .as_ref()
            .and_then(|spec| self.skin.get(spec, target));
        // A band goes on the side facing the screen centre: above a bar at the bottom. The
        // skin's own band takes the place of the bar's shadow (SPEC §5.5). None while the
        // content fades, which a floating bar does in place.
        let above = edge == Edge::Bottom;
        let bitmap = match skin.and_then(|s| s.band.clone()) {
            _ if opacity < 1.0 => None,
            Some(band) => Some(band),
            None if self.settings.bar.shadow => self.shadow.get(target.width, target.scale, above),
            None => None,
        };
        self.band = bitmap.map(|bitmap| Band { bitmap, above });
        paint_bar(
            canvas,
            size,
            &self.theme,
            edge,
            highlight,
            skin.map(|s| &s.bar),
        );
        self.sensors.draw(canvas, &self.theme, size.height);
        // The placer: a 2 px line in the accent colour where the dragged sensor, or the
        // sensors a drop creates, would go (SPEC §6.2, §6.3, §12.2).
        let placer = self.drag.as_ref().and_then(|d| d.target).or_else(|| {
            self.external
                .as_ref()
                .filter(|e| !e.items.is_empty())
                .and_then(|e| e.target)
        });
        if let Some(target) = placer {
            let x = canvas.snap(target.placer_x - 1.0);
            canvas.fill_rect(
                RectF::new(x, 2.0, x + 2.0, size.height - 2.0),
                self.theme.accent,
            );
        }
        // The focus ring in the accent colour, while the bar has the keyboard (SPEC §14.4).
        if let Some(keys) = self.keys.as_ref().filter(|k| k.focused) {
            let place = keys
                .sensor
                .as_deref()
                .and_then(|id| self.sensors.place_of(id));
            let rect = focus::ring(place, size, self.theme.gap);
            canvas.stroke(
                &Shape::RoundedRect {
                    rect,
                    radius: focus::RING_RADIUS,
                },
                &Brush::Solid(self.theme.accent),
                focus::RING_WIDTH,
            );
        }
        if opacity < 1.0 {
            canvas.pop();
        }
    }

    fn band(&self) -> Option<Band> {
        self.band.clone()
    }

    fn accessible(&self) -> AccessibleBar {
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: false,
            animate: self.animate,
            changes: &self.changes,
        };
        let items =
            self.sensors
                .accessible(&env, self.painted.height, self.popups.owner(), |kind| {
                    registry().type_name(kind)
                });
        let focus = self
            .focused_sensor()
            .and_then(|id| items.iter().position(|i| i.id == id));
        AccessibleBar {
            items,
            focused: self.keys.as_ref().is_some_and(|k| k.focused),
            focus,
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
        /// Where each menu was shown: at the pointer (`None`) or beside a rect.
        menus_near: RefCell<Vec<Option<Rect>>>,
        /// The bar took the keyboard focus, and gave it back this often.
        focus_taken: Cell<u32>,
        focus_restored: Cell<u32>,
        /// The window cannot take the focus.
        no_focus: Cell<bool>,
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
        /// Answer confirmations with Cancel.
        decline: Cell<bool>,
        /// The shown hint: where, and what it draws; and its title, for screen readers.
        hint: RefCell<Option<(Rect, String)>>,
        hint_name: RefCell<String>,
        popup_name: RefCell<String>,
        /// The shown popup: where, and what it draws; and how often it was drawn.
        popup: RefCell<Option<(Rect, String)>>,
        popup_draws: Cell<u32>,
        popup_timer: Cell<Option<std::time::Duration>>,
        popup_cursor: Cell<crate::platform::PopupCursor>,
        cursor: Cell<PopupCursor>,
        popup_captured: Cell<bool>,
    }

    impl BarWindow for FakeWindow {
        fn invalidate(&self) {}
        fn set_material(&self, _material: Material) {}
        fn set_tray_icon(&self, _tooltip: Option<&str>) {}
        fn show_menu(&self, entries: &[MenuEntry], near: Option<Rect>) -> Option<u32> {
            self.menus.borrow_mut().push(entries.to_vec());
            self.menus_near.borrow_mut().push(near);
            self.choose.take()
        }
        fn take_focus(&self) -> bool {
            self.focus_taken.set(self.focus_taken.get() + 1);
            !self.no_focus.get()
        }
        fn restore_focus(&self) {
            self.focus_restored.set(self.focus_restored.get() + 1);
        }
        fn set_timer(&self, id: TimerId, delay: Option<std::time::Duration>) {
            match id {
                TimerId::Hint => self.hint_timer.set(delay),
                TimerId::Frame => self.frames.set(self.frames.get() + 1),
                TimerId::Popup => self.popup_timer.set(delay),
                _ => {}
            }
        }
        fn show_hint(
            &self,
            rect: Rect,
            scale: f32,
            name: &str,
            paint: &mut dyn FnMut(&mut dyn Canvas),
        ) {
            let mut canvas = RecordingCanvas::new(scale);
            paint(&mut canvas);
            *self.hint.borrow_mut() = Some((rect, canvas.dump()));
            *self.hint_name.borrow_mut() = name.to_owned();
        }
        fn show_popup(
            &self,
            rect: Rect,
            scale: f32,
            name: &str,
            paint: &mut dyn FnMut(&mut dyn Canvas),
        ) {
            let mut canvas = RecordingCanvas::new(scale);
            paint(&mut canvas);
            *self.popup.borrow_mut() = Some((rect, canvas.dump()));
            *self.popup_name.borrow_mut() = name.to_owned();
            self.popup_draws.set(self.popup_draws.get() + 1);
        }
        fn hide_popup(&self) {
            *self.popup.borrow_mut() = None;
        }
        fn set_popup_cursor(&self, cursor: crate::platform::PopupCursor) {
            self.popup_cursor.set(cursor);
        }
        fn capture_popup_pointer(&self, capture: bool) {
            self.popup_captured.set(capture);
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
        fn resize_to(&self, rect: Rect, _width: u32) -> Option<Rect> {
            self.position.get()?;
            self.position.set(Some(rect));
            Some(rect)
        }
        fn set_cursor(&self, cursor: PopupCursor) {
            self.cursor.set(cursor);
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
            !self.decline.get()
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
        fn add_sensors(
            &self,
            zone: Zone,
            index: usize,
            sensors: Vec<(&'static str, crate::sensors::Props)>,
        ) {
            for (i, (kind, props)) in sensors.into_iter().enumerate() {
                self.log(format!(
                    "add {kind} {} {} {}",
                    zone_id(zone),
                    index + i,
                    serde_json::Value::Object(props)
                ));
            }
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
        fn changed(&self, change: Change) {
            self.log(format!("changed {change:?}"));
        }
        fn set_sensor_state(&self, id: &str, state: crate::sensors::State) {
            self.log(format!("state {id} {}", serde_json::Value::Object(state)));
        }
        fn update_sensor(&self, id: &str, changes: crate::sensors::Props) {
            self.log(format!(
                "update {id} {}",
                serde_json::Value::Object(changes)
            ));
        }
        fn import_layout(&self, format: ImportFormat) {
            self.log(format!("import {format:?}"));
        }
        fn export_layout(&self) {
            self.log("export".into());
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
            UiWaker::new(|| {}),
            PathBuf::from("no-skins"),
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

    #[test]
    fn a_skin_packs_band_goes_on_the_side_facing_the_screen_centre() {
        use crate::bar::skins::tests::{decode_png, pack, rows_png};
        let skins = tempfile::tempdir().expect("temp dir");
        let folder = pack(
            skins.path(),
            "stripes",
            r#"{ "image": "t.png", "shadowBand": 2 }"#,
        );
        rows_png(
            &folder.join("t.png"),
            3,
            &[[10, 20].as_slice(), &[100; 25]].concat(),
        );
        let mut settings = Settings {
            theme: crate::config::model::Theme::Classic,
            ..Settings::default()
        };
        settings.classic.skin = "stripes".into();
        let (mut logic, _window, _host, _config) = bar(settings);
        logic.skin = SkinCache::with_decoder(skins.path().into(), decode_png);
        let size = SizeF::new(WIDTH, 25.0);
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        // At the bottom of the screen the band goes above the bar, as wide as the bar.
        let band = logic.band().expect("a band");
        assert!(band.above);
        assert_eq!((band.bitmap.width(), band.bitmap.height()), (1200, 2));
        // The next frame hands over the same band, which the window need not draw again.
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert!(logic.band().expect("a band").same(&band));
        // Placed at the top: below it.
        if let Some(g) = logic.geometry.as_mut() {
            g.edge = Edge::Top;
        }
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert!(!logic.band().expect("a band").above);
        // Built-in skins and the modern bar have none.
        logic.theme.skin = None;
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert_eq!(logic.band(), None);
    }

    #[test]
    fn a_pack_installed_later_is_drawn_once_the_bar_hears_of_it() {
        use crate::bar::skins::tests::{decode_png, pack, rows_png};
        let skins = tempfile::tempdir().expect("temp dir");
        let mut settings = Settings {
            theme: crate::config::model::Theme::Classic,
            ..Settings::default()
        };
        settings.classic.skin = "stripes".into();
        let (mut logic, window, _host, _config) = bar(settings);
        logic.skin = SkinCache::with_decoder(skins.path().into(), decode_png);
        let size = SizeF::new(WIDTH, 25.0);
        // Not installed: Gloss, without a band.
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert_eq!(logic.band(), None);
        let folder = pack(
            skins.path(),
            "stripes",
            r#"{ "image": "t.png", "shadowBand": 2 }"#,
        );
        rows_png(
            &folder.join("t.png"),
            3,
            &[[10, 20].as_slice(), &[100; 25]].concat(),
        );
        // The skin drawn is kept until the bar hears of it.
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert_eq!(logic.band(), None);
        // Settings installed it and tells the bar (SPEC §12.4).
        let (commands, queue) = mpsc::channel();
        logic.commands = queue;
        commands.send(Command::SkinsChanged).expect("queued");
        logic.on_event(&window, UiEvent::Wake);
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert!(logic.band().is_some());
    }

    #[test]
    fn the_bars_shadow_goes_where_a_skins_band_would() {
        use crate::bar::skins::tests::{decode_png, pack, rows_png};
        let mut settings = Settings::default();
        settings.bar.shadow = true;
        // None while the bar rises and its content fades in (animations on, whatever the OS
        // says: CI's Windows runner has them off).
        let mut animated = settings.clone();
        animated.bar.animations = crate::config::model::Animations::On;
        let (mut rising, ..) = rising_bar(animated);
        rising.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        assert_eq!(rising.band(), None);
        let (mut logic, window, _host, config) = bar(settings);
        let size = SizeF::new(WIDTH, 28.0);
        // The modern bar at the bottom of the screen: 6 px of shadow above it, as wide as it.
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        let band = logic.band().expect("a shadow");
        assert!(band.above);
        assert_eq!((band.bitmap.width(), band.bitmap.height()), (1200, 6));
        // The next frame hands over the same shadow; at 150 % it is wider and 9 px deep.
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert!(logic.band().expect("a shadow").same(&band));
        logic.paint(&mut RecordingCanvas::new(1.5), size);
        let scaled = logic.band().expect("a shadow").bitmap;
        assert_eq!((scaled.width(), scaled.height()), (1800, 9));
        // Placed at the top (or floating nearer the top): below it.
        if let Some(g) = logic.geometry.as_mut() {
            g.edge = Edge::Top;
        }
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert!(!logic.band().expect("a shadow").above);

        // A skin pack's own band takes its place (SPEC §12.4).
        let skins = tempfile::tempdir().expect("temp dir");
        let folder = pack(
            skins.path(),
            "stripes",
            r#"{ "image": "t.png", "shadowBand": 2 }"#,
        );
        rows_png(
            &folder.join("t.png"),
            3,
            &[[10, 20].as_slice(), &[100; 25]].concat(),
        );
        let mut classic = config.clone();
        classic.settings.theme = crate::config::model::Theme::Classic;
        classic.settings.classic.skin = "stripes".into();
        logic.skin = SkinCache::with_decoder(skins.path().into(), decode_png);
        logic.apply_config(&window, classic.clone());
        logic.refresh_theme(&window);
        let size = SizeF::new(WIDTH, 25.0);
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert_eq!(logic.band().expect("the pack's band").bitmap.height(), 2);
        // A built-in skin has none: the shadow again.
        classic.settings.classic.skin = "gloss".into();
        logic.apply_config(&window, classic.clone());
        logic.refresh_theme(&window);
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert_eq!(logic.band().expect("a shadow").bitmap.height(), 6);
        // Turned off: nothing beside the bar.
        classic.settings.bar.shadow = false;
        logic.apply_config(&window, classic);
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert_eq!(logic.band(), None);
    }

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
                pushed: false,
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

        // Dragged into the upper half, its shadow turns below it at once (SPEC §5.1).
        settings.bar.shadow = true;
        let (mut logic, window, _, _) = bar(settings.clone());
        window.position.set(Some(start));
        let size = SizeF::new(WIDTH, 28.0);
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert!(logic.band().expect("a shadow").above);
        press(&mut logic, &window, empty, false);
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(300.0, -300.0)));
        logic.paint(&mut RecordingCanvas::new(1.0), size);
        assert!(!logic.band().expect("a shadow").above);
        release(&mut logic, &window, PointF::new(300.0, -300.0));

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
    fn a_floating_bar_resizes_by_its_ends_while_unlocked() {
        let mut settings = Settings::default();
        settings.bar.mode = BarMode::Floating;
        let place = |logic: &mut BarLogic, window: &FakeWindow, rect: Rect| {
            window.position.set(Some(rect));
            logic.on_event(
                window,
                UiEvent::Placed(BarGeometry {
                    rect,
                    edge: crate::config::model::Edge::Bottom,
                    monitor: crate::platform::MonitorInfo::new(
                        r"\\.\DISPLAY1".into(),
                        "Test".into(),
                        Rect::new(0, 0, 1200, 1080),
                        Rect::new(0, 0, 1200, 1052),
                        1.0,
                        true,
                    ),
                    scale: 1.0,
                }),
            );
        };
        let stored = |host: &FakeHost| {
            host.settings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .bar
                .floating
        };
        let (mut logic, window, host, _) = bar(settings.clone());
        place(&mut logic, &window, Rect::new(200, 1000, 1000, 1028));
        // The resize cursor within 6 px of either end.
        for (x, cursor) in [
            (3.0, PopupCursor::ResizeWidth),
            (400.0, PopupCursor::Arrow),
            (797.0, PopupCursor::ResizeWidth),
        ] {
            hover(&mut logic, &window, PointF::new(x, 14.0));
            assert_eq!(window.cursor.get(), cursor, "at {x}");
        }
        logic.on_event(&window, UiEvent::PointerLeave);
        assert_eq!(window.cursor.get(), PopupCursor::Arrow);

        // The right end, pressed 3 px inside it, follows the pointer at once.
        press(&mut logic, &window, PointF::new(797.0, 14.0), false);
        assert!(window.captured.get());
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(897.0, 14.0)));
        assert_eq!(
            window.position.get(),
            Some(Rect::new(200, 1000, 1100, 1028))
        );
        assert_eq!(window.cursor.get(), PopupCursor::ResizeWidth);
        // Not past the work area.
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(2000.0, 14.0)));
        assert_eq!(
            window.position.get(),
            Some(Rect::new(200, 1000, 1200, 1028))
        );
        release(&mut logic, &window, PointF::new(2000.0, 14.0));
        assert!(!window.captured.get());
        assert_eq!(
            stored(&host),
            Some(FloatingRect {
                x: 200,
                y: 1000,
                width: 1000
            })
        );

        // The left end: the window moves with it, so each move is read from where it is.
        press(&mut logic, &window, PointF::new(2.0, 14.0), false);
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(-98.0, 14.0)));
        assert_eq!(
            window.position.get(),
            Some(Rect::new(100, 1000, 1200, 1028))
        );
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(-48.0, 14.0)));
        assert_eq!(window.position.get(), Some(Rect::new(50, 1000, 1200, 1028)));
        // Not narrower than 200 px.
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(1100.0, 14.0)));
        assert_eq!(
            window.position.get(),
            Some(Rect::new(1000, 1000, 1200, 1028))
        );
        // A lost capture stores it too.
        logic.on_event(&window, UiEvent::CaptureLost);
        assert_eq!(
            stored(&host),
            Some(FloatingRect {
                x: 1000,
                y: 1000,
                width: 200
            })
        );

        // A press without a move stores nothing.
        let (mut logic, window, host, _) = bar(settings.clone());
        place(&mut logic, &window, Rect::new(200, 1000, 1000, 1028));
        press(&mut logic, &window, PointF::new(797.0, 14.0), false);
        release(&mut logic, &window, PointF::new(797.0, 14.0));
        assert_eq!(stored(&host), None);

        // Locked, or docked: the ends do nothing.
        settings.locked = true;
        let (mut logic, window, _, _) = bar(settings);
        place(&mut logic, &window, Rect::new(200, 1000, 1000, 1028));
        hover(&mut logic, &window, PointF::new(3.0, 14.0));
        assert_eq!(window.cursor.get(), PopupCursor::Arrow);
        press(&mut logic, &window, PointF::new(3.0, 14.0), false);
        logic.on_event(&window, UiEvent::PointerMove(PointF::new(-97.0, 14.0)));
        assert_eq!(
            window.position.get(),
            Some(Rect::new(200, 1000, 1000, 1028))
        );
        let (mut logic, window, _, _) = bar(Settings::default());
        hover(&mut logic, &window, PointF::new(3.0, 14.0));
        assert_eq!(window.cursor.get(), PopupCursor::Arrow);
        press(&mut logic, &window, PointF::new(3.0, 14.0), false);
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

    /// Records power actions instead of taking them: the tests run on Windows too.
    #[derive(Default)]
    struct NoPower(Mutex<Vec<String>>);

    impl Platform for NoPower {
        fn capabilities(&self) -> crate::platform::Capabilities {
            crate::platform::Capabilities {
                power_modes: vec!["shutDown".into()],
                ..Default::default()
            }
        }
        fn monitors(&self) -> Vec<crate::platform::MonitorInfo> {
            Vec::new()
        }
        fn appearance(&self) -> OsAppearance {
            OsAppearance::default()
        }
        fn uptime(&self) -> Duration {
            Duration::ZERO
        }
        fn power(&self, action: crate::platform::PowerAction, _force: bool) {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(format!("{action:?}"));
        }
    }

    /// Puts a Power sensor, whose click asks first, in the logo's place.
    fn with_power_sensor(logic: &mut BarLogic, window: &FakeWindow) -> Arc<NoPower> {
        let power = Arc::new(NoPower::default());
        logic.platform = Arc::clone(&power) as Arc<dyn Platform>;
        let mut config = Config {
            settings: logic.settings.clone(),
            layout: logic.layout.clone(),
            state: logic.state.clone(),
            ..Config::default()
        };
        let logo = &mut config.layout.center[0];
        logo.kind = "power".into();
        logo.props = serde_json::json!({ "mode": "shutDown" })
            .as_object()
            .cloned()
            .unwrap_or_default();
        logic.apply_config(window, config);
        logic.sync_sensors(window);
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        power
    }

    #[test]
    fn a_click_that_asks_first_waits_for_the_answer() {
        let (mut logic, window, _host, _) = bar(Settings::default());
        let power = with_power_sensor(&mut logic, &window);
        let actions = || power.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // Declined: nothing happens.
        window.decline.set(true);
        press(&mut logic, &window, LOGO, false);
        release(&mut logic, &window, LOGO);
        assert_eq!(
            *window.confirmed.borrow(),
            ["Do you really want to shut down this computer?"]
        );
        assert!(actions().is_empty());
        // Confirmed: the action is taken.
        window.decline.set(false);
        press(&mut logic, &window, LOGO, false);
        release(&mut logic, &window, LOGO);
        assert_eq!(actions(), ["ShutDown"]);
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
    fn tools_import_and_export_through_the_host() {
        let (mut logic, window, host, _) = bar(Settings::default());
        let empty = PointF::new(300.0, 14.0);
        right_click(&mut logic, &window, empty, MenuCommand::ImportLayout);
        right_click(&mut logic, &window, LOGO, MenuCommand::ExportLayout);
        right_click(&mut logic, &window, empty, MenuCommand::ImportLegacy);
        assert_eq!(host.calls(), ["import Json", "export", "import Legacy"]);
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

    fn key(logic: &mut BarLogic, window: &FakeWindow, key: Key, shift: bool) {
        let modifiers = Modifiers {
            shift,
            ..Modifiers::default()
        };
        logic.on_event(window, UiEvent::Key { key, modifiers });
    }

    /// The focus ring the last frame drew: its rectangle, or `None`.
    fn ring(logic: &mut BarLogic) -> Option<String> {
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        canvas
            .dump()
            .lines()
            .find(|l| l.starts_with("stroke rounded") && l.ends_with("w2"))
            .map(str::to_owned)
    }

    /// The x where the focus ring starts.
    fn ring_left(logic: &mut BarLogic) -> Option<f32> {
        let ring = ring(logic)?;
        let rect = ring.split_whitespace().nth(2)?;
        rect.split(',').next()?.parse().ok()
    }

    /// The x where the ring around sensor `id` starts.
    fn ring_left_of(logic: &BarLogic, id: &str) -> f32 {
        let (x, _) = logic.sensors.place_of(id).expect("placed");
        (x - (logic.theme.gap / 2.0).min(3.0)).max(1.5)
    }

    #[test]
    fn the_hotkey_focuses_the_bar_and_tab_moves_the_ring_in_visual_order() {
        let (mut logic, window, _, config) = bar(Settings::default());
        assert_eq!(ring(&mut logic), None, "no ring before the hotkey");
        // Keys reach only a focused bar.
        key(&mut logic, &window, Key::Tab, false);
        assert_eq!(ring(&mut logic), None);
        logic.focus_bar(&window);
        assert_eq!(window.focus_taken.get(), 1);
        assert_eq!(ring_left(&mut logic), Some(1.5), "around the bar itself");
        // Left, centre, right, each zone in its order; then around again.
        let order: Vec<String> = [
            &config.layout.left,
            &config.layout.center,
            &config.layout.right,
        ]
        .into_iter()
        .flatten()
        .map(|s| s.id.clone())
        .collect();
        assert_eq!(logic.sensors.focus_order(), order);
        for id in order.iter().chain(&order[..1]) {
            key(&mut logic, &window, Key::Tab, false);
            assert_eq!(logic.focused_sensor().as_ref(), Some(id));
            let expected = ring_left_of(&logic, id);
            assert_eq!(ring_left(&mut logic), Some(expected));
        }
        // Shift+Tab before the first wraps to the last; Home and End; the arrows.
        key(&mut logic, &window, Key::Tab, true);
        assert_eq!(logic.focused_sensor().as_ref(), order.last());
        key(&mut logic, &window, Key::Home, false);
        assert_eq!(logic.focused_sensor().as_ref(), order.first());
        key(&mut logic, &window, Key::Right, false);
        assert_eq!(logic.focused_sensor().as_ref(), order.get(1));
        key(&mut logic, &window, Key::Left, false);
        key(&mut logic, &window, Key::End, false);
        assert_eq!(logic.focused_sensor().as_ref(), order.last());
        // The hotkey again keeps the ring where it is.
        logic.focus_bar(&window);
        assert_eq!(logic.focused_sensor().as_ref(), order.last());
        // A window that cannot take the focus: no keyboard on the bar.
        let (mut logic, window, _, _) = bar(Settings::default());
        window.no_focus.set(true);
        logic.focus_bar(&window);
        assert_eq!(ring(&mut logic), None);
    }

    #[test]
    fn screen_readers_get_the_sensors_in_focus_order() {
        let (mut logic, window, _, config) = bar(Settings::default());
        let bar = logic.accessible();
        let ids: Vec<&str> = bar.items.iter().map(|i| i.id.as_str()).collect();
        let order = logic.sensors.focus_order();
        assert_eq!(ids, order.iter().map(String::as_str).collect::<Vec<_>>());
        let logo = &bar.items[3];
        assert_eq!(logo.id, config.layout.center[0].id);
        assert_eq!(logo.name, "Image");
        assert!(logo.value.starts_with("Infodock "), "{}", logo.value);
        let (x, width) = logic.sensors.place_of(&logo.id).expect("placed");
        assert_eq!(logo.rect, RectF::new(x, 0.0, x + width, 28.0));
        assert_eq!(bar.items[0].name, "CPU Graph");
        assert_eq!((bar.focused, bar.focus), (false, None));
        // The keyboard: the bar, then the first sensor.
        logic.focus_bar(&window);
        assert_eq!(
            (logic.accessible().focused, logic.accessible().focus),
            (true, None)
        );
        key(&mut logic, &window, Key::Tab, false);
        assert_eq!(
            logic.accessible().focused_item().map(|i| i.name.as_str()),
            Some("CPU Graph")
        );
        // A Spacer is not listed.
        let mut with_spacer = config.clone();
        with_spacer.layout.left.insert(
            0,
            SensorInstance {
                id: "s_spacer".into(),
                kind: "spacer".into(),
                props: crate::sensors::Props::new(),
            },
        );
        logic.apply_config(&window, with_spacer);
        logic.sync_sensors(&window);
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        assert_eq!(logic.accessible().items.len(), 6);
        assert_eq!(logic.accessible().focus, Some(0), "still on the CPU Graph");
    }

    #[test]
    fn screen_readers_click_expand_collapse_and_focus_sensors() {
        let (mut logic, window, _, mut config) = bar(Settings {
            locked: true,
            clicks_when_locked: ClicksWhenLocked::AltOnly,
            ..Settings::default()
        });
        // Invoke clicks, whatever the Lock rules say for the pointer.
        with_power_sensor(&mut logic, &window);
        window.decline.set(true);
        let power = logic.layout.center[0].id.clone();
        logic.on_event(
            &window,
            UiEvent::Accessible(AccessibleAction::Invoke(power.clone())),
        );
        assert_eq!(window.confirmed.borrow().len(), 1);
        assert_eq!(
            logic.accessible().items[3].popup,
            None,
            "no popup to expand"
        );
        // An unknown sensor: nothing.
        logic.on_event(
            &window,
            UiEvent::Accessible(AccessibleAction::Invoke("gone".into())),
        );
        assert_eq!(window.confirmed.borrow().len(), 1);

        // A Folder Menu in the logo's place expands and collapses.
        let dir = crate::sensors::common::folder_browser::tests::sample_folder();
        let logo = &mut config.layout.center[0];
        logo.kind = "folder-menu".into();
        logo.props = serde_json::json!({ "folder": dir.path().to_string_lossy() })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let id = logo.id.clone();
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        assert_eq!(logic.accessible().items[3].popup, Some(false));
        let act =
            |logic: &mut BarLogic, action| logic.on_event(&window, UiEvent::Accessible(action));
        act(&mut logic, AccessibleAction::Expand(id.clone()));
        assert!(window.popup.borrow().is_some());
        assert_eq!(logic.accessible().items[3].popup, Some(true));
        // The popup window is named by the folder it shows.
        let folder = dir
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        assert_eq!(Some(window.popup_name.borrow().clone()), folder);
        // Expanding an open one keeps it open; collapsing closes it.
        let draws = window.popup_draws.get();
        act(&mut logic, AccessibleAction::Expand(id.clone()));
        assert_eq!(window.popup_draws.get(), draws, "not opened again");
        act(&mut logic, AccessibleAction::Collapse(id.clone()));
        assert!(window.popup.borrow().is_none());
        assert_eq!(logic.accessible().items[3].popup, Some(false));
        act(&mut logic, AccessibleAction::Collapse(id.clone()));

        // Focus: the bar takes the keyboard with the ring on that sensor.
        act(&mut logic, AccessibleAction::Focus(id.clone()));
        assert_eq!(window.focus_taken.get(), 1);
        assert_eq!(logic.focused_sensor(), Some(id.clone()));
        assert_eq!(
            logic.accessible().focused_item().map(|i| i.id.clone()),
            Some(id)
        );
    }

    #[test]
    fn a_spacer_is_passed_over() {
        let (mut logic, window, _, mut config) = bar(Settings::default());
        let spacer = SensorInstance {
            id: "s_spacer".into(),
            kind: "spacer".into(),
            props: crate::sensors::Props::new(),
        };
        config.layout.center.push(spacer);
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        assert!(!logic.sensors.focus_order().contains(&"s_spacer".to_owned()));
        assert_eq!(logic.sensors.focus_order().len(), 6);
    }

    #[test]
    fn the_focused_sensors_hint_shows_after_the_delay() {
        let (mut logic, window, _, _) = bar(Settings::default());
        logic.focus_bar(&window);
        key(&mut logic, &window, Key::Home, false);
        key(&mut logic, &window, Key::End, false);
        assert_eq!(window.hint_timer.get(), Some(Duration::from_millis(600)));
        hint_timer(&mut logic, &window);
        assert!(window.hint.borrow().is_some(), "the Time's hint");
        // Esc: the hint goes with the ring.
        key(&mut logic, &window, Key::Escape, false);
        assert!(window.hint.borrow().is_none());
    }

    #[test]
    fn enter_and_space_click_the_focused_sensor() {
        let (mut logic, window, _, config) = bar(Settings {
            locked: true,
            clicks_when_locked: ClicksWhenLocked::AltOnly,
            ..Settings::default()
        });
        with_power_sensor(&mut logic, &window);
        window.decline.set(true);
        logic.focus_bar(&window);
        // Nothing focused: nothing to click.
        key(&mut logic, &window, Key::Enter, false);
        assert!(window.confirmed.borrow().is_empty());
        logic.keys.as_mut().expect("keys").sensor = Some(config.layout.center[0].id.clone());
        // A deliberate key: even a locked Alt-only bar clicks.
        key(&mut logic, &window, Key::Enter, false);
        key(&mut logic, &window, Key::Space, false);
        assert_eq!(window.confirmed.borrow().len(), 2);
    }

    #[test]
    fn shift_f10_and_the_menu_key_open_the_menu_beside_the_focus() {
        let (mut logic, window, host, config) = bar(Settings::default());
        logic.focus_bar(&window);
        // No sensor focused: the bar menu beside the bar, new sensors at the end of the left zone.
        window.choose.set(Some(MenuCommand::NewSensor.id()));
        key(&mut logic, &window, Key::F10, true);
        assert_eq!(host.calls(), ["open new?zone=left&index=3"]);
        assert_eq!(
            window.menus_near.borrow().last(),
            Some(&Some(Rect::new(0, 1052, 1200, 1080)))
        );
        // F10 alone opens nothing.
        key(&mut logic, &window, Key::F10, false);
        assert_eq!(window.menus.borrow().len(), 1);
        // The logo focused: its menu, beside it.
        let logo = config.layout.center[0].id.clone();
        logic.keys.as_mut().expect("keys").sensor = Some(logo.clone());
        window.choose.set(Some(MenuCommand::Properties.id()));
        key(&mut logic, &window, Key::Menu, false);
        assert_eq!(host.calls().last(), Some(&format!("open sensor/{logo}")));
        let (x, width) = logic.sensors.place_of(&logo).expect("placed");
        let near = Rect::new(x as i32, 1052, (x + width) as i32, 1080);
        assert_eq!(window.menus_near.borrow().last(), Some(&Some(near)));
        // The pointer's menus stay at the pointer.
        right_click(&mut logic, &window, LOGO, MenuCommand::Settings);
        assert_eq!(window.menus_near.borrow().last(), Some(&None));
    }

    #[test]
    fn esc_or_a_click_elsewhere_ends_the_keyboard_on_the_bar() {
        let (mut logic, window, _, _) = bar(Settings::default());
        logic.focus_bar(&window);
        key(&mut logic, &window, Key::Tab, false);
        key(&mut logic, &window, Key::Escape, false);
        assert_eq!(
            window.focus_restored.get(),
            1,
            "back to the window that had it"
        );
        assert_eq!(ring(&mut logic), None);
        key(&mut logic, &window, Key::Tab, false);
        assert_eq!(ring(&mut logic), None, "keys no longer reach it");

        // Another app took the focus: the ring goes, nothing is given back.
        logic.focus_bar(&window);
        logic.on_event(&window, UiEvent::FocusLost { within_app: false });
        assert_eq!(ring(&mut logic), None);
        logic.on_event(&window, UiEvent::FocusGained);
        assert_eq!(ring(&mut logic), None, "a menu's activation brings no ring");
        assert_eq!(window.focus_restored.get(), 1);

        // A confirmation of the bar's took it: the ring waits and comes back with the focus.
        logic.focus_bar(&window);
        key(&mut logic, &window, Key::Tab, false);
        logic.on_event(&window, UiEvent::FocusLost { within_app: true });
        assert_eq!(ring(&mut logic), None);
        logic.on_event(&window, UiEvent::FocusGained);
        assert!(ring(&mut logic).is_some());
        assert!(logic.focused_sensor().is_some(), "on the same sensor");
        // Lost to its own window, then a press on the bar: what was left ends.
        logic.on_event(&window, UiEvent::FocusLost { within_app: true });
        press(&mut logic, &window, PointF::new(300.0, 14.0), false);
        logic.on_event(&window, UiEvent::FocusGained);
        assert_eq!(ring(&mut logic), None);

        // Hiding the bar gives the focus back.
        logic.focus_bar(&window);
        logic.hidden(&window);
        assert_eq!(window.focus_restored.get(), 2);
        assert_eq!(ring(&mut logic), None);
    }

    #[test]
    fn a_removed_sensors_focus_goes_to_the_one_in_its_place() {
        let (mut logic, window, _, mut config) = bar(Settings::default());
        logic.focus_bar(&window);
        let logo = config.layout.center[0].id.clone();
        let date = config.layout.right[0].id.clone();
        logic.keys.as_mut().expect("keys").sensor = Some(logo);
        config.layout.center.clear();
        logic.apply_config(&window, config.clone());
        logic.sync_sensors(&window);
        assert_eq!(logic.focused_sensor(), Some(date));
        // The bar emptied: the ring goes around the bar itself.
        config.layout = Layout::default();
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        assert_eq!(logic.focused_sensor(), None);
        assert_eq!(ring_left(&mut logic), Some(1.5));
        key(&mut logic, &window, Key::Tab, false);
        assert_eq!(logic.focused_sensor(), None, "nothing to move to");
    }

    #[test]
    fn a_popup_opened_from_the_keyboard_gives_the_focus_back_to_the_bar() {
        let (mut logic, window, _, config) = bar(Settings::default());
        let logo = config.layout.center[0].id.clone();
        logic.focus_bar(&window);
        logic.keys.as_mut().expect("keys").sensor = Some(logo.clone());
        let (probe, _, _) = new_probe();
        logic.open_popup(&window, &logo, Box::new(probe));
        // The popup takes the focus, as on a click.
        logic.on_event(&window, UiEvent::FocusLost { within_app: true });
        let esc = UiEvent::Popup(PopupInput::Key {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });
        logic.on_event(&window, esc);
        assert!(window.popup.borrow().is_none());
        assert_eq!(window.focus_taken.get(), 2, "the bar took it back");
        assert_eq!(logic.focused_sensor(), Some(logo.clone()));
        assert!(ring(&mut logic).is_some());

        // Closed by itself (an item chosen): the same.
        let (probe, _, reply) = new_probe();
        logic.open_popup(&window, &logo, Box::new(probe));
        logic.on_event(&window, UiEvent::FocusLost { within_app: true });
        reply.set(PopupReply::Close);
        logic.on_event(
            &window,
            UiEvent::Popup(PopupInput::PointerMove(PointF::new(20.0, 20.0))),
        );
        assert_eq!(window.focus_taken.get(), 3);

        // Closed by a click in another app: the keyboard on the bar ends with it.
        let (probe, _, _) = new_probe();
        logic.open_popup(&window, &logo, Box::new(probe));
        logic.on_event(&window, UiEvent::FocusLost { within_app: true });
        logic.on_event(&window, UiEvent::PopupDeactivated);
        assert_eq!(window.focus_taken.get(), 3);
        assert_eq!(ring(&mut logic), None);
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
        // Screen readers read its text as the window's title.
        assert_eq!(
            *window.hint_name.borrow(),
            format!("Infodock {}", crate::VERSION)
        );
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
                pushed: false,
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

    fn placer_drawn(logic: &mut BarLogic) -> bool {
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        canvas.dump().lines().any(|l| l.ends_with("#0078D4"))
    }

    #[test]
    fn dropping_files_and_links_onto_the_bar_adds_sensors() {
        use crate::platform::{DragItem, DropEffect, DropEffects};
        let (mut logic, window, host, _) = bar(Settings::default());
        let dir = tempfile::tempdir().expect("a temporary folder");
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, b"x").expect("a file");
        let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let items = vec![
            DragItem::Path(path(&file)),
            DragItem::Path(path(&dir.path().join("gone"))),
            DragItem::Path(path(dir.path())),
            DragItem::Url {
                url: "https://example.com/".into(),
                title: Some("Example".into()),
            },
            DragItem::Path("E:\\".into()),
        ];
        let at = PointF::new(1000.0, 14.0);
        let enter = DragEvent::Enter {
            items,
            at,
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        assert_eq!(logic.on_drag(&window, enter), DropEffect::Link);
        assert!(placer_drawn(&mut logic), "where the sensors would go");
        let over = DragEvent::Over {
            at: PointF::new(850.0, 14.0),
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        assert_eq!(logic.on_drag(&window, over), DropEffect::Link);
        let drop = DragEvent::Drop {
            at: PointF::new(850.0, 14.0),
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        assert_eq!(logic.on_drag(&window, drop), DropEffect::Link);
        assert!(!placer_drawn(&mut logic));
        // The right zone's first place: before its first sensor, one sensor per item that
        // exists, in order.
        assert_eq!(
            host.calls(),
            [
                format!(
                    "add shortcut right 0 {}",
                    serde_json::json!({ "target": path(&file) })
                ),
                format!(
                    "add folder-menu right 1 {}",
                    serde_json::json!({ "folder": path(dir.path()) })
                ),
                format!(
                    "add shortcut right 2 {}",
                    serde_json::json!({ "url": "https://example.com/", "title": "Example" })
                ),
                format!(
                    "add disk right 3 {}",
                    serde_json::json!({ "drive": "E:\\" })
                ),
            ]
        );
    }

    #[test]
    fn a_folder_menu_takes_dropped_files_with_a_hint() {
        use crate::platform::{DragItem, DropEffect, DropEffects, FileOp};
        /// Records file operations instead of running them, and knows no volumes, so items
        /// are copied on every OS: this test runs on Windows too.
        #[derive(Default)]
        struct NoFiles(Mutex<Vec<String>>);
        impl Platform for NoFiles {
            fn capabilities(&self) -> crate::platform::Capabilities {
                Default::default()
            }
            fn monitors(&self) -> Vec<crate::platform::MonitorInfo> {
                Vec::new()
            }
            fn appearance(&self) -> OsAppearance {
                OsAppearance::default()
            }
            fn uptime(&self) -> Duration {
                Duration::ZERO
            }
            fn volume_of(&self, _path: &str) -> Option<String> {
                None
            }
            fn file_operation(
                &self,
                op: FileOp,
                paths: Vec<String>,
                _folder: String,
                done: Box<dyn FnOnce() + Send>,
            ) {
                self.0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(format!("{op:?} {}", paths.len()));
                done();
            }
        }

        let (mut logic, window, host, mut config) = bar(Settings::default());
        let files = Arc::new(NoFiles::default());
        logic.platform = Arc::clone(&files) as Arc<dyn Platform>;
        let target = tempfile::tempdir().expect("a temporary folder");
        let source = tempfile::tempdir().expect("a temporary folder");
        let file = source.path().join("report.pdf");
        std::fs::write(&file, b"x").expect("a file");
        // A Folder Menu in the logo's place.
        let logo = &mut config.layout.center[0];
        logo.kind = "folder-menu".into();
        logo.props =
            serde_json::json!({ "folder": target.path().to_string_lossy(), "title": "Inbox" })
                .as_object()
                .cloned()
                .unwrap_or_default();
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        let enter = DragEvent::Enter {
            items: vec![DragItem::Path(file.to_string_lossy().into_owned())],
            at: LOGO,
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        // No volumes known: another volume, so a copy.
        assert_eq!(logic.on_drag(&window, enter), DropEffect::Copy);
        let (_, hint) = window.hint.borrow().clone().expect("the drop hint");
        assert!(hint.contains("\"Copy 'report.pdf' to 'Inbox'\""), "{hint}");
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        let dump = canvas.dump();
        assert!(dump.contains("#000000/78%"), "darkened: {dump}");
        assert!(!dump.lines().any(|l| l.ends_with("#0078D4")), "no placer");
        // Ctrl+Shift: a shortcut; the hint follows the keys.
        let over = DragEvent::Over {
            at: LOGO,
            modifiers: Modifiers {
                ctrl: true,
                shift: true,
                alt: false,
            },
            allowed: DropEffects::ALL,
        };
        assert_eq!(logic.on_drag(&window, over), DropEffect::Link);
        let (_, hint) = window.hint.borrow().clone().expect("the drop hint");
        assert!(
            hint.contains("Create shortcut to 'report.pdf' in 'Inbox'"),
            "{hint}"
        );
        // Off the sensor: the placer again, no hint.
        let away = DragEvent::Over {
            at: PointF::new(850.0, 14.0),
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        assert_eq!(logic.on_drag(&window, away), DropEffect::Link);
        assert!(window.hint.borrow().is_none());
        assert!(placer_drawn(&mut logic));
        let drop = DragEvent::Drop {
            at: LOGO,
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        assert_eq!(logic.on_drag(&window, drop), DropEffect::Copy);
        assert!(window.hint.borrow().is_none());
        assert!(host.calls().is_empty(), "no new sensors");
        assert_eq!(
            *files.0.lock().unwrap_or_else(|e| e.into_inner()),
            ["Copy 1"]
        );
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        assert!(!canvas.dump().contains("#000000/78%"));
    }

    #[test]
    fn what_the_bar_cannot_take_is_refused() {
        use crate::platform::{DragItem, DropEffect, DropEffects};
        let (mut logic, window, host, _) = bar(Settings::default());
        let mail = DragEvent::Enter {
            items: vec![DragItem::Url {
                url: "mailto:me@example.com".into(),
                title: None,
            }],
            at: LOGO,
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        assert_eq!(logic.on_drag(&window, mail), DropEffect::None);
        assert!(!placer_drawn(&mut logic));
        let drop = DragEvent::Drop {
            at: LOGO,
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        assert_eq!(logic.on_drag(&window, drop), DropEffect::None);
        // A source that allows only moving: nothing to create links or copies with.
        let move_only = DropEffects {
            moves: true,
            ..DropEffects::default()
        };
        let enter = DragEvent::Enter {
            items: vec![DragItem::Path("E:\\".into())],
            at: LOGO,
            modifiers: Modifiers::default(),
            allowed: move_only,
        };
        assert_eq!(logic.on_drag(&window, enter), DropEffect::None);
        assert_eq!(logic.on_drag(&window, DragEvent::Leave), DropEffect::None);
        assert!(host.calls().is_empty());
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

    /// What reaches a [`Probe`].
    type Log = Rc<RefCell<Vec<String>>>;

    /// A 100 × 50 popup that logs what reaches it and answers with `reply`.
    struct Probe {
        log: Log,
        reply: Rc<Cell<PopupReply>>,
        wake: Option<Instant>,
        props: Option<crate::sensors::Props>,
    }

    impl Popup for Probe {
        fn kind(&self) -> &'static str {
            "probe"
        }
        fn layout(&mut self, cx: &PopupCtx) -> SizeF {
            self.log
                .borrow_mut()
                .push(format!("layout in {}×{}", cx.room.width, cx.room.height));
            SizeF::new(100.0, 50.0)
        }
        fn draw(&self, canvas: &mut dyn Canvas, origin: PointF, _theme: &Theme) {
            canvas.fill_rect(
                RectF::new(origin.x, origin.y, origin.x + 100.0, origin.y + 50.0),
                crate::platform::Color::rgba(1, 2, 3, 1.0),
            );
        }
        fn input(&mut self, input: &PopupInput, _cx: &PopupCtx) -> PopupReply {
            self.log.borrow_mut().push(format!("{input:?}"));
            self.reply.get()
        }
        fn wake_at(&self) -> Option<Instant> {
            self.wake
        }
        fn on_wake(&mut self, _cx: &PopupCtx) -> PopupReply {
            self.log.borrow_mut().push("wake".into());
            self.wake = None;
            PopupReply::Close
        }
        fn poll(&mut self, _cx: &PopupCtx) -> PopupReply {
            self.log.borrow_mut().push("poll".into());
            PopupReply::Nothing
        }
        fn take_props(&mut self) -> Option<crate::sensors::Props> {
            self.props.take()
        }
    }

    fn new_probe() -> (Probe, Log, Rc<Cell<PopupReply>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let reply = Rc::new(Cell::new(PopupReply::Nothing));
        let probe = Probe {
            log: Rc::clone(&log),
            reply: Rc::clone(&reply),
            wake: None,
            props: None,
        };
        (probe, log, reply)
    }

    #[test]
    fn a_popup_opens_next_to_its_sensor_and_takes_input() {
        let (mut logic, window, _, config) = bar(Settings::default());
        let logo = config.layout.center[0].id.clone();
        let (probe, log, reply) = new_probe();
        logic.open_popup(&window, &logo, Box::new(probe));
        // Centred above the logo (its centre at 596): 100 × 50 content, 4 px padding, the 7 px
        // pointer and a 6 px shadow around it.
        let (rect, dump) = window.popup.borrow().clone().expect("shown");
        assert_eq!(rect, Rect::new(536, 981, 656, 1058));
        assert!(dump.contains("fill rect 10,10 100×50 #010203"), "{dump}");
        assert_eq!(
            log.borrow()[0],
            "layout in 1178×1024",
            "the room above the bar"
        );
        // The logo shows its active state, in the text colour of the OS theme.
        let active = |logic: &mut BarLogic| {
            let mut canvas = RecordingCanvas::new(1.0);
            logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
            canvas
                .dump()
                .lines()
                .any(|l| l.starts_with("fill rounded") && l.ends_with("/16%"))
        };
        assert!(active(&mut logic));

        // Input arrives from the content's corner; a redraw keeps the place.
        reply.set(PopupReply::Redraw);
        let draws = window.popup_draws.get();
        logic.on_event(
            &window,
            UiEvent::Popup(PopupInput::PointerMove(PointF::new(20.0, 30.0))),
        );
        assert_eq!(
            log.borrow().last().map(String::as_str),
            Some("PointerMove(PointF { x: 10.0, y: 20.0 })")
        );
        assert_eq!(window.popup_draws.get(), draws + 1);
        // Work arriving: polled.
        logic.on_event(&window, UiEvent::Wake);
        assert_eq!(log.borrow().last().map(String::as_str), Some("poll"));

        // Esc closes it, without reaching the popup.
        let input = |key| {
            UiEvent::Popup(PopupInput::Key {
                key,
                modifiers: Modifiers::default(),
            })
        };
        logic.on_event(&window, input(Key::Escape));
        assert!(window.popup.borrow().is_none());
        assert!(!log.borrow().iter().any(|l| l.contains("Escape")));
        assert!(!active(&mut logic), "no longer active");
    }

    #[test]
    fn a_popup_closes_on_blur_on_request_and_on_a_press() {
        let (mut logic, window, host, config) = bar(Settings::default());
        let logo = config.layout.center[0].id.clone();
        let (probe, _, _) = new_probe();
        logic.open_popup(&window, &logo, Box::new(probe));
        logic.on_event(&window, UiEvent::PopupDeactivated);
        assert!(logic.popups.owner().is_none(), "closed on blur");

        // Asked to close, e.g. after opening a file; what it changed is saved.
        let (mut probe, _, reply) = new_probe();
        probe.props = Some(
            serde_json::json!({ "popupWidth": 300 })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
        logic.open_popup(&window, &logo, Box::new(probe));
        reply.set(PopupReply::Close);
        logic.on_event(&window, UiEvent::Popup(PopupInput::Char('a')));
        assert!(window.popup.borrow().is_none());
        assert!(
            host.calls()
                .contains(&format!(r#"update {logo} {{"popupWidth":300}}"#)),
            "{:?}",
            host.calls()
        );

        // A wake-up it asked for.
        let (mut probe, log, _) = new_probe();
        probe.wake = Some(Instant::now());
        logic.open_popup(&window, &logo, Box::new(probe));
        assert_eq!(window.popup_timer.get(), Some(Duration::ZERO));
        logic.on_event(&window, UiEvent::Timer(TimerId::Popup));
        assert_eq!(log.borrow().last().map(String::as_str), Some("wake"));
        assert!(window.popup.borrow().is_none(), "it asked to close");
        assert_eq!(window.popup_timer.get(), None);
    }

    #[test]
    fn the_press_that_closes_a_popup_does_not_reopen_it() {
        let (mut logic, window, _host, mut config) = bar(Settings::default());
        // A Power sensor (which asks first) in the logo's place shows whether a click arrives.
        let logo = &mut config.layout.center[0];
        logo.kind = "power".into();
        let id = logo.id.clone();
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        window.decline.set(true);
        let (probe, _, _) = new_probe();
        logic.open_popup(&window, &id, Box::new(probe));
        // The press closes the popup; the click it ends is ignored.
        press(&mut logic, &window, LOGO, false);
        assert!(window.popup.borrow().is_none());
        release(&mut logic, &window, LOGO);
        assert!(window.confirmed.borrow().is_empty(), "no click");
        // No hints show while a popup is open.
        let (probe, _, _) = new_probe();
        logic.open_popup(&window, &id, Box::new(probe));
        hover(&mut logic, &window, PointF::new(LOGO.x + 1.0, 14.0));
        assert_eq!(window.hint_timer.get(), None);
    }

    #[test]
    fn after_a_popup_no_reopening_hint_and_hiding_the_bar_closes_it() {
        let dir = crate::sensors::common::folder_browser::tests::sample_folder();
        let (mut logic, window, _host, mut config) = bar(Settings::default());
        let logo = &mut config.layout.center[0];
        logo.kind = "folder-menu".into();
        logo.props = serde_json::json!({ "folder": dir.path().to_string_lossy() })
            .as_object()
            .cloned()
            .unwrap_or_default();
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        press(&mut logic, &window, LOGO, false);
        release(&mut logic, &window, LOGO);
        assert!(window.popup.borrow().is_some(), "opened");
        // A slow click on the sensor closes its popup and does not open it again.
        press(&mut logic, &window, LOGO, false);
        assert!(window.popup.borrow().is_none(), "closed by the press");
        std::thread::sleep(super::popup::GUARD + Duration::from_millis(50));
        release(&mut logic, &window, LOGO);
        assert!(window.popup.borrow().is_none(), "not opened again");
        // Opened again; moving over the bar meanwhile and closing it with Esc: the pointer is
        // still on the sensor, so its hint stays away.
        logic.on_event(&window, UiEvent::PointerMove(LOGO));
        press(&mut logic, &window, LOGO, false);
        release(&mut logic, &window, LOGO);
        assert!(window.popup.borrow().is_some());
        window.hint_timer.set(None);
        logic.on_event(
            &window,
            UiEvent::PointerMove(PointF::new(LOGO.x + 1.0, 14.0)),
        );
        logic.on_event(
            &window,
            UiEvent::Popup(PopupInput::Key {
                key: Key::Escape,
                modifiers: Modifiers::default(),
            }),
        );
        logic.on_event(&window, UiEvent::PointerMove(LOGO));
        assert_eq!(window.hint_timer.get(), None, "no hint on its way");
        // Hiding the bar (tray, hotkey) closes an open popup.
        std::thread::sleep(super::popup::GUARD);
        press(&mut logic, &window, LOGO, false);
        release(&mut logic, &window, LOGO);
        assert!(window.popup.borrow().is_some());
        logic.hidden(&window);
        assert!(window.popup.borrow().is_none());
    }

    #[test]
    fn a_folder_menu_opens_its_list_closes_on_esc_and_resizes() {
        let dir = crate::sensors::common::folder_browser::tests::sample_folder();
        let (mut logic, window, host, mut config) = bar(Settings::default());
        // A Folder Menu in the logo's place.
        let logo = &mut config.layout.center[0];
        logo.kind = "folder-menu".into();
        logo.props = serde_json::json!({ "folder": dir.path().to_string_lossy() })
            .as_object()
            .cloned()
            .unwrap_or_default();
        logic.apply_config(&window, config);
        logic.sync_sensors(&window);
        logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
        press(&mut logic, &window, LOGO, false);
        release(&mut logic, &window, LOGO);
        let (rect, dump) = window.popup.borrow().clone().expect("the list is shown");
        assert!(dump.contains("\"report 10.pdf\""), "{dump}");
        // 240 px wide, its left edge (inside the shadow) at the sensor's.
        assert_eq!(rect.width(), 240 + 2 * 4 + 2 * 6);
        let key = |key| {
            UiEvent::Popup(PopupInput::Key {
                key,
                modifiers: Modifiers::default(),
            })
        };
        logic.on_event(&window, key(Key::Escape));
        assert!(window.popup.borrow().is_none());

        // Dragging its free right edge: the pointer is captured and the size saved. (Clicks
        // on the sensor wait out the guard after Esc closed it.)
        std::thread::sleep(super::popup::GUARD);
        press(&mut logic, &window, LOGO, false);
        release(&mut logic, &window, LOGO);
        assert!(window.popup.borrow().is_some());
        // The content starts 10 px into the window (shadow and padding).
        let at = |x: f32| PointF::new(10.0 + x, 110.0);
        let input = |logic: &mut BarLogic, input| logic.on_event(&window, UiEvent::Popup(input));
        input(&mut logic, PopupInput::PointerMove(at(239.0)));
        assert_eq!(
            window.popup_cursor.get(),
            crate::platform::PopupCursor::ResizeWidth
        );
        input(
            &mut logic,
            PopupInput::PointerDown {
                button: PointerButton::Left,
                at: at(239.0),
                modifiers: Modifiers::default(),
            },
        );
        assert!(window.popup_captured.get());
        input(&mut logic, PopupInput::PointerMove(at(299.0)));
        input(
            &mut logic,
            PopupInput::PointerUp {
                button: PointerButton::Left,
                at: at(299.0),
                modifiers: Modifiers::default(),
            },
        );
        assert!(!window.popup_captured.get());
        let (rect, _) = window.popup.borrow().clone().expect("still open");
        assert_eq!(rect.width(), 300 + 2 * 4 + 2 * 6, "60 px wider");
        assert!(
            host.calls()
                .iter()
                .any(|c| c.ends_with(r#"{"popupHeight":320,"popupWidth":300}"#)),
            "{:?}",
            host.calls()
        );
    }
}
