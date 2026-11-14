//! Loading and saving `config.json` (SPEC §11.1–11.2).
//!
//! * Writes are debounced (500 ms after the last change, at most 2 s after the first) and
//!   atomic: write `config.json.tmp`, sync, copy the current file to `config.json.bak`, rename.
//!   The current file is copied only while it is known good, so a broken one never becomes the
//!   backup.
//! * A file that fails to parse is renamed to `config.broken-<unix ms>.json`; the `.bak` (or the
//!   first-run defaults) is loaded instead and the caller is told so it can notify the user.
//! * Each change has a [`Revision`]. Listeners run on the thread that made the change, so two
//!   threads' changes can reach them in the opposite order; they keep the newest ([`Newest`]).

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::ConfigError;
use super::migrate::migrate;
use super::model::{Config, SCHEMA_VERSION};

const FILE: &str = "config.json";
const TMP: &str = "config.json.tmp";
const BAK: &str = "config.json.bak";
const DEBOUNCE: Duration = Duration::from_millis(500);
const MAX_DELAY: Duration = Duration::from_secs(2);
const RETRY_AFTER_ERROR: Duration = Duration::from_secs(5);

/// Counts the config's changes: each swap gets the next one, under the store's lock.
pub type Revision = u64;

/// Called after every change with the old and the new config and the new one's revision.
pub type Listener = Box<dyn Fn(&Config, &Config, Revision) + Send + Sync>;
type SharedListener = Arc<dyn Fn(&Config, &Config, Revision) + Send + Sync>;

/// What happened while loading, for logging and user notification.
#[derive(Debug, Default)]
pub struct LoadReport {
    /// No config existed; the first-run config was created.
    pub first_run: bool,
    /// The config was unreadable and was replaced.
    pub recovery: Option<Recovery>,
    pub migrated_from: Option<u32>,
    /// The file came from a newer Infodock (higher schema) and was loaded best-effort.
    pub newer_schema: bool,
}

#[derive(Debug)]
pub struct Recovery {
    /// Where the unreadable file was moved; `None` when it could not be moved (another program
    /// may hold it open) and is still `config.json`, until a write replaces it.
    pub broken_file: Option<PathBuf>,
    pub error: String,
    /// `true` if the backup was loaded, `false` if defaults were used.
    pub from_backup: bool,
}

pub struct ConfigStore {
    shared: Arc<Shared>,
    tx: mpsc::Sender<Msg>,
    writer: Mutex<Option<JoinHandle<()>>>,
    listeners: Mutex<Vec<SharedListener>>,
}

struct Shared {
    dir: PathBuf,
    config: Mutex<Current>,
    /// `config.json` on disk was loaded or written by this store, so it may become the backup.
    /// Not after a recovery: the file is gone or still the broken one, and the backup just
    /// loaded is the previous good version.
    on_disk_good: AtomicBool,
}

struct Current {
    config: Config,
    revision: Revision,
}

enum Msg {
    Dirty,
    #[cfg(test)]
    Flush(mpsc::Sender<()>),
    Stop,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ConfigStore {
    /// Loads (or creates) the config in `dir` and starts the background writer.
    pub fn open(dir: impl Into<PathBuf>, app_version: &str) -> (Self, LoadReport) {
        Self::open_with(dir.into(), app_version, |from, to| fs::rename(from, to))
    }

    /// [`open`](Self::open), with `move_aside` renaming a broken file (a seam for tests).
    fn open_with(
        dir: PathBuf,
        app_version: &str,
        move_aside: impl Fn(&Path, &Path) -> std::io::Result<()>,
    ) -> (Self, LoadReport) {
        let (mut config, report) = load(&dir, app_version, move_aside);
        config.meta.last_written_with = app_version.to_owned();
        let must_write =
            report.first_run || report.recovery.is_some() || report.migrated_from.is_some();

        let shared = Arc::new(Shared {
            dir,
            config: Mutex::new(Current {
                config,
                revision: 0,
            }),
            on_disk_good: AtomicBool::new(report.recovery.is_none()),
        });
        if must_write && let Err(e) = write_now(&shared) {
            tracing::error!("could not write config: {e}");
        }
        let (tx, rx) = mpsc::channel();
        let writer = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("config-writer".into())
                .spawn(move || writer_loop(&shared, &rx))
                .ok()
        };
        let store = Self {
            shared,
            tx,
            writer: Mutex::new(writer),
            listeners: Mutex::new(Vec::new()),
        };
        (store, report)
    }

