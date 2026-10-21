//! Shell icons for the sensors on the bar and the rows of popups, and the sensors' image files
//! (SPEC §4.1): read on a worker thread, since the shell may wait for a network server and a
//! file may lie on a share that does not answer, and kept by path and size. Sensors ask while
//! they measure, popups while they lay out; an icon that arrives wakes the bar thread, which
//! measures and lays out again. The sensors' icons and images are kept; the popups' icons (a
//! long folder has many) are transient: only the newest requests wait to be read, and the
//! least recently asked for are dropped beyond [`CAPACITY`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use crate::platform::{Bitmap, Platform, UiWaker};
use crate::sensors::{Fetched, IconSource, read_image};

/// Transient icons kept at most; beyond it, the least recently asked for go.
const CAPACITY: usize = 256;
/// Transient requests waiting to be read at most (a screenful of rows twice); older ones are
/// dropped, and asked for again if their row shows again.
const QUEUED: usize = 48;

/// What is read from a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    /// The shell's icon of a file, folder or drive.
    Shell,
    /// An image file, decoded (SPEC §9.2).
    Image,
}

/// What, from which path, at which size in physical px.
type Key = (Kind, String, u32);

pub(super) struct IconLoader {
    platform: Arc<dyn Platform>,
    waker: UiWaker,
    inner: RefCell<Inner>,
}

struct Entry {
    /// `None` while it is being read.
    icon: Option<Option<Bitmap>>,
    /// Asked for by popups only.
    transient: bool,
    /// When it was last asked for, by [`Inner::clock`].
    used: u64,
}

#[derive(Default)]
struct Inner {
    cache: HashMap<Key, Entry>,
    /// Counts the requests.
    clock: u64,
    /// Transient keys being read, oldest first.
    queued: VecDeque<Key>,
    /// Started on first use; `Some(None)` if it could not start.
    worker: Option<Option<Worker>>,
}

struct Worker {
    requests: Sender<Key>,
    results: Receiver<(Key, Option<Bitmap>)>,
    /// Requests no longer wanted, skipped by the worker.
    cancelled: Arc<Mutex<HashSet<Key>>>,
}

impl Worker {
    fn start(platform: Arc<dyn Platform>, waker: UiWaker) -> Option<Self> {
        let (requests, queue) = mpsc::channel::<Key>();
        let (answers, results) = mpsc::channel();
        let cancelled = Arc::new(Mutex::new(HashSet::new()));
        let skipped = Arc::clone(&cancelled);
        std::thread::Builder::new()
            .name("icons".into())
            .spawn(move || {
                // Ends when the bar drops its sender.
                for key in queue {
                    let gone = skipped
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&key);
                    if gone {
                        continue;
                    }
                    let icon = match key.0 {
                        Kind::Shell => platform.shell_icon(&key.1, key.2),
                        Kind::Image => read_image(&key.1, key.2),
                    };
                    if answers.send((key, icon)).is_err() {
                        return;
                    }
                    waker.wake();
                }
            })
            .map_err(|e| tracing::error!("could not start the icon reader: {e}"))
            .ok()?;
        Some(Self {
            requests,
            results,
            cancelled,
        })
    }
}

impl Inner {
    /// Drops the least recently asked-for transient icons beyond [`CAPACITY`].
    fn evict(&mut self) {
        let mut ready: Vec<(u64, Key)> = self
            .cache
            .iter()
            .filter(|(_, e)| e.transient && e.icon.is_some())
            .map(|(k, e)| (e.used, k.clone()))
            .collect();
        if ready.len() <= CAPACITY {
            return;
        }
        ready.sort_unstable_by_key(|(used, _)| *used);
        let excess = ready.len() - CAPACITY * 3 / 4;
        for (_, key) in ready.into_iter().take(excess) {
            self.cache.remove(&key);
        }
    }

