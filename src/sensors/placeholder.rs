//! Placeholders (SPEC §4.6) for layout entries that cannot run here: a type this version does
//! not know (kept in the config unchanged, e.g. from a newer version or a type that arrives in
//! a later milestone), or a type whose capabilities this system lacks. They draw a dimmed,
//! outlined label and explain themselves in their hint.

use super::{DrawCtx, HintContent, LayoutCtx, Sensor, SensorCtx};
use crate::i18n::{t, t_args};
use crate::platform::{Canvas, PointF, RectF, Shape, TextLayout, TextStyle};

/// Space between the outline and the label.
const PAD: f32 = 4.0;

#[derive(Debug, Clone, PartialEq)]
enum Missing {
    /// The type id is not registered.
    Unknown,
    /// The type needs a capability this system lacks.
    Unsupported,
}

pub struct Placeholder {
    /// The type's name, or its id when unknown.
    label: String,
    missing: Missing,
    layout: Option<TextLayout>,
}

impl Placeholder {
    /// For a type id that is not registered.
    pub fn unknown(kind: &str) -> Self {
        Self {
            label: kind.to_owned(),
            missing: Missing::Unknown,
            layout: None,
        }
    }

    /// For a registered type that cannot run on this system; `name` is its localised name.
    pub fn unsupported(name: String) -> Self {
        Self {
            label: name,
            missing: Missing::Unsupported,
            layout: None,
        }
    }

    fn message(&self) -> String {
        match self.missing {
            Missing::Unknown => t_args("sensor.placeholder.unknown", &[("type", &self.label)]),
            Missing::Unsupported => t("app.notAvailable"),
        }
    }
}

impl Sensor for Placeholder {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let style = TextStyle::new(&cx.theme.sensor_font, cx.theme.small_size);
        let layout = cx.text.layout(&self.label, &style);
        let width = (layout.width() + 2.0 * PAD).ceil();
        self.layout = Some(layout);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let r = cx.rect;
        let color = cx.theme.sensor_text;
        let frame = RectF::new(r.left + 0.5, r.top + 3.5, r.right - 0.5, r.bottom - 3.5);
        canvas.stroke(
            &Shape::RoundedRect {
                rect: frame,
                radius: 3.0,
            },
            &color.faded(0.3).into(),
            1.0,
        );
        if let Some(layout) = &self.layout {
            let y = canvas.snap(r.top + (r.height() - layout.height()) / 2.0);
            canvas.draw_text(layout, PointF::new(r.left + PAD, y), color.faded(0.55));
        }
    }

    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        Some(HintContent::Text(self.message()))
    }

    fn inactive(&self) -> bool {
        true
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.message()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::theme::Theme;
    use crate::config::model::Settings;
    use crate::platform::OsAppearance;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};

    #[test]
    fn draws_a_dimmed_outlined_label() {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        let mut sensor = Placeholder::unknown("cpu-graph");
        let width = sensor.measure(&LayoutCtx {
            theme: &theme,
            text: &FixedMetrics,
            scale: 1.0,
        });
        // 9 characters × 5 px + 2 × 4 px.
        assert_eq!(width, 53.0);
        let mut canvas = RecordingCanvas::new(1.0);
        sensor.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme,
                rect: RectF::new(7.0, 0.0, 60.0, 28.0),
            },
        );
        assert_eq!(
            canvas.dump(),
            "stroke rounded 7.5,3.5 52×21 r3 #FFFFFF/30% w1\n\
             text \"cpu-graph\" Segoe UI Variable Text 600 10px at 11,8 #FFFFFF/55%\n"
        );
        assert!(sensor.inactive());
    }

    #[test]
    fn explains_itself() {
        let unknown = Placeholder::unknown("rss");
        assert_eq!(
            unknown.message(),
            "This sensor (rss) is not available in this version of Infodock."
        );
        let unsupported = Placeholder::unsupported("Wi-Fi Signal".into());
        assert_eq!(unsupported.message(), "Not available on this system");
    }
}
