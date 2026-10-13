//! The two-line block (SPEC §8.3): a small label line above a value line, both in the sensor
//! font at its small size (modern 10 px, classic 9 px), with tabular digits so changing values
//! do not shift the bar.

use crate::bar::theme::Theme;
use crate::platform::{Canvas, Color, PointF, TextEngine, TextLayout, TextStyle};

/// Where a line goes: starting at an x, or ending at one (right-aligned), logical px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Left(f32),
    Right(f32),
}

/// Two laid-out lines.
pub struct TwoLine {
    pub top: TextLayout,
    pub bottom: TextLayout,
}

/// The text style of both lines.
pub fn style(theme: &Theme) -> TextStyle {
    TextStyle::new(&theme.sensor_font, theme.small_size).tabular()
}

/// The baselines of the two lines in a bar `bar_height` tall: two rows of font size + 1 px,
/// centred as a block, each baseline 1.5 px above its row's bottom. In a 25 px classic bar
/// (9 px font) that is y = 11 and y = 21, as in the spec.
pub fn baselines(theme: &Theme, bar_height: f32) -> [f32; 2] {
    baselines_for(theme.small_size, bar_height)
}

/// [`baselines`] for two lines of `size` px, e.g. Date's 11 px month and year (SPEC §9.4).
pub fn baselines_for(size: f32, bar_height: f32) -> [f32; 2] {
    let row = size + 1.0;
    let top = (bar_height - 2.0 * row) / 2.0;
    [top + row - 1.5, top + 2.0 * row - 1.5]
}

impl TwoLine {
    pub fn new(text: &dyn TextEngine, theme: &Theme, top: &str, bottom: &str) -> Self {
        let style = style(theme);
        Self {
            top: text.layout(top, &style),
            bottom: text.layout(bottom, &style),
        }
    }

    /// The wider line's width.
    pub fn width(&self) -> f32 {
        self.top.width().max(self.bottom.width())
    }

    /// Draws both lines in `color`; `bar_top` and `bar_height` are the sensor rect's.
    pub fn draw(
        &self,
        canvas: &mut dyn Canvas,
        theme: &Theme,
        (bar_top, bar_height): (f32, f32),
        (top, bottom): (Align, Align),
        color: Color,
    ) {
        let [first, second] = baselines(theme, bar_height);
        for (layout, align, baseline) in [(&self.top, top, first), (&self.bottom, bottom, second)] {
            let x = match align {
                Align::Left(x) => x,
                Align::Right(x) => x - layout.width(),
            };
            let y = canvas.snap(bar_top + baseline - layout.baseline());
            canvas.draw_text(layout, PointF::new(canvas.snap(x), y), color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{Settings, Theme as ThemeId};
    use crate::platform::OsAppearance;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};

    #[test]
    fn classic_baselines_match_the_spec() {
        let settings = Settings {
            theme: ThemeId::Classic,
            ..Settings::default()
        };
        let classic = Theme::resolve(&settings, OsAppearance::default());
        assert_eq!(baselines(&classic, 25.0), [11.0, 21.0]);
    }

    #[test]
    fn draws_a_label_and_a_right_aligned_value() {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        // Modern: 10 px, rows of 11 px centred in 28 px, so baselines at 12.5 and 23.5.
        assert_eq!(baselines(&theme, 28.0), [12.5, 23.5]);
        let block = TwoLine::new(&FixedMetrics, &theme, "CPU", "37%");
        // Fixed metrics: 5 px per character at 10 px.
        assert_eq!(block.width(), 15.0);
        let mut canvas = RecordingCanvas::new(1.0);
        let white = Color::WHITE;
        block.draw(
            &mut canvas,
            &theme,
            (0.0, 28.0),
            (Align::Left(8.0), Align::Right(32.0)),
            white,
        );
        let dump = canvas.dump();
        let lines: Vec<&str> = dump.lines().collect();
        // Baselines minus the fixed metrics' baseline (the font size), snapped.
        assert!(lines[0].starts_with("text \"CPU\""), "{dump}");
        assert!(lines[0].contains(" 10px at 8,3 "), "{dump}");
        assert!(lines[1].contains(" 10px at 17,14 "), "{dump}");
    }
}
