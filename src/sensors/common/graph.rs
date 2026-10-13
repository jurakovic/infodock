//! Graphs (SPEC §8.3): a plot of one value per logical px column, newest at the right edge,
//! over an optional background and grid, with a two-line block to its right. The CPU, Memory
//! and Swap graphs draw one series; the dual graph (`dual_graph.rs`) two.

use serde::Deserialize;

use super::history::History;
use super::two_line::{self, Align, TwoLine};
use crate::bar::theme::Theme;
use crate::platform::{Canvas, Color, RectF, TextEngine};
use crate::sensors::color;
use crate::sensors::schema::{FieldKind, Lit, PropField};
use crate::sensors::{ColorChoice, ColorModeKind, DrawCtx};

/// The properties every graph has (SPEC §9.8), for the types' schemas.
pub const INTERVAL: PropField = PropField::new(
    "interval",
    FieldKind::Interval {
        min: 1.0,
        max: 10.0,
    },
    "sensor.field.interval",
    Lit::Int(1),
);
pub const WIDTH: PropField = PropField::new(
    "width",
    FieldKind::Slider {
        min: 50.0,
        max: 150.0,
        step: 5.0,
        unit: Some("common.px"),
    },
    "sensor.graph.field.width",
    Lit::Int(50),
);
pub const BK_COLOR: PropField = PropField::new(
    "bkColor",
    FieldKind::ColorMode { dynamic: false },
    "sensor.graph.field.bkColor",
    Lit::ColorMode("standard", "#000000"),
);
pub const BK_OPACITY: PropField = PropField::new(
    "bkOpacity",
    FieldKind::Opacity { min: 0 },
    "sensor.graph.field.bkOpacity",
    Lit::Int(0),
);
pub const GRID_COLOR: PropField = PropField::new(
    "gridColor",
    FieldKind::ColorMode { dynamic: false },
    "sensor.graph.field.gridColor",
    Lit::ColorMode("standard", "#A0FF9B"),
);
pub const GRID_OPACITY: PropField = PropField::new(
    "gridOpacity",
    FieldKind::Opacity { min: 0 },
    "sensor.graph.field.gridOpacity",
    Lit::Int(0),
);
pub const GRAPH_COLOR: PropField = PropField::new(
    "graphColor",
    FieldKind::ColorMode { dynamic: true },
    "sensor.graph.field.graphColor",
    Lit::ColorMode("custom", "#00FF00"),
);
pub const GRAPH_OPACITY: PropField = PropField::new(
    "graphOpacity",
    FieldKind::Opacity { min: 20 },
    "sensor.graph.field.graphOpacity",
    Lit::Int(55),
);

/// The standard colours of a graph's background and grid (SPEC §9.8).
const STANDARD_BACKGROUND: Color = Color::BLACK;
const STANDARD_GRID: Color = Color::rgba(0xA0, 0xFF, 0x9B, 1.0);

/// A graph's look, as stored (the fields above but the interval).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphStyle {
    pub width: u32,
    pub bk_color: ColorChoice,
    pub bk_opacity: u32,
    pub grid_color: ColorChoice,
    pub grid_opacity: u32,
    pub graph_color: ColorChoice,
    pub graph_opacity: u32,
}

fn opacity(percent: u32) -> f32 {
    percent.min(100) as f32 / 100.0
}

impl GraphStyle {
    /// The plot's width: 50–150, rounded down to a multiple of 5 (SPEC §9.8).
    pub fn plot_width(&self) -> f32 {
        (self.width.clamp(50, 150) / 5 * 5) as f32
    }

    pub fn backdrop(&self) -> Backdrop {
        Backdrop {
            background: color::resolve(&self.bk_color, STANDARD_BACKGROUND, 0.0)
                .with_alpha(opacity(self.bk_opacity)),
            grid: color::resolve(&self.grid_color, STANDARD_GRID, 0.0)
                .with_alpha(opacity(self.grid_opacity)),
        }
    }

    /// The columns' colour; Standard is the sensor text colour (SPEC §8.4).
    pub fn columns(&self, theme: &Theme) -> Columns {
        let alpha = opacity(self.graph_opacity);
        match self.graph_color.mode {
            ColorModeKind::Dynamic => Columns::Dynamic(alpha),
            _ => Columns::Solid(
                color::resolve(&self.graph_color, theme.sensor_text, 0.0).with_alpha(alpha),
            ),
        }
    }
}

/// Between the plot and the text, logical px.
pub const GAP: f32 = 5.0;
/// The grid's cells, logical px.
const GRID_CELL: f32 = 5.0;

