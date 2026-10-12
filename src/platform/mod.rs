//! The OS layer (SPEC §4.6). Everything OS-specific sits behind these traits so other platforms
//! can be added without touching sensors. Windows lives in `windows/`; every other OS gets the
//! portable stub in `stub.rs` (enough to develop and test on Linux).
//!
//! The spec's `Shell`, `Icons`, `Power`, `Devices` and `Locale` traits are added with the
//! milestones that need them (M2–M3).

pub mod bitmap;
pub mod canvas;
pub mod geometry;
pub mod recording;
#[cfg(not(windows))]
mod stub;
pub mod text;
#[cfg(windows)]
mod windows;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use serde::Serialize;

use crate::config::model::{BarMode, Edge, Material, Settings, ZOrder};
pub use bitmap::{Bitmap, ImageError, decode_image, load_image};
pub use canvas::{Brush, Canvas, Color, GradientStop, PointF, RectF, Shape, SizeF};
pub use geometry::{MonitorInfo, Rect};
pub use text::{Font, TextEngine, TextLayout, TextStyle};

/// Which sensors and features exist on this system (SPEC §4.6). Flags turn on as their
/// implementations land.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub appbar: bool,
    pub recycle_bin: bool,
    pub start_menu: bool,
    pub wifi: bool,
    pub battery: bool,
    pub power_modes: Vec<String>,
    pub eject: bool,
    pub per_core_cpu: bool,
}

impl Capabilities {
    pub fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::RecycleBin => self.recycle_bin,
            Capability::StartMenu => self.start_menu,
            Capability::Wifi => self.wifi,
            Capability::Battery => self.battery,
            Capability::Eject => self.eject,
            Capability::PerCoreCpu => self.per_core_cpu,
        }
    }
}

/// A capability a sensor type can require (SPEC §4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    RecycleBin,
    StartMenu,
    Wifi,
    Battery,
    Eject,
    PerCoreCpu,
}

/// The OS appearance settings the bar follows (SPEC §10.3, §12.1–12.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OsAppearance {
    /// Apps use dark mode.
    pub dark: bool,
    /// "Transparency effects" is on; materials are disabled while it is off.
    pub transparency: bool,
    /// The user's accent colour.
    pub accent: Color,
    /// "Animation effects" is on (SPEC §5.5).
    pub animations: bool,
}

impl Default for OsAppearance {
    fn default() -> Self {
        Self {
            dark: true,
            transparency: true,
            // The Windows default accent.
            accent: Color::rgba(0x00, 0x78, 0xD4, 1.0),
            animations: true,
        }
    }
}