    /// A snapshot of the current config.
    pub fn get(&self) -> Config {
        lock(&self.shared.config).config.clone()
    }

    /// A snapshot of the current config with its revision, for a listener's [`Newest`].
    pub fn get_revised(&self) -> (Config, Revision) {
        let current = lock(&self.shared.config);
        (current.config.clone(), current.revision)
    }

    /// Registers a listener that runs after every change, on the thread that made it, with no
    /// store lock held. Two threads' changes can reach it in the opposite order of their
    /// revisions: what it keeps should be the newest ([`Newest`]).
    pub fn on_change(&self, listener: Listener) {
        lock(&self.listeners).push(Arc::from(listener));
    }

    /// Applies `f` to a copy of the config. On success the copy replaces the config, a write is
    /// scheduled and listeners run; on error nothing changes.
    pub fn update<R>(
        &self,
        f: impl FnOnce(&mut Config) -> Result<R, ConfigError>,
    ) -> Result<(R, Config), ConfigError> {
        let (old, new, revision, result) = {
            let mut current = lock(&self.shared.config);
            let mut next = current.config.clone();
            let result = f(&mut next)?;
            if next == current.config {
                return Ok((result, next));
            }
            current.revision += 1;
            let old = std::mem::replace(&mut current.config, next.clone());
            (old, next, current.revision, result)
        };
        let _ = self.tx.send(Msg::Dirty);
        // Outside the lock, so a listener may read or change the config.
        let listeners: Vec<SharedListener> = lock(&self.listeners).clone();
        for listener in &listeners {
            listener(&old, &new, revision);
        }
        Ok((result, new))
    }

    /// Writes pending changes now and waits (up to 2 s) until they are on disk.
    #[cfg(test)]
    fn flush(&self) {
        let (ack_tx, ack_rx) = mpsc::channel();
        if self.tx.send(Msg::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(Duration::from_secs(2));
        }
    }

    /// Writes pending changes and stops the writer thread. Call once on exit.
    pub fn shutdown(&self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(handle) = lock(&self.writer).take() {
            let _ = handle.join();
        }
    }
}

/// What a listener keeps of the config: the newest of the changes it was handed. Two threads'
/// changes can reach it in the opposite order of their revisions (each runs the listeners on its
/// own thread, after the store's lock), so one older than what it holds is dropped.
pub struct Newest<T> {
    held: Mutex<(Revision, T)>,
}

impl<T> Newest<T> {
    /// Holds `value`, of `revision` (from [`ConfigStore::get_revised`]).
    pub fn new(value: T, revision: Revision) -> Self {
        Self {
            held: Mutex::new((revision, value)),
        }
    }

    /// Takes `value` if `revision` is newer than what is held, once `apply` ran with the held
    /// value and `value`. `apply` runs under the holder's lock, so what it does happens in the
    /// order of the revisions. `false` when `value` is older and was dropped.
    pub fn offer(&self, revision: Revision, value: T, apply: impl FnOnce(&T, &T)) -> bool {
        let mut held = lock(&self.held);
        if revision <= held.0 {
            tracing::debug!("config revision {revision} dropped: {} is newer", held.0);
            return false;
        }
        apply(&held.1, &value);
        *held = (revision, value);
        true
    }

    /// A copy of the held value.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        lock(&self.held).1.clone()
    }
}

