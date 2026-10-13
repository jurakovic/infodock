//! The dual graph of the CPU dual graph and the Network Graph (SPEC §8.3): the plot split into
//! two halves with a 1 px gap; the top series grows up from the middle, the bottom one down
//! from the middle (`flip`) or up from the bottom edge.

use super::graph::{self, Columns, Growth};
use super::history::History;
use crate::platform::{Canvas, RectF};

/// One series: its values, what counts as full (the owning sensor's scale) and its colour.
pub struct Series<'a> {
    pub history: &'a History,
    pub max: f32,
    pub columns: Columns,
}

/// The two halves of `plot` at `scale`: equally tall in whole physical px, with at least one
/// physical px between them (two when the plot's height is even).
pub fn halves(plot: RectF, scale: f32) -> (RectF, RectF) {
    let scale = scale.max(f32::EPSILON);
    let height = (plot.height() * scale).round();
    let half = ((height - 1.0) / 2.0).floor().max(0.0) / scale;
    (
        RectF::new(plot.left, plot.top, plot.right, plot.top + half),
        RectF::new(plot.left, plot.bottom - half, plot.right, plot.bottom),
    )
}

/// Draws both series in `plot` (over a backdrop the caller drew).
pub fn draw(canvas: &mut dyn Canvas, plot: RectF, top: &Series, bottom: &Series, flip: bool) {
    let (upper, lower) = halves(plot, canvas.scale());
    graph::draw_columns(canvas, upper, top.history, top.max, top.columns, Growth::Up);
    let growth = if flip { Growth::Down } else { Growth::Up };
    graph::draw_columns(
        canvas,
        lower,
        bottom.history,
        bottom.max,
        bottom.columns,
        growth,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Color;
    use crate::platform::recording::RecordingCanvas;

    fn history(value: f32) -> History {
        let mut h = History::new(10);
        h.push(value);
        h
    }

    #[test]
    fn halves_with_a_gap() {
        // A 28 px bar: the plot is 24 px, so two halves of 11 px around a 2 px gap.
        let plot = RectF::new(0.0, 2.0, 50.0, 26.0);
        let (upper, lower) = halves(plot, 1.0);
        assert_eq!(upper, RectF::new(0.0, 2.0, 50.0, 13.0));
        assert_eq!(lower, RectF::new(0.0, 15.0, 50.0, 26.0));
        // At 150 % the plot is 36 physical px: halves of 17 and a 2 px gap.
        let (upper, lower) = halves(plot, 1.5);
        assert_eq!(upper.height() * 1.5, 17.0);
        assert_eq!(lower.top, 26.0 - 17.0 / 1.5);
        // A classic 25 px bar: 21 px, halves of 10 around 1 px.
        let classic = RectF::new(0.0, 2.0, 50.0, 23.0);
        let (upper, lower) = halves(classic, 1.0);
        assert_eq!((upper.bottom, lower.top), (12.0, 13.0));
    }

    #[test]
    fn top_grows_up_and_bottom_down_unless_unflipped() {
        let plot = RectF::new(0.0, 0.0, 10.0, 21.0);
        let (receive, send) = (history(1.0), history(0.5));
        let blue = Columns::Solid(Color::rgba(0x20, 0xB7, 0xFF, 0.86));
        let orange = Columns::Solid(Color::rgba(0xFF, 0xAD, 0x5B, 0.86));
        let top = Series {
            history: &receive,
            max: 1.0,
            columns: blue,
        };
        let bottom = Series {
            history: &send,
            max: 1.0,
            columns: orange,
        };
        let mut canvas = RecordingCanvas::new(1.0);
        draw(&mut canvas, plot, &top, &bottom, true);
        assert_eq!(
            canvas.dump(),
            "fill rect 9,0 1×10 #20B7FF/86%\nfill rect 9,11 1×5 #FFAD5B/86%\n"
        );
        let mut canvas = RecordingCanvas::new(1.0);
        draw(&mut canvas, plot, &top, &bottom, false);
        assert!(
            canvas.dump().ends_with("fill rect 9,16 1×5 #FFAD5B/86%\n"),
            "{}",
            canvas.dump()
        );
    }
}
