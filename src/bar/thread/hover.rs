//! The hover hint (SPEC §7.1, §7.3): when it shows and hides, and where.

use super::*;

impl BarLogic {
    /// Closes the hint now and cancels its delay (SPEC §7.1).
    pub(super) fn dismiss_hint(&mut self, window: &dyn BarWindow) {
        let step = self.hover.dismiss();
        self.apply_hint(window, step);
    }

    /// Carries out what the hover logic decided.
    pub(super) fn apply_hint(&mut self, window: &dyn BarWindow, step: Step) {
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
    pub(super) fn anchor(&self, id: &str) -> Option<(Anchor, f32)> {
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
    pub(super) fn show_hint(&self, window: &dyn BarWindow, id: &str) -> bool {
        let env = sensor_env!(self, false);
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
        let Some(hint) = Prepared::new(content, self.text.as_ref(), &anchor, &self.theme) else {
            return false;
        };
        let theme = &self.theme;
        window.show_hint(hint.rect(scale), scale, &name, &mut |canvas| {
            hint.paint(canvas, theme);
        });
        true
    }

    /// Redraws a shown hint, whose sensor or theme may have changed (SPEC §7.1: live).
    pub(super) fn refresh_hint(&mut self, window: &dyn BarWindow) {
        if !self.settings.popups_enabled {
            self.dismiss_hint(window);
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
    pub(super) fn refresh_changed_hint(&mut self, window: &dyn BarWindow) {
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
}
