//! The OS layer (SPEC §4.6). Everything OS-specific sits behind these traits so other platforms
//! can be added without touching sensors. Windows lives in `windows/`; every other OS gets the
//! portable stub in `stub.rs` (enough to develop and test on Linux).
//!
//! The spec's `Shell`, `Icons`, `Power` and `Devices` traits are added with the milestones that
//! need them (M2–M3).

pub mod geometry;
#[cfg(not(windows))]
mod stub;
#[cfg(windows)]
mod windows;

use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;

use crate::config::model::{BarMode, Edge, Settings, ZOrder};
pub use geometry::{MonitorInfo, Rect};

/// Which sensors and features exist on this system (SPEC §4.6). Flags turn on as their
/// implementations land.
#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

pub trait Platform: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    /// All active monitors, physical pixels.
    fn monitors(&self) -> Vec<MonitorInfo>;
    /// The OS "transparency effects" setting; materials are disabled while it is off (SPEC §10.3).
    fn transparency_effects(&self) -> bool;
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

/// The bar's geometry after placement, sent to the UI as `bar-geometry` (SPEC §4.5).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

pub type AppBarSink = Box<dyn Fn(AppBarEvent) + Send + Sync>;

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
    #[error("{0}")]
    Tauri(#[from] tauri::Error),
}

/// The platform for this OS.
pub fn current() -> Arc<dyn Platform> {
    #[cfg(windows)]
    return Arc::new(windows::WindowsPlatform);
    #[cfg(not(windows))]
    return Arc::new(stub::StubPlatform);
}

/// Takes over placement of the bar window. Call on the UI (main) thread, before the window is
/// shown; the window becomes visible once the first placement is done.
pub fn attach_appbar(
    window: &tauri::WebviewWindow,
    sink: AppBarSink,
) -> Result<Arc<dyn AppBar>, PlatformError> {
    #[cfg(windows)]
    return windows::appbar::attach(window, sink);
    #[cfg(not(windows))]
    return stub::attach_appbar(window, sink);
}
