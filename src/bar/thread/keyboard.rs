//! The keyboard on the bar and UI Automation (SPEC §14.4): the focus ring, keys, the menu
//! from the keyboard, and what screen readers see and ask.

use super::*;

impl BarLogic {
    /// The hotkey focuses the bar (SPEC §14.4): the window takes the keyboard focus and the
    /// ring goes around the bar itself, or stays on its sensor when the bar had the focus.
    pub(super) fn focus_bar(&mut self, window: &dyn BarWindow) {
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
    pub(super) fn focused_sensor(&self) -> Option<String> {
        self.keys.as_ref()?.sensor.clone()
    }

    /// A key on the focused bar: Tab and the arrows move the ring, Enter and Space click its
    /// sensor, Shift+F10 and the Menu key open its menu, Esc gives the focus back (§14.4).
    pub(super) fn key(&mut self, window: &dyn BarWindow, key: Key, modifiers: Modifiers) {
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
                    self.dismiss_hint(window);
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
    pub(super) fn move_focus(&mut self, window: &dyn BarWindow, to: Move) {
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
            let step = self
                .hover
                .over(id.as_deref(), popups.show_delay(), popups.hide_delay());
            self.apply_hint(window, step);
        }
    }

    /// The focused sensor's menu, or the bar menu with no sensor focused, beside it (§6.4).
    pub(super) fn keyboard_menu(&mut self, window: &dyn BarWindow) {
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
    pub(super) fn leave_keyboard(&mut self, window: &dyn BarWindow, restore: bool) {
        let Some(keys) = self.keys.take() else {
            return;
        };
        window.invalidate();
        self.dismiss_hint(window);
        if restore && keys.focused {
            window.restore_focus();
        }
    }

    /// The bar window lost the focus: to a popup or a confirmation of its own the ring waits
    /// for it to come back; anywhere else (a click elsewhere) the keyboard on the bar ends.
    pub(super) fn focus_lost(&mut self, window: &dyn BarWindow, within_app: bool) {
        match self.keys.as_mut() {
            Some(keys) if within_app => {
                keys.focused = false;
                window.invalidate();
            }
            Some(_) => self.leave_keyboard(window, false),
            None => {}
        }
    }

    pub(super) fn focus_gained(&mut self, window: &dyn BarWindow) {
        if let Some(keys) = self.keys.as_mut().filter(|k| !k.focused) {
            keys.focused = true;
            window.invalidate();
        }
    }

    /// What a screen reader asks of a sensor (SPEC §14.4): a click, its popup opened or
    /// closed, the focus ring moved to it. Deliberate, like a key, so the Lock rules for the
    /// pointer do not apply.
    pub(super) fn accessible_action(&mut self, window: &dyn BarWindow, action: AccessibleAction) {
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
                self.dismiss_hint(window);
                self.click(window, &id, false);
            }
            AccessibleAction::Expand(id) if !open => {
                self.dismiss_hint(window);
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
    pub(super) fn refocus(&mut self, window: &dyn BarWindow) {
        if self.keys.as_ref().is_some_and(|k| !k.focused) {
            if window.take_focus() {
                self.focus_gained(window);
            } else {
                self.leave_keyboard(window, false);
            }
        }
    }

    /// The bar as screen readers see it (SPEC §14.4): its sensors in focus order.
    pub(super) fn accessible_bar(&self) -> AccessibleBar {
        let env = sensor_env!(self, false);
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
