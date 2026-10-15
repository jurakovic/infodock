//! `image` — a picture, by default the Infodock logo (SPEC §9.2).

use std::path::Path;

use serde::Deserialize;

use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, ColorChoice, ColorModeKind, DrawCtx, HintContent, LayoutCtx, Props, Sensor,
    SensorCtx, SensorType, State, typed,
};
use crate::i18n::t;
use crate::platform::{Bitmap, Canvas, Color, RectF, decode_image, load_image};

/// The default image (Appendix C): monochrome white, so a custom colour tints it.
const LOGO: &[u8] = include_bytes!("../../assets/logo.svg");

const SCHEMA: &[PropField] = &[
    PropField::new(
        "image",
        FieldKind::File {
            extensions: &["png", "jpg", "jpeg", "gif", "bmp", "ico", "svg"],
        },
        "sensor.image.field.image",
        Lit::Str(""),
    )
    .describe("sensor.image.field.image.help"),
    PropField::new(
        "color",
        FieldKind::ColorMode { dynamic: false },
        "sensor.field.color",
        Lit::ColorMode("standard", "#FFFFFF"),
    )
    .describe("sensor.image.field.color.help"),
    PropField::new(
        "opacity",
        FieldKind::Opacity { min: 20 },
        "sensor.field.opacity",
        Lit::Int(100),
    ),
    PropField::new(
        "popupText",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.image.field.popupText",
        Lit::Str(""),
    )
    .describe("sensor.image.field.popupText.help"),
];

pub struct ImageType;