fn writer_loop(shared: &Shared, rx: &mpsc::Receiver<Msg>) {
    // `Some((first change, write due))` while there are unwritten changes.
    let mut pending: Option<(Instant, Instant)> = None;
    loop {
        let msg = match pending {
            Some((_, due)) => rx.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Dirty) => {
                let now = Instant::now();
                let first = pending.map_or(now, |(first, _)| first);
                pending = Some((first, (now + DEBOUNCE).min(first + MAX_DELAY)));
            }
            #[cfg(test)]
            Ok(Msg::Flush(ack)) => {
                if pending.is_some() {
                    pending = write_or_retry(shared);
                }
                let _ = ack.send(());
            }
            Err(RecvTimeoutError::Timeout) => pending = write_or_retry(shared),
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => {
                if pending.is_some() {
                    let _ = write_or_retry(shared);
                }
                return;
            }
        }
    }
}

fn write_or_retry(shared: &Shared) -> Option<(Instant, Instant)> {
    match write_now(shared) {
        Ok(()) => None,
        Err(e) => {
            tracing::error!("could not write config: {e}");
            let now = Instant::now();
            Some((now, now + RETRY_AFTER_ERROR))
        }
    }
}

fn write_now(shared: &Shared) -> Result<(), ConfigError> {
    let json = {
        let current = lock(&shared.config);
        serde_json::to_string_pretty(&current.config)?
    };
    write_atomic(
        &shared.dir,
        &json,
        shared.on_disk_good.load(Ordering::SeqCst),
    )?;
    shared.on_disk_good.store(true, Ordering::SeqCst);
    Ok(())
}

/// Writes `json` as the config; the file it replaces becomes the backup if it is `good`.
fn write_atomic(dir: &Path, json: &str, good: bool) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(TMP);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    let main = dir.join(FILE);
    if good && main.exists() {
        // Copy rather than rename, so a crash between the two steps never leaves no config.
        fs::copy(&main, dir.join(BAK))?;
    }
    fs::rename(&tmp, &main)
}

