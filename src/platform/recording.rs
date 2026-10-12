//! A canvas that records draw calls, and a text engine with fixed metrics (SPEC §4.6, §16).
//! Drawing code is tested against them on every OS; the stub platform uses them as its
//! renderer.
//!
//! Fixed metrics: every character advances half the font size, a line is 1.25 × the size tall
//! and the baseline sits one font size below the top of its line.

use std::fmt::{self, Write as _};
use std::rc::Rc;

use super::bitmap::Bitmap;
use super::canvas::{Brush, Canvas, Color, PointF, RectF, Shape, SizeF};
use super::text::{TextEngine, TextLayout, TextStyle};

const ADVANCE: f32 = 0.5;
const LINE_HEIGHT: f32 = 1.25;
const ELLIPSIS: char = '…';

/// What a recorded layout shows.
struct RecordedText {
    text: String,
    style: TextStyle,
}

/// Lays text out with the fixed metrics described in the module docs.
#[derive(Debug, Default)]
pub struct FixedMetrics;

impl TextEngine for FixedMetrics {
    fn layout(&self, text: &str, style: &TextStyle) -> TextLayout {
        let advance = style.size * ADVANCE;
        let lines: Vec<String> = text
            .split('\n')
            .map(|line| fit_line(line, advance, style.max_width))
            .collect();
        let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let size = SizeF::new(
            widest as f32 * advance,
            lines.len() as f32 * style.size * LINE_HEIGHT,
        );
        let recorded = RecordedText {
            text: lines.join("\n"),
            style: style.clone(),
        };
        TextLayout::new(size, style.size, Rc::new(recorded))
    }
}

/// `line`, ending in an ellipsis if it is wider than `max_width`.
fn fit_line(line: &str, advance: f32, max_width: Option<f32>) -> String {
    let count = line.chars().count();
    match max_width {
        Some(max) if count as f32 * advance > max && advance > 0.0 => {
            let keep = ((max / advance).floor() as usize).saturating_sub(1);
            line.chars().take(keep).chain([ELLIPSIS]).collect()
        }
        _ => line.to_owned(),
    }
}

/// One recorded draw call; `Display` gives a compact line for snapshot tests.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawOp {
    Fill(Shape, Brush),
    Stroke(Shape, Brush, f32),
    Line(PointF, PointF, Brush, f32),
    /// The text as laid out (with any ellipsis), its font and size, origin and colour.
    Text {
        text: String,
        font: String,
        origin: PointF,
        color: Color,
    },
    Bitmap {
        width: u32,
        height: u32,
        dest: RectF,
        opacity: f32,
    },
    PushClip(RectF),
    PushOpacity(f32),
    Pop,
}

fn num(v: f32) -> String {
    // Two decimals at most, without trailing zeros.
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

struct Rect(RectF);

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.0;
        write!(
            f,
            "{},{} {}×{}",
            num(r.left),
            num(r.top),
            num(r.width()),
            num(r.height())
        )
    }
}

struct Point(PointF);

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", num(self.0.x), num(self.0.y))
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Shape::Rect(r) => write!(f, "rect {}", Rect(*r)),
            Shape::RoundedRect { rect, radius } => {
                write!(f, "rounded {} r{}", Rect(*rect), num(*radius))
            }
            Shape::Polygon(points) => {
                f.write_str("polygon")?;
                for p in points {
                    write!(f, " {}", Point(*p))?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for Brush {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Brush::Solid(c) => f.write_str(&c.to_hex()),
            Brush::Linear { start, end, stops } => {
                write!(f, "linear {}→{}", Point(*start), Point(*end))?;
                for s in stops {
                    write!(f, " {}@{}", s.color.to_hex(), num(s.offset))?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for DrawOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawOp::Fill(shape, brush) => write!(f, "fill {shape} {brush}"),
            DrawOp::Stroke(shape, brush, width) => {
                write!(f, "stroke {shape} {brush} w{}", num(*width))
            }
            DrawOp::Line(from, to, brush, width) => write!(
                f,
                "line {}→{} {brush} w{}",
                Point(*from),
                Point(*to),
                num(*width)
            ),
            DrawOp::Text {
                text,
                font,
                origin,
                color,
            } => write!(
                f,
                "text {text:?} {font} at {} {}",
                Point(*origin),
                color.to_hex()
            ),
            DrawOp::Bitmap {
                width,
                height,
                dest,
                opacity,
            } => write!(
                f,
                "bitmap {width}×{height} into {} opacity {}",
                Rect(*dest),
                num(*opacity)
            ),
            DrawOp::PushClip(r) => write!(f, "clip {}", Rect(*r)),
            DrawOp::PushOpacity(o) => write!(f, "opacity {}", num(*o)),
            DrawOp::Pop => f.write_str("pop"),
        }
    }
}

/// Records every call; text must come from [`FixedMetrics`].
#[derive(Debug)]
pub struct RecordingCanvas {
    scale: f32,
    pub ops: Vec<DrawOp>,
}

impl RecordingCanvas {
    pub fn new(scale: f32) -> Self {
        Self {
            scale,
            ops: Vec::new(),
        }
    }

    /// One line per call, for snapshot comparisons.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for op in &self.ops {
            let _ = writeln!(out, "{op}");
        }
        out
    }
}

impl Canvas for RecordingCanvas {
    fn scale(&self) -> f32 {
        self.scale
    }

