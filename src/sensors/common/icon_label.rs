//! Icon + two-line (SPEC §8.3): Battery, Wi-Fi and Recycle Bin. An icon at the sensor's left,
//! centred vertically, then a two-line block after a small gap.

use super::icon::BarIcon;
use super::two_line::{Align, TwoLine};
use crate::platform::{Canvas, Color, RectF};
use crate::sensors::DrawCtx;

/// Between the icon and the text, logical px.
pub const GAP: f32 = 3.0;

/// An icon and two lines, laid out.
pub struct IconLabel {
    /// Its place stays while there is nothing to draw.
    icon: BarIcon,
    text: TwoLine,
}

impl IconLabel {
    /// `icon` as measured.
    pub fn new(icon: BarIcon, text: TwoLine) -> Self {
        Self { icon, text }
    }

    /// The icon as drawn, for tests.
    #[cfg(test)]
    pub fn icon(&self) -> Option<&super::icon::Icon> {
        self.icon.icon()
    }

    /// Where the text starts, from the sensor's left edge.
    fn text_x(&self) -> f32 {
        self.icon.width() + GAP
    }

    /// The whole width, rounded up to whole logical px.
    pub fn width(&self) -> f32 {
        (self.text_x() + self.text.width()).ceil()
    }

    /// Draws the icon and both lines, left-aligned after it, in `color`.
    pub fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx, color: Color) {
        let rect = cx.rect;
        let icon_rect = RectF::new(
            rect.left,
            rect.top,
            rect.left + self.icon.width(),
            rect.bottom,
        );
        self.icon.draw(
            canvas,
            &DrawCtx {
                theme: cx.theme,
                rect: icon_rect,
            },
        );
        let x = cx.rect.left + self.text_x();
        self.text.draw(
            canvas,
            cx.theme,
            (cx.rect.top, cx.rect.height()),
            (Align::Left(x), Align::Left(x)),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Bitmap;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sensors::common::icon::Icon;
    use crate::sensors::test_support::theme;

    #[test]
    fn icon_then_two_lines() {
        let theme = theme();
        let bitmap = Bitmap::from_rgba(24, 24, vec![255; 24 * 24 * 4]).expect("a bitmap");
        let block = IconLabel::new(
            BarIcon::laid_out(Some(Icon::new(bitmap, None, 1.0, None)), 24.0),
            TwoLine::new(&FixedMetrics, &theme, "Recycle Bin", "3 items"),
        );
        // 24 + 3 + 11 characters × 5 px.
        assert_eq!(block.width(), 82.0);
        let mut canvas = RecordingCanvas::new(1.0);
        block.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme,
                rect: RectF::new(7.0, 0.0, 89.0, 28.0),
            },
            Color::WHITE,
        );
        let dump = canvas.dump();
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines[0], "bitmap 24×24 into 7,2 24×24 opacity 1", "{dump}");
        assert!(
            lines[1].contains("\"Recycle Bin\"") && lines[1].contains(" at 34,3 "),
            "{dump}"
        );
        assert!(
            lines[2].contains("\"3 items\"") && lines[2].contains(" at 34,14 "),
            "{dump}"
        );
    }

    #[test]
    fn without_an_icon_its_place_stays() {
        let theme = theme();
        let block = IconLabel::new(
            BarIcon::laid_out(None, 19.0),
            TwoLine::new(&FixedMetrics, &theme, "A", "B"),
        );
        assert_eq!(block.width(), 27.0);
    }
}
