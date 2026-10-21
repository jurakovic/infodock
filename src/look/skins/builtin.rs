//! The built-in skins (SPEC §12.4), drawn pixel by pixel from the base colour: Flat, Gloss,
//! Brushed metal, Carbon and Glass. Plain functions, so they are tested on every OS.

use super::Target;
use crate::config::model::Edge;
use crate::platform::{Bitmap, Color};

/// Gloss: the white over the upper half, from the top to the middle.
const GLOSS_TOP: f32 = 0.30;
const GLOSS_MIDDLE: f32 = 0.05;
/// Gloss: the darker line at the bottom.
const GLOSS_LINE: f32 = 0.30;
/// Brushed metal: white at the top and black at the bottom of the vertical gradient.
const BRUSHED_LIGHT: f32 = 0.16;
const BRUSHED_DARK: f32 = 0.20;
/// Brushed metal: the line texture's strength, and the streaks' length in logical px.
const BRUSHED_TEXTURE: f32 = 0.04;
const BRUSHED_STREAK: f32 = 16.0;
/// Carbon: the checker's cells, logical px, and the light cells' white.
const CARBON_CELL: f32 = 4.0;
const CARBON_LIGHT: f32 = 0.06;
/// Glass: the base colour's alpha, the inner glow's strength at the edges and its reach (a share
/// of the height), and the highlight on the edge facing the screen centre.
const GLASS_ALPHA: f32 = 0.55;
const GLASS_GLOW: f32 = 0.10;
const GLASS_GLOW_REACH: f32 = 1.0 / 3.0;
const GLASS_HIGHLIGHT: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltIn {
    Flat,
    Gloss,
    Brushed,
    Carbon,
    Glass,
}

impl BuiltIn {
    pub const ALL: [Self; 5] = [
        Self::Flat,
        Self::Gloss,
        Self::Brushed,
        Self::Carbon,
        Self::Glass,
    ];

    /// The id stored in `classic.skin`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Gloss => "gloss",
            Self::Brushed => "brushed",
            Self::Carbon => "carbon",
            Self::Glass => "glass",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|skin| skin.id() == id)
    }

    /// The skin at `target`'s size with `base` as its colour; the base's alpha is ignored, since
    /// the bar draws the skin with the background opacity (SPEC §12.3).
    pub fn render(self, base: Color, target: Target) -> Option<Bitmap> {
        let mut pixels = Vec::with_capacity(target.width as usize * target.height as usize * 4);
        for y in 0..target.height {
            for x in 0..target.width {
                pixels.extend(self.pixel(base, x, y, target).bytes());
            }
        }
        Bitmap::from_premultiplied(target.width, target.height, pixels)
    }

    fn pixel(self, base: Color, x: u32, y: u32, t: Target) -> Px {
        let height = t.height as f32;
        // The pixel's centre, and how far down the bar it is (0–1).
        let yc = y as f32 + 0.5;
        let down = yc / height;
        // A line of one logical px, in whole physical px.
        let line = (t.scale.round() as u32).max(1);
        let opaque = Px::opaque(base);
        match self {
            Self::Flat => opaque,
            Self::Gloss => {
                let half = height / 2.0;
                let mut px = opaque;
                if yc < half {
                    let gloss = GLOSS_TOP + (GLOSS_MIDDLE - GLOSS_TOP) * yc / half;
                    px = px.over(Color::WHITE, gloss);
                }
                if y + line >= t.height {
                    px = px.over(Color::BLACK, GLOSS_LINE);
                }
                px
            }
            Self::Brushed => {
                let px = opaque
                    .over(Color::WHITE, BRUSHED_LIGHT * (1.0 - down))
                    .over(Color::BLACK, BRUSHED_DARK * down);
                // A line per physical row, lighter or darker, with streaks along it.
                let streak = streak(x as f32 / (BRUSHED_STREAK * t.scale), y);
                let texture = 0.65 * signed(hash(y)) + 0.35 * streak;
                if texture >= 0.0 {
                    px.over(Color::WHITE, BRUSHED_TEXTURE * texture)
                } else {
                    px.over(Color::BLACK, -BRUSHED_TEXTURE * texture)
                }
            }
            Self::Carbon => {
                // Diamonds CARBON_CELL px across, alternating like a checker board.
                let cell = (CARBON_CELL * t.scale).max(2.0);
                let (x, y) = (x as f32, y as f32);
                let u = ((x + y) / cell).floor();
                let v = ((x - y) / cell).floor();
                if (u + v).rem_euclid(2.0) == 1.0 {
                    opaque.over(Color::WHITE, CARBON_LIGHT)
                } else {
                    opaque
                }
            }
            Self::Glass => {
                let mut px = Px::opaque(base).faded(GLASS_ALPHA);
                // A soft glow inside both long edges.
                let reach = height * GLASS_GLOW_REACH;
                let distance = yc.min(height - yc);
                if distance < reach {
                    let glow = 1.0 - distance / reach;
                    px = px.over(Color::WHITE, GLASS_GLOW * glow * glow);
                }
                let inner = match t.edge {
                    Edge::Bottom => y < line,
                    Edge::Top => y + line >= t.height,
                };
                if inner {
                    px = px.over(Color::WHITE, GLASS_HIGHLIGHT);
                }
                px
            }
        }
    }
}