    fn fill(&mut self, shape: &Shape, brush: &Brush) {
        self.ops.push(DrawOp::Fill(shape.clone(), brush.clone()));
    }

    fn stroke(&mut self, shape: &Shape, brush: &Brush, width: f32) {
        self.ops
            .push(DrawOp::Stroke(shape.clone(), brush.clone(), width));
    }

    fn draw_line(&mut self, from: PointF, to: PointF, brush: &Brush, width: f32) {
        self.ops.push(DrawOp::Line(from, to, brush.clone(), width));
    }

    fn draw_text(&mut self, layout: &TextLayout, origin: PointF, color: Color) {
        let (text, font) = match layout.inner::<RecordedText>() {
            Some(r) => {
                let f = &r.style.font;
                let font = format!(
                    "{} {}{}{}",
                    f.family,
                    f.weight,
                    if f.italic { " italic" } else { "" },
                    format_args!(" {}px", num(r.style.size))
                );
                (r.text.clone(), font)
            }
            None => (String::new(), "?".into()),
        };
        self.ops.push(DrawOp::Text {
            text,
            font,
            origin,
            color,
        });
    }

    fn draw_bitmap(&mut self, bitmap: &Bitmap, dest: RectF, opacity: f32) {
        self.ops.push(DrawOp::Bitmap {
            width: bitmap.width(),
            height: bitmap.height(),
            dest,
            opacity,
        });
    }

    fn push_clip(&mut self, rect: RectF) {
        self.ops.push(DrawOp::PushClip(rect));
    }

    fn push_opacity(&mut self, opacity: f32) {
        self.ops.push(DrawOp::PushOpacity(opacity));
    }

    fn pop(&mut self) {
        self.ops.push(DrawOp::Pop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::FontStyle;
    use crate::platform::text::Font;

    fn style(size: f32) -> TextStyle {
        TextStyle::new(&Font::new("Arial", FontStyle::Bold), size)
    }

    #[test]
    fn fixed_metrics() {
        let layout = FixedMetrics.layout("CPU 37%", &style(10.0));
        assert_eq!(layout.size(), SizeF::new(35.0, 12.5));
        assert_eq!(layout.baseline(), 10.0);

        let two = FixedMetrics.layout("ab\nlonger", &style(10.0));
        assert_eq!(two.size(), SizeF::new(30.0, 25.0));
    }

    #[test]
    fn long_lines_end_in_an_ellipsis() {
        let layout = FixedMetrics.layout("Network", &style(10.0).max_width(20.0));
        assert_eq!(layout.width(), 20.0);
        let mut canvas = RecordingCanvas::new(1.0);
        canvas.draw_text(&layout, PointF::new(1.0, 2.5), Color::WHITE);
        assert_eq!(
            canvas.dump(),
            "text \"Net…\" Arial 700 10px at 1,2.5 #FFFFFF\n"
        );
        // Fits: unchanged.
        let short = FixedMetrics.layout("Net", &style(10.0).max_width(20.0));
        assert_eq!(short.width(), 15.0);
    }

    #[test]
    fn records_calls() {
        let mut c = RecordingCanvas::new(1.5);
        c.fill_rect(
            RectF::new(0.0, 0.0, 100.0, 28.0),
            Color::BLACK.with_alpha(0.8),
        );
        c.push_clip(RectF::new(1.0, 1.0, 9.0, 9.0));
        c.push_opacity(0.5);
        c.stroke(
            &Shape::RoundedRect {
                rect: RectF::new(0.5, 0.5, 9.5, 9.5),
                radius: 2.0,
            },
            &Brush::Solid(Color::WHITE),
            1.0,
        );
        c.fill(
            &Shape::Polygon(vec![
                PointF::new(0.0, 0.0),
                PointF::new(13.0, 0.0),
                PointF::new(6.5, 7.0),
            ]),
            &Brush::Linear {
                start: PointF::new(0.0, 0.0),
                end: PointF::new(0.0, 7.0),
                stops: vec![
                    crate::platform::GradientStop {
                        offset: 0.0,
                        color: Color::WHITE,
                    },
                    crate::platform::GradientStop {
                        offset: 1.0,
                        color: Color::BLACK,
                    },
                ],
            },
        );
        c.draw_line(
            PointF::new(0.0, 0.0),
            PointF::new(4.0, 0.0),
            &Color::WHITE.into(),
            1.0,
        );
        let bitmap = Bitmap::from_rgba(2, 2, vec![0; 16]).expect("valid");
        c.draw_bitmap(&bitmap, RectF::new(0.0, 0.0, 4.0, 4.0), 0.78);
        c.pop();
        c.pop();
        assert_eq!(c.snap(1.4), 4.0 / 3.0);
        assert_eq!(
            c.dump(),
            "fill rect 0,0 100×28 #000000/80%\n\
             clip 1,1 8×8\n\
             opacity 0.5\n\
             stroke rounded 0.5,0.5 9×9 r2 #FFFFFF w1\n\
             fill polygon 0,0 13,0 6.5,7 linear 0,0→0,7 #FFFFFF@0 #000000@1\n\
             line 0,0→4,0 #FFFFFF w1\n\
             bitmap 2×2 into 0,0 4×4 opacity 0.78\n\
             pop\n\
             pop\n"
        );
    }
}
