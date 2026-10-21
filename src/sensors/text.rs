//! `text` — a line of text (SPEC §9.1).

use serde::Deserialize;
use serde_json::Value;

use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{Category, DrawCtx, LayoutCtx, Props, Sensor, SensorCtx, SensorType, State, typed};
use crate::config::model::FontStyle;
use crate::platform::{Canvas, Color, Font, PointF, TextLayout, TextStyle};

const SCHEMA: &[PropField] = &[
    PropField::new(
        "text",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.text.field.text",
        Lit::Str("Text"),
    )
    .required("sensor.text.field.text.required"),
    PropField::new("font", FieldKind::Font, "sensor.field.font", Lit::Str("")),
    PropField::new(
        "fontStyle",
        FieldKind::FontStyle,
        "sensor.field.fontStyle",
        Lit::Str("normal"),
    ),
    PropField::new(
        "fontSize",
        FieldKind::Slider {
            min: 9.0,
            max: 25.0,
            step: 1.0,
            unit: Some("common.px"),
        },
        "sensor.field.fontSize",
        Lit::Int(13),
    ),
    PropField::new(
        "shadow",
        FieldKind::Bool,
        "sensor.field.shadow",
        Lit::Bool(false),
    )
    .describe("sensor.field.shadow.help"),
];

/// SPEC §9.1.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("text", "text"),
    LegacyKey::new("font", "font"),
    LegacyKey::new("fontstyle", "fontStyle"),
    LegacyKey::new("fontsize", "fontSize"),
    LegacyKey::new("shadow", "shadow"),
];

pub struct TextType;

impl SensorType for TextType {
    fn id(&self) -> &'static str {
        "text"
    }

    fn category(&self) -> Category {
        Category::General
    }

    fn name(&self) -> &'static str {
        "sensor.text.name"
    }

    fn description(&self) -> &'static str {
        "sensor.text.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockText"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Text {
            props: typed(props)?,
            layout: None,
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        props
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextProps {
    text: String,
    /// `""` follows the global sensor font.
    font: String,
    font_style: FontStyle,
    font_size: f32,
    shadow: bool,
}

struct Text {
    props: TextProps,
    layout: Option<TextLayout>,
}

/// The shadow colour for text in `color` (SPEC §9.1): black under light text, white under dark.
pub(super) fn shadow_color(color: Color) -> Color {
    let grey = 255.0 * (0.2 * color.r + 0.5 * color.g + 0.3 * color.b);
    if grey >= 128.0 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

impl Sensor for Text {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let family = if self.props.font.is_empty() {
            &cx.theme.sensor_font.family
        } else {
            &self.props.font
        };
        let style = TextStyle::new(
            &Font::new(family, self.props.font_style),
            self.props.font_size,
        );
        let layout = cx.text.layout(&self.props.text, &style);
        let shadow = if self.props.shadow { 1.0 } else { 0.0 };
        let width = (layout.width() + shadow).ceil();
        self.layout = Some(layout);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(layout) = &self.layout else {
            return;
        };
        let r = cx.rect;
        let color = cx.theme.sensor_text;
        let y = canvas.snap(r.top + (r.height() - layout.height()) / 2.0);
        if self.props.shadow {
            let shadow = shadow_color(color).with_alpha(color.a);
            canvas.draw_text(layout, PointF::new(r.left + 1.0, y + 1.0), shadow);
        }
        canvas.draw_text(layout, PointF::new(r.left, y), color);
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.props.text.clone()
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
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty());
        TextType.create(&props, &State::new()).expect("valid props")
    }

    #[test]
    fn summary_is_the_text() {
        let summary = |stored: serde_json::Value| {
            let stored = stored.as_object().cloned().unwrap_or_default();
            TextType.summary(&resolve(SCHEMA, &stored, &Props::new()).0)
        };
        assert_eq!(summary(json!({ "text": "Hello" })), Some("Hello".into()));
        assert_eq!(summary(json!({ "text": "" })), None);
    }

    #[test]
    fn draws_one_centred_line() {
        let mut sensor = create(json!({}));
        // "Text": 4 × 6.5 px at 13 px.
        assert_eq!(measure(sensor.as_mut()), 26.0);
        assert_eq!(
            draw(sensor.as_ref(), 26.0),
            "text \"Text\" Segoe UI Variable Text 400 13px at 7,6 #FFFFFF\n"
        );
    }

    #[test]
    fn custom_font_and_shadow() {
        let mut sensor = create(json!({
            "text": "Hi", "font": "Consolas", "fontStyle": "boldItalic", "fontSize": 20,
            "shadow": true
        }));
        assert_eq!(measure(sensor.as_mut()), 21.0);
        assert_eq!(
            draw(sensor.as_ref(), 21.0),
            "text \"Hi\" Consolas 700 italic 20px at 8,3 #000000\n\
             text \"Hi\" Consolas 700 italic 20px at 7,2 #FFFFFF\n"
        );
    }

    #[test]
    fn shadow_contrasts_with_the_text() {
        assert_eq!(shadow_color(Color::WHITE), Color::BLACK);
        assert_eq!(
            shadow_color(Color::rgba(0x1A, 0x1A, 0x1A, 1.0)),
            Color::WHITE
        );
        // Pure green: grey = 0.5 × 255 = 127.5 < 128.
        assert_eq!(shadow_color(Color::rgba(0, 255, 0, 1.0)), Color::WHITE);
    }
}
