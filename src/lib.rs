//! Infodock core (SPEC §4): start-up, the main-thread UI (Settings window, hotkey) and shutdown.
//! The bar, with the tray icon, runs on its own thread (`bar`).

/// Connects a Slint component's `I18n` global to the shared catalog (SPEC §13). Every component
/// instance has its own copy of the global.
macro_rules! install_i18n {
    ($component:expr) => {{
        let i18n = $component.global::<$crate::ui::I18n>();
        i18n.on_lookup(|key, _| $crate::i18n::t(&key).into());
        i18n.on_lookup_with(|key, name, value, _| {
            $crate::i18n::t_args(&key, &[(&name, &value)]).into()
        });
        // Bindings evaluated before the lookups existed (e.g. the window title) translate again.
        i18n.set_revision(i18n.get_revision() + 1);
    }};
}
pub(crate) use install_i18n;

mod app;
pub mod bar;
pub mod config;
mod hotkey;
pub mod i18n;
mod logging;
pub mod platform;
pub mod providers;
pub mod sampler;
pub mod sensors;
mod settings;
#[cfg(test)]
mod test_util;

/// The Slint UI compiled from `ui/app.slint`.
mod ui {
    slint::include_modules!();
}

use std::sync::Arc;

use app::{App, AppHost};
use bar::BarController;
use config::ConfigStore;
use platform::Instance;

/// Names the app's data folders and its single-instance lock (SPEC §2.3).
pub const IDENTIFIER: &str = "app.infodock.desktop";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() {
    logging::install_panic_hook();

    // Held until the process ends; a second launch hands over to this instance (SPEC §6.5).
    let _instance = match platform::single_instance(IDENTIFIER) {
        Instance::Primary(guard) => guard,
        Instance::Secondary => {
            platform::activate_running_instance();
            return;
        }
    };

    let dirs = platform::app_dirs(IDENTIFIER);
    if let Err(e) = logging::init(&dirs.logs) {
        eprintln!("logging unavailable: {e}");
    }
    tracing::info!("Infodock {VERSION} starting");

    let (store, report) = ConfigStore::open(&dirs.config, VERSION);
    log_load_report(&report);
    let platform = platform::current();
    fill_machine_defaults(&store, platform.as_ref());

    // The Settings window uses Slint's winit backend on this thread; software rendering keeps
    // GPU drivers out of the process (SPEC §3, §14.1).
    if let Err(e) = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()
    {
        tracing::error!("the UI toolkit could not start: {e}");
        return;
    }

    let app = Arc::new(App::new(store, platform));
    let bar = match BarController::create(
        &app.store,
        Arc::clone(&app.platform),
        Arc::new(AppHost(Arc::clone(&app))),
    ) {
        Ok(bar) => bar,
        Err(e) => {
            tracing::error!("the bar could not start: {e}");
            app.shutdown();
            return;
        }
    };
    app.set_bar(bar);
    app.start_sampler();

    hotkey::init(&app);
    settings::watch(&app);
    if !app.platform.capabilities().appbar {
        // No bar on this OS (development builds): open Settings so something is visible.
        settings::open(&app, "general");
    }

    if let Err(e) = slint::run_event_loop_until_quit() {
        tracing::error!("the event loop failed: {e}");
    }
    app.shutdown();
}

/// Stores the machine-derived props that sensors lack, e.g. after first run or an import
/// (SPEC §11.2).
fn fill_machine_defaults(store: &ConfigStore, platform: &dyn platform::Platform) {
    let ctx = sensors::DefaultsCtx { platform };
    let result = store.update(|config| {
        Ok(sensors::registry::registry().fill_machine_defaults(&mut config.layout, &ctx))
    });
    if let Ok((true, _)) = result {
        tracing::info!("stored machine-derived sensor props");
    }
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
