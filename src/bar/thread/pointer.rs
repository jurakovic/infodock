//! The pointer on the bar (SPEC §5.1, §6.1–6.2): presses and clicks, the reorder drag,
//! moving and resizing a floating bar, and which sensor the pointer is over.

use super::*;

impl BarLogic {
    /// A press on the bar. A left press on a sensor dims it until release, if it can lead to a
    /// click or a drag (SPEC §6.1); the right button's menu comes on release, and middle clicks
    /// are reserved.
    pub(super) fn pointer_down(
        &mut self,
        window: &dyn BarWindow,
        button: PointerButton,
        at: PointF,
        modifiers: Modifiers,
    ) {
        // A press on the bar after the focus went elsewhere ends what is left of the keyboard
        // on it, so the ring does not come back with a menu's activation.
        if self.keys.as_ref().is_some_and(|k| !k.focused) {
            self.keys = None;
        }
        if button != PointerButton::Left {
            return;
        }
        // Any click closes the hint (SPEC §7.1) and an open popup, which keeps the focus since
        // the bar never takes it (§7.2).
        self.dismiss_hint(window);
        let had_popup = self.popups.owner().map(str::to_owned);
        self.close_popup(window);
        if let (Some(end), Some(g)) = (self.resize_end(at), &self.geometry) {
            let x = g.rect.left + (f64::from(at.x) * g.scale).round() as i32;
            window.capture_pointer(true);
            self.resizing = Some(Resizing {
                drag: Resize::new(end, g.rect, x),
                width: None,
            });
            return;
        }
        let Some(hit) = self.sensors.hit(at.x) else {
            if self.settings.bar.mode == BarMode::Floating && !self.settings.locked {
                window.capture_pointer(true);
                self.moving = Some(Moving {
                    press: Press::new(String::new(), at),
                    rect: None,
                });
            }
            return;
        };
        if !self.settings.clicks_allowed(modifiers.alt) {
            return;
        }
        window.capture_pointer(true);
        if self.sensors.set_pressed(Some(&hit.entry.id)) {
            window.invalidate();
        }
        let closed_popup = had_popup.as_deref() == Some(hit.entry.id.as_str());
        self.press = Some(Press {
            closed_popup,
            ..Press::new(hit.entry.id, at)
        });
    }

    /// The end of a floating, unlocked bar under `at`, by which it is resized (SPEC §5.1).
    pub(super) fn resize_end(&self, at: PointF) -> Option<resize::End> {
        if self.settings.bar.mode != BarMode::Floating || self.settings.locked {
            return None;
        }
        let g = self.geometry.as_ref()?;
        resize::end_at(at.x, (f64::from(g.rect.width()) / g.scale) as f32)
    }

    pub(super) fn set_cursor(&mut self, window: &dyn BarWindow, cursor: PopupCursor) {
        if self.cursor != cursor {
            self.cursor = cursor;
            window.set_cursor(cursor);
        }
    }

