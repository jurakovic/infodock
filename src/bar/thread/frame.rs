//! The bar's motion and painting (SPEC §5.5): the rise and the slide out, the hover
//! highlight, and the frame the bar draws.

use super::*;

impl BarLogic {
    /// Draws the next frame of the bar's animations and asks for another while one runs.
    pub(super) fn frame(&mut self, window: &dyn BarWindow, now: Instant) {
        window.invalidate();
        // A finished fade rests at its end, whatever the clock says when painting.
        for fade in [&mut self.shown, &mut self.highlight] {
            if !fade.running(now) {
                *fade = Fade::at(fade.target(), now);
            }
        }
        if let Some(g) = &self.geometry {
            // A docked bar slides from its edge; a floating one only fades.
            let off = 1.0 - self.shown.value(now);
            let dy = if self.settings.bar.mode == BarMode::Docked {
                let dy = (off * g.rect.height() as f32).round() as i32;
                if g.edge == Edge::Top { -dy } else { dy }
            } else {
                0
            };
            // Highlight-only frames leave the window where it is.
            if self.slid != Some(dy) {
                self.slid = Some(dy);
                window.slide(dy);
            }
        }
        if self.sliding_out && !self.shown.running(now) {
            self.sliding_out = false;
            self.rise_pending = true;
            self.host.redock();
            // Already placed at the edge it docks to (the edge changed back meanwhile, or a
            // placement came during the slide): no new placement follows, so it rises now.
            let edge = self.settings.bar.edge;
            if self.geometry.as_ref().is_some_and(|g| g.edge == edge) {
                self.rise(window);
            }
        }
        if self.highlight.running(now) || self.shown.running(now) {
            window.set_timer(TimerId::Frame, Some(anim::FRAME));
        }
    }

    /// Starts the rise into place after a placement (SPEC §5.5).
    pub(super) fn rise(&mut self, window: &dyn BarWindow) {
        if !std::mem::take(&mut self.rise_pending) || self.sliding_out {
            return;
        }
        let now = Instant::now();
        if self.animate {
            self.shown = Fade::at(0.0, now);
            self.shown.go(1.0, RISE, Easing::Out, now);
        } else {
            self.shown = Fade::at(1.0, now);
        }
        self.frame(window, now);
    }

    /// Fades the hover highlight in while the pointer is over the bar and the setting is on,
    /// and out otherwise (SPEC §5.5).
    pub(super) fn refresh_highlight(&mut self, window: &dyn BarWindow) {
        let on = self.inside && self.settings.bar.highlight_on_hover;
        let (to, easing) = if on {
            (1.0, Easing::In)
        } else {
            (0.0, Easing::Out)
        };
        if self.highlight.target() == to {
            return;
        }
        let full = if self.animate {
            HIGHLIGHT
        } else {
            Duration::ZERO
        };
        self.highlight.go(to, full, easing, Instant::now());
        window.invalidate();
        window.set_timer(TimerId::Frame, Some(anim::FRAME));
    }

    /// Draws the bar into a cleared canvas of `size` logical px: the background, the
    /// sensors, the placer and the focus ring, faded while the bar rises (SPEC §5.5).
    pub(super) fn paint_frame(&mut self, canvas: &mut dyn Canvas, size: SizeF) {
        self.painted = size;
        self.sensors
            .update_layout(&layout_ctx!(self, canvas.scale()), size.width);
        let now = Instant::now();
        // The content fades in while the bar rises (SPEC §5.5).
        let opacity = self.shown.value(now);
        if opacity < 1.0 {
            canvas.push_opacity(opacity);
        }
        let highlight = self.highlight.value(now);
        // The edge it is placed on: the nearer one for a floating bar, the old one while it
        // slides out (SPEC §12.2).
        let edge = self
            .geometry
            .as_ref()
            .map_or(self.settings.bar.edge, |g| g.edge);
        let target = skins::Target::of(size, canvas.scale(), edge);
        let skin = self
            .theme
            .skin
            .as_ref()
            .and_then(|spec| self.skin.get(spec, target));
        // A band goes on the side facing the screen centre: above a bar at the bottom. The
        // skin's own band takes the place of the bar's shadow (SPEC §5.5). None while the
        // content fades, which a floating bar does in place.
        let above = edge == Edge::Bottom;
        let bitmap = match skin.and_then(|s| s.band.clone()) {
            _ if opacity < 1.0 => None,
            Some(band) => Some(band),
            None if self.settings.bar.shadow => self.shadow.get(target.width, target.scale, above),
            None => None,
        };
        self.band = bitmap.map(|bitmap| Band { bitmap, above });
        paint_bar(
            canvas,
            size,
            &self.theme,
            edge,
            highlight,
            skin.map(|s| &s.bar),
        );
        self.sensors.draw(canvas, &self.theme, size.height);
        // The placer: a 2 px line in the accent colour where the dragged sensor, or the
        // sensors a drop creates, would go (SPEC §6.2, §6.3, §12.2).
        let placer = self.drag.as_ref().and_then(|d| d.target).or_else(|| {
            self.external
                .as_ref()
                .filter(|e| !e.items.is_empty())
                .and_then(|e| e.target)
        });
        if let Some(target) = placer {
            let x = canvas.snap(target.placer_x - 1.0);
            canvas.fill_rect(
                RectF::new(x, 2.0, x + 2.0, size.height - 2.0),
                self.theme.accent,
            );
        }
        // The focus ring in the accent colour, while the bar has the keyboard (SPEC §14.4).
        if let Some(keys) = self.keys.as_ref().filter(|k| k.focused) {
            let place = keys
                .sensor
                .as_deref()
                .and_then(|id| self.sensors.place_of(id));
            let rect = focus::ring(place, size, self.theme.gap);
            canvas.stroke(
                &Shape::RoundedRect {
                    rect,
                    radius: focus::RING_RADIUS,
                },
                &Brush::Solid(self.theme.accent),
                focus::RING_WIDTH,
            );
        }
        if opacity < 1.0 {
            canvas.pop();
        }
    }
}
