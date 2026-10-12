//! Text layout with DirectWrite (SPEC §4.6). Layouts are drawn by the Direct2D canvas in
//! `render.rs`, which reads the `IDWriteTextLayout` back from the [`TextLayout`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::rc::Rc;

use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_LINE_METRICS, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteInlineObject,
    IDWriteTextFormat, IDWriteTextLayout, IDWriteTypography,
};
use windows::core::{BOOL, HSTRING, Result, w};

use crate::platform::{Font, SizeF, TextEngine, TextLayout, TextStyle};

/// Used for families that are not installed: Segoe UI Variable exists from Windows 11 on.
const FALLBACK_FAMILY: &str = "Segoe UI";
/// Layout width when no maximum is set; lines never wrap.
const UNBOUNDED: f32 = 1_000_000.0;

/// The DirectWrite layout inside a [`TextLayout`].
pub(super) struct DwLayout(pub(super) IDWriteTextLayout);

struct Format {
    format: IDWriteTextFormat,
    /// Created on first use.
    ellipsis: Option<IDWriteInlineObject>,
}

pub struct DirectWrite {
    factory: IDWriteFactory,
    fonts: IDWriteFontCollection,
    tabular: IDWriteTypography,
    /// Requested family → installed family.
    families: RefCell<HashMap<String, String>>,
    /// By font and size (as bits).
    formats: RefCell<HashMap<(Font, u32), Format>>,
    /// A layout failed once; later failures are logged at debug level.
    failed: Cell<bool>,
}

impl DirectWrite {
    pub fn new() -> Result<Self> {
        // SAFETY: plain factory creation and queries; the objects are released on drop.
        unsafe {
            let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut fonts = None;
            factory.GetSystemFontCollection(&mut fonts, false)?;
            let fonts = fonts.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
            let tabular = factory.CreateTypography()?;
            tabular.AddFontFeature(DWRITE_FONT_FEATURE {
                nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                parameter: 1,
            })?;
            Ok(Self {
                factory,
                fonts,
                tabular,
                families: RefCell::new(HashMap::new()),
                formats: RefCell::new(HashMap::new()),
                failed: Cell::new(false),
            })
        }
    }

    fn installed(&self, family: &str) -> bool {
        let mut index = 0;
        let mut exists = BOOL(0);
        // SAFETY: both out pointers are valid for the call.
        let found = unsafe {
            self.fonts
                .FindFamilyName(&HSTRING::from(family), &mut index, &mut exists)
        };
        found.is_ok() && exists.as_bool()
    }

    /// The installed family to use for `requested`.
    fn family(&self, requested: &str) -> String {
        if let Some(family) = self.families.borrow().get(requested) {
            return family.clone();
        }
        let family = if self.installed(requested) {
            requested.to_owned()
        } else {
            tracing::info!("font {requested:?} is not installed; using {FALLBACK_FAMILY}");
            FALLBACK_FAMILY.to_owned()
        };
        self.families
            .borrow_mut()
            .insert(requested.to_owned(), family.clone());
        family
    }

    /// The text format for `font` at `size`, and its ellipsis sign if `ellipsis` is set.
    fn format(
        &self,
        font: &Font,
        size: f32,
        ellipsis: bool,
    ) -> Result<(IDWriteTextFormat, Option<IDWriteInlineObject>)> {
        let mut formats = self.formats.borrow_mut();
        let entry = match formats.entry((font.clone(), size.to_bits())) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(slot) => {
                let family = self.family(&font.family);
                let style = if font.italic {
                    DWRITE_FONT_STYLE_ITALIC
                } else {
                    DWRITE_FONT_STYLE_NORMAL
                };
                // SAFETY: plain factory call with valid arguments.
                let format = unsafe {
                    self.factory.CreateTextFormat(
                        &HSTRING::from(family),
                        None,
                        DWRITE_FONT_WEIGHT(i32::from(font.weight)),
                        style,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        w!("en-us"),
                    )?
                };
                // SAFETY: plain setter on the new format.
                unsafe { format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
                slot.insert(Format {
                    format,
                    ellipsis: None,
                })
            }
        };
        if ellipsis && entry.ellipsis.is_none() {
            // SAFETY: plain factory call with a valid format.
            entry.ellipsis =
                Some(unsafe { self.factory.CreateEllipsisTrimmingSign(&entry.format)? });
        }
        Ok((
            entry.format.clone(),
            entry.ellipsis.clone().filter(|_| ellipsis),
        ))
    }

    fn try_layout(&self, text: &str, style: &TextStyle) -> Result<TextLayout> {
        let (format, ellipsis) = self.format(&style.font, style.size, style.max_width.is_some())?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: plain DirectWrite calls; the out structures are valid for each call.
        unsafe {
            let layout = self.factory.CreateTextLayout(
                &wide,
                &format,
                style.max_width.unwrap_or(UNBOUNDED).max(0.0),
                UNBOUNDED,
            )?;
            if let Some(sign) = &ellipsis {
                let trimming = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                layout.SetTrimming(&trimming, sign)?;
            }
            if style.tabular && !wide.is_empty() {
                let all = DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: wide.len() as u32,
                };
                layout.SetTypography(&self.tabular, all)?;
            }
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            let mut lines = vec![DWRITE_LINE_METRICS::default(); metrics.lineCount.max(1) as usize];
            let mut count = 0;
            layout.GetLineMetrics(Some(&mut lines), &mut count)?;
            // A trimmed line fits its maximum width; never report more.
            let width = style
                .max_width
                .map_or(metrics.width, |max| metrics.width.min(max));
            let size = SizeF::new(width, metrics.height);
            Ok(TextLayout::new(
                size,
                lines[0].baseline,
                Rc::new(DwLayout(layout)),
            ))
        }
    }
}

impl TextEngine for DirectWrite {
    fn layout(&self, text: &str, style: &TextStyle) -> TextLayout {
        self.try_layout(text, style).unwrap_or_else(|e| {
            if self.failed.replace(true) {
                tracing::debug!("text layout failed: {e}");
            } else {
                tracing::warn!("text layout failed: {e}");
            }
            TextLayout::empty()
        })
    }
}
