//! `spacer` — empty space, optionally with a divider line (SPEC §9.3).

use std::time::{Duration, Instant};

use serde::Deserialize;

use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, Dirty, DrawCtx, LayoutCtx, Props, Sensor, SensorCtx, SensorType, State, typed,
};
use crate::bar::anim::{Easing, FRAME, Fade};
use crate::platform::{Canvas, RectF};

const SCHEMA: &[PropField] = &[
    PropField::new(
        "showDivider",
        FieldKind::Bool,
        "sensor.spacer.field.showDivider",
        Lit::Bool(false),
    ),
    PropField::new(
        "width",
        FieldKind::Slider {
            min: 3.0,
            max: 145.0,
            step: 1.0,
            unit: Some("common.px"),
        },
        "sensor.spacer.field.width",
        Lit::Int(9),
    ),
];

/// SPEC §9.3.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("show", "showDivider"),
    LegacyKey::new("width", "width"),
];

pub struct SpacerType;

impl SensorType for SpacerType {
    fn id(&self) -> &'static str {
        "spacer"
    }

    fn category(&self) -> Category {
        Category::General
    }

    fn name(&self) -> &'static str {
        "sensor.spacer.name"
    }

    fn description(&self) -> &'static str {
        "sensor.spacer.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockSpacer"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Spacer {
            props: typed(props)?,
            hover: Fade::at(0.0, Instant::now()),
            shown: 0.0,
            next_frame: None,
        }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpacerProps {
    show_divider: bool,
    width: f32,
}

/// The divider's opacity (SPEC §9.3).
const DIVIDER_OPACITY: f32 = 0.4;
/// A hidden divider fades in on hover, and out again (SPEC §9.3).
const FADE_IN: Duration = Duration::from_millis(200);
const FADE_OUT: Duration = Duration::from_millis(300);

struct Spacer {
    props: SpacerProps,
    /// How far a hidden divider has faded in, 0–1.
    hover: Fade,
    /// The value of `hover` as last drawn.
    shown: f32,
    next_frame: Option<Instant>,
}

impl Spacer {
    /// Moves to the fade's value at `now`; returns whether it changed.
    fn advance(&mut self, now: Instant) -> Dirty {
        self.next_frame = self.hover.running(now).then(|| now + FRAME);
        let value = self.hover.value(now);
        let changed = value != self.shown;
        self.shown = value;
        changed
    }
}

impl Sensor for Spacer {
    fn measure(&mut self, _cx: &LayoutCtx) -> f32 {
        self.props.width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        // Without the divider it is a transparent hit area, where the divider shows on hover.
        let opacity = if self.props.show_divider {
            DIVIDER_OPACITY
        } else {
            DIVIDER_OPACITY * self.shown
        };
        if opacity <= 0.0 {
            return;
        }
        let r = cx.rect;
        let x = r.left + ((self.props.width - 1.0) / 2.0).round();
        let line = canvas.snap(1.0).max(1.0 / canvas.scale());
        canvas.fill_rect(
            RectF::new(x, r.top + 3.0, x + line, r.bottom - 3.0),
            cx.theme.sensor_text.faded(opacity),
        );
    }

    fn on_hover(&mut self, hovered: bool, now: Instant, cx: &SensorCtx) -> Dirty {
        if self.props.show_divider {
            return false;
        }
        let (to, full) = if hovered {
            (1.0, FADE_IN)
        } else {
            (0.0, FADE_OUT)
        };
        let full = if cx.animate { full } else { Duration::ZERO };
        self.hover.go(to, full, Easing::Out, now);
        self.advance(now)
    }

    fn wake_at(&self, _now: Instant) -> Option<Instant> {
        self.next_frame
    }

    fn on_wake(&mut self, now: Instant, _cx: &SensorCtx) -> Dirty {
        self.advance(now)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{draw, measure};

    fn create(stored: serde_json::Value) -> Box<dyn Sensor> {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, _) = resolve(SCHEMA, &stored, &Props::new());
        SpacerType
            .create(&props, &State::new())
            .expect("valid props")
    }

    #[test]
    fn is_empty_space_by_default() {
        let mut sensor = create(json!({}));
        assert_eq!(measure(sensor.as_mut()), 9.0);
        assert_eq!(draw(sensor.as_ref(), 9.0), "");
    }

    #[test]
    fn hidden_divider_fades_in_on_hover() {
        let theme = crate::sensors::test_support::theme();
        let settings = crate::config::model::Settings::default();
        let platform = crate::platform::current();
        let cx = |animate| SensorCtx {
            id: "s_1",
            settings: &settings,
            theme: &theme,
            locked: false,
            alt: false,
            animate,
            changes: &crate::sampler::ChangeSink::NONE,
            platform: platform.as_ref(),
        };
        let mut sensor = create(json!({}));
        measure(sensor.as_mut());
        let t0 = Instant::now();
        sensor.on_hover(true, t0, &cx(true));
        assert_eq!(sensor.wake_at(t0), Some(t0 + FRAME));
        // Ease-out at half time: seven eighths of 40 %.
        assert!(sensor.on_wake(t0 + FADE_IN / 2, &cx(true)));
        assert_eq!(
            draw(sensor.as_ref(), 9.0),
            "fill rect 11,3 1×22 #FFFFFF/35%\n"
        );
        assert!(sensor.on_wake(t0 + FADE_IN, &cx(true)));
        assert_eq!(sensor.wake_at(t0 + FADE_IN), None);
        assert!(draw(sensor.as_ref(), 9.0).ends_with("#FFFFFF/40%\n"));

        // Without animations it is gone at once.
        assert!(sensor.on_hover(false, t0 + FADE_IN, &cx(false)));
        assert_eq!(draw(sensor.as_ref(), 9.0), "");
        assert_eq!(sensor.wake_at(t0 + FADE_IN), None);
    }

    #[test]
    fn divider_is_centred() {
        let mut sensor = create(json!({ "showDivider": true, "width": 20 }));
        assert_eq!(measure(sensor.as_mut()), 20.0);
        // x = 7 + round(19 / 2) = 17; 28 − 6 px tall.
        assert_eq!(
            draw(sensor.as_ref(), 20.0),
            "fill rect 17,3 1×22 #FFFFFF/40%\n"
        );
    }
}
