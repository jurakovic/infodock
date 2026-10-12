//! The resolved bar theme (SPEC §12.1): theme defaults, user settings and the OS appearance
//! combined into the values all bar drawing reads.

use crate::config::model::{
    BackgroundColorMode, InactiveMode, Material, Settings, Theme as ThemeId, ThemeDefaults,
};
use crate::platform::{Color, Font, OsAppearance};

/// Modern theme in OS light mode, when the user has not customised colours (SPEC §12.2).
const LIGHT_BAR_BACKGROUND: Color = Color::rgba(0xF3, 0xF3, 0xF3, 1.0);
const LIGHT_SENSOR_TEXT: Color = Color::rgba(0x1A, 0x1A, 0x1A, 1.0);
const DEFAULT_TEXT_COLOR: &str = "#FFFFFF";

/// How inactive sensors draw their icon or graphic (SPEC §8.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Inactive {
    /// Drawn with this opacity (0–1).
    Opacity(f32),
    /// Tinted with this colour.
    Colorize(Color),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub id: ThemeId,
    /// Logical px.
    pub bar_height: u32,
    /// Tint with the configured opacity as alpha.
    pub bar_background: Color,
    /// 1 px line on the edge facing the screen centre (modern only, SPEC §12.2).
    pub hairline: Option<Color>,
    pub material: Material,
    /// Edge padding and gap between sensors (SPEC §5.4).
    pub pad: f32,
    pub gap: f32,
    pub sensor_text: Color,
    pub sensor_font: Font,
    pub small_size: f32,
    pub medium_size: f32,
    pub large_size: f32,
    /// Background with the configured opacity as alpha.
    pub popup_background: Color,
    pub popup_text: Color,
    pub popup_strong_text: Color,
    pub popup_font: Font,
    pub popup_font_size: f32,
    pub popup_radius: f32,
    pub popup_shadow: bool,
    /// Focus ring and drop placer (SPEC §6.2, §14.4).
    pub accent: Color,
    pub inactive: Inactive,
}

/// A configured `#RRGGBB`, or `fallback` if it does not parse (settings are sanitised on load).
fn color(hex: &str, fallback: Color) -> Color {
    Color::from_hex(hex).unwrap_or(fallback)
}

fn percent(value: u32) -> f32 {
    value.min(100) as f32 / 100.0
}

