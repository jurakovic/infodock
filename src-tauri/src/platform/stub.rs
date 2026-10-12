//! Portable fallback for systems without a native implementation yet (SPEC §2.2: only Windows
//! ships in v1). There is no bar window: the bar thread runs the handler for queued work, so the
//! rest of the app (tray, Settings, config) can be developed and tested on Linux.

use std::sync::{Arc, mpsc};

use super::{
    AppBar, BarHandler, BarThread, BarWindow, Capabilities, DockRequest, HandlerFactory,
    MonitorInfo, OsAppearance, Platform, PlatformError, UiEvent, UiWaker,
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
        waker: UiWaker::new(move || {
            let _ = wake_tx.send(());
        }),
    })
}
