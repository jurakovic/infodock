//! The bar logic on the bar thread (SPEC §4.1, §4.4): applies the config and the OS appearance,
//! keeps the sensors, their wake-up timer and layout, and draws the bar. It handles the pointer
//! (clicks, the reorder drag, moving and resizing a floating bar; §5.1, §6.1–6.2), the context
//! menus and the tray (§6.4–6.5), the hover hint and the interactive popups (§7), drops from
//! other apps (§6.3), the keyboard and UI Automation (§14.4), and the bar's motion: showing,
//! hiding, the rise animation and sliding out to change edges (§5.5).
//!
//! This module holds [`BarLogic`]'s state, its construction, the event dispatch, the config and
//! the tray; each other job is a child module with its own `impl BarLogic` block (`pointer`,
//! `hover`, `menu`, `popups`, `sensors`, `external_drop`, `keyboard`, `frame`). Children see
//! the fields; their methods are `pub(super)`, so the dispatch and the siblings call them.
//! The tests are split the same way under `tests/`.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::drop::{self, Dropped};
use super::focus::{self, Move};
use super::hint::{self, Hover, Prepared, Step};
use super::icons::IconLoader;
use super::interact::{self, DropTarget, Press};
use super::menus::{self, MenuCommand};
use super::paint::paint_bar;
use super::placement::{Anchor, physical_rect};
use super::popup::{self, Open, PopupHost};
use super::resize::{self, Resize};
use super::sensors::{BarSensors, Env, Hit};
use super::shadow::ShadowCache;
use super::tray::{self, TrayCommand};
use super::{BarHost, Command};
use crate::config::Config;
use crate::config::exchange::ImportFormat;
use crate::config::model::Zone;
use crate::config::model::{
    BarMode, Edge, FloatingRect, Layout, Material, SensorInstance, Settings,
};
use crate::i18n::{t, t_args};
use crate::look::anim::{self, Easing, Fade};
use crate::look::skins::{self, SkinCache};
use crate::look::theme::Theme;
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

/// The sensors' [`Env`] from the bar logic's fields, with Alt held or not. A macro rather than
/// a method, so it borrows only the fields it names and `self.sensors` can be borrowed mutably
/// beside it.
macro_rules! sensor_env {
    ($logic:expr, $alt:expr) => {
        Env {
            settings: &$logic.settings,
            platform: $logic.platform.as_ref(),
            alt: $alt,
            animate: $logic.animate,
            changes: &$logic.changes,
        }
    };
}

mod external_drop;
mod frame;
mod hover;
mod keyboard;
mod menu;
mod pointer;
mod popups;
mod sensors;
#[cfg(test)]
mod tests;

/// How often Esc is checked during a drag.
const ESC_POLL: Duration = Duration::from_millis(50);
/// The hover highlight fades in and out over this long (SPEC §5.5).
const HIGHLIGHT: Duration = Duration::from_millis(300);
/// The bar rises into place, or slides out, over this long (SPEC §5.5).
const RISE: Duration = Duration::from_millis(300);
/// How often the bar checks whether the capability probe has answered (SPEC §14.1), and how
/// long it checks before waiting for it.
const PROBE_POLL: Duration = Duration::from_millis(25);
const PROBE_PATIENCE: Duration = Duration::from_secs(2);

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
    /// Since when the sensors wait for the capability probe; `None` once it has answered.
    probe_since: Option<Instant>,
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
        let numbers = Numbers::from(&platform.locale(config.settings.date_locale));
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
            probe_since: None,
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
        self.dismiss_hint(window);
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
        if anim::slides_out(old, new) {
            if self.animate && self.geometry.is_some() && self.host.bar_visible() {
                if !self.sliding_out {
                    self.dismiss_hint(window);
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

    /// The bar thread was woken (SPEC §4.4): takes every queued command, so a burst of config
    /// changes and samples costs one theme refresh and one sync; icons and popup work that
    /// arrived meanwhile are taken too.
    fn drain(&mut self, window: &dyn BarWindow) {
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

    fn on_timer(&mut self, window: &dyn BarWindow, id: TimerId) {
        match id {
            TimerId::Wake => {
                let env = sensor_env!(self, false);
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
            TimerId::Probe => {
                // A probe that never started is waited for after a while.
                let patient = self
                    .probe_since
                    .is_some_and(|since| since.elapsed() < PROBE_PATIENCE);
                if self.platform.probed_capabilities().is_some() || !patient {
                    self.platform.capabilities();
                    self.probe_since = None;
                    self.sync_sensors(window);
                } else {
                    window.set_timer(TimerId::Probe, Some(PROBE_POLL));
                }
            }
            TimerId::Popup => {
                if let Some(reply) = self.with_popup(|open, cx| open.popup.on_wake(cx)) {
                    self.popup_reply(window, reply);
                }
            }
        }
    }

    /// The bar was hidden: nothing may stay on screen pointing at a bar that is gone.
    fn hidden(&mut self, window: &dyn BarWindow) {
        self.leave_keyboard(window, true);
        self.cancel_drag(window);
        self.dismiss_hint(window);
        self.close_popup(window);
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
        match event {
            UiEvent::Started => {
                self.refresh_theme(window);
                self.refresh_tray(window);
                self.sync_sensors(window);
            }
            UiEvent::Wake => self.drain(window),
            UiEvent::Timer(id) => self.on_timer(window, id),
            UiEvent::PointerDown {
                button,
                at,
                modifiers,
            } => self.pointer_down(window, button, at, modifiers),
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
            UiEvent::PointerUp { .. } => {}
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
                self.dismiss_hint(window);
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
        self.paint_frame(canvas, size);
    }

    fn band(&self) -> Option<Band> {
        self.band.clone()
    }

    fn accessible(&self) -> AccessibleBar {
        self.accessible_bar()
    }
}
