//! The hover hint's content (SPEC §7.1): a rounded bubble with a pointer triangle towards the
//! sensor and an optional soft shadow, around multi-line text whose tab-separated cells line
//! up in columns ("label:\tvalue"). The hint window (platform) shows what this draws; timing
//! lives in `thread.rs`, placement in `placement.rs`.

use std::time::Duration;

use super::placement::{Anchor, Placement, place};
use super::theme::Theme;
use crate::config::model::Edge;
use crate::platform::{
    Brush, Canvas, Color, PointF, RectF, Shape, SizeF, TextEngine, TextLayout, TextStyle,
};

/// Space between the bubble's edge and the text.
const PADDING: f32 = 6.0;
/// The smallest bubble.
const MIN_SIZE: SizeF = SizeF::new(30.0, 20.0);
/// The pointer triangle's width and height.
const POINTER: SizeF = SizeF::new(13.0, 7.0);
/// Space between tab-separated columns.
const COLUMN_GAP: f32 = 12.0;
/// The shadow's reach around the bubble; also the window's margin when there is a shadow.
const SHADOW: f32 = 6.0;

/// Text laid out in rows of cells.
pub struct HintText {
    rows: Vec<Vec<TextLayout>>,
    /// Where each column starts, from the text's left edge.
    columns: Vec<f32>,
    line_height: f32,
    size: SizeF,
}

impl HintText {
    pub fn new(text: &str, engine: &dyn TextEngine, theme: &Theme) -> Self {
        let style = TextStyle::new(&theme.popup_font, theme.popup_font_size);
        let rows: Vec<Vec<TextLayout>> = text
            .split('\n')
            .map(|line| {
                line.split('\t')
                    .map(|cell| engine.layout(cell, &style))
                    .collect()
            })
            .collect();
        let line_height = engine.layout("Ag", &style).height();
        // Each column is as wide as its widest cell, except that the last cell of a row may
        // run on.
        let count = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0.0_f32; count];
        for row in &rows {
            for (i, cell) in row.iter().enumerate().take(row.len().saturating_sub(1)) {
                widths[i] = widths[i].max(cell.width());
            }
        }
        let mut columns = Vec::with_capacity(count);
        let mut x = 0.0;
        for width in &widths {
            columns.push(x);
            x += width + COLUMN_GAP;
        }
        let width = rows
            .iter()
            .map(|row| {
                row.last()
                    .map_or(0.0, |cell| columns[row.len() - 1] + cell.width())
            })
            .fold(0.0, f32::max);
        let size = SizeF::new(width.ceil(), rows.len() as f32 * line_height);
        Self {
            rows,
            columns,
            line_height,
            size,
        }
    }

    fn draw(&self, canvas: &mut dyn Canvas, origin: PointF, color: Color) {
        for (r, row) in self.rows.iter().enumerate() {
            let y = canvas.snap(origin.y + r as f32 * self.line_height);
            for (cell, x) in row.iter().zip(&self.columns) {
                canvas.draw_text(cell, PointF::new(origin.x + x, y), color);
            }
        }
    }
}

/// The hint window's geometry for one hint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HintFrame {
    /// The window, in logical screen px.
    pub window: RectF,
    /// The bubble, from the window's top-left corner.
    pub bubble: RectF,
    /// The pointer triangle's left edge, from the bubble's left edge.
    pub pointer_x: f32,
    /// The pointer points down (the bar is at the bottom).
    pub down: bool,
}

impl HintFrame {
    /// Places a hint for `text` at `anchor` (SPEC §7.1, §7.3).
    pub fn new(text: &HintText, anchor: &Anchor, theme: &Theme) -> Self {
        let bubble = SizeF::new(
            (text.size.width + 2.0 * PADDING).max(MIN_SIZE.width),
            (text.size.height + 2.0 * PADDING).max(MIN_SIZE.height),
        );
        let content = SizeF::new(bubble.width, bubble.height + POINTER.height);
        let Placement { x, y, pointer_x } =
            place(anchor, content, POINTER.width, theme.popup_radius);
        let margin = if theme.popup_shadow { SHADOW } else { 0.0 };
        let down = anchor.edge == Edge::Bottom;
        let top = margin + if down { 0.0 } else { POINTER.height };
        Self {
            window: RectF::new(
                x - margin,
                y - margin,
                x + content.width + margin,
                y + content.height + margin,
            ),
            bubble: RectF::new(margin, top, margin + bubble.width, top + bubble.height),
            pointer_x,
            down,
        }
    }
}

