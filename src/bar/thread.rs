//! The bar logic on the bar thread (SPEC §4.1, §4.4): applies the config and the OS appearance,
//! keeps the sensors, their wake-up timer and layout, and draws the bar. Interaction arrives in
//! later M1 slices.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use super::interact::{self, Press};
use super::menus::{self, MenuCommand};
use super::paint::paint_bar;
use super::sensors::{BarSensors, Env};
use super::theme::Theme;
use super::tray::{self, TrayCommand};
use super::{BarHost, Command};
use crate::config::Config;
use crate::config::model::Zone;
use crate::config::model::{Layout, Material, SensorInstance, Settings};
use crate::i18n::{t, t_args};
use crate::platform::{
    BarHandler, BarWindow, Canvas, Modifiers, OsAppearance, Platform, PointF, PointerButton, SizeF,
    TextEngine, UiEvent,
};
use crate::sensors::registry::registry;
use crate::sensors::{ClickResult, LayoutCtx, State};

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
    sensors: BarSensors,
    text: Rc<dyn TextEngine>,
    appearance: OsAppearance,
    theme: Theme,
    /// The material last applied to the window; `None` until the first application.
    material: Option<Material>,
    /// Whether the tray icon is shown; `None` until first applied.
    tray_shown: Option<bool>,
    platform: Arc<dyn Platform>,
    commands: Receiver<Command>,
    host: Arc<dyn BarHost>,
}

