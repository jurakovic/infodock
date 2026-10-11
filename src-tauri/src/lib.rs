//! Infodock core: app builder, windows and plugins (SPEC §4.2).

mod commands;
pub mod config;
mod error;

use tauri::{Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

use crate::config::ConfigStore;

/// Event broadcast to all windows after every config change (SPEC §4.4 step 8).
pub const EVENT_CONFIG_CHANGED: &str = "config-changed";

pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let version = app.package_info().version.to_string();
            let (store, report) = ConfigStore::open(dir, &version);
            if let Some(r) = &report.recovery {
                tracing::warn!(
                    "config was unreadable ({}); moved to {}; restored from {}",
                    r.error,
                    r.broken_file.display(),
                    if r.from_backup { "backup" } else { "defaults" }
                );
            }
            let handle = app.handle().clone();
            store.on_change(Box::new(move |_, new| {
                if let Err(e) = handle.emit(EVENT_CONFIG_CHANGED, new) {
                    tracing::warn!("could not broadcast config change: {e}");
                }
            }));
            app.manage(store);

            WebviewWindowBuilder::new(app, "bar", WebviewUrl::App("bar".into()))
                .title("Infodock")
                .decorations(false)
                .transparent(true)
                .skip_taskbar(true)
                .resizable(false)
                .shadow(false)
                .always_on_top(true)
                .inner_size(800.0, 28.0)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::get_config,
            commands::config::update_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Infodock");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            app.state::<ConfigStore>().shutdown();
        }
    });
}
