//! `text` — a line of text (SPEC §9.1).

use serde::Deserialize;
use serde_json::Value;

use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx, SensorType, State, typed,
};
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

/// A text the bar narrows (SPEC §5.4) keeps this many ems: a character or two and the ellipsis.
const MIN_EMS: f32 = 2.0;

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
            width: 0.0,
            cut: None,
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
    /// The whole text, and the width it measured.
    layout: Option<TextLayout>,
    width: f32,
    /// The text ending in an ellipsis, while the bar gives it less than its width.
    cut: Option<TextLayout>,
}

impl Text {
    fn style(&self, cx: &LayoutCtx) -> TextStyle {
        let family = if self.props.font.is_empty() {
            &cx.theme.sensor_font.family
        } else {
            &self.props.font
        };
        TextStyle::new(
            &Font::new(family, self.props.font_style),
            self.props.font_size,
        )
    }

    /// The shadow's offset, logical px.
    fn shadow(&self) -> f32 {
        if self.props.shadow { 1.0 } else { 0.0 }
    }
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
        let layout = cx.text.layout(&self.props.text, &self.style(cx));
        self.width = (layout.width() + self.shadow()).ceil();
        self.layout = Some(layout);
        self.cut = None;
        self.width
    }

    fn min_width(&self) -> Option<f32> {
        Some((self.props.font_size * MIN_EMS + self.shadow()).ceil())
    }

    fn fit(&mut self, width: f32, cx: &LayoutCtx) {
        self.cut = (width < self.width).then(|| {
            let style = self.style(cx).max_width((width - self.shadow()).max(0.0));
            cx.text.layout(&self.props.text, &style)
        });
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(layout) = self.cut.as_ref().or(self.layout.as_ref()) else {
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

    /// None, unless the text is cut: then the whole text (SPEC §5.4).
    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        self.cut
            .is_some()
            .then(|| HintContent::Text(self.props.text.clone()))
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
    use crate::sensors::test_support::{draw, fit, measure, theme};

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
    fn a_narrower_width_cuts_the_text_and_its_hint_shows_it_all() {
        let theme = theme();
        let settings = crate::config::model::Settings::default();
        let platform = crate::platform::current();
        let cx = SensorCtx {
            id: "s_1",
            settings: &settings,
            theme: &theme,
            locked: false,
            alt: false,
            animate: true,
            changes: &crate::sampler::ChangeSink::NONE,
            platform: platform.as_ref(),
        };
        let mut sensor = create(json!({ "text": "Overlapping" }));
        // 11 × 6.5 px; two ems of 13 px at least.
        assert_eq!(measure(sensor.as_mut()), 72.0);
        assert_eq!(sensor.min_width(), Some(26.0));
        assert_eq!(sensor.hint(&cx), None, "no hint while it is whole");

        // 40 px hold five characters and the ellipsis.
        fit(sensor.as_mut(), 40.0);
        assert_eq!(
            draw(sensor.as_ref(), 40.0),
            "text \"Overl…\" Segoe UI Variable Text 400 13px at 7,6 #FFFFFF\n"
        );
        assert_eq!(
            sensor.hint(&cx),
            Some(HintContent::Text("Overlapping".into()))
        );

        // Its whole width again: whole, without a hint.
        fit(sensor.as_mut(), 72.0);
        assert!(draw(sensor.as_ref(), 72.0).contains("\"Overlapping\""));
        assert_eq!(sensor.hint(&cx), None);

        // With a shadow, both copies are cut, and the minimum makes room for it.
        let mut sensor = create(json!({ "text": "Overlapping", "shadow": true }));
        assert_eq!(measure(sensor.as_mut()), 73.0);
        assert_eq!(sensor.min_width(), Some(27.0));
        fit(sensor.as_mut(), 41.0);
        assert_eq!(draw(sensor.as_ref(), 41.0).matches("\"Overl…\"").count(), 2);
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
