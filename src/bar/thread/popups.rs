//! The interactive popup (SPEC §7.2): opening it beside its sensor, laying it out, its
//! input, wake-ups and replies, and closing it.

use super::*;

impl BarLogic {
    /// Opens `popup` for sensor `id` next to it (SPEC §7.2): the hint closes, the sensor shows
    /// its active state, and the popup window takes the focus.
    pub(super) fn open_popup(&mut self, window: &dyn BarWindow, id: &str, popup: Box<dyn Popup>) {
        self.dismiss_hint(window);
        self.close_popup(window);
        self.popups.open = Some(Open {
            owner: id.to_owned(),
            popup,
            frame: None,
            room: SizeF::new(0.0, 0.0),
            monitor_height: 0.0,
            free_left: false,
            captured: false,
        });
        if self.sensors.set_active(Some(id)) {
            window.invalidate();
        }
        self.relayout_popup(window);
        self.arm_popup_timer(window);
    }

    /// Lays the open popup out, places it next to its sensor and shows it; closes it when its
    /// sensor is gone.
    pub(super) fn relayout_popup(&mut self, window: &dyn BarWindow) {
        let Some(owner) = self.popups.owner().map(str::to_owned) else {
            return;
        };
        // Placed by its sensor, which may have moved.
        self.sensors
            .refresh_layout(&self.theme, self.text.as_ref(), &self.icons);
        let Some((anchor, _)) = self.anchor(&owner) else {
            self.close_popup(window);
            return;
        };
        let mut room = popup::room(&anchor);
        let Some(open) = self.popups.open.as_mut() else {
            return;
        };
        // A grip being dragged: no wider than its side has room for, so it does not flip.
        if open.captured && open.popup.align() == PopupAlign::LeftEdge {
            room.width = room.width.min(popup::room_beside(&anchor, open.free_left));
        }
        open.room = room;
        open.monitor_height = anchor.monitor.height();
        self.with_popup(|open, cx| {
            let size = open.popup.layout(cx);
            open.place(size, &anchor, cx.theme);
        });
        self.render_popup(window);
    }

    /// Draws the open popup again where it is.
    pub(super) fn render_popup(&self, window: &dyn BarWindow) {
        let (Some(open), Some(geometry)) = (&self.popups.open, &self.geometry) else {
            return;
        };
        let Some(frame) = open.frame else {
            return;
        };
        let scale = geometry.scale as f32;
        let theme = &self.theme;
        let name = Some(open.popup.name())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| t("app.name"));
        let rect = physical_rect(frame.window, scale);
        window.show_popup(rect, scale, &name, &mut |canvas| {
            popup::paint(canvas, open, theme);
        });
    }

    /// Closes the open popup, if any: its window hides, its sensor's active state ends, and
    /// the sensor ignores clicks for a moment (SPEC §7.2).
    pub(super) fn close_popup(&mut self, window: &dyn BarWindow) {
        let Some(mut open) = self.popups.close(Instant::now()) else {
            return;
        };
        if open.captured {
            window.capture_popup_pointer(false);
        }
        window.hide_popup();
        window.set_timer(TimerId::Popup, None);
        if let Some(changes) = open.popup.take_props() {
            self.host.update_sensor(&open.owner, changes);
        }
        if self.sensors.set_active(None) {
            window.invalidate();
        }
    }

    /// Carries out what the popup asked for after an input, a wake-up or a poll.
    pub(super) fn popup_reply(&mut self, window: &dyn BarWindow, reply: PopupReply) {
        if let Some(open) = self.popups.open.as_mut() {
            // The pointer's shape and capture follow the popup (a grip being dragged).
            window.set_popup_cursor(open.popup.cursor());
            let captures = open.popup.captures();
            if open.captured != captures {
                open.captured = captures;
                window.capture_popup_pointer(captures);
            }
            if let Some(changes) = open.popup.take_props() {
                let owner = open.owner.clone();
                self.host.update_sensor(&owner, changes);
            }
        }
        match reply {
            PopupReply::Nothing => {}
            PopupReply::Redraw => self.render_popup(window),
            PopupReply::Relayout => self.relayout_popup(window),
            PopupReply::Close => {
                self.close_popup(window);
                self.refocus(window);
            }
        }
        self.arm_popup_timer(window);
    }

    /// Runs `f` on the open popup with its context, the one place it is built; `None` when
    /// none is open.
    pub(super) fn with_popup<R>(&mut self, f: impl FnOnce(&mut Open, &PopupCtx) -> R) -> Option<R> {
        let scale = self.geometry.as_ref().map_or(1.0, |g| g.scale as f32);
        let edge = self
            .geometry
            .as_ref()
            .map_or(self.settings.bar.edge, |g| g.edge);
        let open = self.popups.open.as_mut()?;
        let cx = PopupCtx {
            theme: &self.theme,
            text: self.text.as_ref(),
            scale,
            platform: &self.platform,
            icons: &self.icons.transient(),
            waker: &self.waker,
            room: open.room,
            monitor_height: open.monitor_height,
            edge,
            free_left: open.free_left,
            animate: self.animate,
            now: Instant::now(),
        };
        Some(f(open, &cx))
    }

    /// Input in the popup window: Esc closes it (SPEC §7.2), the rest goes to the popup.
    pub(super) fn popup_input(&mut self, window: &dyn BarWindow, input: PopupInput) {
        if matches!(
            input,
            PopupInput::Key {
                key: Key::Escape,
                ..
            }
        ) {
            self.close_popup(window);
            self.refocus(window);
            return;
        }
        let reply = self.with_popup(|open, cx| {
            let input = open.to_content(input);
            open.popup.input(&input, cx)
        });
        if let Some(reply) = reply {
            self.popup_reply(window, reply);
        }
    }

    /// Sets the popup timer to the open popup's next wake-up.
    pub(super) fn arm_popup_timer(&self, window: &dyn BarWindow) {
        let now = Instant::now();
        let at = self.popups.open.as_ref().and_then(|o| o.popup.wake_at());
        window.set_timer(
            TimerId::Popup,
            at.map(|at| at.saturating_duration_since(now)),
        );
    }
}