pub trait Platform: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    /// All active monitors, physical pixels.
    fn monitors(&self) -> Vec<MonitorInfo>;
    fn appearance(&self) -> OsAppearance;
    /// The time since the system started (the `uptime` topic, SPEC Appendix A).
    fn uptime(&self) -> std::time::Duration;
    /// The installed font families, sorted by name (SPEC §4.5: lists of fonts); none where
    /// the platform cannot list them.
    fn font_families(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Where and how the bar should sit (SPEC §5.1–5.3), derived from the settings.
#[derive(Debug, Clone, PartialEq)]
pub struct DockRequest {
    pub visible: bool,
    pub edge: Edge,
    pub mode: BarMode,
    /// `"primary"` or a [`MonitorInfo::key`].
    pub monitor: String,
    /// Logical px.
    pub height: u32,
    pub reserve_space: bool,
    pub z_order: ZOrder,
    pub hide_on_fullscreen: bool,
}

impl DockRequest {
    pub fn from_settings(s: &Settings, visible: bool) -> Self {
        Self {
            visible,
            edge: s.bar.edge,
            mode: s.bar.mode,
            monitor: s.bar.monitor.clone(),
            height: s.bar_height(),
            reserve_space: s.bar.reserve_space,
            z_order: s.bar.z_order,
            hide_on_fullscreen: s.bar.hide_on_fullscreen,
        }
    }
}

/// The bar's geometry after placement.
#[derive(Debug, Clone, PartialEq)]
pub struct BarGeometry {
    /// Physical pixels.
    pub rect: Rect,
    pub edge: Edge,
    pub monitor: MonitorInfo,
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppBarEvent {
    Placed(BarGeometry),
    /// A full-screen app opened (`true`) or closed on the bar's monitor (SPEC §5.3).
    Fullscreen(bool),
}

/// Receives AppBar events on the bar thread. Platform-internal: the bar window forwards them to
/// the handler as [`UiEvent`]s.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) type AppBarSink = Box<dyn Fn(AppBarEvent) + Send + Sync>;

/// Docks the bar window and reserves screen space (SPEC §5.2). Implementations are thread-safe;
/// changes are applied asynchronously on the window's UI thread.
pub trait AppBar: Send + Sync {
    /// Applies a new placement. Repeating the current request is cheap.
    fn configure(&self, request: DockRequest);
    /// Recomputes the placement, e.g. after the monitor layout changed.
    fn refresh(&self);
    /// Brings the bar to the front and focuses it. With z-order `bottom` it stays in front until
    /// it loses focus (SPEC §5.3).
    fn raise(&self);
    /// Releases the reserved screen space. Safe to call repeatedly and from the panic hook.
    fn release(&self);
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0}")]
    Os(String),
}

/// The platform for this OS.
pub fn current() -> Arc<dyn Platform> {
    #[cfg(windows)]
    return Arc::new(windows::WindowsPlatform);
    #[cfg(not(windows))]
    return Arc::new(stub::StubPlatform);
}

// ---------------------------------------------------------------------------------------------
// Drawing: text and images (SPEC §4.6)
// ---------------------------------------------------------------------------------------------

/// The text engine for this thread's windows: DirectWrite on Windows, fixed metrics elsewhere.
pub fn text_engine() -> Rc<dyn TextEngine> {
    #[cfg(windows)]
    match windows::text::DirectWrite::new() {
        Ok(engine) => return Rc::new(engine),
        Err(e) => tracing::error!("text rendering is unavailable: {e}"),
    }
    Rc::new(recording::FixedMetrics)
}

/// Decodes a raster image (everything but SVG); see [`decode_image`].
fn decode_raster(
    bytes: &[u8],
    format: bitmap::ImageFormat,
    height: Option<u32>,
) -> Result<Bitmap, ImageError> {
    #[cfg(windows)]
    return windows::image::decode(bytes, format, height);
    #[cfg(not(windows))]
    {
        let _ = (bytes, format, height);
        Err(ImageError::Unsupported)
    }
}

/// What a file or folder picker asks for (SPEC §4.6 `IFileOpenDialog`).
pub struct PathRequest<'a> {
    pub title: &'a str,
    /// Pick a folder instead of a file.
    pub folder: bool,
    /// The file types offered first, e.g. `["png", "jpg"]`, under `types_name`; "All files"
    /// (`all_files`) is always offered too.
    pub extensions: &'a [&'a str],
    pub types_name: &'a str,
    pub all_files: &'a str,
}

/// Shows the OS file or folder picker, modal to the native window `owner`, on the calling
/// thread. `None` if the user cancelled, or where there is no picker.
pub fn pick_path(owner: Option<isize>, request: &PathRequest) -> Option<String> {
    #[cfg(windows)]
    return windows::shell::pick_path(owner, request);
    #[cfg(not(windows))]
    {
        let _ = (owner, request);
        None
    }
}

/// Renders `width`×`height` physical px at `scale` with the bar's renderer, e.g. for previews
/// and the screenshots CI publishes (SPEC §12.4, §16). Windows only for now.
pub fn render_offscreen(
    width: u32,
    height: u32,
    scale: f32,
    paint: &mut dyn FnMut(&mut dyn Canvas),
) -> Result<Bitmap, PlatformError> {
    #[cfg(windows)]
    return windows::render::render_offscreen(width, height, scale, paint);
    #[cfg(not(windows))]
    {
        let _ = (width, height, scale, paint);
        Err(PlatformError::Os("no renderer on this OS".into()))
    }
}

