//! The OS layer (SPEC §4.6). Everything OS-specific sits behind these traits so other platforms
//! can be added without touching sensors. Windows lives in `windows/`; every other OS gets the
//! portable stub in `stub.rs` (enough to develop and test on Linux).
//!
//! The spec's `Shell`, `Icons`, `Power`, `Devices` and `Locale` traits are added with the
//! milestones that need them (M2–M3).

pub mod canvas;
pub mod geometry;
#[cfg(not(windows))]
mod stub;
#[cfg(windows)]
mod windows;

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use crate::config::model::{BarMode, Edge, Material, Settings, ZOrder};
pub use canvas::{Canvas, Color, RectF, SizeF};
pub use geometry::{MonitorInfo, Rect};

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

/// The OS appearance settings the bar follows (SPEC §10.3, §12.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OsAppearance {
    /// Apps use dark mode.
    pub dark: bool,
    /// "Transparency effects" is on; materials are disabled while it is off.
    pub transparency: bool,
}

impl Default for OsAppearance {
    fn default() -> Self {
        Self {
            dark: true,
            transparency: true,
        }
    }
}

pub trait Platform: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    /// All active monitors, physical pixels.
    fn monitors(&self) -> Vec<MonitorInfo>;
    fn appearance(&self) -> OsAppearance;
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