fn load(
    dir: &Path,
    app_version: &str,
    move_aside: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> (Config, LoadReport) {
    let main = dir.join(FILE);
    let bak = dir.join(BAK);
    let mut report = LoadReport::default();

    let main_result = main.exists().then(|| parse_file(&main));
    match main_result {
        Some(Ok((config, migrated_from, newer))) => {
            report.migrated_from = migrated_from;
            report.newer_schema = newer;
            return (config, report);
        }
        Some(Err(e)) => {
            let broken = dir.join(format!("config.broken-{}.json", unix_ms()));
            let moved = match move_aside(&main, &broken) {
                Ok(()) => true,
                Err(re) => {
                    tracing::error!("could not move broken config aside: {re}");
                    false
                }
            };
            let from_backup = bak.exists().then(|| parse_file(&bak)).and_then(Result::ok);
            report.recovery = Some(Recovery {
                broken_file: moved.then_some(broken),
                error: e.to_string(),
                from_backup: from_backup.is_some(),
            });
            if let Some((config, migrated_from, _)) = from_backup {
                report.migrated_from = migrated_from;
                return (config, report);
            }
        }
        None => {
            // A missing config with a readable backup means an interrupted first write.
            if let Some(Ok((config, migrated_from, _))) = bak.exists().then(|| parse_file(&bak)) {
                report.migrated_from = migrated_from;
                return (config, report);
            }
            report.first_run = true;
        }
    }
    (Config::first_run(app_version), report)
}

type Parsed = (Config, Option<u32>, bool);

fn parse_file(path: &Path) -> Result<Parsed, ConfigError> {
    let text = fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    if !value.is_object() {
        return Err(ConfigError::Invalid("config is not a JSON object".into()));
    }
    let migrated = migrate(value);
    let mut config: Config = serde_json::from_value(migrated.value)?;
    config.schema = SCHEMA_VERSION;
    config.settings.sanitize();
    Ok((config, migrated.from, migrated.newer))
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::Edge;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn set_edge_top(store: &ConfigStore) {
        store
            .update(|c| {
                c.settings.bar.edge = Edge::Top;
                Ok(())
            })
            .unwrap();
    }

    fn read(dir: &Path, name: &str) -> Config {
        serde_json::from_str(&fs::read_to_string(dir.join(name)).unwrap()).unwrap()
    }

    #[test]
    fn first_run_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let (store, report) = ConfigStore::open(dir.path(), "1.0.0");
        assert!(report.first_run);
        assert_eq!(read(dir.path(), FILE), store.get());
        assert_eq!(store.get().layout.left.len(), 3);
        store.shutdown();
    }

    #[test]
    fn existing_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE),
            r#"{ "schema": 1, "settings": { "bar": { "edge": "top" } } }"#,
        )
        .unwrap();
        let (store, report) = ConfigStore::open(dir.path(), "1.0.0");
        assert!(!report.first_run && report.recovery.is_none());
        assert_eq!(store.get().settings.bar.edge, Edge::Top);
        assert!(
            store.get().layout.left.is_empty(),
            "no first-run layout for an existing file"
        );
        store.shutdown();
    }

    #[test]
    fn changes_are_written_on_flush_and_previous_version_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        set_edge_top(&store);
        store.flush();
        assert_eq!(read(dir.path(), FILE).settings.bar.edge, Edge::Top);
        assert_eq!(read(dir.path(), BAK).settings.bar.edge, Edge::Bottom);
        assert!(!dir.path().join(TMP).exists());
        store.shutdown();
    }

    #[test]
    fn writes_are_debounced() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        set_edge_top(&store);
        assert_eq!(
            read(dir.path(), FILE).settings.bar.edge,
            Edge::Bottom,
            "not written immediately"
        );
        std::thread::sleep(DEBOUNCE + Duration::from_millis(400));
        assert_eq!(read(dir.path(), FILE).settings.bar.edge, Edge::Top);
        store.shutdown();
    }

    #[test]
    fn shutdown_writes_pending_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        set_edge_top(&store);
        store.shutdown();
        assert_eq!(read(dir.path(), FILE).settings.bar.edge, Edge::Top);
    }

    #[test]
    fn broken_file_falls_back_to_backup() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(BAK),
            r#"{ "settings": { "locked": true } }"#,
        )
        .unwrap();
        fs::write(dir.path().join(FILE), "{ not json").unwrap();
        let (store, report) = ConfigStore::open(dir.path(), "1.0.0");
        let recovery = report.recovery.expect("recovery reported");
        assert!(recovery.from_backup);
        let broken = recovery.broken_file.expect("moved aside");
        assert_eq!(fs::read_to_string(&broken).unwrap(), "{ not json");
        assert!(store.get().settings.locked);
        assert!(
            read(dir.path(), FILE).settings.locked,
            "recovered config is written back"
        );
        store.shutdown();
    }

    #[test]
    fn broken_file_without_backup_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE), "[]").unwrap();
        let (store, report) = ConfigStore::open(dir.path(), "1.0.0");
        assert!(!report.recovery.unwrap().from_backup);
        assert_eq!(store.get().layout.left.len(), 3);
        store.shutdown();
    }

    #[test]
    fn missing_file_with_backup_uses_backup() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(BAK),
            r#"{ "settings": { "locked": true } }"#,
        )
        .unwrap();
        let (store, report) = ConfigStore::open(dir.path(), "1.0.0");
        assert!(!report.first_run);
        assert!(store.get().settings.locked);
        store.shutdown();
    }

    #[test]
    fn listeners_run_on_change_only() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        store.on_change(Box::new(move |old, new, _| {
            assert_ne!(old, new);
            c.fetch_add(1, Ordering::SeqCst);
        }));
        set_edge_top(&store);
        set_edge_top(&store); // no change the second time
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        store.shutdown();
    }

    #[test]
    fn each_change_has_the_next_revision() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = Arc::clone(&seen);
        store.on_change(Box::new(move |_, _, revision| lock(&s).push(revision)));
        let start = store.get_revised().1;
        set_edge_top(&store);
        set_edge_top(&store); // no change, no revision
        store
            .update(|c| {
                c.settings.locked = true;
                Ok(())
            })
            .unwrap();
        assert_eq!(*lock(&seen), [start + 1, start + 2]);
        assert_eq!(store.get_revised().1, start + 2);
        store.shutdown();
    }

    #[test]
    fn a_listener_keeps_the_newest_of_two_threads_changes() {
        // Thread A's change is notified only after thread B's (its listeners run late, as on a
        // busy thread): what the listener keeps must still be the store's config.
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        let store = Arc::new(store);
        let (a_parked, parked) = mpsc::channel::<()>();
        let (release, a_released) = mpsc::channel::<()>();
        let a_released = Mutex::new(a_released);
        store.on_change(Box::new(move |_, new, _| {
            // A's change (the bar locked) waits until B's was notified.
            if new.settings.locked && new.settings.bar.edge == Edge::Bottom {
                let _ = a_parked.send(());
                let _ = lock(&a_released).recv();
            }
        }));
        let (config, revision) = store.get_revised();
        let kept = Arc::new(Newest::new(config, revision));
        let applied = Arc::new(Mutex::new(Vec::new()));
        let (k, a) = (Arc::clone(&kept), Arc::clone(&applied));
        store.on_change(Box::new(move |_, new, revision| {
            k.offer(revision, new.clone(), |_, new| lock(&a).push(new.clone()));
        }));

        let a = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                store
                    .update(|c| {
                        c.settings.locked = true;
                        Ok(())
                    })
                    .unwrap();
            })
        };
        parked.recv().unwrap();
        set_edge_top(&store); // B, while A's listeners wait
        release.send(()).unwrap();
        a.join().unwrap();

        assert_eq!(kept.get(), store.get());
        assert_eq!(
            *lock(&applied),
            [store.get()],
            "A's older change was dropped"
        );
        store.shutdown();
    }

    #[test]
    fn a_broken_file_that_cannot_be_moved_aside_never_becomes_the_backup() {
        // On Windows another program can hold the file open; the rename fails there.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE), "{ broken").unwrap();
        let good = r#"{ "settings": { "locked": true } }"#;
        fs::write(dir.path().join(BAK), good).unwrap();
        let (store, report) = ConfigStore::open_with(dir.path().to_path_buf(), "1.0.0", |_, _| {
            Err(std::io::Error::other("held open"))
        });
        let recovery = report.recovery.unwrap();
        assert!(recovery.from_backup);
        assert_eq!(recovery.broken_file, None, "no file was moved aside");
        assert!(store.get().settings.locked);
        // The first write, at once, replaced the broken file and kept the good backup rather
        // than a copy of the broken file.
        assert_eq!(fs::read_to_string(dir.path().join(BAK)).unwrap(), good);
        // The file it wrote is good: the next write backs it up.
        set_edge_top(&store);
        store.shutdown();
        let (backup, main) = (read(dir.path(), BAK), read(dir.path(), FILE));
        assert!(backup.settings.locked && backup.settings.bar.edge == Edge::Bottom);
        assert_eq!(main.settings.bar.edge, Edge::Top);
    }

    #[test]
    fn listeners_run_without_the_lock_held() {
        // A listener that updates the store (as the tray does via the main thread) must not
        // deadlock on the listener list.
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        let store = Arc::new(store);
        let weak = Arc::downgrade(&store);
        store.on_change(Box::new(move |_, new, _| {
            if !new.settings.locked
                && let Some(s) = weak.upgrade()
            {
                s.update(|c| {
                    c.settings.locked = true;
                    Ok(())
                })
                .unwrap();
            }
        }));
        set_edge_top(&store);
        assert!(store.get().settings.locked);
        store.shutdown();
    }

    #[test]
    fn failed_update_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        let before = store.get();
        let r: Result<((), Config), _> = store.update(|c| {
            c.settings.locked = true;
            Err(ConfigError::Invalid("no".into()))
        });
        assert!(r.is_err());
        assert_eq!(store.get(), before);
        store.shutdown();
    }

    #[test]
    fn version_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE),
            r#"{ "meta": { "createdWith": "0.9.0", "lastWrittenWith": "0.9.0" } }"#,
        )
        .unwrap();
        let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
        set_edge_top(&store);
        store.shutdown();
        let meta = read(dir.path(), FILE).meta;
        assert_eq!(
            (meta.created_with.as_str(), meta.last_written_with.as_str()),
            ("0.9.0", "1.0.0")
        );
    }
}
