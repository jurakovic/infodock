//! Icons on the bar (SPEC §8.3, §8.4, §8.5): the one building block for every sensor that
//! shows one. The fields a type offers for it (the user's own image, the colour that tints it,
//! its opacity) and their legacy keys; how tall each kind of icon is; and [`BarIcon`], which
//! reads the user's image or the sensor's own (a shell icon or a built-in SVG) at its physical
//! size, tints it (Custom colour, §8.4), fades it while inactive and draws it at the sensor's
//! left, centred vertically.

use serde::Deserialize;

use crate::look::theme::{Inactive, Theme};
use crate::platform::{Bitmap, Canvas, Color, RectF, decode_image};
use crate::sensors::legacy::LegacyKey;
use crate::sensors::schema::{FieldKind, Lit, PropField};
use crate::sensors::{ColorChoice, ColorModeKind, DrawCtx, Fetched, IconSource, LayoutCtx, MsgKey};

/// The formats of a user's own icon (SPEC §8.4).
pub const IMAGES: &[&str] = &["ico", "png", "gif", "jpg", "jpeg", "svg"];

/// The user's own icon, for a type's schema; `help` says what shows without one.
pub const fn image(help: MsgKey) -> PropField {
    PropField::new(
        "image",
        FieldKind::File { extensions: IMAGES },
        "sensor.field.image",
        Lit::Str(""),
    )
    .describe(help)
}

/// The colour that tints the icon (SPEC §8.4: Standard or Custom).
pub const COLOR: PropField = PropField::new(
    "color",
    FieldKind::ColorMode { dynamic: false },
    "sensor.field.color",
    Lit::ColorMode("standard", "#FFFFFF"),
)
.describe("sensor.field.iconColor.help");

/// The icon's opacity, percent.
pub const OPACITY: PropField = PropField::new(
    "opacity",
    FieldKind::Opacity { min: 20 },
    "sensor.field.opacity",
    Lit::Int(100),
);

/// The XML keys of those fields (SPEC §11.4).
pub const LEGACY_IMAGE: LegacyKey = LegacyKey::new("image", "image");
pub const LEGACY_COLOR: LegacyKey = LegacyKey::new("colorize", "color");
pub const LEGACY_OPACITY: LegacyKey = LegacyKey::new("opacity", "opacity");

/// An icon's look as stored, for types with all three fields to flatten into their props.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconStyle {
    /// Empty: the sensor's own icon.
    pub image: String,
    /// Custom tints the icon (SPEC §8.4).
    pub color: ColorChoice,
    /// Percent.
    pub opacity: u32,
}

impl IconStyle {
    /// The look of an icon `size` tall with this style: the user's image, else `own`.
    pub fn look<'a>(&'a self, size: IconSize, own: Own<'a>) -> Look<'a> {
        Look {
            size,
            image: &self.image,
            own,
            color: Some(&self.color),
            opacity: self.opacity,
            inactive: false,
        }
    }
}

/// How tall a sensor's icon is: the one place for every icon on the bar (SPEC §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSize {
    /// Shortcut, Disk and Recycle Bin: bar height − 4 (SPEC §9.16–§9.18).
    Launcher,
    /// Folder Menu, Start Menu, Power and Eject USB: 21 px (SPEC §9.19–§9.22).
    Small,
    /// Image: bar height − 1 (SPEC §9.2).
    Picture,
    /// Wi-Fi: bar height − 8, at least 12 px (SPEC §9.15).
    Signal,
    /// Battery: 23 px, less in a bar lower than 25 px (SPEC §9.14).
    Battery,
}

impl IconSize {
    /// The height in logical px on a bar of `theme`'s height.
    pub fn logical(self, theme: &Theme) -> f32 {
        let less = |px: u32| theme.bar_height.saturating_sub(px).max(1) as f32;
        match self {
            Self::Launcher => less(4),
            Self::Small => 21.0,
            Self::Picture => less(1),
            Self::Signal => theme.bar_height.saturating_sub(8).max(12) as f32,
            Self::Battery => less(2).min(23.0),
        }
    }

