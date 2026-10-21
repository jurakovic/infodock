//! The interactive popup host (SPEC §7.2): which sensor's popup is open, where its frame is,
//! and the 500 ms during which a sensor whose popup has just closed ignores clicks. The bar
//! logic (`thread.rs`) opens, feeds and closes it; the platform's popup window shows what
//! [`paint`] draws.

use std::time::{Duration, Instant};

use super::hint::{self, HintFrame};
use super::placement::Anchor;
use crate::look::theme::Theme;
use crate::platform::{Canvas, PointF, PopupInput, SizeF};
use crate::sensors::popup::{Popup, PopupAlign};

/// A sensor whose popup has just closed ignores clicks this long, so the click that closed it
/// does not open it again (SPEC §7.2).
pub const GUARD: Duration = Duration::from_millis(500);
/// Space between the bubble's edge and the popup's content.
pub const PADDING: f32 = 4.0;
/// Space kept from the monitor's edges (SPEC §7.3).
const MARGIN: f32 = 7.0;

/// The most room a popup's content may take next to `anchor`: the monitor's width, and the
/// height on the popup's side of the bar, less margins, pointer, shadow and padding.
pub fn room(anchor: &Anchor) -> SizeF {
    let m = anchor.monitor;
    let beside = match anchor.edge {
        crate::config::model::Edge::Bottom => anchor.bar.top - m.top,
        crate::config::model::Edge::Top => m.bottom - anchor.bar.bottom,
    };
    let frame = 2.0 * PADDING + hint::POINTER.height + hint::SHADOW + MARGIN;
    SizeF::new(
        (m.width() - 2.0 * (MARGIN + PADDING)).max(0.0),
        (beside - frame).max(0.0),
    )
}

/// The widest content that fits on the monitor beside `anchor` with its left edge at the
/// sensor's (or, with `free_left`, its right edge at the sensor's), logical px: the room of a
/// popup whose grip is dragged, so it keeps its side.
pub fn room_beside(anchor: &Anchor, free_left: bool) -> f32 {
    let m = anchor.monitor;
    let reach = if free_left {
        anchor.rect.right - (m.left + MARGIN)
    } else {
        m.right - MARGIN - anchor.rect.left
    };
    (reach - 2.0 * PADDING).max(0.0)
}

/// An open popup.
pub struct Open {
    /// The sensor that opened it.
    pub owner: String,
    pub popup: Box<dyn Popup>,
    /// Where it is; `None` until it was laid out and placed.
    pub frame: Option<HintFrame>,
    /// The room it was laid out in.
    pub room: SizeF,
    /// The monitor's height, logical px.
    pub monitor_height: f32,
    /// Its left edge is the free one (its right edge is lined up with the sensor).
    pub free_left: bool,
    /// Whether the window captures the pointer for it.
    pub captured: bool,
}

impl Open {
    /// The content's top-left corner in the window, logical px.
    pub fn origin(&self) -> Option<PointF> {
        self.frame
            .map(|f| PointF::new(f.bubble.left + PADDING, f.bubble.top + PADDING))
    }

    /// `input` in window px as the content sees it, from its top-left corner.
    pub fn to_content(&self, input: PopupInput) -> PopupInput {
        let Some(o) = self.origin() else {
            return input;
        };
        let shift = |at: PointF| PointF::new(at.x - o.x, at.y - o.y);
        match input {
            PopupInput::PointerMove(at) => PopupInput::PointerMove(shift(at)),
            PopupInput::PointerDown {
                button,
                at,
                modifiers,
            } => PopupInput::PointerDown {
                button,
                at: shift(at),
                modifiers,
            },
            PopupInput::PointerUp {
                button,
                at,
                modifiers,
            } => PopupInput::PointerUp {
                button,
                at: shift(at),
                modifiers,
            },
            PopupInput::Wheel { lines, at } => PopupInput::Wheel {
                lines,
                at: shift(at),
            },
            other => other,
        }
    }

    /// Frames content of `size` at `anchor`. While the popup captures the pointer (a grip is
    /// dragged), it stays on the side it was lined up on.
    pub fn place(&mut self, size: SizeF, anchor: &Anchor, theme: &Theme) {
        let mut align = self.popup.align();
        if self.captured && self.free_left && align == PopupAlign::LeftEdge {
            align = PopupAlign::RightEdge;
        }
        let frame = HintFrame::framed(size, anchor, theme, PADDING, align);
        let left = frame.window.left + frame.bubble.left;
        // Lined up on the left unless its right edge had to be.
        self.free_left = matches!(align, PopupAlign::LeftEdge | PopupAlign::RightEdge)
            && (left - anchor.rect.left).abs() > 0.5;
        self.monitor_height = anchor.monitor.height();
        self.frame = Some(frame);
    }
}

/// The popup state of the bar.
#[derive(Default)]
pub struct PopupHost {
    pub open: Option<Open>,
    /// The sensor whose popup closed last, and when.
    closed: Option<(String, Instant)>,
}

impl PopupHost {
    /// The sensor whose popup is open.
    pub fn owner(&self) -> Option<&str> {
        self.open.as_ref().map(|o| o.owner.as_str())
    }

    /// Whether a click on sensor `id` at `now` is ignored: its popup closed less than
    /// [`GUARD`] ago.
    pub fn guarded(&self, id: &str, now: Instant) -> bool {
        self.closed
            .as_ref()
            .is_some_and(|(owner, at)| owner == id && now.saturating_duration_since(*at) < GUARD)
    }

