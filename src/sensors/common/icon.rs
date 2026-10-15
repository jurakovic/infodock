//! An icon on the bar (SPEC §8.3, §8.5): the launchers' view. A bitmap decoded at its physical
//! size, centred vertically, optionally tinted (Custom colour, §8.4), drawn at the sensor's
//! opacity; an inactive sensor's icon fades or takes the inactive colour.

use crate::bar::theme::{Inactive, Theme};
use crate::platform::{Bitmap, Canvas, Color, RectF, decode_image};
use crate::sensors::{ColorChoice, ColorModeKind, DrawCtx};

/// The colour an icon is tinted with (SPEC §8.4): Custom tints any icon. Standard leaves shell
/// icons and the user's own images as they are, and gives Infodock's built-in icons, which are
/// monochrome white, the sensor text colour, so they read on a light bar too.
pub fn tint(choice: &ColorChoice, built_in: bool, theme: &Theme) -> Option<Color> {
    match choice.mode {
        ColorModeKind::Custom => Color::from_hex(&choice.color),
        ColorModeKind::Standard | ColorModeKind::Dynamic => built_in.then_some(theme.sensor_text),
    }
}

/// The icon size in physical px for `logical` px at `scale`.
pub fn physical(logical: f32, scale: f32) -> u32 {
    (logical * scale).round().max(1.0) as u32
}

/// An icon ready to draw.
pub struct Icon {
    bitmap: Bitmap,
    /// 0–1, with the inactive fade.
    opacity: f32,
}

impl Icon {
    /// `bitmap` tinted with `tint` (or the inactive colour while `inactive` says so), at
    /// `opacity` (0–1) and faded while inactive.
    pub fn new(
        bitmap: Bitmap,
        tint: Option<Color>,
        opacity: f32,
        inactive: Option<Inactive>,
    ) -> Self {
        let (tint, opacity) = match inactive {
            Some(Inactive::Opacity(fade)) => (tint, opacity * fade),
            Some(Inactive::Colorize(color)) => (Some(color), opacity),
            None => (tint, opacity),
        };
        let bitmap = match tint {
            Some(tint) => bitmap.tinted(tint),
            None => bitmap,
        };
        Self { bitmap, opacity }
    }

    /// An embedded SVG `px` physical px tall; `None` (logged) if it cannot be decoded.
    pub fn svg(bytes: &[u8], px: u32) -> Option<Bitmap> {
        decode_image(bytes, Some(px))
            .map_err(|e| tracing::error!("a built-in icon cannot be decoded: {e}"))
            .ok()
    }

    /// The width in logical px at `scale`, rounded up to whole px.
    pub fn width(&self, scale: f32) -> f32 {
        (self.bitmap.width() as f32 / scale).ceil()
    }

    /// The bitmap as drawn, for tests.
    #[cfg(test)]
    pub fn bitmap(&self) -> &Bitmap {
        &self.bitmap
    }

    /// Draws the icon at the left of `cx.rect`, centred vertically, on whole physical px so it
    /// stays sharp.
    pub fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let scale = canvas.scale();
        let (width, height) = (
            self.bitmap.width() as f32 / scale,
            self.bitmap.height() as f32 / scale,
        );
        let left = canvas.snap(cx.rect.left);
        let top = canvas.snap(cx.rect.top + (cx.rect.height() - height) / 2.0);
        let dest = RectF::new(left, top, left + width, top + height);
        canvas.draw_bitmap(&self.bitmap, dest, self.opacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::recording::RecordingCanvas;
    use crate::sensors::test_support::theme;

    fn white(px: u32) -> Bitmap {
        Bitmap::from_rgba(px, px, vec![255; (px * px * 4) as usize]).expect("a bitmap")
    }

    fn drawn(icon: &Icon, scale: f32) -> String {
        let mut canvas = RecordingCanvas::new(scale);
        icon.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme(),
                rect: RectF::new(7.0, 0.0, 31.0, 28.0),
            },
        );
        canvas.dump()
    }

    #[test]
    fn centred_on_whole_pixels() {
        let icon = Icon::new(white(24), None, 1.0, None);
        assert_eq!(icon.width(1.0), 24.0);
        assert_eq!(drawn(&icon, 1.0), "bitmap 24×24 into 7,2 24×24 opacity 1\n");
        // At 150 %: 36 physical px, 24 logical; 7 → 10.5 → 11 physical px.
        let icon = Icon::new(white(physical(24.0, 1.5)), None, 0.5, None);
        assert_eq!(
            drawn(&icon, 1.5),
            "bitmap 36×36 into 7.33,2 24×24 opacity 0.5\n"
        );
    }

    #[test]
    fn built_in_icons_follow_the_text_colour() {
        let theme = theme();
        let standard = ColorChoice {
            mode: ColorModeKind::Standard,
            color: "#FF0000".into(),
        };
        assert_eq!(tint(&standard, true, &theme), Some(theme.sensor_text));
        assert_eq!(tint(&standard, false, &theme), None, "shell icons stay");
        let custom = ColorChoice {
            mode: ColorModeKind::Custom,
            ..standard
        };
        assert_eq!(tint(&custom, false, &theme), Color::from_hex("#FF0000"));
    }

    #[test]
    fn tinted_and_inactive() {
        let red = Color::rgba(255, 0, 0, 1.0);
        let tinted = Icon::new(white(2), Some(red), 1.0, None);
        assert_eq!(&tinted.bitmap().pixels()[..4], &[255, 0, 0, 255]);
        // Inactive by opacity: fades on top of its own opacity.
        let faded = Icon::new(white(2), Some(red), 0.5, Some(Inactive::Opacity(0.78)));
        assert_eq!(faded.opacity, 0.39);
        assert_eq!(&faded.bitmap().pixels()[..4], &[255, 0, 0, 255]);
        // Colourised: the inactive colour instead of the tint.
        let grey = Color::rgba(0x55, 0x55, 0x55, 1.0);
        let colourised = Icon::new(white(2), Some(red), 1.0, Some(Inactive::Colorize(grey)));
        assert_eq!(&colourised.bitmap().pixels()[..4], &[0x55, 0x55, 0x55, 255]);
    }
}
