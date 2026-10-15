//! The bar's context menus (SPEC §6.4), built from the current state each time they open, with
//! a sensor's own items in its menu.

use crate::config::model::{NewSensorMenu, Settings};
use crate::i18n::t;
use crate::platform::{Capabilities, MenuEntry};
use crate::sensors::registry::Registry;
use crate::sensors::{Category, MenuId, MenuItem, SensorType};

/// A chosen menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    /// Opens the sensor picker in Settings.
    NewSensor,
    /// Adds a sensor of the type at this index of [`addable_types`] (submenu mode).
    Add(usize),
    Properties,
    Remove,
    LayoutManager,
    ImportLayout,
    ExportLayout,
    ImportLegacy,
    Help,
    Settings,
    ToggleLock,
    TogglePopups,
    Quit,
    /// One of the sensor's own items (SPEC §6.4).
    Sensor(MenuId),
}

/// Ids of [`MenuCommand::Sensor`] start here.
const SENSOR_BASE: u32 = 500;
/// Ids of [`MenuCommand::Add`] start here.
const ADD_BASE: u32 = 1000;

impl MenuCommand {
    const FIXED: [Self; 12] = [
        Self::NewSensor,
        Self::Properties,
        Self::Remove,
        Self::LayoutManager,
        Self::ImportLayout,
        Self::ExportLayout,
        Self::ImportLegacy,
        Self::Help,
        Self::Settings,
        Self::ToggleLock,
        Self::TogglePopups,
        Self::Quit,
    ];

    pub fn id(self) -> u32 {
        match self {
            Self::Add(index) => ADD_BASE + index as u32,
            // Sensor items beyond the range are not offered (see `sensor_menu`).
            Self::Sensor(item) => SENSOR_BASE + u32::from(item),
            fixed => Self::FIXED.iter().position(|c| *c == fixed).unwrap_or(0) as u32 + 1,
        }
    }

    pub fn from_id(id: u32) -> Option<Self> {
        if id >= ADD_BASE {
            return Some(Self::Add((id - ADD_BASE) as usize));
        }
        if id >= SENSOR_BASE {
            return MenuId::try_from(id - SENSOR_BASE).ok().map(Self::Sensor);
        }
        Self::FIXED.get((id as usize).checked_sub(1)?).copied()
    }
}

/// The types offered for adding, in picker order (SPEC §6.6): available ones only.
pub fn addable_types(registry: &Registry, caps: &Capabilities) -> Vec<&'static dyn SensorType> {
    let mut types: Vec<_> = registry
        .types()
        .iter()
        .copied()
        .filter(|ty| Registry::available(*ty, caps))
        .collect();
    types.sort_by_cached_key(|ty| (ty.category(), t(ty.name()).to_lowercase()));
    types
}

fn item(command: MenuCommand, key: &str) -> MenuEntry {
    MenuEntry::item(command.id(), t(key))
}

/// "New sensor…", or with `newSensorMenu = submenu` a submenu of the types by category.
fn new_sensor(settings: &Settings, registry: &Registry, caps: &Capabilities) -> MenuEntry {
    if settings.new_sensor_menu == NewSensorMenu::Dialog {
        return item(MenuCommand::NewSensor, "menu.newSensor");
    }
    let mut categories: Vec<(Category, Vec<MenuEntry>)> = Vec::new();
    for (index, ty) in addable_types(registry, caps).into_iter().enumerate() {
        let entry = MenuEntry::item(MenuCommand::Add(index).id(), t(ty.name()));
        match categories.last_mut() {
            Some((category, entries)) if *category == ty.category() => entries.push(entry),
            _ => categories.push((ty.category(), vec![entry])),
        }
    }
    let entries = categories
        .into_iter()
        .map(|(category, entries)| MenuEntry::submenu(t(category.name()), entries))
        .collect();
    MenuEntry::submenu(t("menu.newSensor.submenu"), entries)
}

/// Layout tools; the items of later milestones are greyed out until they exist.
fn tools() -> MenuEntry {
    MenuEntry::submenu(
        t("menu.tools"),
        vec![
            item(MenuCommand::LayoutManager, "menu.layoutManager"),
            item(MenuCommand::ImportLayout, "menu.importLayout").enabled(false),
            item(MenuCommand::ExportLayout, "menu.exportLayout").enabled(false),
            item(MenuCommand::ImportLegacy, "menu.importLegacy").enabled(false),
            MenuEntry::Separator,
            item(MenuCommand::Help, "menu.help").enabled(false),
        ],
    )
}

fn toggles(settings: &Settings) -> [MenuEntry; 2] {
    [
        MenuEntry::check(
            MenuCommand::ToggleLock.id(),
            t("tray.lock"),
            settings.locked,
        ),
        MenuEntry::check(
            MenuCommand::TogglePopups.id(),
            t("tray.popups"),
            settings.popups_enabled,
        ),
    ]
}

/// The menu of the bar's background (SPEC §6.4).
pub fn bar_menu(settings: &Settings, registry: &Registry, caps: &Capabilities) -> Vec<MenuEntry> {
    let mut entries = vec![new_sensor(settings, registry, caps), MenuEntry::Separator];
    entries.extend([tools(), item(MenuCommand::Settings, "tray.settings")]);
    entries.push(MenuEntry::Separator);
    entries.extend(toggles(settings));
    entries.extend([MenuEntry::Separator, item(MenuCommand::Quit, "tray.quit")]);
    entries
}