    /// Takes the open popup out, starting its sensor's guard at `now`.
    pub fn close(&mut self, now: Instant) -> Option<Open> {
        let open = self.open.take()?;
        self.closed = Some((open.owner.clone(), now));
        Some(open)
    }
}

/// Draws the popup's frame and content into a cleared canvas the size of its window.
pub fn paint(canvas: &mut dyn Canvas, open: &Open, theme: &Theme) {
    let (Some(frame), Some(origin)) = (open.frame, open.origin()) else {
        return;
    };
    hint::paint_bubble(canvas, &frame, theme);
    open.popup.draw(canvas, origin, theme);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{Edge, Settings};
    use crate::platform::{Modifiers, OsAppearance, PointerButton, RectF};
    use crate::sensors::popup::{PopupAlign, PopupCtx, PopupReply};

    struct Square;

    impl Popup for Square {
        fn kind(&self) -> &'static str {
            "square"
        }
        fn align(&self) -> PopupAlign {
            PopupAlign::LeftEdge
        }
        fn layout(&mut self, _cx: &PopupCtx) -> SizeF {
            SizeF::new(100.0, 50.0)
        }
        fn draw(&self, _canvas: &mut dyn Canvas, _origin: PointF, _theme: &Theme) {}
        fn input(&mut self, _input: &PopupInput, _cx: &PopupCtx) -> PopupReply {
            PopupReply::Nothing
        }
    }

    #[test]
    fn a_closed_popup_guards_its_sensor_for_a_moment() {
        let mut host = PopupHost {
            open: Some(Open {
                owner: "f".into(),
                popup: Box::new(Square),
                frame: None,
                room: SizeF::new(0.0, 0.0),
                monitor_height: 0.0,
                free_left: false,
                captured: false,
            }),
            ..PopupHost::default()
        };
        assert_eq!(host.owner(), Some("f"));
        let t0 = Instant::now();
        assert!(host.close(t0).is_some());
        assert_eq!(host.owner(), None);
        assert!(host.guarded("f", t0 + Duration::from_millis(499)));
        assert!(!host.guarded("other", t0));
        assert!(!host.guarded("f", t0 + GUARD));
        assert!(host.close(t0).is_none(), "nothing open");
    }

    #[test]
    fn input_reaches_the_content_from_its_corner() {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        let mut open = Open {
            owner: "f".into(),
            popup: Box::new(Square),
            frame: None,
            room: SizeF::new(0.0, 0.0),
            monitor_height: 0.0,
            free_left: false,
            captured: false,
        };
        let bar = RectF::new(0.0, 1052.0, 1920.0, 1080.0);
        let anchor = Anchor {
            rect: RectF::new(900.0, bar.top, 960.0, bar.bottom),
            bar,
            monitor: RectF::new(0.0, 0.0, 1920.0, 1080.0),
            edge: Edge::Bottom,
        };
        // Above the bar: 1052 px less margins, pointer, shadow and padding.
        assert_eq!(room(&anchor), SizeF::new(1898.0, 1024.0));
        open.place(SizeF::new(100.0, 50.0), &anchor, &theme);
        let frame = open.frame.expect("placed");
        // Left edges lined up: the window starts a shadow's width further left.
        assert_eq!(frame.window.left + 6.0, 900.0);
        assert!(!open.free_left);
        assert_eq!(open.monitor_height, 1080.0);
        // The bubble is above the pointer, with the shadow around it.
        let origin = open.origin().expect("placed");
        assert_eq!(origin, PointF::new(10.0, 10.0));
        let down = PopupInput::PointerDown {
            button: PointerButton::Left,
            at: PointF::new(15.0, 30.0),
            modifiers: Modifiers::default(),
        };
        assert_eq!(
            open.to_content(down),
            PopupInput::PointerDown {
                button: PointerButton::Left,
                at: PointF::new(5.0, 20.0),
                modifiers: Modifiers::default(),
            }
        );
    }

    #[test]
    fn a_popup_being_resized_keeps_its_side() {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        let mut open = Open {
            owner: "f".into(),
            popup: Box::new(Square),
            frame: None,
            room: SizeF::new(0.0, 0.0),
            monitor_height: 0.0,
            free_left: false,
            captured: false,
        };
        let bar = RectF::new(0.0, 1052.0, 1920.0, 1080.0);
        let anchor = Anchor {
            rect: RectF::new(1700.0, bar.top, 1721.0, bar.bottom),
            bar,
            monitor: RectF::new(0.0, 0.0, 1920.0, 1080.0),
            edge: Edge::Bottom,
        };
        let left = |open: &Open| {
            let frame = open.frame.expect("placed");
            frame.window.left + frame.bubble.left
        };
        // Left-aligned while it fits; while its grip is dragged, no wider than that side.
        open.place(SizeF::new(100.0, 50.0), &anchor, &theme);
        assert_eq!(left(&open), 1700.0);
        assert_eq!(room_beside(&anchor, false), 1920.0 - 7.0 - 1700.0 - 8.0);
        // Too wide to fit: right-aligned; dragged narrower, it stays so.
        open.place(SizeF::new(400.0, 50.0), &anchor, &theme);
        assert!(open.free_left);
        open.captured = true;
        open.place(SizeF::new(150.0, 50.0), &anchor, &theme);
        assert!(open.free_left);
        assert_eq!(left(&open) + 150.0 + 8.0, 1721.0, "right edges lined up");
        // Released: lined up on the left again.
        open.captured = false;
        open.place(SizeF::new(150.0, 50.0), &anchor, &theme);
        assert!(!open.free_left);
        assert_eq!(room_beside(&anchor, true), 1721.0 - 7.0 - 8.0);
    }
}
