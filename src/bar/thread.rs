//! The bar logic on the bar thread (SPEC §4.1, §4.4): applies settings and the OS appearance,
//! and draws the bar. Sensors, layout and interaction arrive in later M1 slices.

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use super::theme::Theme;
use super::tray::{self, TrayCommand};
use super::{BarHost, Command};
use crate::config::model::{Edge, Material, Settings};
use crate::platform::{
    BarHandler, BarWindow, Canvas, OsAppearance, Platform, RectF, SizeF, UiEvent,
};

pub(super) struct BarLogic {
    settings: Settings,
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
        settings: Settings,
        platform: Arc<dyn Platform>,
        commands: Receiver<Command>,
        host: Arc<dyn BarHost>,
    ) -> Self {
        let appearance = platform.appearance();
        Self {
            theme: Theme::resolve(&settings, appearance),
            settings,
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
            window.invalidate();
        }
    }
}

impl BarHandler for BarLogic {
    fn on_event(&mut self, window: &dyn BarWindow, event: UiEvent) {
        match event {
            UiEvent::Started => {
                self.refresh_theme(window);
                self.refresh_tray(window);
            }
            UiEvent::Wake => {
                while let Ok(command) = self.commands.try_recv() {
                    match command {
                        Command::Settings(settings) => {
                            let edge_changed = settings.bar.edge != self.settings.bar.edge;
                            self.settings = *settings;
                            if edge_changed {
                                window.invalidate();
                            }
                        }
                    }
                }
                self.refresh_theme(window);
                self.refresh_tray(window);
            }
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
        canvas.fill_rect(RectF::from_size(size), self.theme.bar_background);
        if let Some(hairline) = self.theme.hairline {
            // On the edge facing the screen centre (SPEC §12.2).
            let top = match self.settings.bar.edge {
                Edge::Bottom => 0.0,
                Edge::Top => size.height - 1.0,
            };
            canvas.fill_rect(RectF::new(0.0, top, size.width, top + 1.0), hairline);
        }
    }
}
