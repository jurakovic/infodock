//! Portable fallback for systems without a native implementation yet (SPEC §2.2: only Windows
//! ships in v1). There is no bar window: the bar thread runs the handler for queued work, so the
//! rest of the app (tray, Settings, config) can be developed and tested on Linux.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use super::{
    AppBar, AppDirs, BarHandler, BarThread, BarWindow, Capabilities, DockRequest, HandlerFactory,
    Instance, InstanceGuard, MenuEntry, MonitorInfo, OsAppearance, Platform, PlatformError,
    TrayIcon, UiEvent, UiWaker,
};
use crate::config::model::Material;

pub struct StubPlatform;

impl Platform for StubPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        Vec::new()
    }

    fn appearance(&self) -> OsAppearance {
        OsAppearance::default()
    }
}

struct StubAppBar;

impl AppBar for StubAppBar {
    fn configure(&self, request: DockRequest) {
        tracing::debug!("no native bar on this OS; ignoring {request:?}");
    }

    fn refresh(&self) {}

    fn raise(&self) {}

    fn release(&self) {}
}

struct NoWindow;

impl BarWindow for NoWindow {
    fn invalidate(&self) {}

    fn set_material(&self, _material: Material) {}

    fn set_tray_icon(&self, _tooltip: Option<&str>) {}

    fn show_menu(&self, _entries: &[MenuEntry]) -> Option<u32> {
        None
    }
}

struct NoTray;

impl TrayIcon for NoTray {
    fn remove(&self) {}
}

pub fn start_bar_thread(make_handler: HandlerFactory) -> Result<BarThread, PlatformError> {
    let (wake_tx, wake_rx) = mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("bar".into())
        .spawn(move || {
            let mut handler: Box<dyn BarHandler> = make_handler();
            handler.on_event(&NoWindow, UiEvent::Started);
            while wake_rx.recv().is_ok() {
                handler.on_event(&NoWindow, UiEvent::Wake);
            }
        })
        .map_err(|e| PlatformError::Os(format!("could not start the bar thread: {e}")))?;
    tracing::info!("no native bar on this OS; the bar is not shown");
    Ok(BarThread {
        appbar: Arc::new(StubAppBar),
        tray: Arc::new(NoTray),
        waker: UiWaker::new(move || {
            let _ = wake_tx.send(());
        }),
    })
}

/// XDG base directories, falling back to `~/.config` and `~/.local/share`.
pub fn app_dirs(identifier: &str) -> AppDirs {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let base = |var: &str, fallback: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(fallback))
    };
    AppDirs {
        config: base("XDG_CONFIG_HOME", ".config").join(identifier),
        logs: base("XDG_DATA_HOME", ".local/share")
            .join(identifier)
            .join("logs"),
    }
}

/// No lock: development builds may run side by side.
pub fn single_instance(_identifier: &str) -> Instance {
    Instance::Primary(InstanceGuard::new(()))
}