/// The bubble with its pointer as one polygon-free pair of shapes, in `color`.
fn fill_bubble(canvas: &mut dyn Canvas, frame: &HintFrame, radius: f32, grow: f32, color: Color) {
    let b = frame.bubble.inset(-grow);
    canvas.fill(
        &Shape::RoundedRect {
            rect: b,
            radius: radius + grow,
        },
        &Brush::Solid(color),
    );
    let left = frame.bubble.left + frame.pointer_x - grow;
    let right = left + POINTER.width + 2.0 * grow;
    let tip_x = (left + right) / 2.0;
    let points = if frame.down {
        let base = frame.bubble.bottom;
        vec![
            PointF::new(left, base - 1.0),
            PointF::new(right, base - 1.0),
            PointF::new(tip_x, base + POINTER.height + grow),
        ]
    } else {
        let base = frame.bubble.top;
        vec![
            PointF::new(left, base + 1.0),
            PointF::new(right, base + 1.0),
            PointF::new(tip_x, base - POINTER.height - grow),
        ]
    };
    canvas.fill(&Shape::Polygon(points), &Brush::Solid(color));
}

/// Draws the hint into a cleared canvas the size of the frame's window.
pub fn paint(canvas: &mut dyn Canvas, frame: &HintFrame, text: &HintText, theme: &Theme) {
    let radius = theme.popup_radius;
    if theme.popup_shadow {
        // A soft shadow: rings fading outwards (SPEC §7.1).
        let steps = SHADOW as usize;
        for step in (1..=steps).rev() {
            let alpha = 0.035 * (steps + 1 - step) as f32 / steps as f32;
            fill_bubble(
                canvas,
                frame,
                radius,
                step as f32,
                Color::BLACK.with_alpha(alpha),
            );
        }
    }
    fill_bubble(canvas, frame, radius, 0.0, theme.popup_background);
    let origin = PointF::new(frame.bubble.left + PADDING, frame.bubble.top + PADDING);
    let free = frame.bubble.height() - 2.0 * PADDING - text.size.height;
    text.draw(
        canvas,
        PointF::new(origin.x, origin.y + (free / 2.0).max(0.0)),
        theme.popup_text,
    );
}

/// What the bar does about the hint after an event (see [`Hover`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Idle,
    /// Show this sensor's hint now, or swap the shown hint for it.
    Show(String),
    /// Hide the hint now and stop the hint timer.
    Hide,
    /// Run the hint timer for this long; [`Hover::timer`] follows.
    Wait(Duration),
    StopWaiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Show,
    Hide,
}

/// When the hint shows and hides (SPEC §7.1): after the show delay over a sensor, until the
/// hide delay after the pointer left it; moving to another sensor while a hint is shown swaps
/// it at once. A click, a press, a menu or a drag dismisses the hint until the pointer moves to
/// another sensor.
#[derive(Debug, Default)]
pub struct Hover {
    /// The sensor under the pointer that may show a hint.
    hovered: Option<String>,
    /// The sensor whose hint is shown.
    shown: Option<String>,
    pending: Option<Pending>,
    /// No hint for this sensor until the pointer leaves it.
    dismissed: Option<String>,
}

impl Hover {
    /// The sensor whose hint is shown.
    pub fn shown(&self) -> Option<&str> {
        self.shown.as_deref()
    }