    /// Keeps at most [`QUEUED`] transient requests waiting; older ones are cancelled.
    fn trim_queue(&mut self) {
        while self.queued.len() > QUEUED {
            let Some(key) = self.queued.pop_front() else {
                break;
            };
            let waiting = self
                .cache
                .get(&key)
                .is_some_and(|e| e.transient && e.icon.is_none());
            if waiting && let Some(Some(worker)) = &self.worker {
                self.cache.remove(&key);
                worker
                    .cancelled
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(key);
            }
        }
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

    /// The icons for popup rows, asked for as transient.
    pub fn transient(&self) -> TransientIcons<'_> {
        TransientIcons(self)
    }

    /// Takes the icons that arrived; returns whether any did, so the sensors measure again.
    pub fn receive(&self) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(Some(worker)) = &inner.worker else {
            return false;
        };
        let arrived: Vec<_> = worker.results.try_iter().collect();
        let any = !arrived.is_empty();
        let clock = inner.clock;
        for (key, icon) in arrived {
            inner.queued.retain(|k| *k != key);
            let entry = inner.cache.entry(key).or_insert(Entry {
                icon: None,
                transient: true,
                used: clock,
            });
            entry.icon = Some(icon);
        }
        inner.evict();
        any
    }

    fn request(&self, kind: Kind, path: &str, px: u32, transient: bool) -> Fetched {
        let mut inner = self.inner.borrow_mut();
        inner.clock += 1;
        let clock = inner.clock;
        let key = (kind, path.to_owned(), px);
        if let Some(entry) = inner.cache.get_mut(&key) {
            entry.used = clock;
            // A sensor's icon is kept, whoever asked first.
            entry.transient &= transient;
            return match &entry.icon {
                Some(Some(icon)) => Fetched::Ready(icon.clone()),
                Some(None) => Fetched::None,
                None => Fetched::Loading,
            };
        }
        let (platform, waker) = (&self.platform, &self.waker);
        let worker = inner
            .worker
            .get_or_insert_with(|| Worker::start(Arc::clone(platform), waker.clone()));
        let sent = worker
            .as_ref()
            .is_some_and(|w| w.requests.send(key.clone()).is_ok());
        if !sent {
            return Fetched::None;
        }
        if transient {
            inner.queued.push_back(key.clone());
        }
        inner.cache.insert(
            key,
            Entry {
                icon: None,
                transient,
                used: clock,
            },
        );
        inner.trim_queue();
        Fetched::Loading
    }
}

impl IconSource for IconLoader {
    fn shell_icon(&self, path: &str, px: u32) -> Fetched {
        self.request(Kind::Shell, path, px, false)
    }

    fn image(&self, path: &str, px: u32) -> Fetched {
        self.request(Kind::Image, path, px, false)
    }

    fn forget(&self, path: &str) {
        // Icons being read stay: their answer arrives and is used.
        self.inner
            .borrow_mut()
            .cache
            .retain(|(_, p, _), e| p != path || e.icon.is_none());
    }
}

/// [`IconLoader`] as popups see it: their icons are transient.
pub(super) struct TransientIcons<'a>(&'a IconLoader);