    /// The height in physical px at `scale`.
    pub fn physical(self, theme: &Theme, scale: f32) -> u32 {
        physical(self.logical(theme), scale)
    }
}

/// The icon a sensor shows when the user has no image of their own.
pub enum Own<'a> {
    /// A built-in SVG, monochrome white (Appendix C).
    Svg(&'static [u8]),
    /// The shell icon of `path`; `svg` when the shell has none. While `refresh` is set, the
    /// icon is read again (and `refresh` cleared) the next time the shell is asked for it.
    Shell {
        path: &'a str,
        refresh: &'a mut bool,
        svg: &'static [u8],
    },
}

/// What a [`BarIcon`] shows and how.
pub struct Look<'a> {
    pub size: IconSize,
    /// The user's image file; empty for the sensor's own icon.
    pub image: &'a str,
    pub own: Own<'a>,
    /// `None`: Standard, for types without a colour field.
    pub color: Option<&'a ColorChoice>,
    /// Percent.
    pub opacity: u32,
    /// Drawn as inactive (SPEC §8.5).
    pub inactive: bool,
}

impl Look<'_> {
    /// The built-in `svg`, `size` tall, in the sensor text colour at `opacity` percent: the
    /// icons the user cannot change.
    pub fn built_in(size: IconSize, svg: &'static [u8], opacity: u32) -> Look<'static> {
        Look {
            size,
            image: "",
            own: Own::Svg(svg),
            color: None,
            opacity,
            inactive: false,
        }
    }

    pub fn inactive(self, inactive: bool) -> Self {
        Self { inactive, ..self }
    }
}

/// A sensor's icon on the bar, laid out: the icon, or nothing while its image or shell icon is
/// being read (or if it cannot be decoded), and the width it takes either way.
#[derive(Default)]
pub struct BarIcon {
    icon: Option<Icon>,
    /// Logical px, whole.
    width: f32,
}

impl BarIcon {
    /// Reads the icon for `look` at `cx`'s scale: the user's image if set and readable, else
    /// the sensor's own; while either is read, its square stays free. Returns the width.
    pub fn measure(&mut self, look: Look, cx: &LayoutCtx) -> f32 {
        let px = look.size.physical(cx.theme, cx.scale);
        let bitmap = match custom_image(look.image, px, cx.icons) {
            Fetched::Ready(image) => Some((image, false)),
            Fetched::Loading => None,
            Fetched::None => own_icon(look.own, px, cx.icons),
        };
        let opacity = look.opacity.min(100) as f32 / 100.0;
        let inactive = look.inactive.then_some(cx.theme.inactive);
        self.icon = bitmap.map(|(bitmap, built_in)| {
            let tint = match look.color {
                Some(choice) => tint(choice, built_in, cx.theme),
                None => built_in.then_some(cx.theme.sensor_text),
            };
            Icon::new(bitmap, tint, opacity, inactive)
        });
        self.width = self
            .icon
            .as_ref()
            .map_or((px as f32 / cx.scale).ceil(), |i| i.width(cx.scale));
        self.width
    }

    /// The width in logical px, whole, as [`measure`](Self::measure) returned it.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// The icon as drawn, for tests.
    #[cfg(test)]
    pub fn icon(&self) -> Option<&Icon> {
        self.icon.as_ref()
    }

    /// `icon` taking `width` logical px, for tests.
    #[cfg(test)]
    pub fn laid_out(icon: Option<Icon>, width: f32) -> Self {
        Self { icon, width }
    }

    /// Draws the icon at the left of `cx.rect`, centred vertically.
    pub fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        if let Some(icon) = &self.icon {
            icon.draw(canvas, cx);
        }
    }
}

