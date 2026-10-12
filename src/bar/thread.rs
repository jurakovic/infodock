//! The bar logic on the bar thread (SPEC §4.1, §4.4): applies the config and the OS appearance,
//! keeps the sensors, their wake-up timer and layout, and draws the bar. Interaction arrives in
//! later M1 slices.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use super::paint::paint_bar;
use super::sensors::{BarSensors, Env};
use super::theme::Theme;
use super::tray::{self, TrayCommand};
use super::{BarHost, Command};
use crate::config::Config;
use crate::config::model::{Layout, Material, SensorInstance, Settings};
use crate::platform::{
    BarHandler, BarWindow, Canvas, OsAppearance, Platform, SizeF, TextEngine, UiEvent,
};
use crate::sensors::registry::registry;
use crate::sensors::{LayoutCtx, State};

pub(super) struct BarLogic {
    settings: Settings,
    layout: Layout,
    state: BTreeMap<String, State>,
    /// Entries shown instead of the saved ones while Settings previews them, by id.
    previews: HashMap<String, SensorInstance>,
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
            TrayCommand::OpenSettings => self.host.open_settings(),
            TrayCommand::Quit => self.host.quit(),
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
