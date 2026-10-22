//! The sensors' life on the bar (SPEC §4.4): creating them from the layout, their
//! subscriptions, samples, wake-ups and clock changes, and the state they want stored.

use super::*;

impl BarLogic {
    /// Creates sensors for new or changed layout entries, showing previews where there are.
    /// Until the capability probe has answered, only the sensors whose type needs no
    /// capability are shown, so the first frame does not wait for it and no sensor shows as
    /// unavailable for a moment (SPEC §14.1); the others join once it has.
    pub(super) fn sync_sensors(&mut self, window: &dyn BarWindow) {
        let probed = self.platform.probed_capabilities();
        let waiting = probed.is_none();
        let caps = probed.unwrap_or_default();
        let mut shown = self.layout.clone();
        for entry in shown.all_mut() {
            if let Some(preview) = self.previews.get(&entry.id) {
                *entry = preview.clone();
            }
        }
        if waiting {
            for zone in [Zone::Left, Zone::Center, Zone::Right] {
                shown.zone_mut(zone).retain(|entry| {
                    registry()
                        .get(&entry.kind)
                        .is_none_or(|ty| ty.requires().is_empty())
                });
            }
            self.probe_since.get_or_insert_with(Instant::now);
            window.set_timer(TimerId::Probe, Some(PROBE_POLL));
        }
        let order = self.sensors.focus_order();
        if self.sensors.sync(&shown, &self.state, &caps, registry()) {
            // A removed sensor's focus goes to the one that took its place (SPEC §14.4).
            if let Some(keys) = self.keys.as_mut()
                && let Some(id) = keys.sensor.take()
            {
                keys.sensor = focus::after_change(&order, &self.sensors.focus_order(), &id);
            }
            // New clocks read the time now, so the next frame does not show them empty.
            let env = sensor_env!(self, false);
            self.sensors.wake(Instant::now(), &env);
            self.store_states();
            window.invalidate();
            self.refresh_hint(window);
            // Its sensor may have moved, or gone.
            self.relayout_popup(window);
        }
        let subscriptions = self.sensors.subscriptions();
        let created = self.sensors.take_created();
        for (id, topics) in subscription_changes(&self.subscribed, &subscriptions, &created) {
            self.host.subscribe(&id, topics);
        }
        self.subscribed = subscriptions;
        self.arm_timer(window);
    }

    /// Hands a sample to its sensors and redraws what changed.
    pub(super) fn deliver(&mut self, window: &dyn BarWindow, delivery: &Delivery) {
        let env = sensor_env!(self, false);
        if self.sensors.deliver(delivery, &env, Instant::now()) {
            window.invalidate();
            self.refresh_changed_hint(window);
        }
        self.store_states();
    }

    /// The locale for dates and numbers may have changed (the setting, or the OS regional
    /// format): it is read again, and every sensor measures again with it (SPEC §8.6).
    pub(super) fn locale_changed(&mut self, window: &dyn BarWindow) {
        self.locale = self.platform.locale(self.settings.date_locale);
        self.numbers = Numbers::from(&self.locale);
        self.sensors.invalidate_all();
        window.invalidate();
        self.refresh_hint(window);
        self.relayout_popup(window);
    }

    /// The wall clock jumped: clocks read it again and move their wake-ups.
    pub(super) fn clock_changed(&mut self, window: &dyn BarWindow) {
        let env = sensor_env!(self, false);
        if self.sensors.clock_changed(Instant::now(), &env) {
            window.invalidate();
            self.refresh_hint(window);
        }
        self.arm_timer(window);
    }

    /// Hands the state and props sensors want stored to the host (SPEC §11.2, §9.14).
    pub(super) fn store_states(&mut self) {
        for (id, state) in self.sensors.take_states() {
            self.host.set_sensor_state(&id, state);
        }
        for (id, changes) in self.sensors.take_props() {
            self.host.update_sensor(&id, changes);
        }
    }

    /// Sets the window's timer to the earliest wake-up any sensor asked for (SPEC §4.4).
    pub(super) fn arm_timer(&self, window: &dyn BarWindow) {
        let now = Instant::now();
        window.set_timer(
            TimerId::Wake,
            self.sensors
                .next_wake(now)
                .map(|at| at.saturating_duration_since(now)),
        );
    }
}
