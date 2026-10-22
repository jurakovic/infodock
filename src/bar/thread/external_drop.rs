//! Drags from other apps over the bar (SPEC §6.3): the placer, a sensor that takes the
//! drop itself with its floating hint, and the drop.

use super::*;

impl BarLogic {
    /// Another app's drag enters the bar: the hint closes, and the items the bar can take are
    /// kept for the drop (SPEC §6.3).
    pub(super) fn drag_enter(
        &mut self,
        window: &dyn BarWindow,
        items: &[crate::platform::DragItem],
    ) {
        self.cancel_drag(window);
        self.dismiss_hint(window);
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
    pub(super) fn drag_over(
        &mut self,
        window: &dyn BarWindow,
        at: PointF,
        drag: DragInfo,
    ) -> DropEffect {
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
            let env = sensor_env!(self, drag.modifiers.alt);
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
    pub(super) fn show_drop_hint(&self, window: &dyn BarWindow, id: &str, text: &str) {
        let Some((anchor, scale)) = self.anchor(id) else {
            return;
        };
        let content = crate::sensors::HintContent::Text(text.to_owned());
        let Some(hint) = Prepared::new(content, self.text.as_ref(), &anchor, &self.theme) else {
            return;
        };
        let theme = &self.theme;
        window.show_hint(
            hint.rect(scale),
            scale,
            &hint::spoken(text),
            &mut |canvas| {
                hint.paint(canvas, theme);
            },
        );
    }

    /// Another app's drag left, or dropped: the placer, the darkened sensor and the floating
    /// hint go.
    pub(super) fn drag_end(&mut self, window: &dyn BarWindow) -> Option<ExternalDrag> {
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
    pub(super) fn drag_drop(
        &mut self,
        window: &dyn BarWindow,
        at: PointF,
        drag: DragInfo,
    ) -> DropEffect {
        let effect = self.drag_over(window, at, drag);
        let Some(external) = self.drag_end(window) else {
            return DropEffect::None;
        };
        if effect == DropEffect::None {
            return DropEffect::None;
        }
        if let Some((id, _, Some(_))) = &external.onto {
            let env = sensor_env!(self, drag.modifiers.alt);
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
