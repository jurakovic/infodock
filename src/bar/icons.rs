//! Shell icons for the sensors on the bar (SPEC §4.1): read on a worker thread, since the shell
//! may wait for a network server, and kept by path and size. Sensors ask while they measure; an
//! icon that arrives wakes the bar thread, which measures its sensors again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::platform::{Bitmap, Platform, UiWaker};
use crate::sensors::{IconSource, ShellIcon};

/// More icons than this are not kept: the ready ones are dropped and read again when asked.
const CAPACITY: usize = 256;

/// A path and a size in physical px.
type Key = (String, u32);

pub(super) struct IconLoader {
    platform: Arc<dyn Platform>,
    waker: UiWaker,
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    /// What is known by key; `None` while it is being read.
    cache: HashMap<Key, Option<Option<Bitmap>>>,
    /// Started on first use; `Some(None)` if it could not start.
    worker: Option<Option<Worker>>,
}

struct Worker {
    requests: Sender<Key>,
    results: Receiver<(Key, Option<Bitmap>)>,
}

impl Worker {
    fn start(platform: Arc<dyn Platform>, waker: UiWaker) -> Option<Self> {
        let (requests, queue) = mpsc::channel::<Key>();
        let (answers, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("icons".into())
            .spawn(move || {
                // Ends when the bar drops its sender.
                for (path, px) in queue {
                    let icon = platform.shell_icon(&path, px);
                    if answers.send(((path, px), icon)).is_err() {
                        return;
                    }
                    waker.wake();
                }
            })
            .map_err(|e| tracing::error!("could not start the icon reader: {e}"))
            .ok()?;
        Some(Self { requests, results })
    }
}

impl IconLoader {
    pub fn new(platform: Arc<dyn Platform>, waker: UiWaker) -> Self {
        Self {
            platform,
            waker,
            inner: RefCell::default(),
        }
    }

    /// Takes the icons that arrived; returns whether any did, so the sensors measure again.
    pub fn receive(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(Some(worker)) = &inner.worker else {
            return false;
        };
        let arrived: Vec<_> = worker.results.try_iter().collect();
        let any = !arrived.is_empty();
        for (key, icon) in arrived {
            inner.cache.insert(key, Some(icon));
        }
        any
    }
}

impl IconSource for IconLoader {
    fn shell_icon(&self, path: &str, px: u32) -> ShellIcon {
        let mut inner = self.inner.borrow_mut();
        let key = (path.to_owned(), px);
        match inner.cache.get(&key) {
            Some(Some(Some(icon))) => return ShellIcon::Ready(icon.clone()),
            Some(Some(None)) => return ShellIcon::None,
            Some(None) => return ShellIcon::Loading,
            None => {}
        }
        if inner.cache.len() >= CAPACITY {
            inner.cache.retain(|_, known| known.is_none());
        }
        let (platform, waker) = (&self.platform, &self.waker);
        let worker = inner
            .worker
            .get_or_insert_with(|| Worker::start(Arc::clone(platform), waker.clone()));
        let sent = worker
            .as_ref()
            .is_some_and(|w| w.requests.send(key.clone()).is_ok());
        if !sent {
            return ShellIcon::None;
        }
        inner.cache.insert(key, None);
        ShellIcon::Loading
    }

    fn forget(&self, path: &str) {
        // Icons being read stay: their answer arrives and is used.
        self.inner
            .borrow_mut()
            .cache
            .retain(|(p, _), known| p != path || known.is_none());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::platform::{Capabilities, MonitorInfo, OsAppearance};

    /// Has a 2 px icon for `app.exe` only.
    struct Shell;

    impl Platform for Shell {
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn monitors(&self) -> Vec<MonitorInfo> {
            Vec::new()
        }
        fn appearance(&self) -> OsAppearance {
            OsAppearance::default()
        }
        fn uptime(&self) -> Duration {
            Duration::ZERO
        }
        fn shell_icon(&self, path: &str, px: u32) -> Option<Bitmap> {
            (path == "app.exe").then(|| {
                Bitmap::from_rgba(px, px, vec![255; (px * px * 4) as usize]).expect("a bitmap")
            })
        }
    }

    #[test]
    fn icons_arrive_from_the_worker_and_wake_the_bar() {
        let (woken, wakes) = mpsc::channel();
        let waker = UiWaker::new(move || {
            let _ = woken.send(());
        });
        let icons = IconLoader::new(Arc::new(Shell), waker);
        assert_eq!(icons.shell_icon("app.exe", 2), ShellIcon::Loading);
        assert_eq!(icons.shell_icon("missing.exe", 2), ShellIcon::Loading);
        // Asked again while being read: still loading, not asked twice.
        assert_eq!(icons.shell_icon("app.exe", 2), ShellIcon::Loading);
        let mut arrived = 0;
        while arrived < 2 {
            wakes
                .recv_timeout(Duration::from_secs(5))
                .expect("woken by the worker");
            if icons.receive() {
                arrived = icons.inner.borrow().cache.values().flatten().count();
            }
        }
        assert!(matches!(icons.shell_icon("app.exe", 2), ShellIcon::Ready(b) if b.width() == 2));
        assert_eq!(icons.shell_icon("missing.exe", 2), ShellIcon::None);
        // Another size is another icon.
        assert_eq!(icons.shell_icon("app.exe", 3), ShellIcon::Loading);
        // Forgotten: read again.
        icons.forget("missing.exe");
        assert_eq!(icons.shell_icon("missing.exe", 2), ShellIcon::Loading);
    }
}