    pub(super) fn pointer_moved(&mut self, window: &dyn BarWindow, at: PointF) {
        if !self.inside {
            self.inside = true;
            self.refresh_highlight(window);
        }
        let pressed = self.moving.is_some() || self.drag.is_some() || self.press.is_some();
        let resizes = self.resizing.is_some() || (!pressed && self.resize_end(at).is_some());
        let cursor = if resizes {
            PopupCursor::ResizeWidth
        } else {
            PopupCursor::Arrow
        };
        self.set_cursor(window, cursor);
        if let Some(resizing) = &mut self.resizing {
            // The dragged end follows the pointer, in screen px since the window moves with
            // the left end.
            if let Some(g) = self.geometry.as_mut() {
                let x = g.rect.left + (f64::from(at.x) * g.scale).round() as i32;
                let (rect, width) = resizing.drag.rect(x, g.scale, g.monitor.work_area);
                if rect != g.rect
                    && let Some(rect) = window.resize_to(rect, width)
                {
                    g.rect = rect;
                    resizing.width = Some(width);
                }
            }
            return;
        }
        if let Some(moving) = &mut self.moving {
            moving.press.moved_to(at);
            if moving.press.moved {
                // The window follows, so the pointer stays at the pressed point inside it.
                let scale = self.geometry.as_ref().map_or(1.0, |g| g.scale);
                let dx = (f64::from(at.x - moving.press.at.x) * scale).round() as i32;
                let dy = (f64::from(at.y - moving.press.at.y) * scale).round() as i32;
                if (dx, dy) != (0, 0)
                    && let Some(rect) = window.move_by(dx, dy)
                {
                    moving.rect = Some(rect);
                    // The nearer edge follows, so the shadow and hairline turn while it moves.
                    if let Some(g) = self.geometry.as_mut() {
                        g.rect = rect;
                        let edge = nearer_edge(rect, &g.monitor);
                        if edge != g.edge {
                            g.edge = edge;
                            window.invalidate();
                        }
                    }
                }
            }
            return;
        }
        if let Some(drag) = &mut self.drag {
            let target = interact::drop_target(
                at.x,
                &self.sensors.spans(),
                self.sensors.bar_width(),
                self.theme.pad,
                self.theme.gap,
            );
            if drag.target != Some(target) {
                drag.target = Some(target);
                window.invalidate();
            }
            return;
        }
        if let Some(press) = &mut self.press {
            // Past the threshold an unlocked press becomes a reorder drag; a locked one just
            // stops being a click.
            if press.moved_to(at) {
                let id = press.id.clone();
                match self.sensors.find(&id).filter(|_| !self.settings.locked) {
                    Some(source) => self.start_drag(window, id, source, at),
                    None => {
                        self.sensors.set_pressed(None);
                        window.invalidate();
                    }
                }
            }
            return;
        }
        let over = self.sensors.hit(at.x).map(|h| h.entry.id);
        self.set_hovered(window, over.clone());
        // No hints over an open popup; the hover state waits, so the sensor whose popup closes
        // stays without its hint while the pointer remains over it (SPEC §7.1).
        if self.popups.open.is_some() {
            return;
        }
        let over = over.filter(|_| self.settings.popups_enabled);
        let popups = &self.settings.popups;
        let step = self
            .hover
            .over(over.as_deref(), popups.show_delay(), popups.hide_delay());
        self.apply_hint(window, step);
    }

    /// Starts dragging `id` from `source`; the sensor stays dimmed in place and a placer shows
    /// where it would go (SPEC §6.2). Hints stay closed meanwhile.
    pub(super) fn start_drag(
        &mut self,
        window: &dyn BarWindow,
        id: String,
        source: (Zone, usize),
        at: PointF,
    ) {
        self.dismiss_hint(window);
        self.drag = Some(Drag {
            id,
            source,
            target: None,
        });
        window.set_timer(TimerId::Drag, Some(ESC_POLL));
        self.pointer_moved(window, at);
    }

    /// Ends a drag without moving anything (Esc, lost capture, a menu).
    pub(super) fn cancel_drag(&mut self, window: &dyn BarWindow) {
        if self.drag.take().is_some() || self.press.is_some() {
            self.press = None;
            window.set_timer(TimerId::Drag, None);
            window.capture_pointer(false);
            self.sensors.set_pressed(None);
            window.invalidate();
        }
    }

    pub(super) fn pointer_left(&mut self, window: &dyn BarWindow) {
        if self.resizing.is_none() {
            self.set_cursor(window, PopupCursor::Arrow);
        }
        let step = self.hover.leave(self.settings.popups.hide_delay());
        self.apply_hint(window, step);
        self.inside = false;
        self.refresh_highlight(window);
        self.set_hovered(window, None);
    }