    /// The pointer is over sensor `id`, or over none (or hints are off).
    pub fn over(&mut self, id: Option<&str>, show_delay: Duration, hide_delay: Duration) -> Step {
        if self.dismissed.as_deref().is_some_and(|d| Some(d) != id) {
            self.dismissed = None;
        }
        let id = id.filter(|id| self.dismissed.as_deref() != Some(*id));
        if self.hovered.as_deref() == id {
            return Step::Idle;
        }
        self.hovered = id.map(str::to_owned);
        match (&self.shown, &self.hovered) {
            (Some(shown), Some(hovered)) if shown == hovered => {
                // Back over the shown sensor within the hide delay.
                self.pending = None;
                Step::StopWaiting
            }
            (Some(_), Some(hovered)) => {
                self.pending = None;
                self.shown = Some(hovered.clone());
                Step::Show(hovered.clone())
            }
            (Some(_), None) => {
                self.pending = Some(Pending::Hide);
                Step::Wait(hide_delay)
            }
            (None, Some(_)) => {
                self.pending = Some(Pending::Show);
                Step::Wait(show_delay)
            }
            (None, None) => {
                self.pending = None;
                Step::StopWaiting
            }
        }
    }

    /// The pointer left the bar.
    pub fn leave(&mut self, hide_delay: Duration) -> Step {
        self.dismissed = None;
        self.over(None, Duration::ZERO, hide_delay)
    }

    /// The hint timer is due.
    pub fn timer(&mut self) -> Step {
        match self.pending.take() {
            Some(Pending::Show) => match &self.hovered {
                Some(hovered) => {
                    self.shown = Some(hovered.clone());
                    Step::Show(hovered.clone())
                }
                None => Step::Idle,
            },
            Some(Pending::Hide) => {
                self.shown = None;
                Step::Hide
            }
            None => Step::Idle,
        }
    }

    /// A click, a press, a menu or a drag.
    pub fn dismiss(&mut self) -> Step {
        self.dismissed = self.hovered.take().or_else(|| self.shown.clone());
        self.pending = None;
        match self.shown.take() {
            Some(_) => Step::Hide,
            None => Step::StopWaiting,
        }
    }

    /// The hint that was to be shown has no content: nothing is shown.
    pub fn nothing_shown(&mut self) {
        self.shown = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::Settings;
    use crate::platform::OsAppearance;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};

    fn theme(shadow: bool) -> Theme {
        let mut s = Settings::default();
        s.popups.shadow = shadow;
        Theme::resolve(&s, OsAppearance::default())
    }

    fn anchor(edge: Edge) -> Anchor {
        let bar = match edge {
            Edge::Bottom => RectF::new(0.0, 1052.0, 1920.0, 1080.0),
            Edge::Top => RectF::new(0.0, 0.0, 1920.0, 28.0),
        };
        Anchor {
            rect: RectF::new(900.0, bar.top, 960.0, bar.bottom),
            bar,
            monitor: RectF::new(0.0, 0.0, 1920.0, 1080.0),
            edge,
        }
    }

    #[test]
    fn tab_separated_cells_line_up() {
        // 12 px popup text: 6 px per character, 15 px lines.
        let text = HintText::new(
            "CPU:\t37%\nKernel time:\t5%\nA longer line",
            &FixedMetrics,
            &theme(true),
        );
        // "Kernel time:" is 12 characters: the values start at 72 + 12.
        assert_eq!(text.columns, [0.0, 84.0]);
        assert_eq!(text.size, SizeF::new(102.0, 45.0));
    }

    #[test]
    fn frame_above_a_bottom_bar_with_a_shadow_margin() {
        let theme = theme(true);
        let text = HintText::new("Infodock 1.0", &FixedMetrics, &theme);
        let frame = HintFrame::new(&text, &anchor(Edge::Bottom), &theme);
        // Bubble 72 + 12 wide, 15 + 12 tall, pointer 7 below it; 6 px of shadow around.
        assert_eq!(frame.bubble, RectF::new(6.0, 6.0, 90.0, 33.0));
        assert_eq!((frame.window.width(), frame.window.height()), (96.0, 46.0));
        // The content ends at the bar: 1052 − 34 = 1018, minus the margin.
        assert_eq!(frame.window.top, 1012.0);
        assert!(frame.down);
    }

