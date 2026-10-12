//! The tray menu (SPEC §4.1, §6.5): Show/Hide bar · Lock · Popups · Settings… · Quit. Built from
//! the current state each time it opens, so labels and check marks are always current.

use crate::config::model::Settings;
use crate::i18n::t;
use crate::platform::MenuEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleBar = 1,
    ToggleLock,
    TogglePopups,
    OpenSettings,
    Quit,
}

impl TrayCommand {
    const ALL: [Self; 5] = [
        Self::ToggleBar,
        Self::ToggleLock,
        Self::TogglePopups,
        Self::OpenSettings,
        Self::Quit,
    ];

    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|c| *c as u32 == id)
    }
}

pub fn menu(settings: &Settings, bar_visible: bool) -> Vec<MenuEntry> {
    use TrayCommand as C;
    let toggle = if bar_visible {
        "tray.hideBar"
    } else {
        "tray.showBar"
    };
    vec![
        MenuEntry::item(C::ToggleBar as u32, t(toggle)),
        MenuEntry::Separator,
        MenuEntry::check(C::ToggleLock as u32, t("tray.lock"), settings.locked),
        MenuEntry::check(
            C::TogglePopups as u32,
            t("tray.popups"),
            settings.popups_enabled,
        ),
        MenuEntry::Separator,
        MenuEntry::item(C::OpenSettings as u32, t("tray.settings")),
        MenuEntry::Separator,
        MenuEntry::item(C::Quit as u32, t("tray.quit")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(entries: &[MenuEntry], id: TrayCommand) -> bool {
        entries
            .iter()
            .any(|e| matches!(e, MenuEntry::Item { id: i, checked: true, .. } if *i == id as u32))
    }

    fn label(entries: &[MenuEntry], id: TrayCommand) -> String {
        entries
            .iter()
            .find_map(|e| match e {
                MenuEntry::Item { id: i, label, .. } if *i == id as u32 => Some(label.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn check_marks_follow_the_settings() {
        let mut s = Settings::default();
        let menu_now = menu(&s, true);
        assert!(!checked(&menu_now, TrayCommand::ToggleLock));
        assert!(checked(&menu_now, TrayCommand::TogglePopups));

        s.locked = true;
        s.popups_enabled = false;
        let menu_now = menu(&s, true);
        assert!(checked(&menu_now, TrayCommand::ToggleLock));
        assert!(!checked(&menu_now, TrayCommand::TogglePopups));
    }

    #[test]
    fn first_item_names_the_visibility_change() {
        let s = Settings::default();
        assert_eq!(label(&menu(&s, true), TrayCommand::ToggleBar), "Hide bar");
        assert_eq!(label(&menu(&s, false), TrayCommand::ToggleBar), "Show bar");
    }

    #[test]
    fn ids_round_trip() {
        for c in TrayCommand::ALL {
            assert_eq!(TrayCommand::from_id(c as u32), Some(c));
        }
        assert_eq!(TrayCommand::from_id(0), None);
        assert_eq!(TrayCommand::from_id(99), None);
    }
}
