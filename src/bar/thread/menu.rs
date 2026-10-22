//! The bar and sensor menus (SPEC §6.4) and what their commands do.

use super::*;

impl BarLogic {
    /// The bar or sensor menu at `at` (SPEC §6.4), and what its command does.
    pub(super) fn context_menu(&mut self, window: &dyn BarWindow, at: PointF) {
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
    pub(super) fn menu(
        &mut self,
        window: &dyn BarWindow,
        hit: Option<Hit>,
        place: (Zone, usize),
        near: Option<Rect>,
    ) {
        self.dismiss_hint(window);
        self.close_popup(window);
        self.cancel_drag(window);
        let caps = self.platform.capabilities();
        let env = sensor_env!(self, false);
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
            MenuCommand::Help => self.host.open_settings("help"),
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
                    let env = sensor_env!(self, false);
                    if self.sensors.on_menu(&entry.id, item, &env) {
                        window.invalidate();
                        self.refresh_changed_hint(window);
                    }
                }
            }
        }
    }
}
