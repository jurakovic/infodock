//! The meter of the CPU, Memory, Swap and Disk meters (SPEC §8.3): a thin vertical bar filled
//! from the bottom, with a two-line block beside it.
//!
//! ```text
//! [▮] LABEL
//!     value
//! ```

use super::two_line::{self, Align, TwoLine};
use crate::look::theme::{Inactive, Theme};
use crate::platform::{Canvas, Color, RectF, TextEngine};
use crate::sensors::DrawCtx;
use crate::sensors::color;
use crate::sensors::format;

/// The bar's width, logical px; it is 6 px shorter than the bar and vertically centred.
const BAR_WIDTH: f32 = 4.0;
/// Where the text starts.
const TEXT_X: f32 = 8.0;
/// The empty bar: the sensor text colour at 31 %.
const BACKGROUND_OPACITY: f32 = 0.31;
/// The fill in Standard mode: the sensor text colour at 59 %.
const STANDARD_FILL_OPACITY: f32 = 0.59;

pub struct Meter {
    text: TwoLine,
    /// Where the value line ends (right-aligned), from the sensor's left edge; `None` puts it
    /// left-aligned under the label.
    value_end: Option<f32>,
}

impl Meter {
    /// A meter showing `label` and `value`. A right-aligned value sits in a column that fits
    /// `100%`, so the bar does not shift as the value changes; the Disk Meter's byte modes
    /// put it under the label instead (SPEC §9.12).
    pub fn new(
        engine: &dyn TextEngine,
        theme: &Theme,
        label: &str,
        value: &str,
        right_aligned: bool,
    ) -> Self {
        let text = TwoLine::new(engine, theme, label, value);
        let value_end = right_aligned.then(|| {
            let reserve = engine
                .layout(&format::percent(100.0), &two_line::style(theme))
                .width();
            TEXT_X + reserve.max(text.bottom.width())
        });
        Self { text, value_end }
    }

    /// Logical px, whole.
    pub fn width(&self) -> f32 {
        let label = TEXT_X + self.text.top.width();
        let value = self.value_end.unwrap_or(TEXT_X + self.text.bottom.width());
        label.max(value).ceil()
    }

    /// The fill colour of Standard mode (SPEC §8.3).
    pub fn standard_fill(theme: &Theme) -> Color {
        let text = theme.sensor_text;
        text.with_alpha(text.a * STANDARD_FILL_OPACITY)
    }

    /// Draws the bar filled to `fraction` (0..1) with `fill`, and the text. An `inactive`
    /// meter dims its bar, not its text (SPEC §8.5).
    pub fn draw(
        &self,
        canvas: &mut dyn Canvas,
        cx: &DrawCtx,
        fraction: f32,
        fill: Color,
        inactive: Option<Inactive>,
    ) {
        let r = cx.rect;
        let text = cx.theme.sensor_text;
        let dim = |c: Color| inactive.map_or(c, |how| color::inactive(c, how));
        let height = (r.height() - 6.0).max(1.0);
        let top = canvas.snap(r.top + (r.height() - height) / 2.0);
        let bottom = canvas.snap(top + height);
        let (left, right) = (canvas.snap(r.left), canvas.snap(r.left + BAR_WIDTH));
        let background = text.with_alpha(text.a * BACKGROUND_OPACITY);
        canvas.fill_rect(RectF::new(left, top, right, bottom), dim(background));
        let fraction = if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        };
        let fill_top = canvas.snap(bottom - fraction * (bottom - top));
        if fill_top < bottom {
            canvas.fill_rect(RectF::new(left, fill_top, right, bottom), dim(fill));
        }
        let value = match self.value_end {
            Some(end) => Align::Right(r.left + end),
            None => Align::Left(r.left + TEXT_X),
        };
        self.text.draw(
            canvas,
            cx.theme,
            (r.top, r.height()),
            (Align::Left(r.left + TEXT_X), value),
            text,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sensors::test_support::theme;

    fn drawn(meter: &Meter, scale: f32, fraction: f32, inactive: Option<Inactive>) -> String {
        let theme = theme();
        let mut canvas = RecordingCanvas::new(scale);
        let rect = RectF::new(7.0, 0.0, 7.0 + meter.width(), 28.0);
        let fill = Meter::standard_fill(&theme);
        meter.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme,
                rect,
            },
            fraction,
            fill,
            inactive,
        );
        canvas.dump()
    }

    #[test]
    fn bar_fill_and_text() {
        let theme = theme();
        let meter = Meter::new(&FixedMetrics, &theme, "CPU", "37%", true);
        // "100%" is 20 px at 10 px (fixed metrics), so the value ends at 28.
        assert_eq!(meter.width(), 28.0);
        let dump = drawn(&meter, 1.0, 0.5, None);
        let lines: Vec<&str> = dump.lines().collect();
        // 22 px tall, centred in 28: 3..25; half full: 14..25.
        assert_eq!(lines[0], "fill rect 7,3 4×22 #FFFFFF/31%");
        assert_eq!(lines[1], "fill rect 7,14 4×11 #FFFFFF/59%");
        assert!(
            lines[2].contains("\"CPU\"") && lines[2].contains(" at 15,3 "),
            "{dump}"
        );
        // Right-aligned: 7 + 28 − 15.
        assert!(
            lines[3].contains("\"37%\"") && lines[3].contains(" at 20,14 "),
            "{dump}"
        );
    }

    #[test]
    fn empty_full_and_inactive() {
        let theme = theme();
        let meter = Meter::new(&FixedMetrics, &theme, "HD", "241 GB", false);
        // Left-aligned value: 8 + 6 × 5 px.
        assert_eq!(meter.width(), 38.0);
        let empty = drawn(&meter, 1.0, 0.0, None);
        assert_eq!(empty.lines().filter(|l| l.starts_with("fill")).count(), 1);
        assert!(
            empty
                .lines()
                .nth(2)
                .is_some_and(|l| l.contains(" at 15,14 "))
        );
        let grey = Color::rgba(0x55, 0x55, 0x55, 1.0);
        let full = drawn(&meter, 1.0, 7.0, Some(Inactive::Colorize(grey)));
        let lines: Vec<&str> = full.lines().collect();
        assert_eq!(lines[0], "fill rect 7,3 4×22 #555555/31%");
        assert_eq!(
            lines[1], "fill rect 7,3 4×22 #555555/59%",
            "clamped, inactive"
        );
        assert!(
            lines[2].ends_with("#FFFFFF"),
            "the text keeps its colour: {full}"
        );
    }

    #[test]
    fn whole_physical_px_at_150_percent() {
        let theme = theme();
        let meter = Meter::new(&FixedMetrics, &theme, "CPU", "37%", true);
        let dump = drawn(&meter, 1.5, 0.37, None);
        // 7 → 10.5 → 11 physical px, 3 → 4.5 → 5: every edge on a whole physical px.
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines[0], "fill rect 7.33,3.33 4×22 #FFFFFF/31%");
        assert_eq!(lines[1], "fill rect 7.33,17.33 4×8 #FFFFFF/59%");
    }
}