/// How the columns are coloured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Columns {
    Solid(Color),
    /// Each column takes the dynamic gradient's colour of its own value (SPEC §8.4), at this
    /// opacity.
    Dynamic(f32),
}

/// The layers under the columns; transparent ones are not drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backdrop {
    pub background: Color,
    pub grid: Color,
}

/// Which way columns grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Growth {
    /// From the area's bottom edge up.
    Up,
    /// From its top edge down.
    Down,
}

/// The plot of a graph `width` logical px wide in the sensor's `rect`: 4 px shorter than the
/// bar, at y = 2.
pub fn plot_rect(rect: RectF, width: f32) -> RectF {
    RectF::new(
        rect.left,
        rect.top + 2.0,
        rect.left + width,
        rect.bottom - 2.0,
    )
}

/// One physical px, in logical px.
fn hairline(canvas: &dyn Canvas) -> f32 {
    canvas.snap(1.0).max(1.0 / canvas.scale())
}

/// The background, then a grid of 5 px cells: vertical lines counted from the left, horizontal
/// ones from the bottom, each one physical px.
pub fn draw_backdrop(canvas: &mut dyn Canvas, plot: RectF, backdrop: Backdrop) {
    if backdrop.background.a > 0.0 {
        canvas.fill_rect(plot, backdrop.background);
    }
    if backdrop.grid.a > 0.0 {
        let line = hairline(canvas);
        let mut x = plot.left + GRID_CELL;
        while x < plot.right {
            let left = canvas.snap(x);
            canvas.fill_rect(
                RectF::new(left, plot.top, left + line, plot.bottom),
                backdrop.grid,
            );
            x += GRID_CELL;
        }
        let mut y = plot.bottom - GRID_CELL;
        while y > plot.top {
            let top = canvas.snap(y);
            canvas.fill_rect(
                RectF::new(plot.left, top, plot.right, top + line),
                backdrop.grid,
            );
            y -= GRID_CELL;
        }
    }
}