// ---------------------------------------------------------------------------------------------
// The bar thread (SPEC §4.1)
// ---------------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------------
// App start-up: data folders and the single-instance lock (SPEC §11.1, §6.5)
// ---------------------------------------------------------------------------------------------

/// Where the app keeps its files (SPEC §11.1).
#[derive(Debug, Clone, PartialEq)]
pub struct AppDirs {
    /// Config and skin packs: `%APPDATA%\<identifier>` on Windows.
    pub config: PathBuf,
    /// Log files: `%LOCALAPPDATA%\<identifier>\logs` on Windows.
    pub logs: PathBuf,
}

/// The per-user folders for the app with this identifier.
pub fn app_dirs(identifier: &str) -> AppDirs {
    #[cfg(windows)]
    return windows::instance::app_dirs(identifier);
    #[cfg(not(windows))]
    return stub::app_dirs(identifier);
}

/// Held for the app's lifetime; dropping it lets another instance start.
pub struct InstanceGuard(#[allow(dead_code)] Box<dyn Send>);

impl InstanceGuard {
    fn new(inner: impl Send + 'static) -> Self {
        Self(Box::new(inner))
    }
}

/// The result of claiming the single-instance lock.
pub enum Instance {
    /// This is the only instance.
    Primary(InstanceGuard),
    /// Another instance runs already; ask it to show itself with [`activate_running_instance`].
    Secondary,
}

/// Claims the per-user single-instance lock. If the lock cannot be created, this instance runs
/// as the primary one.
pub fn single_instance(identifier: &str) -> Instance {
    #[cfg(windows)]
    return windows::instance::single_instance(identifier);
    #[cfg(not(windows))]
    return stub::single_instance(identifier);
}

/// Asks the running instance to show the bar and open Settings (SPEC §6.5). Waits up to a few
/// seconds for its bar window to exist.
pub fn activate_running_instance() {
    #[cfg(windows)]
    windows::instance::activate_running_instance();
}

/// Events for the bar logic, delivered on the bar thread.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    /// The bar window exists; sent once before any other event.
    Started,
    /// Work was queued for the bar thread ([`UiWaker::wake`]).
    Wake,
    /// The bar was placed or moved.
    Placed(BarGeometry),
    /// A full-screen app opened (`true`) or closed on the bar's monitor (SPEC §5.3).
    Fullscreen(bool),
    /// Light/dark mode, accent colour or transparency effects may have changed.
    AppearanceChanged,
    /// The OS asks the app to close (graceful `taskkill`, Restart Manager).
    CloseRequested,
    /// A second launch asked this instance to show itself (SPEC §6.5).
    Activate,
    /// The tray icon was clicked or selected with the keyboard (SPEC §6.5).
    TrayClick,
    /// The tray icon's menu was requested (right click, menu key).
    TrayMenu,
    /// A timer set with [`BarWindow::set_timer`] is due.
    Timer(TimerId),
    /// The pointer moved over the window (or anywhere while captured); logical px from the
    /// window's top-left corner.
    PointerMove(PointF),
    /// The pointer left the window.
    PointerLeave,
    PointerDown {
        button: PointerButton,
        at: PointF,
        modifiers: Modifiers,
    },
    PointerUp {
        button: PointerButton,
        at: PointF,
        modifiers: Modifiers,
    },
    /// Another window took the pointer capture.
    CaptureLost,
}

/// The bar logic's timers; each runs independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerId {
    /// The sensors' earliest wake-up (SPEC §4.4).
    Wake,
    /// The hint's show or hide delay (SPEC §7.1).
    Hint,
    /// Polls Esc while a reorder drag runs (SPEC §6.2).
    Drag,
    /// The next frame of a bar animation (SPEC §5.5).
    Frame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

/// Modifier keys held during an input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

/// An entry of a native menu: the tray menu, and the context menus of SPEC §6.4.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuEntry {
    Item {
        id: u32,
        label: String,
        checked: bool,
        enabled: bool,
    },
    Separator,
    Submenu {
        label: String,
        entries: Vec<MenuEntry>,
    },
}