impl SensorType for ImageType {
    fn id(&self) -> &'static str {
        "image"
    }

    fn category(&self) -> Category {
        Category::General
    }

    fn name(&self) -> &'static str {
        "sensor.image.name"
    }

    fn description(&self) -> &'static str {
        "sensor.image.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockImage"]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Image {
            props: typed(props)?,
            bitmap: None,
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        let path = props
            .get("image")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        Some(if path.is_empty() {
            t("sensor.image.logo")
        } else {
            let name = Path::new(path).file_name().and_then(|n| n.to_str());
            name.unwrap_or(path).to_owned()
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageProps {
    /// A file path; `""` is the logo.
    image: String,
    /// Custom tints the image (SPEC §8.4); standard leaves it unchanged.
    color: ColorChoice,
    /// Percent.
    opacity: u32,
    /// `%v` is replaced by the app version.
    popup_text: String,
}

struct Image {
    props: ImageProps,
    /// Decoded at the bar's physical height − 1 px.
    bitmap: Option<Bitmap>,
}

impl Image {
    /// The image `height` physical px tall, tinted; the logo if the file cannot be read.
    fn load(&self, height: u32) -> Option<Bitmap> {
        let decoded = if self.props.image.is_empty() {
            decode_image(LOGO, Some(height))
        } else {
            load_image(Path::new(&self.props.image), Some(height)).or_else(|e| {
                tracing::warn!("image {:?} cannot be shown: {e}", self.props.image);
                decode_image(LOGO, Some(height))
            })
        };
        let bitmap = decoded
            .map_err(|e| tracing::error!("the logo cannot be decoded: {e}"))
            .ok()?;
        let tint = match self.props.color.mode {
            ColorModeKind::Custom => Color::from_hex(&self.props.color.color),
            ColorModeKind::Standard | ColorModeKind::Dynamic => None,
        };
        Some(match tint {
            Some(tint) => bitmap.tinted(tint),
            None => bitmap,
        })
    }

    fn popup_text(&self) -> String {
        self.props.popup_text.replace("%v", crate::VERSION)
    }
}

impl Sensor for Image {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        // The image is one px less than the bar tall, keeping its aspect ratio (SPEC §9.2).
        let logical = cx.theme.bar_height.saturating_sub(1).max(1) as f32;
        let height = (logical * cx.scale).round().max(1.0) as u32;
        if self.bitmap.as_ref().is_none_or(|b| b.height() != height) {
            self.bitmap = self.load(height);
        }
        self.bitmap
            .as_ref()
            .map_or(0.0, |b| (b.width() as f32 / cx.scale).ceil())
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(bitmap) = &self.bitmap else {
            return;
        };
        let scale = canvas.scale();
        // On whole physical px, so the bitmap is copied pixel for pixel, not blurred (at 150 %
        // a 7 px padding is 10.5 px).
        let (left, top) = (canvas.snap(cx.rect.left), canvas.snap(cx.rect.top));
        let dest = RectF::new(
            left,
            top,
            left + bitmap.width() as f32 / scale,
            top + bitmap.height() as f32 / scale,
        );
        canvas.draw_bitmap(bitmap, dest, self.props.opacity.min(100) as f32 / 100.0);
    }

    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        let text = self.popup_text();
        (!text.is_empty()).then_some(HintContent::Text(text))
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.popup_text()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::platform::recording::FixedMetrics;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{draw, measure, theme};

    fn image(stored: serde_json::Value) -> Image {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty());
        Image {
            props: typed(&props).expect("valid props"),
            bitmap: None,
        }
    }

    #[test]
    fn shows_the_logo_one_px_less_than_the_bar() {
        let mut sensor = image(json!({}));
        // The logo is square: 27 × 27 on a 28 px bar.
        assert_eq!(measure(&mut sensor), 27.0);
        assert_eq!(
            draw(&sensor, 27.0),
            "bitmap 27×27 into 7,0 27×27 opacity 1\n"
        );

        // At 150 % it is decoded at its physical size: 41 px, 27.3 logical px wide.
        let width = sensor.measure(&LayoutCtx {
            theme: &theme(),
            text: &FixedMetrics,
            scale: 1.5,
            numbers: Default::default(),
            icons: &crate::sensors::NoIcons,
        });
        assert_eq!(width, 28.0);
        assert_eq!(sensor.bitmap.as_ref().map(Bitmap::height), Some(41));
        // Drawn from a whole physical px: 7 logical px is 10.5, so 11 (7.33 logical).
        let mut canvas = crate::platform::recording::RecordingCanvas::new(1.5);
        let rect = RectF::new(7.0, 0.0, 35.0, 28.0);
        sensor.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme(),
                rect,
            },
        );
        assert!(
            canvas
                .dump()
                .starts_with("bitmap 41×41 into 7.33,0 27.33×27.33"),
            "{}",
            canvas.dump()
        );
    }

    #[test]
    fn custom_colour_tints_and_opacity_fades() {
        let mut sensor = image(json!({
            "color": { "mode": "custom", "color": "#FF0000" }, "opacity": 50
        }));
        measure(&mut sensor);
        let bitmap = sensor.bitmap.as_ref().expect("decoded");
        // Every visible pixel is red.
        assert!(
            bitmap
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| px[1] == 0 && px[2] == 0)
        );
        assert!(draw(&sensor, 27.0).ends_with("opacity 0.5\n"));
    }

    #[test]
    fn missing_file_falls_back_to_the_logo() {
        let mut sensor = image(json!({ "image": "Z:\\no\\such\\file.png" }));
        assert_eq!(measure(&mut sensor), 27.0);
    }

    #[test]
    fn hint_is_the_popup_text_with_the_version() {
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
            platform: platform.as_ref(),
        };
        assert_eq!(image(json!({})).hint(&cx), None);
        assert_eq!(
            image(json!({ "popupText": "Infodock %v" })).hint(&cx),
            Some(HintContent::Text(format!("Infodock {}", crate::VERSION)))
        );
    }

    #[test]
    fn summary_is_the_file_name_or_the_logo() {
        let summary = |stored: serde_json::Value| {
            let stored = stored.as_object().cloned().unwrap_or_default();
            ImageType.summary(&resolve(SCHEMA, &stored, &Props::new()).0)
        };
        assert_eq!(summary(json!({})), Some("Infodock logo".into()));
        assert_eq!(
            summary(json!({ "image": "C:/Pictures/cat.png" })),
            Some("cat.png".into())
        );
    }
}
