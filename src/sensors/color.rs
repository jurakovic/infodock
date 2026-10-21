//! Colours of sensor graphics (SPEC §8.4–8.5): colour modes, the dynamic gradient, and how an
//! inactive sensor's icon or graphic is drawn.

use super::{ColorChoice, ColorModeKind};
use crate::look::theme::Inactive;
use crate::platform::Color;

/// The dynamic gradient (SPEC §8.4): green at 0, yellow at 0.5, red at 1. Values outside
/// 0..1 are clamped; NaN counts as 0.
pub fn gradient(value: f32) -> Color {
    let v = if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    };
    let channel = |x: f32| (x * 510.0).round().min(255.0) as u8;
    if v < 0.5 {
        Color::rgba(channel(v), 255, 0, 1.0)
    } else {
        Color::rgba(255, channel(1.0 - v), 0, 1.0)
    }
}

/// The colour a `colorMode` value draws with: `standard` (what Standard means for this part,
/// e.g. the sensor text colour at 59 % for a meter's fill), the custom colour, or the gradient
/// at `value` (0..1).
pub fn resolve(choice: &ColorChoice, standard: Color, value: f32) -> Color {
    match choice.mode {
        ColorModeKind::Standard => standard,
        ColorModeKind::Custom => Color::from_hex(&choice.color).unwrap_or(standard),
        ColorModeKind::Dynamic => gradient(value),
    }
}

/// `color` for an inactive sensor's graphic (SPEC §8.5): faded, or the inactive colour keeping
/// the graphic's own opacity. Labels keep their colour.
pub fn inactive(color: Color, how: Inactive) -> Color {
    match how {
        Inactive::Opacity(opacity) => color.with_alpha(color.a * opacity),
        Inactive::Colorize(tint) => tint.with_alpha(color.a),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(mode: ColorModeKind, color: &str) -> ColorChoice {
        ColorChoice {
            mode,
            color: color.into(),
        }
    }

    #[test]
    fn gradient_runs_green_yellow_red() {
        assert_eq!(gradient(0.0), Color::rgba(0, 255, 0, 1.0));
        assert_eq!(gradient(0.25), Color::rgba(128, 255, 0, 1.0));
        assert_eq!(gradient(0.5), Color::rgba(255, 255, 0, 1.0));
        assert_eq!(gradient(0.75), Color::rgba(255, 128, 0, 1.0));
        assert_eq!(gradient(1.0), Color::rgba(255, 0, 0, 1.0));
        assert_eq!(gradient(2.0), gradient(1.0));
        assert_eq!(gradient(f32::NAN), gradient(0.0));
    }

    #[test]
    fn colour_modes() {
        let text = Color::rgba(255, 255, 255, 0.59);
        let standard = choice(ColorModeKind::Standard, "#FF0000");
        assert_eq!(resolve(&standard, text, 0.9), text);
        let custom = choice(ColorModeKind::Custom, "#20B7FF");
        assert_eq!(
            resolve(&custom, text, 0.9),
            Color::rgba(0x20, 0xB7, 0xFF, 1.0)
        );
        let dynamic = choice(ColorModeKind::Dynamic, "#20B7FF");
        assert_eq!(resolve(&dynamic, text, 1.0), Color::rgba(255, 0, 0, 1.0));
        let broken = choice(ColorModeKind::Custom, "nope");
        assert_eq!(resolve(&broken, text, 0.0), text);
    }

    #[test]
    fn inactive_graphics_fade_or_take_the_inactive_colour() {
        let fill = Color::rgba(0, 255, 0, 0.5);
        assert_eq!(
            inactive(fill, Inactive::Opacity(0.78)),
            Color::rgba(0, 255, 0, 0.39)
        );
        let grey = Color::rgba(0x55, 0x55, 0x55, 1.0);
        assert_eq!(
            inactive(fill, Inactive::Colorize(grey)),
            Color::rgba(0x55, 0x55, 0x55, 0.5)
        );
    }
}