impl IconSource for TransientIcons<'_> {
    fn shell_icon(&self, path: &str, px: u32) -> Fetched {
        self.0.request(Kind::Shell, path, px, true)
    }

    fn image(&self, path: &str, px: u32) -> Fetched {
        self.0.request(Kind::Image, path, px, true)
    }

    fn forget(&self, path: &str) {
        self.0.forget(path);
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
        assert_eq!(icons.shell_icon("app.exe", 2), Fetched::Loading);
        assert_eq!(icons.shell_icon("missing.exe", 2), Fetched::Loading);
        // Asked again while being read: still loading, not asked twice.
        assert_eq!(icons.shell_icon("app.exe", 2), Fetched::Loading);
        let mut arrived = 0;
        while arrived < 2 {
            wakes
                .recv_timeout(Duration::from_secs(5))
                .expect("woken by the worker");
            if icons.receive() {
                arrived = icons
                    .inner
                    .borrow()
                    .cache
                    .values()
                    .filter(|e| e.icon.is_some())
                    .count();
            }
        }
        assert!(matches!(icons.shell_icon("app.exe", 2), Fetched::Ready(b) if b.width() == 2));
        assert_eq!(icons.shell_icon("missing.exe", 2), Fetched::None);
        // Another size is another icon.
        assert_eq!(icons.shell_icon("app.exe", 3), Fetched::Loading);
        // Forgotten: read again.
        icons.forget("missing.exe");
        assert_eq!(icons.shell_icon("missing.exe", 2), Fetched::Loading);
    }

    #[test]
    fn image_files_are_read_on_the_worker() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("dot.svg");
        std::fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2"/></svg>"#,
        )
        .expect("written");
        let path = path.to_string_lossy().into_owned();
        let (woken, wakes) = mpsc::channel();
        let icons = IconLoader::new(
            Arc::new(Shell),
            UiWaker::new(move || {
                let _ = woken.send(());
            }),
        );
        assert_eq!(icons.image(&path, 6), Fetched::Loading);
        assert_eq!(icons.image("missing.png", 6), Fetched::Loading);
        // Not a shell icon of the same path.
        assert_eq!(icons.shell_icon(&path, 6), Fetched::Loading);
        let mut arrived = 0;
        while arrived < 3 {
            wakes
                .recv_timeout(Duration::from_secs(5))
                .expect("woken by the worker");
            icons.receive();
            arrived = icons
                .inner
                .borrow()
                .cache
                .values()
                .filter(|e| e.icon.is_some())
                .count();
        }
        // Decoded at the height asked for, keeping its aspect ratio; a file that cannot be
        // read has none.
        assert!(
            matches!(icons.image(&path, 6), Fetched::Ready(b) if (b.width(), b.height()) == (12, 6))
        );
        assert_eq!(icons.image("missing.png", 6), Fetched::None);
        assert_eq!(icons.shell_icon(&path, 6), Fetched::None);
    }

    #[test]
    fn popup_icons_are_transient_and_bounded() {
        let icons = IconLoader::new(Arc::new(Shell), UiWaker::new(|| {}));
        assert_eq!(icons.shell_icon("app.exe", 2), Fetched::Loading);
        let rows = icons.transient();
        // Scrolling through many rows: only the newest wait to be read.
        for i in 0..200 {
            assert_eq!(rows.shell_icon(&format!("row{i}"), 2), Fetched::Loading);
        }
        {
            let inner = icons.inner.borrow();
            assert_eq!(inner.queued.len(), QUEUED);
            assert!(
                inner
                    .cache
                    .contains_key(&(Kind::Shell, "row199".to_owned(), 2))
            );
            assert!(
                !inner
                    .cache
                    .contains_key(&(Kind::Shell, "row0".to_owned(), 2)),
                "cancelled"
            );
            assert!(
                inner
                    .cache
                    .contains_key(&(Kind::Shell, "app.exe".to_owned(), 2)),
                "the sensor's"
            );
        }
        // Many transient icons arrived: the least recently asked for go, never the sensor's.
        {
            let mut inner = icons.inner.borrow_mut();
            for i in 0..300u64 {
                inner.cache.insert(
                    (Kind::Shell, format!("old{i}"), 2),
                    Entry {
                        icon: Some(None),
                        transient: true,
                        used: i,
                    },
                );
            }
            inner.cache.insert(
                (Kind::Shell, "app.exe".to_owned(), 2),
                Entry {
                    icon: Some(None),
                    transient: false,
                    used: 0,
                },
            );
            inner.evict();
            assert!(
                inner
                    .cache
                    .contains_key(&(Kind::Shell, "app.exe".to_owned(), 2))
            );
            assert!(
                !inner
                    .cache
                    .contains_key(&(Kind::Shell, "old0".to_owned(), 2))
            );
            assert!(
                inner
                    .cache
                    .contains_key(&(Kind::Shell, "old299".to_owned(), 2))
            );
        }
        // A popup asking for the sensor's icon leaves it kept.
        assert_eq!(rows.shell_icon("app.exe", 2), Fetched::None);
        assert!(!icons.inner.borrow().cache[&(Kind::Shell, "app.exe".to_owned(), 2)].transient);
    }
}
