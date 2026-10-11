//! Logging (SPEC §11.1) and the panic hook (SPEC §5.2 step 6, §14.2).

use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::thread::ThreadId;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

/// Overridden by `RUST_LOG`.
const DEFAULT_FILTER: &str = if cfg!(debug_assertions) {
    "info,infodock_lib=debug"
} else {
    "info"
};

static GUARD: Mutex<Option<WorkerGuard>> = Mutex::new(None);
static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();
type Cleanup = Box<dyn Fn() + Send>;
static FATAL_CLEANUP: Mutex<Vec<Cleanup>> = Mutex::new(Vec::new());

/// Starts logging to `<log_dir>/infodock.YYYY-MM-DD.log` (daily files, 7 kept), plus stderr in
/// debug builds.
pub fn init(log_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(log_dir)?;
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("infodock")
        .filename_suffix("log")
        .max_log_files(7)
        .build(log_dir)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let file = fmt::layer().with_writer(writer).with_ansi(false);
    let stderr = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
    tracing_subscriber::registry()
        .with(filter)
        .with(file)
        .with(stderr)
        .try_init()?;
    *GUARD.lock().unwrap_or_else(PoisonError::into_inner) = Some(guard);
    Ok(())
}

/// Flushes and closes the log file. Call once on exit.
pub fn shutdown() {
    GUARD.lock().unwrap_or_else(PoisonError::into_inner).take();
}

/// Logs panics. A panic on the main thread ends the app, so it also runs the registered fatal
/// cleanups (e.g. releasing the AppBar reservation) and flushes the log. Call from `main`'s
/// thread before the event loop starts.
pub fn install_panic_hook() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(
            "panic: {info}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        if MAIN_THREAD.get() == Some(&std::thread::current().id()) {
            // try_lock: a panic inside a cleanup must not deadlock here.
            if let Ok(cleanups) = FATAL_CLEANUP.try_lock() {
                for cleanup in cleanups.iter() {
                    cleanup();
                }
            }
            shutdown();
        }
        default_hook(info);
    }));
}

/// Registers work that must run if the app dies from a panic on the main thread.
pub fn on_fatal_panic(cleanup: Cleanup) {
    FATAL_CLEANUP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(cleanup);
}