    /// Tells the sensors the pointer entered or left (e.g. the Spacer's divider fades in).
    pub(super) fn set_hovered(&mut self, window: &dyn BarWindow, id: Option<String>) {
        if self.hovered == id {
            return;
        }
        let env = sensor_env!(self, false);
        let now = Instant::now();
        let mut dirty = false;
        if let Some(old) = &self.hovered {
            dirty |= self.sensors.on_hover(old, false, now, &env);
        }
        if let Some(new) = &id {
            dirty |= self.sensors.on_hover(new, true, now, &env);
        }
        self.hovered = id;
        if dirty {
            window.invalidate();
        }
        self.arm_timer(window);
    }

    /// Stores where a moved floating bar is now (SPEC §5.1).
    pub(super) fn end_moving(&mut self, window: &dyn BarWindow) {
        let Some(moving) = self.moving.take() else {
            return;
        };
        window.capture_pointer(false);
        if let Some(rect) = moving.rect {
            self.host.change_settings(Box::new(move |s| {
                let width = s.bar.floating.map_or(DEFAULT_FLOATING_WIDTH, |f| f.width);
                s.bar.floating = Some(FloatingRect {
                    x: rect.left,
                    y: rect.top,
                    width,
                });
            }));
        }
    }

    /// Stores the width and place of a resized floating bar (SPEC §5.1); the *Width* setting
    /// follows.
    pub(super) fn end_resizing(&mut self, window: &dyn BarWindow) {
        let Some(resizing) = self.resizing.take() else {
            return;
        };
        window.capture_pointer(false);
        if let (Some(width), Some(g)) = (resizing.width, &self.geometry) {
            let rect = g.rect;
            self.host.change_settings(Box::new(move |s| {
                s.bar.floating = Some(FloatingRect {
                    x: rect.left,
                    y: rect.top,
                    width,
                });
            }));
        }
    }

    /// Releasing over the pressed sensor without having moved is a click.
    pub(super) fn pointer_up(&mut self, window: &dyn BarWindow, at: PointF, modifiers: Modifiers) {
        if self.resizing.is_some() {
            self.end_resizing(window);
            return;
        }
        if self.moving.is_some() {
            self.end_moving(window);
            return;
        }
        if let Some(drag) = self.drag.take() {
            self.press = None;
            window.set_timer(TimerId::Drag, None);
            window.capture_pointer(false);
            self.sensors.set_pressed(None);
            window.invalidate();
            if let Some(target) = drag.target
                && !interact::is_no_move(drag.source, &target)
            {
                let index = index_after_removal(drag.source, target.zone, target.insertion);
                self.host.move_sensor(&drag.id, target.zone, index);
            }
            return;
        }
        let Some(press) = self.press.take() else {
            return;
        };
        window.capture_pointer(false);
        if self.sensors.set_pressed(None) {
            window.invalidate();
        }
        let over = self
            .sensors
            .hit(at.x)
            .is_some_and(|h| h.entry.id == press.id);
        if press.moved || !over || !self.settings.clicks_allowed(modifiers.alt) {
            return;
        }
        // The press that closed this sensor's popup does not open it again, nor does a click
        // right after its popup closed by losing the focus (SPEC §7.2).
        if press.closed_popup || self.popups.guarded(&press.id, Instant::now()) {
            return;
        }
        self.click(window, &press.id, modifiers.alt);
    }

    /// Clicks sensor `id`, with the pointer or the keyboard (SPEC §6.1, §14.4).
    pub(super) fn click(&mut self, window: &dyn BarWindow, id: &str, alt: bool) {
        let env = sensor_env!(self, alt);
        match self.sensors.click(id, &env) {
            ClickResult::None => {}
            ClickResult::Handled => window.invalidate(),
            // Asked first (SPEC §9.21); the sensor acts only if the user agrees.
            ClickResult::Confirm(question) => {
                if window.confirm(&question.message, &question.action)
                    && self.sensors.confirmed(id, &env)
                {
                    window.invalidate();
                }
            }
            ClickResult::Popup(popup) => self.open_popup(window, id, popup),
        }
        // A click may change what a sensor shows for good (SPEC §9.14).
        self.store_states();
    }
}
