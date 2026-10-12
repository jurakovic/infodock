//! Text layout and measurement (SPEC §4.6). A [`TextEngine`] lays text out once; the resulting
//! [`TextLayout`] knows its size for the zone layout and is drawn with [`super::Canvas`].
//! Sensors keep their layouts and re-create them when their text or style changes (§14.1).

use std::any::Any;
use std::fmt;
use std::rc::Rc;

use super::canvas::SizeF;
use crate::config::model::FontStyle;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Font {
    pub family: String,
    /// CSS-style weight: 400 normal, 600 semibold, 700 bold. Fonts without a semibold face
    /// render semibold as bold (SPEC §8.2).
    pub weight: u16,
    pub italic: bool,
}

impl Font {
    pub fn new(family: &str, style: FontStyle) -> Self {
        let (weight, italic) = match style {
            FontStyle::Normal => (400, false),
            FontStyle::Semibold => (600, false),
            FontStyle::Bold => (700, false),
            FontStyle::Italic => (400, true),
            FontStyle::SemiboldItalic => (600, true),
            FontStyle::BoldItalic => (700, true),
        };
        Self {
            family: family.to_owned(),
            weight,
            italic,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font: Font,
    /// Logical px.
    pub size: f32,
    /// All digits equally wide (OpenType `tnum`), so changing numbers do not shift the bar
    /// (SPEC §5.4).
    pub tabular: bool,
    /// Lines longer than this end in an ellipsis.
    pub max_width: Option<f32>,
}

impl TextStyle {
    pub fn new(font: &Font, size: f32) -> Self {
        Self {
            font: font.clone(),
            size,
            tabular: false,
            max_width: None,
        }
    }

    pub fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }
}

/// Text laid out by a [`TextEngine`], measured and ready to draw. Cheap to clone; bar thread
/// only.
#[derive(Clone)]
pub struct TextLayout {
    size: SizeF,
    baseline: f32,
    /// The engine's own layout object, read back by the canvas of the same platform.
    inner: Rc<dyn Any>,
}

impl TextLayout {
    pub(crate) fn new(size: SizeF, baseline: f32, inner: Rc<dyn Any>) -> Self {
        Self {
            size,
            baseline,
            inner,
        }
    }

    /// A layout that measures zero and draws nothing, for text the OS could not lay out.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn empty() -> Self {
        Self::new(SizeF::default(), 0.0, Rc::new(()))
    }

    /// Width and height of all lines, in logical px.
    pub fn size(&self) -> SizeF {
        self.size
    }

    pub fn width(&self) -> f32 {
        self.size.width
    }

    pub fn height(&self) -> f32 {
        self.size.height
    }

    /// Distance from the top of the layout to the first line's baseline.
    pub fn baseline(&self) -> f32 {
        self.baseline
    }

    pub(crate) fn inner<T: 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref()
    }
}

impl fmt::Debug for TextLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextLayout")
            .field("size", &self.size)
            .field("baseline", &self.baseline)
            .finish_non_exhaustive()
    }
}

/// Lays out text (bar thread only).
pub trait TextEngine {
    /// Lays out `text` on one line; `\n` starts another. Never fails: if the OS cannot lay the
    /// text out, the result is empty.
    fn layout(&self, text: &str, style: &TextStyle) -> TextLayout;
}