/// The menu of a sensor (SPEC §6.4). `editable`: its type has properties to show; `own`: the
/// sensor's own items, between *Remove sensor* and *Tools*.
pub fn sensor_menu(
    settings: &Settings,
    registry: &Registry,
    caps: &Capabilities,
    editable: bool,
    own: &[MenuItem],
) -> Vec<MenuEntry> {
    let mut entries = vec![
        new_sensor(settings, registry, caps),
        item(MenuCommand::Properties, "menu.properties").enabled(editable),
        item(MenuCommand::Remove, "menu.remove"),
        MenuEntry::Separator,
    ];
    let own: Vec<MenuEntry> = own
        .iter()
        .filter(|i| u32::from(i.id) < ADD_BASE - SENSOR_BASE)
        .map(|i| MenuEntry::item(MenuCommand::Sensor(i.id).id(), &i.label).enabled(i.enabled))
        .collect();
    if !own.is_empty() {
        entries.extend(own);
        entries.push(MenuEntry::Separator);
    }
    entries.extend([tools(), item(MenuCommand::Settings, "tray.settings")]);
    entries.extend(toggles(settings));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::registry::registry;

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                MenuEntry::Item {
                    label,
                    checked,
                    enabled,
                    ..
                } => format!(
                    "{}{label}{}",
                    if *checked { "✓ " } else { "" },
                    if *enabled { "" } else { " (off)" }
                ),
                MenuEntry::Separator => "—".into(),
                MenuEntry::Submenu { label, entries } => {
                    format!("{label} ▸ [{}]", labels(entries).join(", "))
                }
            })
            .collect()
    }

    #[test]
    fn bar_menu_items() {
        let settings = Settings {
            popups_enabled: true,
            ..Settings::default()
        };
        assert_eq!(
            labels(&bar_menu(&settings, registry(), &Capabilities::default())),
            [
                "New sensor…",
                "—",
                "Tools ▸ [Layout manager…, Import layout… (off), Export layout… (off), \
                 Import legacy XML layout… (off), —, Help (off)]",
                "Settings…",
                "—",
                "Lock",
                "✓ Popups",
                "—",
                "Quit Infodock"
            ]
        );
    }

    #[test]
    fn sensor_menu_items() {
        let settings = Settings {
            locked: true,
            popups_enabled: false,
            ..Settings::default()
        };
        let entries = sensor_menu(&settings, registry(), &Capabilities::default(), false, &[]);
        let labels = labels(&entries);
        assert_eq!(
            labels[..5],
            [
                "New sensor…",
                "Properties… (off)",
                "Remove sensor",
                "—",
                "Tools ▸ [Layout manager…, Import layout… (off), Export layout… (off), \
                 Import legacy XML layout… (off), —, Help (off)]"
            ]
        );
        assert_eq!(labels[labels.len() - 2..], ["✓ Lock", "Popups"]);
    }

    #[test]
    fn a_sensors_own_items_sit_between_remove_and_tools() {
        let own = [
            MenuItem::new(0, "Eject"),
            MenuItem::new(1, "Empty").enabled(false),
            // Beyond the id range: not offered.
            MenuItem::new(600, "Lost"),
        ];
        let entries = sensor_menu(
            &Settings::default(),
            registry(),
            &Capabilities::default(),
            true,
            &own,
        );
        let labels = labels(&entries);
        assert_eq!(labels[2..6], ["Remove sensor", "—", "Eject", "Empty (off)"]);
        assert_eq!(labels[6], "—");
        assert!(labels[7].starts_with("Tools"));
        let MenuEntry::Item { id, .. } = &entries[5] else {
            panic!("an item");
        };
        assert_eq!(MenuCommand::from_id(*id), Some(MenuCommand::Sensor(1)));
    }

    #[test]
    fn submenu_lists_the_types_by_category() {
        let settings = Settings {
            new_sensor_menu: NewSensorMenu::Submenu,
            ..Settings::default()
        };
        let entries = bar_menu(&settings, registry(), &Capabilities::default());
        assert_eq!(
            labels(&entries)[0],
            "New sensor ▸ [General ▸ [Image, Spacer, Text], Info ▸ [Date, Time, Uptime], System ▸ [CPU Graph, CPU Meter, Disk Meter, Memory Graph, Memory Meter, Swap Graph, Swap Meter], Network ▸ [Network Graph], Launchers ▸ [Shortcut]]"
        );
        let MenuEntry::Submenu { entries, .. } = &entries[0] else {
            panic!("a submenu");
        };
        let MenuEntry::Submenu { entries, .. } = &entries[0] else {
            panic!("a category");
        };
        let MenuEntry::Item { id, .. } = &entries[2] else {
            panic!("an item");
        };
        let Some(MenuCommand::Add(index)) = MenuCommand::from_id(*id) else {
            panic!("an add command");
        };
        let types = addable_types(registry(), &Capabilities::default());
        assert_eq!(types[index].id(), "text");
    }

    #[test]
    fn ids_round_trip() {
        for command in MenuCommand::FIXED {
            assert_eq!(MenuCommand::from_id(command.id()), Some(command));
        }
        assert_eq!(
            MenuCommand::from_id(MenuCommand::Add(4).id()),
            Some(MenuCommand::Add(4))
        );
        assert_eq!(
            MenuCommand::from_id(MenuCommand::Sensor(499).id()),
            Some(MenuCommand::Sensor(499))
        );
        assert_eq!(MenuCommand::from_id(0), None);
        assert_eq!(MenuCommand::from_id(13), None);
    }
}