impl MenuEntry {
    pub fn item(id: u32, label: impl Into<String>) -> Self {
        Self::Item {
            id,
            label: label.into(),
            checked: false,
            enabled: true,
        }
    }

    pub fn check(id: u32, label: impl Into<String>, checked: bool) -> Self {
        Self::Item {
            id,
            label: label.into(),
            checked,
            enabled: true,
        }
    }

    /// The same item, greyed out unless `enabled`.
    pub fn enabled(mut self, enabled: bool) -> Self {
        if let Self::Item { enabled: e, .. } = &mut self {
            *e = enabled;
        }
        self
    }

    pub fn submenu(label: impl Into<String>, entries: Vec<MenuEntry>) -> Self {
        Self::Submenu {
            label: label.into(),
            entries,
        }
    }
}

/// The bar logic. Called by the native bar window, always on the bar thread.
pub trait BarHandler {
    fn on_event(&mut self, window: &dyn BarWindow, event: UiEvent);
    /// Draws the bar into a cleared canvas of `size` logical px.
    fn paint(&mut self, canvas: &mut dyn Canvas, size: SizeF);
}

/// The native bar window as the bar logic sees it (bar thread only).
pub trait BarWindow {
    /// Schedules a repaint.
    fn invalidate(&self);
    /// Applies a window material; [`Material::None`] removes it.
    fn set_material(&self, material: Material);
    /// Shows the notification-area icon with this tooltip, or removes it with `None`.
    fn set_tray_icon(&self, tooltip: Option<&str>);
    /// Shows a menu at the pointer and returns the chosen item's id. Runs the menu's modal loop;
    /// events that arrive meanwhile are delivered after the current handler call.
    fn show_menu(&self, entries: &[MenuEntry]) -> Option<u32>;
    /// Delivers [`UiEvent::Timer`] once after `delay` (it may come a few ms early), replacing the
    /// previous timer with this id; `None` cancels it.
    fn set_timer(&self, id: TimerId, delay: Option<std::time::Duration>);
    /// Shows the hint window (SPEC §7.1) at `rect` (physical screen px) with what `paint` draws
    /// at `scale`: fading in when it was hidden, updated in place when it was shown.
    fn show_hint(&self, rect: Rect, scale: f32, paint: &mut dyn FnMut(&mut dyn Canvas));
    fn hide_hint(&self);
    /// While captured, pointer events keep coming when the pointer leaves the window.
    fn capture_pointer(&self, capture: bool);
    /// Whether Esc is held now, read without focus (the bar never takes it; SPEC §6.2).
    fn escape_down(&self) -> bool;
    /// Asks a yes/no question in a native dialog (SPEC §6.4): `message` as the main text and
    /// `action` on the confirming button. Returns whether the user confirmed.
    fn confirm(&self, message: &str, action: &str) -> bool;
}

/// Removes the tray icon from any thread, e.g. on exit.
pub trait TrayIcon: Send + Sync {
    fn remove(&self);
}

/// Wakes the bar thread from any thread; its handler then receives [`UiEvent::Wake`]. Wakes
/// that arrive before the handler ran are coalesced.
#[derive(Clone)]
pub struct UiWaker(Arc<dyn Fn() + Send + Sync>);

impl UiWaker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(wake))
    }

    pub fn wake(&self) {
        (self.0)()
    }
}

/// Builds the bar logic on the bar thread.
pub type HandlerFactory = Box<dyn FnOnce() -> Box<dyn BarHandler> + Send>;

/// Handles to a running bar thread, usable from any thread.
pub struct BarThread {
    pub appbar: Arc<dyn AppBar>,
    pub tray: Arc<dyn TrayIcon>,
    pub waker: UiWaker,
}

/// Starts the bar thread: creates the bar window and its AppBar, builds the handler with
/// `make_handler` on that thread and runs the message loop. Returns once the window exists; the
/// window becomes visible with the first [`AppBar::configure`].
pub fn start_bar_thread(make_handler: HandlerFactory) -> Result<BarThread, PlatformError> {
    #[cfg(windows)]
    return windows::window::start_bar_thread(make_handler);
    #[cfg(not(windows))]
    return stub::start_bar_thread(make_handler);
}