impl BarLogic {
    pub(super) fn new(
        config: Config,
        platform: Arc<dyn Platform>,
        commands: Receiver<Command>,
        host: Arc<dyn BarHost>,
    ) -> Self {
        let appearance = platform.appearance();
        Self {
            theme: Theme::resolve(&config.settings, appearance),
            settings: config.settings,
            layout: config.layout,
            state: config.state,
            previews: HashMap::new(),
            press: None,
            sensors: BarSensors::default(),
            text: crate::platform::text_engine(),
            appearance,
            material: None,
            tray_shown: None,
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
        let Some(hit) = self.sensors.hit(at.x) else {
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
        // Past the threshold a reorder drag starts (M1 S8); until then the press just stops
        // being a click.
        if let Some(press) = &mut self.press
            && press.moved_to(at)
            && self.sensors.set_pressed(None)
        {
            window.invalidate();
        }
    }

    /// Releasing over the pressed sensor without having moved is a click.
    fn pointer_up(&mut self, window: &dyn BarWindow, at: PointF, modifiers: Modifiers) {
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
        };
        if self.sensors.click(&press.id, &env) == ClickResult::Handled {
            window.invalidate();
        }
    }

    /// The bar or sensor menu at `at` (SPEC §6.4), and what its command does.
    fn context_menu(&mut self, window: &dyn BarWindow, at: PointF) {
        if self.press.take().is_some() {
            window.capture_pointer(false);
            self.sensors.set_pressed(None);
            window.invalidate();
        }
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
        let theme = Theme::resolve(&self.settings, self.appearance);
        if self.material != Some(theme.material) {
            window.set_material(theme.material);
            self.material = Some(theme.material);
        }
        if theme != self.theme {
            self.theme = theme;
            self.sensors.invalidate_all();
            window.invalidate();
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
            window.invalidate();
        }
        self.arm_timer(window);
    }

    /// Sets the window's timer to the earliest wake-up any sensor asked for (SPEC §4.4).
    fn arm_timer(&self, window: &dyn BarWindow) {
        let now = Instant::now();
        window.set_timer(
            self.sensors
                .next_wake(now)
                .map(|at| at.saturating_duration_since(now)),
        );
    }

    fn on_timer(&mut self, window: &dyn BarWindow) {
        let env = Env {
            settings: &self.settings,
            theme: &self.theme,
            platform: self.platform.as_ref(),
            alt: false,
        };
        if self.sensors.wake(Instant::now(), &env) {
            window.invalidate();
        }
        self.arm_timer(window);
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
                while let Ok(command) = self.commands.try_recv() {
                    match command {
                        Command::Config(config) => {
                            let config = *config;
                            if config.settings.bar.edge != self.settings.bar.edge
                                || config.settings.sensor_spacing != self.settings.sensor_spacing
                            {
                                window.invalidate();
                            }
                            layout_changed |=
                                config.layout != self.layout || config.state != self.state;
                            self.settings = config.settings;
                            self.layout = config.layout;
                            self.state = config.state;
                        }
                        Command::Preview(sensor) => {
                            self.previews.insert(sensor.id.clone(), *sensor);
                            layout_changed = true;
                        }
                        Command::EndPreview(id) => {
                            layout_changed |= self.previews.remove(&id).is_some();
                        }
                    }
                }
                self.refresh_theme(window);
                self.refresh_tray(window);
                if layout_changed {
                    self.sync_sensors(window);
                }
            }
            UiEvent::Timer => self.on_timer(window),
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
            // Hover arrives with the hint window (M1 S7); middle clicks are reserved (§6.1).
            UiEvent::PointerDown { .. } | UiEvent::PointerUp { .. } | UiEvent::PointerLeave => {}
            UiEvent::AppearanceChanged => {
                let appearance = self.platform.appearance();
                if appearance != self.appearance {
                    tracing::info!("OS appearance changed: {appearance:?}");
                    self.appearance = appearance;
                    self.refresh_theme(window);
                }
            }
            UiEvent::Placed(_) => window.invalidate(),
            // Hint and popup windows arrive later in M1; they hide here (SPEC §5.3).
            UiEvent::Fullscreen(_) => {}
            UiEvent::CloseRequested => self.host.quit(),
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
        };
        self.sensors.update_layout(&cx, size.width);
        paint_bar(canvas, size, &self.theme, self.settings.bar.edge);
        self.sensors.draw(canvas, &self.theme, size.height);
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
    }

    impl BarWindow for FakeWindow {
        fn invalidate(&self) {}
        fn set_material(&self, _material: Material) {}
        fn set_tray_icon(&self, _tooltip: Option<&str>) {}
        fn show_menu(&self, entries: &[MenuEntry]) -> Option<u32> {
            self.menus.borrow_mut().push(entries.to_vec());
            self.choose.take()
        }
        fn set_timer(&self, _delay: Option<std::time::Duration>) {}
        fn capture_pointer(&self, capture: bool) {
            self.captured.set(capture);
        }
        fn confirm(&self, message: &str, _action: &str) -> bool {
            self.confirmed.borrow_mut().push(message.to_owned());
            true
        }
    }

    #[derive(Default)]
    struct FakeHost {
        calls: Mutex<Vec<String>>,
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
        fn change_settings(&self, _change: Box<dyn FnOnce(&mut Settings) + Send>) {
            self.log("change settings".into());
        }
        fn add_sensor(&self, kind: &str, zone: Zone, index: usize) -> Option<String> {
            self.log(format!("add {kind} {} {index}", zone_id(zone)));
            Some("s_new".into())
        }
        fn remove_sensor(&self, id: &str) {
            self.log(format!("remove {id}"));
        }
    }

    const WIDTH: f32 = 1200.0;

    /// The bar logic with the first-run layout, laid out on a 1200 px bar.
    fn bar(settings: Settings) -> (BarLogic, FakeWindow, Arc<FakeHost>, Config) {
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
        let window = FakeWindow::default();
        logic.on_event(&window, UiEvent::Started);
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
    fn a_press_dims_the_sensor_until_release() {
        let (mut logic, window, _, _) = bar(Settings::default());
        press(&mut logic, &window, LOGO, false);
        assert!(window.captured.get());
        assert!(dimmed(&mut logic));
        release(&mut logic, &window, LOGO);
        assert!(!window.captured.get());
        assert!(!dimmed(&mut logic));

        // Moving away first: no longer dimmed, and releasing is no click.
        press(&mut logic, &window, LOGO, false);
        logic.on_event(
            &window,
            UiEvent::PointerMove(PointF::new(LOGO.x + 5.0, 14.0)),
        );
        assert!(!dimmed(&mut logic));
        release(&mut logic, &window, LOGO);
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
}
