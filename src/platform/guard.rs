//! The panic boundary of the callbacks the OS calls (SPEC §14.2): window procedures, hooks, the
//! drop target. A panic must not unwind into the OS, which aborts the process with nothing
//! flushed and the AppBar still reserved.

use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `callback`, which the OS called as `what` (e.g. "the bar window's procedure"). After a
/// panic in it, which the panic hook has logged, the OS gets `default()`, the answer it would
/// have without us, and the app goes on with its state as the panic left it (SPEC §14.2), hence
/// `AssertUnwindSafe`.
pub fn guard<T>(what: &str, default: impl FnOnce() -> T, callback: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(answer) => answer,
        Err(_) => {
            tracing::error!("{what} panicked; the default answer was given and the app goes on");
            default()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use super::guard;

    /// Collects what is logged while a test runs.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);

    impl Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_panicking_callback_gives_the_default_answer_and_is_logged() {
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            assert_eq!(guard("a test callback", || 0, || 7), 7, "no panic");
            let answer = guard("a test callback", || -1, || -> i32 { panic!("on purpose") });
            assert_eq!(answer, -1);
        });
        let logged = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("a test callback panicked; the default answer was given"),
            "{logged}"
        );
        assert_eq!(logged.lines().count(), 1, "only the panic is logged");
    }
}