/// `value / max`, clamped to 0..1; NaN and a non-positive `max` give 0.
pub fn fraction(value: f32, max: f32) -> f32 {
    if max > 0.0 && value.is_finite() {
        (value / max).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Draws `history` as columns in `area`, the newest at its right edge: each one logical px
/// wide (its edges on whole physical px) and `value / max` of the area tall.
pub fn draw_columns(
    canvas: &mut dyn Canvas,
    area: RectF,
    history: &History,
    max: f32,
    columns: Columns,
    growth: Growth,
) {
    for (i, value) in history.iter().rev().enumerate() {
        let right = area.right - i as f32;
        if right <= area.left {
            break;
        }
        let f = fraction(value, max);
        if f <= 0.0 {
            continue;
        }
        let height = f * area.height();
        let (top, bottom) = match growth {
            Growth::Up => (area.bottom - height, area.bottom),
            Growth::Down => (area.top, area.top + height),
        };
        let rect = RectF::new(
            canvas.snap((right - 1.0).max(area.left)),
            canvas.snap(top),
            canvas.snap(right),
            canvas.snap(bottom),
        );
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            continue;
        }
        let color = match columns {
            Columns::Solid(color) => color,
            Columns::Dynamic(opacity) => color::gradient(f).with_alpha(opacity),
        };
        canvas.fill_rect(rect, color);
    }
}

/// A graph sensor's text: a two-line block right-aligned in a column wide enough for
/// `reserve` (e.g. `100%`) or the text, whichever is wider.
pub struct GraphText {
    pub text: TwoLine,
    pub column: f32,
}

impl GraphText {
    pub fn new(
        engine: &dyn TextEngine,
        theme: &Theme,
        top: &str,
        bottom: &str,
        reserve: &str,
    ) -> Self {
        let text = TwoLine::new(engine, theme, top, bottom);
        let reserve = engine.layout(reserve, &two_line::style(theme)).width();
        let column = reserve.max(text.width());
        Self { text, column }
    }

    /// The sensor's width with a plot `plot_width` wide, whole logical px.
    pub fn sensor_width(&self, plot_width: f32) -> f32 {
        (plot_width + GAP + self.column).ceil()
    }

    /// Draws both lines right-aligned to the sensor's right edge.
    pub fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let r = cx.rect;
        let end = Align::Right(r.right);
        self.text.draw(
            canvas,
            cx.theme,
            (r.top, r.height()),
            (end, end),
            cx.theme.sensor_text,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sensors::test_support::theme;

    fn history(values: &[f32], capacity: usize) -> History {
        let mut h = History::new(capacity);
        values.iter().for_each(|v| h.push(*v));
        h
    }

    #[test]
    fn columns_newest_at_the_right() {
        let mut canvas = RecordingCanvas::new(1.0);
        let plot = plot_rect(RectF::new(10.0, 0.0, 60.0, 28.0), 50.0);
        assert_eq!(plot, RectF::new(10.0, 2.0, 60.0, 26.0));
        let green = Color::rgba(0, 255, 0, 0.55);
        let h = history(&[100.0, 0.0, 50.0], 50);
        draw_columns(
            &mut canvas,
            plot,
            &h,
            100.0,
            Columns::Solid(green),
            Growth::Up,
        );
        assert_eq!(
            canvas.dump(),
            "fill rect 59,14 1×12 #00FF00/55%\nfill rect 57,2 1×24 #00FF00/55%\n"
        );
    }

    #[test]
    fn dynamic_columns_and_growing_down() {
        let mut canvas = RecordingCanvas::new(1.0);
        let area = RectF::new(0.0, 0.0, 10.0, 10.0);
        let h = history(&[1.0, 0.5], 10);
        draw_columns(
            &mut canvas,
            area,
            &h,
            1.0,
            Columns::Dynamic(0.86),
            Growth::Down,
        );
        assert_eq!(
            canvas.dump(),
            "fill rect 9,0 1×5 #FFFF00/86%\nfill rect 8,0 1×10 #FF0000/86%\n"
        );
    }

    #[test]
    fn only_as_many_columns_as_the_plot_is_wide() {
        let mut canvas = RecordingCanvas::new(1.0);
        let h = history(&[1.0; 20], 20);
        let area = RectF::new(0.0, 0.0, 5.0, 10.0);
        draw_columns(
            &mut canvas,
            area,
            &h,
            1.0,
            Columns::Solid(Color::WHITE),
            Growth::Up,
        );
        assert_eq!(canvas.dump().lines().count(), 5);
    }

    #[test]
    fn columns_on_whole_physical_px_at_150_percent() {
        let mut canvas = RecordingCanvas::new(1.5);
        let area = RectF::new(7.0, 2.0, 57.0, 26.0);
        let h = history(&[1.0, 1.0], 50);
        draw_columns(
            &mut canvas,
            area,
            &h,
            1.0,
            Columns::Solid(Color::WHITE),
            Growth::Up,
        );
        // 56..57 → 84..85.5 → 84..86 physical px; 55..56 → 82.5..84 → 83..84.
        assert_eq!(
            canvas.dump(),
            "fill rect 56,2 1.33×24 #FFFFFF\nfill rect 55.33,2 0.67×24 #FFFFFF\n"
        );
    }

    #[test]
    fn backdrop_and_grid() {
        let mut canvas = RecordingCanvas::new(1.0);
        let plot = RectF::new(0.0, 2.0, 12.0, 14.0);
        let grid = Color::rgba(0xA0, 0xFF, 0x9B, 0.5);
        let backdrop = Backdrop {
            background: Color::BLACK,
            grid,
        };
        draw_backdrop(&mut canvas, plot, backdrop);
        assert_eq!(
            canvas.dump(),
            "fill rect 0,2 12×12 #000000\n\
             fill rect 5,2 1×12 #A0FF9B/50%\n\
             fill rect 10,2 1×12 #A0FF9B/50%\n\
             fill rect 0,9 12×1 #A0FF9B/50%\n\
             fill rect 0,4 12×1 #A0FF9B/50%\n"
        );
        // Transparent layers draw nothing (the defaults: SPEC §9.8).
        let mut canvas = RecordingCanvas::new(1.0);
        let none = Backdrop {
            background: Color::BLACK.with_alpha(0.0),
            grid: grid.with_alpha(0.0),
        };
        draw_backdrop(&mut canvas, plot, none);
        assert_eq!(canvas.dump(), "");
    }

    #[test]
    fn text_column_fits_the_reserve() {
        let theme = theme();
        let text = GraphText::new(&FixedMetrics, &theme, "CPU", "7%", "100%");
        assert_eq!(text.column, 20.0);
        assert_eq!(text.sensor_width(50.0), 75.0);
        let mut canvas = RecordingCanvas::new(1.0);
        let rect = RectF::new(0.0, 0.0, 75.0, 28.0);
        text.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme,
                rect,
            },
        );
        let dump = canvas.dump();
        assert!(
            dump.contains("\"CPU\"") && dump.contains(" at 60,3 "),
            "{dump}"
        );
        assert!(
            dump.contains("\"7%\"") && dump.contains(" at 65,14 "),
            "{dump}"
        );
    }
}
