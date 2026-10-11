//! Portable fallback for systems without a native implementation yet (SPEC §2.2: only Windows
//! ships in v1). Places the bar with Tauri's window API and reserves no screen space.
//!
//! Known limitation: on Linux, GTK keeps a mapped WebKitGTK window at least 200 px tall, so a
//! live move (e.g. bottom → top) leaves the bar taller than requested until restart.

use std::sync::{Arc, Mutex, PoisonError};

use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

use super::geometry::{Rect, dock_strip, select_monitor, to_physical};
use super::{
    AppBar, AppBarEvent, AppBarSink, BarGeometry, Capabilities, DockRequest, MonitorInfo, Platform,
    PlatformError,
};

pub struct StubPlatform;

impl Platform for StubPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        Vec::new()
    }

    fn transparency_effects(&self) -> bool {
        true
    }
}

fn tauri_monitors(window: &WebviewWindow) -> Vec<MonitorInfo> {
    let primary = window
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| *m.position());
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let (pos, size) = (m.position(), m.size());
            let to_i32 = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
            let bounds = Rect::new(
                pos.x,
                pos.y,
                pos.x + to_i32(size.width),
                pos.y + to_i32(size.height),
            );
            let name = m
                .name()
                .cloned()
                .unwrap_or_else(|| format!("Monitor {}", i + 1));
            let device = format!("monitor{i}");
            MonitorInfo::new(
                device,
                name,
                bounds,
                bounds,
                m.scale_factor(),
                Some(*pos) == primary,
            )
        })
        .collect()
}

struct StubAppBar {
    window: WebviewWindow,
    sink: AppBarSink,
    request: Mutex<Option<DockRequest>>,
}

impl StubAppBar {
    fn apply(&self) {
        let Some(req) = self
            .request
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        else {
            return;
        };
        if !req.visible {
            let _ = self.window.hide();
            return;
        }
        let monitors = tauri_monitors(&self.window);
        let Some(monitor) = select_monitor(&monitors, &req.monitor) else {
            return;
        };
        let rect = dock_strip(
            monitor.work_area,
            req.edge,
            to_physical(req.height, monitor.scale),
        );
        let _ = self.window.set_size(PhysicalSize::new(
            rect.width().max(1) as u32,
            rect.height().max(1) as u32,
        ));
        let _ = self
            .window
            .set_position(PhysicalPosition::new(rect.left, rect.top));
        let _ = self.window.show();
        (self.sink)(AppBarEvent::Placed(BarGeometry {
            rect,
            edge: req.edge,
            monitor: monitor.clone(),
            scale: monitor.scale,
        }));
    }
}

impl AppBar for StubAppBar {
    fn configure(&self, request: DockRequest) {
        *self.request.lock().unwrap_or_else(PoisonError::into_inner) = Some(request);
        self.apply();
    }

    fn refresh(&self) {
        self.apply();
    }

    fn raise(&self) {
        let _ = self.window.show();
        let _ = self.window.set_focus();
    }

    fn release(&self) {}
}

pub fn attach_appbar(
    window: &WebviewWindow,
    sink: AppBarSink,
) -> Result<Arc<dyn AppBar>, PlatformError> {
    Ok(Arc::new(StubAppBar {
        window: window.clone(),
        sink,
        request: Mutex::new(None),
    }))
}