/// A premultiplied pixel, each channel 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Px {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Px {
    fn opaque(c: Color) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
            a: 1.0,
        }
    }

    fn faded(self, opacity: f32) -> Self {
        Self {
            r: self.r * opacity,
            g: self.g * opacity,
            b: self.b * opacity,
            a: self.a * opacity,
        }
    }

    /// `color` at `alpha` composed over this pixel.
    fn over(self, color: Color, alpha: f32) -> Self {
        let keep = 1.0 - alpha;
        Self {
            r: color.r * alpha + self.r * keep,
            g: color.g * alpha + self.g * keep,
            b: color.b * alpha + self.b * keep,
            a: alpha + self.a * keep,
        }
    }

    fn bytes(self) -> [u8; 4] {
        let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [
            channel(self.r),
            channel(self.g),
            channel(self.b),
            channel(self.a),
        ]
    }
}

/// A well-mixed 32-bit hash (MurmurHash3's finaliser), for textures that look random but are
/// the same on every frame.
fn hash(mut n: u32) -> u32 {
    n ^= n >> 16;
    n = n.wrapping_mul(0x85EB_CA6B);
    n ^= n >> 13;
    n = n.wrapping_mul(0xC2B2_AE35);
    n ^ (n >> 16)
}

/// `n` mapped to −1–1.
fn signed(n: u32) -> f32 {
    n as f32 / u32::MAX as f32 * 2.0 - 1.0
}

