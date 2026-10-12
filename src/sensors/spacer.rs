//! `spacer` — empty space, optionally with a divider line (SPEC §9.3).

use serde::Deserialize;

use super::schema::{FieldKind, Lit, PropField};
use super::{Category, DrawCtx, LayoutCtx, Props, Sensor, SensorType, State, typed};
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

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Spacer {
            props: typed(props)?,
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

struct Spacer {
    props: SpacerProps,
}

impl Sensor for Spacer {
    fn measure(&mut self, _cx: &LayoutCtx) -> f32 {
        self.props.width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        // Without the divider it is a transparent hit area. The divider fading in on hover
        // arrives with hover tracking (M1 S7).
        if !self.props.show_divider {
            return;
        }
        let r = cx.rect;
        let x = r.left + ((self.props.width - 1.0) / 2.0).round();
        let line = canvas.snap(1.0).max(1.0 / canvas.scale());
        canvas.fill_rect(
            RectF::new(x, r.top + 3.0, x + line, r.bottom - 3.0),
            cx.theme.sensor_text.faded(DIVIDER_OPACITY),
        );
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
