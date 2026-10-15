//! Interactive popups (SPEC §7.2): a sensor's click can open one (Folder Menu, Start Menu,
//! Eject USB). A popup lays itself out, draws itself and handles its own input inside the
//! shared `popup` window, which the bar frames (bubble, pointer, shadow), places next to the
//! sensor and shows; it closes on blur, Esc, a second click on its sensor, or when it asks to.

use std::sync::Arc;
use std::time::Instant;

use super::{IconSource, Props};
use crate::bar::theme::Theme;
use crate::config::model::{Edge, Settings};
use crate::platform::{
    Canvas, Platform, PointF, PopupCursor, PopupInput, SizeF, TextEngine, UiWaker,
};

/// How a popup lines up with its sensor (SPEC §7.3, §9.19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAlign {
    /// Centred over the sensor, as hints are.
    Center,
    /// Its left edge at the sensor's left edge; its right edge at the sensor's right edge when
    /// that would overflow the monitor.
    LeftEdge,
}

/// What an input or a wake-up changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupReply {
    Nothing,
    /// Draw again; the size stays.
    Redraw,
    /// Lay out again (the size may change), then draw.
    Relayout,
    /// Close the popup, e.g. after opening a file.
    Close,
}

/// What a popup may use (bar thread).
pub struct PopupCtx<'a> {
    pub theme: &'a Theme,
    pub settings: &'a Settings,
    pub text: &'a dyn TextEngine,
    /// Physical px per logical px.
    pub scale: f32,
    /// Shared, so a popup's own workers can use it too.
    pub platform: &'a Arc<dyn Platform>,
    /// Shell icons, read on the bar's worker; a popup gets *loading* and is laid out again
    /// when they arrive.
    pub icons: &'a dyn IconSource,
    /// Wakes the bar thread from the popup's own workers, which then calls [`Popup::poll`].
    pub waker: &'a UiWaker,
    /// The most room the content may take, logical px (the monitor beside the bar).
    pub room: SizeF,
    /// The monitor's height, logical px.
    pub monitor_height: f32,
    /// The bar's edge: the popup is above a bottom bar and below a top one.
    pub edge: Edge,
    /// The popup's left edge is the free one, not lined up with the sensor (its right edge is,
    /// SPEC §9.19); otherwise its right edge is.
    pub free_left: bool,
    /// Transitions animate (SPEC §5.5); otherwise they jump.
    pub animate: bool,
    pub now: Instant,
}

/// An open popup. Lives on the bar thread, from the click that opens it until it closes.
pub trait Popup {
    /// Its kind, e.g. `folder-browser`.
    fn kind(&self) -> &'static str;
    fn align(&self) -> PopupAlign {
        PopupAlign::Center
    }
    /// Lays the content out; returns its size in logical px, at most `cx.room`.
    fn layout(&mut self, cx: &PopupCtx) -> SizeF;
    /// Draws the laid-out content with its top-left corner at `origin`.
    fn draw(&self, canvas: &mut dyn Canvas, origin: PointF, theme: &Theme);
    /// Input, in logical px from the content's top-left corner. Esc never arrives: it closes
    /// the popup.
    fn input(&mut self, input: &PopupInput, cx: &PopupCtx) -> PopupReply;
    /// When it next needs [`Popup::on_wake`]: an animation frame, or a deadline.
    fn wake_at(&self) -> Option<Instant> {
        None
    }
    fn on_wake(&mut self, _cx: &PopupCtx) -> PopupReply {
        PopupReply::Nothing
    }
    /// The bar thread was woken: work from the popup's own threads may have arrived.
    fn poll(&mut self, _cx: &PopupCtx) -> PopupReply {
        PopupReply::Nothing
    }
    /// The pointer's shape, e.g. over a resize grip; asked after each input.
    fn cursor(&self) -> PopupCursor {
        PopupCursor::Arrow
    }
    /// Whether its pointer events should keep coming while the pointer is outside it (a grip
    /// being dragged); asked after each input.
    fn captures(&self) -> bool {
        false
    }
    /// Props of its sensor to save, e.g. a size the user dragged (SPEC §9.19); each change is
    /// taken once.
    fn take_props(&mut self) -> Option<Props> {
        None
    }
}