impl Theme {
    pub fn resolve(s: &Settings, os: OsAppearance) -> Self {
        let d = ThemeDefaults::of(s.theme);
        let modern = s.theme == ThemeId::Modern;
        let custom_background =
            s.background.color_mode == BackgroundColorMode::Custom && s.background.color.is_some();
        let light =
            modern && !os.dark && !custom_background && s.sensors.text_color == DEFAULT_TEXT_COLOR;

        let theme_background = if light {
            LIGHT_BAR_BACKGROUND
        } else {
            color(&d.background_color, Color::BLACK)
        };
        let background = match (s.background.color_mode, &s.background.color) {
            (BackgroundColorMode::Custom, Some(c)) => color(c, theme_background),
            _ => theme_background,
        };
        let white = Color::WHITE;
        let sensor_text = if light {
            LIGHT_SENSOR_TEXT
        } else {
            color(&s.sensors.text_color, white)
        };
        let hairline = modern.then(|| {
            if light {
                Color::rgba(0, 0, 0, 0.08)
            } else {
                Color::rgba(255, 255, 255, 0.08)
            }
        });
        let material = if os.transparency {
            s.background.material.unwrap_or(d.background_material)
        } else {
            Material::None
        };
        let popup_background = s
            .popups
            .background
            .as_deref()
            .unwrap_or(&d.popup_background);

        Self {
            id: s.theme,
            bar_height: s.bar_height(),
            bar_background: background.with_alpha(percent(
                s.background.opacity.unwrap_or(d.background_opacity),
            )),
            hairline,
            material,
            pad: 7.0,
            gap: s.sensor_spacing as f32,
            sensor_text,
            sensor_font: Font::new(
                s.sensors.font.as_deref().unwrap_or(&d.sensor_font),
                s.sensors.font_style.unwrap_or(d.sensor_font_style),
            ),
            small_size: if modern { 10.0 } else { 9.0 },
            medium_size: 11.0,
            large_size: 15.0,
            popup_background: color(popup_background, Color::BLACK)
                .with_alpha(percent(s.popups.opacity.unwrap_or(d.popup_opacity))),
            popup_text: color(&s.popups.text_color, white),
            popup_strong_text: color(&s.popups.strong_text_color, white),
            popup_font: Font::new(
                s.popups.font.as_deref().unwrap_or(&d.popup_font),
                s.popups.font_style.unwrap_or(d.popup_font_style),
            ),
            popup_font_size: s.popups.font_size as f32,
            popup_radius: if modern { 8.0 } else { 6.0 },
            popup_shadow: s.popups.shadow,
            accent: os.accent,
            inactive: match s.sensors.inactive_mode {
                InactiveMode::Opacity => Inactive::Opacity(percent(s.sensors.inactive_opacity)),
                InactiveMode::Colorize => Inactive::Colorize(color(
                    &s.sensors.inactive_color,
                    Color::rgba(0x55, 0x55, 0x55, 1.0),
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::FontStyle;

    const ACCENT: Color = Color::rgba(0x00, 0x78, 0xD4, 1.0);
    const DARK: OsAppearance = OsAppearance {
        dark: true,
        transparency: true,
        accent: ACCENT,
    };
    const LIGHT: OsAppearance = OsAppearance {
        dark: false,
        transparency: true,
        accent: ACCENT,
    };

    fn hex(s: &str) -> Color {
        Color::from_hex(s).expect("valid colour")
    }

    #[test]
    fn modern_defaults() {
        let t = Theme::resolve(&Settings::default(), DARK);
        assert_eq!(t.bar_height, 28);
        assert_eq!(t.bar_background, hex("#1C1C1F").with_alpha(0.8));
        assert_eq!(t.hairline, Some(Color::rgba(255, 255, 255, 0.08)));
        assert_eq!(t.material, Material::Acrylic);
        assert_eq!(t.sensor_text, hex("#FFFFFF"));
        assert_eq!(
            t.sensor_font,
            Font::new("Segoe UI Variable Text", FontStyle::Semibold)
        );
        assert_eq!(t.sensor_font.weight, 600);
        assert_eq!(t.small_size, 10.0);
        assert_eq!(t.popup_background, hex("#202024").with_alpha(0.92));
        assert_eq!(t.popup_radius, 8.0);
        assert_eq!(t.gap, 10.0);
        assert_eq!(t.accent, ACCENT);
        assert_eq!(t.inactive, Inactive::Opacity(0.78));
    }

    #[test]
    fn classic_defaults() {
        let s = Settings {
            theme: ThemeId::Classic,
            ..Settings::default()
        };
        let t = Theme::resolve(&s, DARK);
        assert_eq!(t.bar_height, 25);
        assert_eq!(t.bar_background, hex("#2A2A2E").with_alpha(0.9));
        assert_eq!(t.sensor_font.family, "Arial");
        assert_eq!(t.sensor_font.weight, 700);
        assert_eq!(t.small_size, 9.0);
        assert_eq!(t.popup_background, hex("#000000").with_alpha(0.78));
        assert_eq!(t.popup_radius, 6.0);
        assert_eq!(t.hairline, None);
        assert_eq!(t.material, Material::None);
    }

    #[test]
    fn user_overrides() {
        let mut s = Settings::default();
        s.bar.height = Some(33);
        s.background.color = Some("#102030".into());
        s.background.opacity = Some(55);
        s.background.material = Some(Material::Mica);
        s.sensors.font = Some("Consolas".into());
        s.sensors.font_style = Some(FontStyle::BoldItalic);
        s.popups.font_size = 14;
        let t = Theme::resolve(&s, DARK);
        assert_eq!(t.bar_height, 33);
        assert_eq!(t.bar_background, hex("#102030").with_alpha(0.55));
        assert_eq!(t.material, Material::Mica);
        assert_eq!(
            t.sensor_font,
            Font {
                family: "Consolas".into(),
                weight: 700,
                italic: true
            }
        );
        assert_eq!(t.popup_font_size, 14.0);
    }

    #[test]
    fn standard_background_ignores_the_custom_colour() {
        let mut s = Settings::default();
        s.background.color_mode = BackgroundColorMode::Standard;
        s.background.color = Some("#FF0000".into());
        assert_eq!(
            Theme::resolve(&s, DARK).bar_background,
            hex("#1C1C1F").with_alpha(0.8)
        );
    }

    #[test]
    fn follows_light_mode_only_when_colours_are_not_customised() {
        let t = Theme::resolve(&Settings::default(), LIGHT);
        assert_eq!(t.bar_background, hex("#F3F3F3").with_alpha(0.8));
        assert_eq!(t.sensor_text, hex("#1A1A1A"));
        assert_eq!(t.hairline, Some(Color::rgba(0, 0, 0, 0.08)));

        let mut custom = Settings::default();
        custom.background.color = Some("#1C1C1F".into());
        let t = Theme::resolve(&custom, LIGHT);
        assert_eq!(t.bar_background, hex("#1C1C1F").with_alpha(0.8));
        assert_eq!(t.sensor_text, hex("#FFFFFF"));

        let mut text = Settings::default();
        text.sensors.text_color = "#FFCC00".into();
        assert_eq!(Theme::resolve(&text, LIGHT).sensor_text, hex("#FFCC00"));

        let classic = Settings {
            theme: ThemeId::Classic,
            ..Settings::default()
        };
        assert_eq!(
            Theme::resolve(&classic, LIGHT).bar_background,
            hex("#2A2A2E").with_alpha(0.9)
        );
    }

    #[test]
    fn inactive_style() {
        let mut s = Settings::default();
        s.sensors.inactive_mode = InactiveMode::Colorize;
        s.sensors.inactive_color = "#123456".into();
        assert_eq!(
            Theme::resolve(&s, DARK).inactive,
            Inactive::Colorize(hex("#123456"))
        );
    }

    #[test]
    fn follows_the_accent_colour() {
        let os = OsAppearance {
            accent: hex("#E81123"),
            ..DARK
        };
        assert_eq!(
            Theme::resolve(&Settings::default(), os).accent,
            hex("#E81123")
        );
    }

    #[test]
    fn no_material_without_transparency_effects() {
        let os = OsAppearance {
            transparency: false,
            ..DARK
        };
        assert_eq!(
            Theme::resolve(&Settings::default(), os).material,
            Material::None
        );
    }
}
