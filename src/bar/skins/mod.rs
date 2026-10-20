//! The classic bar's background (SPEC §12.3–12.4): a skin drawn into a bitmap of the bar's
//! physical size, which the bar draws with the background opacity, so the opacity applies to the
//! skin only. The bitmap is kept until the skin, its colour, the bar's size or its edge change.

mod builtin;

pub use builtin::BuiltIn;

use crate::config::model::Edge;
use crate::platform::{Bitmap, Color, SizeF};

/// The skin used when the configured one is not on this machine (SPEC §12.4).
pub const FALLBACK: BuiltIn = BuiltIn::Gloss;
/// `classic.skin` for the user's own image, `classic.customSkin`.
pub const CUSTOM: &str = "custom";

/// The classic bar's skin as the settings choose it (SPEC §10.3).
#[derive(Debug, Clone, PartialEq)]
pub struct SkinSpec {
    /// `classic.skin`: a built-in skin's id or [`CUSTOM`].
    pub id: String,
    /// `classic.customSkin`.
    pub custom_image: String,
    /// The built-in skins' base colour: `background.color`, or the theme's; opaque.
    pub base: Color,
}

/// What a skin is drawn for: the bar's size in physical px, its scale and the edge it is
/// placed on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub edge: Edge,
}

impl Target {
    /// A bar of `size` logical px at `scale`, in the whole physical px its window has.
    pub fn of(size: SizeF, scale: f32, edge: Edge) -> Self {
        Self {
            width: (size.width * scale).round() as u32,
            height: (size.height * scale).round() as u32,
            scale,
            edge,
        }
    }
}

/// The skin `spec` chooses, drawn for `target`; `None` for an empty bar.
pub fn render(spec: &SkinSpec, target: Target) -> Option<Bitmap> {
    // The custom image arrives with the skin packs; until then it is Flat, as without a file.
    let skin = if spec.id == CUSTOM {
        BuiltIn::Flat
    } else {
        BuiltIn::from_id(&spec.id).unwrap_or(FALLBACK)
    };
    skin.render(spec.base, target)
}

/// The bar's rendered skin, kept between frames.
#[derive(Default)]
pub struct SkinCache {
    drawn: Option<(SkinSpec, Target, Option<Bitmap>)>,
}

impl SkinCache {
    /// The skin for `spec` and `target`, rendered again only when either changed.
    pub fn get(&mut self, spec: &SkinSpec, target: Target) -> Option<&Bitmap> {
        let fresh = matches!(&self.drawn, Some((s, t, _)) if s == spec && *t == target);
        if !fresh {
            self.drawn = Some((spec.clone(), target, render(spec, target)));
        }
        self.drawn
            .as_ref()
            .and_then(|(_, _, bitmap)| bitmap.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str) -> SkinSpec {
        SkinSpec {
            id: id.into(),
            custom_image: String::new(),
            base: Color::rgba(0x2A, 0x2A, 0x2E, 1.0),
        }
    }

    #[test]
    fn target_is_the_window_in_physical_px() {
        let t = Target::of(SizeF::new(800.0, 25.0), 1.5, Edge::Top);
        assert_eq!((t.width, t.height), (1200, 38));
    }

    #[test]
    fn unknown_skins_fall_back_to_gloss_and_custom_to_flat() {
        let t = Target::of(SizeF::new(20.0, 25.0), 1.0, Edge::Bottom);
        let gloss = BuiltIn::Gloss.render(spec("").base, t);
        assert_eq!(render(&spec("gloss"), t), gloss);
        assert_eq!(render(&spec("my-pack"), t), gloss);
        assert_eq!(
            render(&spec(CUSTOM), t),
            BuiltIn::Flat.render(spec("").base, t)
        );
        assert_eq!(
            render(&spec("carbon"), t),
            BuiltIn::Carbon.render(spec("").base, t)
        );
    }

    #[test]
    fn cache_renders_again_only_after_a_change() {
        let mut cache = SkinCache::default();
        let t = Target::of(SizeF::new(20.0, 25.0), 1.0, Edge::Bottom);
        let first = cache.get(&spec("gloss"), t).cloned().expect("rendered");
        let again = cache.get(&spec("gloss"), t).cloned().expect("rendered");
        // The very same pixels, so the renderer's upload cache keeps its copy.
        assert!(std::sync::Arc::ptr_eq(
            first.shared_pixels(),
            again.shared_pixels()
        ));
        let wider = Target { width: 30, ..t };
        assert_eq!(
            cache.get(&spec("gloss"), wider).map(Bitmap::width),
            Some(30)
        );
        let flat = cache.get(&spec("flat"), wider).cloned().expect("rendered");
        assert_ne!(
            flat,
            BuiltIn::Gloss.render(spec("").base, wider).expect("x")
        );
        let empty = Target { width: 0, ..t };
        assert!(cache.get(&spec("flat"), empty).is_none());
    }
}
