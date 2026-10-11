//! Infodock core: app builder, windows and plugins (SPEC §4.2).

mod commands;
pub mod config;
mod error;
pub mod i18n;
mod logging;
pub mod platform;
mod windowing;

use std::sync::Arc;

use tauri::{Emitter, Manager, RunEvent};

use crate::config::ConfigStore;
use crate::windowing::bar::BarController;

/// Event broadcast to all windows after every config change (SPEC §4.4 step 8).
pub const EVENT_CONFIG_CHANGED: &str = "config-changed";

pub fn run() {
    logging::install_panic_hook();

    let app = tauri::Builder::default()
        .setup(|app| {
            let log_dir = app.path().app_log_dir()?;
            if let Err(e) = logging::init(&log_dir) {
                eprintln!("logging unavailable: {e}");
            }
            let version = app.package_info().version.to_string();
            tracing::info!("Infodock {version} starting");

            let (store, report) = ConfigStore::open(app.path().app_config_dir()?, &version);
            log_load_report(&report);
            let handle = app.handle().clone();
            store.on_change(Box::new(move |_, new| {
                if let Err(e) = handle.emit(EVENT_CONFIG_CHANGED, new) {
                    tracing::warn!("could not broadcast config change: {e}");
                }
            }));
            app.manage(store);
            app.manage(platform::current());

            let bar = BarController::create(app.handle())?;
            app.manage(bar);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::config::get_config,
            commands::config::update_settings,
            commands::system::capabilities,
            commands::system::list_monitors,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Infodock");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(bar) = app.try_state::<Arc<BarController>>() {
                bar.release();
            }
            if let Some(store) = app.try_state::<ConfigStore>() {
                store.shutdown();
            }
            tracing::info!("Infodock exiting");
            logging::shutdown();
        }
    });
}

fn log_load_report(report: &config::store::LoadReport) {
    if report.first_run {
        tracing::info!("first run: created the default config");
    }
    if let Some(from) = report.migrated_from {
        tracing::info!("config migrated from schema {from}");
    }
    if report.newer_schema {
        tracing::warn!("config was written by a newer Infodock; loaded best-effort");
    }
    if let Some(r) = &report.recovery {
        // A toast follows once notifications exist (M3, SPEC §11.2).
        tracing::warn!(
            "config was unreadable ({}); moved to {}; restored from {}",
            r.error,
            r.broken_file.display(),
            if r.from_backup { "backup" } else { "defaults" }
        );
    }
}