    #[test]
    fn frame_below_a_top_bar_without_shadow() {
        let theme = theme(false);
        let text = HintText::new("x", &FixedMetrics, &theme);
        let frame = HintFrame::new(&text, &anchor(Edge::Top), &theme);
        // The minimum bubble, with the pointer above it.
        assert_eq!(frame.bubble, RectF::new(0.0, 7.0, 30.0, 34.0));
        assert_eq!(frame.window.top, 28.0);
        assert!(!frame.down);
    }

    #[test]
    fn paints_bubble_pointer_and_text() {
        let theme = theme(false);
        let text = HintText::new("Hi", &FixedMetrics, &theme);
        let frame = HintFrame::new(&text, &anchor(Edge::Bottom), &theme);
        let mut canvas = RecordingCanvas::new(1.0);
        paint(&mut canvas, &frame, &text, &theme);
        let dump = canvas.dump();
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("fill rounded 0,0 30×27 r8 #202024/92%"));
        assert!(lines[1].starts_with("fill polygon"));
        assert!(lines[2].starts_with("text \"Hi\" Segoe UI Variable Text 400 12px at 6,6"));
    }

    const SHOW: Duration = Duration::from_millis(600);
    const HIDE: Duration = Duration::from_millis(100);

    fn over(h: &mut Hover, id: Option<&str>) -> Step {
        h.over(id, SHOW, HIDE)
    }

    #[test]
    fn shows_after_the_delay_and_hides_after_leaving() {
        let mut h = Hover::default();
        assert_eq!(over(&mut h, Some("a")), Step::Wait(SHOW));
        assert_eq!(over(&mut h, Some("a")), Step::Idle);
        assert_eq!(h.timer(), Step::Show("a".into()));
        assert_eq!(h.shown(), Some("a"));
        assert_eq!(over(&mut h, None), Step::Wait(HIDE));
        assert_eq!(h.timer(), Step::Hide);
        assert_eq!(h.shown(), None);
    }

    #[test]
    fn leaving_before_the_delay_shows_nothing() {
        let mut h = Hover::default();
        over(&mut h, Some("a"));
        assert_eq!(h.leave(HIDE), Step::StopWaiting);
        assert_eq!(h.timer(), Step::Idle);
    }

    #[test]
    fn swaps_at_once_between_sensors() {
        let mut h = Hover::default();
        over(&mut h, Some("a"));
        h.timer();
        // Through a gap between sensors, within the hide delay.
        assert_eq!(over(&mut h, None), Step::Wait(HIDE));
        assert_eq!(over(&mut h, Some("b")), Step::Show("b".into()));
        assert_eq!(h.timer(), Step::Idle, "the hide was cancelled");
        // Back to b's hint after a short excursion: it stays.
        over(&mut h, None);
        assert_eq!(over(&mut h, Some("b")), Step::StopWaiting);
        assert_eq!(h.shown(), Some("b"));
    }

    #[test]
    fn a_click_dismisses_until_another_sensor() {
        let mut h = Hover::default();
        over(&mut h, Some("a"));
        h.timer();
        assert_eq!(h.dismiss(), Step::Hide);
        assert_eq!(
            over(&mut h, Some("a")),
            Step::Idle,
            "still over a: no new hint"
        );
        assert_eq!(over(&mut h, Some("b")), Step::Wait(SHOW));
        assert_eq!(
            over(&mut h, Some("a")),
            Step::Wait(SHOW),
            "a may show again"
        );
    }

    #[test]
    fn a_sensor_without_hint_shows_nothing() {
        let mut h = Hover::default();
        over(&mut h, Some("a"));
        assert_eq!(h.timer(), Step::Show("a".into()));
        h.nothing_shown();
        assert_eq!(h.shown(), None);
        assert_eq!(over(&mut h, Some("b")), Step::Wait(SHOW));
    }
}