/// Smooth value noise along row `y` at `position` (in streak lengths), −1–1; each row starts
/// at its own offset, so the streaks do not line up.
fn streak(position: f32, y: u32) -> f32 {
    let p = position + (hash(y ^ 0x9E37_79B9) % 64) as f32;
    let i = p.floor();
    let f = p - i;
    let at = |i: f32| signed(hash((i as u32).wrapping_mul(0x9E37_79B1) ^ hash(y)));
    let smooth = f * f * (3.0 - 2.0 * f);
    at(i) + (at(i + 1.0) - at(i)) * smooth
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Color = Color::rgba(0x2A, 0x2A, 0x2E, 1.0);

    fn target(width: u32, height: u32, scale: f32, edge: Edge) -> Target {
        Target {
            width,
            height,
            scale,
            edge,
        }
    }

    /// The premultiplied RGBA at `x`, `y`.
    fn at(bitmap: &Bitmap, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * bitmap.width() + x) * 4) as usize;
        let p = bitmap.pixels();
        [p[i], p[i + 1], p[i + 2], p[i + 3]]
    }

    fn render(skin: BuiltIn, t: Target) -> Bitmap {
        skin.render(BASE, t).expect("rendered")
    }

    #[test]
    fn ids_round_trip() {
        for skin in BuiltIn::ALL {
            assert_eq!(BuiltIn::from_id(skin.id()), Some(skin));
        }
        assert_eq!(BuiltIn::from_id("custom"), None);
        assert_eq!(BuiltIn::from_id("Gloss"), None);
    }

    #[test]
    fn flat_is_the_base_colour_everywhere() {
        let bitmap = render(BuiltIn::Flat, target(40, 25, 1.0, Edge::Bottom));
        assert!(
            bitmap
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| *px == [0x2A, 0x2A, 0x2E, 255])
        );
        // The base colour's own alpha does not matter.
        let half = BuiltIn::Flat
            .render(BASE.with_alpha(0.5), target(2, 2, 1.0, Edge::Bottom))
            .expect("rendered");
        assert_eq!(at(&half, 0, 0), [0x2A, 0x2A, 0x2E, 255]);
    }

    #[test]
    fn gloss_lightens_the_upper_half_and_darkens_the_bottom_line() {
        let bitmap = render(BuiltIn::Gloss, target(10, 25, 1.0, Edge::Bottom));
        // The top row's centre is 0.5 px into the gradient: white at 29 %.
        assert_eq!(
            at(&bitmap, 0, 0)[0],
            (42.0 * 0.71 + 255.0 * 0.29_f32).round() as u8
        );
        // Lighter towards the top, plain base from the middle down.
        let column: Vec<u8> = (0..25).map(|y| at(&bitmap, 3, y)[0]).collect();
        assert!(column[..13].windows(2).all(|w| w[0] > w[1]), "{column:?}");
        assert_eq!(at(&bitmap, 0, 12), [0x2A, 0x2A, 0x2E, 255]);
        // The bottom line is darker, whatever the edge.
        assert_eq!(at(&bitmap, 0, 24)[0], (0x2A as f32 * 0.7).round() as u8);
        let top_bar = render(BuiltIn::Gloss, target(10, 25, 1.0, Edge::Top));
        assert_eq!(top_bar, bitmap);
        // At 200 % the line is two px.
        let large = render(BuiltIn::Gloss, target(10, 50, 2.0, Edge::Bottom));
        assert_eq!(at(&large, 0, 48), at(&large, 0, 49));
        assert!(at(&large, 0, 47)[0] > at(&large, 0, 48)[0]);
    }

    #[test]
    fn brushed_runs_from_light_to_dark_with_a_faint_texture() {
        let t = target(200, 25, 1.0, Edge::Bottom);
        let bitmap = render(BuiltIn::Brushed, t);
        let row_mean = |y: u32| {
            (0..t.width)
                .map(|x| f32::from(at(&bitmap, x, y)[0]))
                .sum::<f32>()
                / t.width as f32
        };
        assert!(row_mean(0) > row_mean(12) && row_mean(12) > row_mean(24));
        // The texture changes a pixel by at most 4 % of white or black.
        for y in 0..t.height {
            let fy = (y as f32 + 0.5) / 25.0;
            let plain = Px::opaque(BASE)
                .over(Color::WHITE, BRUSHED_LIGHT * (1.0 - fy))
                .over(Color::BLACK, BRUSHED_DARK * fy)
                .bytes()[0];
            for x in 0..t.width {
                let delta = (i32::from(at(&bitmap, x, y)[0]) - i32::from(plain)).abs();
                assert!(delta <= 11, "{x},{y}: {delta}");
            }
        }
        // Rows differ from each other, and the texture varies along a row.
        assert_ne!(row_mean(5), row_mean(6));
        let row: Vec<u8> = (0..t.width).map(|x| at(&bitmap, x, 5)[0]).collect();
        assert!(row.iter().any(|&v| v != row[0]));
        // The same on every render.
        assert_eq!(render(BuiltIn::Brushed, t), bitmap);
    }

    #[test]
    fn carbon_is_a_diagonal_checker() {
        let bitmap = render(BuiltIn::Carbon, target(16, 8, 1.0, Edge::Bottom));
        let plain = [0x2A, 0x2A, 0x2E, 255];
        let light = Px::opaque(BASE).over(Color::WHITE, CARBON_LIGHT).bytes();
        let values: Vec<[u8; 4]> = (0..8)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .map(|(x, y)| at(&bitmap, x, y))
            .collect();
        assert!(values.iter().all(|v| *v == plain || *v == light));
        // A checker board turned by 45°: diamonds 4 px across, plain ones centred on rows 0,
        // 4, …, light ones on rows 2, 6, … (cells that share an edge differ).
        assert_eq!(at(&bitmap, 1, 0), at(&bitmap, 2, 1));
        for (x, y) in [(2, 0), (6, 0), (2, 4)] {
            assert_eq!(at(&bitmap, x, y), plain, "{x},{y}");
        }
        for (x, y) in [(0, 2), (4, 2), (4, 6)] {
            assert_eq!(at(&bitmap, x, y), light, "{x},{y}");
        }
        // At 150 % the cells grow with the scale: 6 px across.
        let large = render(BuiltIn::Carbon, target(24, 12, 1.5, Edge::Bottom));
        assert_eq!(at(&large, 3, 0), plain);
        assert_eq!(at(&large, 6, 3), light);
        assert_eq!(at(&large, 3, 6), plain);
    }

    #[test]
    fn glass_is_translucent_with_a_highlight_towards_the_screen_centre() {
        let bottom = render(BuiltIn::Glass, target(4, 25, 1.0, Edge::Bottom));
        // The middle is the base colour at reduced alpha.
        assert_eq!(at(&bottom, 0, 12)[3], (GLASS_ALPHA * 255.0).round() as u8);
        // Brighter and more opaque at the edges (the glow), most at the highlighted one.
        assert!(at(&bottom, 0, 1)[3] > at(&bottom, 0, 12)[3]);
        assert!(at(&bottom, 0, 23)[3] > at(&bottom, 0, 12)[3]);
        assert!(at(&bottom, 0, 0)[0] > at(&bottom, 0, 24)[0] + 40);
        // On a top bar the highlight is at the bottom.
        let top = render(BuiltIn::Glass, target(4, 25, 1.0, Edge::Top));
        assert_eq!(at(&top, 0, 24), at(&bottom, 0, 0));
        assert_eq!(at(&top, 0, 0), at(&bottom, 0, 24));
        // Premultiplied: no channel above alpha.
        assert!(
            bottom
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| px[..3].iter().all(|&c| c <= px[3]))
        );
    }

    #[test]
    fn nothing_to_render_for_an_empty_target() {
        assert!(
            BuiltIn::Flat
                .render(BASE, target(0, 25, 1.0, Edge::Bottom))
                .is_none()
        );
    }
}
