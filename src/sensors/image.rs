//! `image` — a picture, by default the Infodock logo (SPEC §9.2).

use std::path::Path;

use serde::Deserialize;

use super::common::icon::{self, IconSize, IconStyle};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, DrawCtx, Fetched, HintContent, IconSource, LayoutCtx, Props, Sensor, SensorCtx,
    SensorType, State, typed,
};
use crate::i18n::t;
use crate::look::theme::Theme;
use crate::platform::{Bitmap, Canvas, Color, RectF, decode_image};

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
    icon::COLOR.describe("sensor.image.field.color.help"),
    icon::OPACITY,
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

/// SPEC §9.2.
const LEGACY_KEYS: &[LegacyKey] = &[
    icon::LEGACY_IMAGE,
    icon::LEGACY_COLOR,
    icon::LEGACY_OPACITY,
    LegacyKey::new("text", "popupText"),
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

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Image {
            props: typed(props)?,
            bitmap: None,
            loaded: None,
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
    /// Its image empty: the logo. Custom tints the image (SPEC §8.4); standard leaves it
    /// unchanged.
    #[serde(flatten)]
    look: IconStyle,
    /// `%v` is replaced by the app version.
    popup_text: String,
}

struct Image {
    props: ImageProps,
    /// Decoded at the bar's physical height − 1 px.
    bitmap: Option<Bitmap>,
    /// The height and sensor text colour `bitmap` was made for.
    loaded: Option<(u32, Color)>,
}

impl Image {
    /// The image `height` physical px tall, tinted; the logo if the file cannot be read; `None`
    /// while the bar reads the file (SPEC §9.2). The logo is built in, so with Standard colour
    /// it takes the sensor text colour (§8.4).
    fn load(&self, height: u32, theme: &Theme, icons: &dyn IconSource) -> Option<Option<Bitmap>> {
        let file = match self.props.look.image.as_str() {
            "" => None,
            path => match icons.image(path, height) {
                Fetched::Loading => return None,
                Fetched::None => None,
                Fetched::Ready(bitmap) => Some(bitmap),
            },
        };
        let built_in = file.is_none();
        let bitmap = match file {
            Some(bitmap) => Some(bitmap),
            None => decode_image(LOGO, Some(height))
                .map_err(|e| tracing::error!("the logo cannot be decoded: {e}"))
                .ok(),
        };
        Some(bitmap.map(
            |bitmap| match icon::tint(&self.props.look.color, built_in, theme) {
                Some(tint) => bitmap.tinted(tint),
                None => bitmap,
            },
        ))
    }

    fn popup_text(&self) -> String {
        self.props.popup_text.replace("%v", crate::VERSION)
    }
}

impl Sensor for Image {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        // The image keeps its aspect ratio (SPEC §9.2).
        let height = IconSize::Picture.physical(cx.theme, cx.scale);
        // Loaded again for another size or text colour (the logo follows it).
        let key = (height, cx.theme.sensor_text);
        // While the file is read it keeps what it had (nothing when new); the bar measures
        // again once it arrives.
        if self.loaded != Some(key)
            && let Some(bitmap) = self.load(height, cx.theme, cx.icons)
        {
            self.bitmap = bitmap;
            self.loaded = Some(key);
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
        canvas.draw_bitmap(
            bitmap,
            dest,
            self.props.look.opacity.min(100) as f32 / 100.0,
        );
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
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{
        Harness, Later, draw, measure, measure_with, resolved, theme,
    };

    fn image(stored: serde_json::Value) -> Image {
        let props = resolved(SCHEMA, stored);
        Image {
            props: typed(&props).expect("valid props"),
            bitmap: None,
            loaded: None,
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
        let h = Harness::new();
        let width = sensor.measure(&LayoutCtx {
            scale: 1.5,
            ..h.layout()
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
    fn takes_no_room_until_its_file_arrives() {
        let mut sensor = image(json!({ "image": "\\\\server\\share\\wide.png" }));
        let loader = Later::default();
        assert_eq!(measure_with(&mut sensor, &loader), 0.0, "being read");
        assert_eq!(draw(&sensor, 0.0), "");
        assert_eq!(
            loader.asked(),
            [("\\\\server\\share\\wide.png".to_owned(), 27)]
        );
        // It arrived: 54 × 27, drawn.
        loader.arrive(Bitmap::from_rgba(54, 27, vec![255; 54 * 27 * 4]).expect("a bitmap"));
        assert_eq!(measure_with(&mut sensor, &loader), 54.0);
        assert_eq!(
            draw(&sensor, 54.0),
            "bitmap 54×27 into 7,0 54×27 opacity 1\n"
        );
        // A file that cannot be read shows the logo.
        let mut sensor = image(json!({ "image": "\\\\server\\share\\gone.png" }));
        let loader = Later::default();
        assert_eq!(measure_with(&mut sensor, &loader), 0.0);
        loader.fail();
        assert_eq!(measure_with(&mut sensor, &loader), 27.0);
    }

    #[test]
    fn hint_is_the_popup_text_with_the_version() {
        let h = Harness::new();
        let cx = h.ctx();
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