/// The sensor's own icon `px` physical px tall; the bitmap says whether it is built in.
/// `None` while the shell icon is read.
fn own_icon(own: Own, px: u32, icons: &dyn IconSource) -> Option<(Bitmap, bool)> {
    let built_in = |svg| Icon::svg(svg, px).map(|b| (b, true));
    match own {
        Own::Svg(svg) => built_in(svg),
        Own::Shell { path, refresh, svg } => {
            if std::mem::take(refresh) {
                icons.forget(path);
            }
            match icons.shell_icon(path, px) {
                Fetched::Ready(icon) => Some((icon, false)),
                Fetched::Loading => None,
                Fetched::None => built_in(svg),
            }
        }
    }
}

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

/// A user's image file (an `image` prop), `px` physical px tall, from the bar's reader (SPEC
/// §9.2): [`Fetched::None`] when `path` is empty or the file cannot be read (logged), so the
/// sensor shows its built-in icon; [`Fetched::Loading`] while it is read.
fn custom_image(path: &str, px: u32, icons: &dyn IconSource) -> Fetched {
    if path.is_empty() {
        return Fetched::None;
    }
    icons.image(path, px)
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
    use std::cell::RefCell;

    use super::*;
    use crate::platform::recording::RecordingCanvas;
    use crate::sensors::NoIcons;
    use crate::sensors::test_support::{Harness, Later, theme};

    const DRIVE: &[u8] = include_bytes!("../../../assets/icons/drive.svg");

    fn white(px: u32) -> Bitmap {
        Bitmap::from_rgba(px, px, vec![255; (px * px * 4) as usize]).expect("a bitmap")
    }

    fn red() -> Color {
        Color::rgba(255, 0, 0, 1.0)
    }

    /// Whether every pixel is red or black (tinted red).
    fn all_red(icon: &BarIcon) -> bool {
        let bitmap = icon.icon().expect("an icon").bitmap();
        bitmap
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| px[1] == 0 && px[2] == 0)
    }

    /// A white shell icon for `C:\` only and image files from a [`Later`]; records what the
    /// shell is asked.
    #[derive(Default)]
    struct Shell {
        files: Later,
        log: RefCell<Vec<String>>,
    }

    impl IconSource for Shell {
        fn shell_icon(&self, path: &str, px: u32) -> Fetched {
            self.log.borrow_mut().push(format!("icon {path} {px}"));
            if path == "C:\\" {
                Fetched::Ready(white(px))
            } else {
                Fetched::None
            }
        }

        fn image(&self, path: &str, px: u32) -> Fetched {
            self.files.image(path, px)
        }

        fn forget(&self, path: &str) {
            self.log.borrow_mut().push(format!("forget {path}"));
        }
    }

    /// The shell icon of `C:\\`.
    fn c_drive(refresh: &mut bool) -> Own<'_> {
        Own::Shell {
            path: "C:\\",
            refresh,
            svg: DRIVE,
        }
    }

    /// Measures `icon` for `look` on a 28 px bar at scale 1, in `theme`.
    fn measured(icon: &mut BarIcon, look: Look, icons: &dyn IconSource, theme: &Theme) -> f32 {
        let h = Harness::new();
        icon.measure(
            look,
            &LayoutCtx {
                theme,
                icons,
                ..h.layout()
            },
        )
    }

    fn drawn_bar_icon(icon: &BarIcon) -> String {
        let mut canvas = RecordingCanvas::new(1.0);
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
    fn one_size_policy() {
        let sizes = |bar_height| {
            let theme = Theme {
                bar_height,
                ..theme()
            };
            [
                IconSize::Launcher,
                IconSize::Small,
                IconSize::Picture,
                IconSize::Signal,
                IconSize::Battery,
            ]
            .map(|size| size.logical(&theme))
        };
        assert_eq!(sizes(28), [24.0, 21.0, 27.0, 20.0, 23.0]);
        assert_eq!(sizes(25), [21.0, 21.0, 24.0, 17.0, 23.0]);
        assert_eq!(sizes(14), [10.0, 21.0, 13.0, 12.0, 12.0]);
        let theme = theme();
        assert_eq!(IconSize::Launcher.physical(&theme, 1.5), 36);
        assert_eq!(IconSize::Small.physical(&theme, 1.5), 32);
    }

    #[test]
    fn the_users_image_else_the_sensors_own() {
        let style = IconStyle {
            image: "D:\\me.png".into(),
            color: ColorChoice {
                mode: ColorModeKind::Standard,
                color: "#FFFFFF".into(),
            },
            opacity: 50,
        };
        let shell = Shell::default();
        let mut refresh = false;
        let mut icon = BarIcon::default();
        let mut measure = |icon: &mut BarIcon| {
            let look = style.look(IconSize::Launcher, c_drive(&mut refresh));
            measured(icon, look, &shell, &theme())
        };
        // Being read: its square stays free, and the shell is not asked meanwhile.
        assert_eq!(measure(&mut icon), 24.0);
        assert_eq!(drawn_bar_icon(&icon), "");
        assert!(shell.log.borrow().is_empty());
        assert_eq!(shell.files.asked(), [("D:\\me.png".to_owned(), 24)]);
        // It arrived: as it is (Standard), at its opacity, as wide as it is.
        let wide = Bitmap::from_rgba(48, 24, vec![255; 48 * 24 * 4]).expect("a bitmap");
        shell.files.arrive(wide);
        assert_eq!(measure(&mut icon), 48.0);
        assert_eq!(icon.width(), 48.0);
        assert_eq!(
            drawn_bar_icon(&icon),
            "bitmap 48×24 into 7,2 48×24 opacity 0.5\n"
        );
        // It cannot be read: the shell icon.
        shell.files.fail();
        assert_eq!(measure(&mut icon), 24.0);
        assert_eq!(*shell.log.borrow(), ["icon C:\\ 24"]);
    }

    #[test]
    fn a_shell_icon_is_read_again_once_when_asked() {
        let shell = Shell::default();
        let mut refresh = true;
        let mut icon = BarIcon::default();
        let look = Look {
            own: Own::Shell {
                path: "Q:\\",
                refresh: &mut refresh,
                svg: DRIVE,
            },
            ..Look::built_in(IconSize::Launcher, DRIVE, 100)
        };
        measured(&mut icon, look, &shell, &theme());
        assert!(!refresh, "taken");
        // Q: has none: the built-in drive icon.
        assert_eq!(*shell.log.borrow(), ["forget Q:\\", "icon Q:\\ 24"]);
        assert!(icon.icon().is_some());
    }

    #[test]
    fn built_in_icons_take_the_text_colour_others_stay() {
        let theme = Theme {
            sensor_text: red(),
            ..theme()
        };
        let mut icon = BarIcon::default();
        measured(
            &mut icon,
            Look::built_in(IconSize::Small, DRIVE, 100),
            &NoIcons,
            &theme,
        );
        assert!(all_red(&icon));
        // The shell's icon stays white, a custom colour tints it.
        let shell = Shell::default();
        let custom = ColorChoice {
            mode: ColorModeKind::Custom,
            color: "#FF0000".into(),
        };
        for (color, tinted) in [(None, false), (Some(&custom), true)] {
            let mut refresh = false;
            let look = Look {
                own: c_drive(&mut refresh),
                color,
                ..Look::built_in(IconSize::Launcher, DRIVE, 100)
            };
            measured(&mut icon, look, &shell, &theme);
            assert_eq!(all_red(&icon), tinted);
        }
    }

    #[test]
    fn inactive_icons_fade() {
        let mut icon = BarIcon::default();
        let look = Look::built_in(IconSize::Launcher, DRIVE, 50).inactive(true);
        measured(&mut icon, look, &NoIcons, &theme());
        assert!(drawn_bar_icon(&icon).ends_with("opacity 0.39\n"));
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
