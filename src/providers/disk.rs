//! `disk/<X:\>` (SPEC Appendix A): a volume's size and free space. A drive that is missing or
//! has no medium is a sample too (`present: false`), so sensors can show it as absent.
//!
//! Volumes are read on a worker thread: a network drive whose server is away can take the
//! SMB timeout to answer, and the sampler must not stop for it (SPEC §14.1–14.2). The sampler
//! waits [`WAIT`] for a reading; a later one is used at the topic's next tick.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::platform::{DiskSpace, Platform};
use crate::sampler::DiskSample;

/// How long the sampler waits for a volume to answer.
const WAIT: Duration = Duration::from_millis(200);

pub fn sample(space: Option<DiskSpace>) -> DiskSample {
    match space {
        Some(space) => DiskSample {
            present: true,
            total: space.total,
            free: space.free.min(space.total),
        },
        None => DiskSample::default(),
    }
}

/// The worker thread and what it has answered.
pub struct Disks {
    requests: Sender<String>,
    results: Receiver<(String, DiskSample)>,
    /// The newest reading of each volume.
    latest: HashMap<String, DiskSample>,
    /// Volumes asked for and not answered yet.
    pending: HashSet<String>,
}

impl Disks {
    /// Starts the worker; `None` if the thread cannot be started.
    pub fn new(platform: Arc<dyn Platform>) -> Option<Self> {
        let (requests, queue) = mpsc::channel::<String>();
        let (answers, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("disks".into())
            .spawn(move || {
                // Ends when the sampler drops its sender.
                for root in queue {
                    let reading = sample(platform.disk_space(&root));
                    if answers.send((root, reading)).is_err() {
                        return;
                    }
                }
            })
            .map_err(|e| tracing::error!("could not start the disk reader: {e}"))
            .ok()?;
        Some(Self {
            requests,
            results,
            latest: HashMap::new(),
            pending: HashSet::new(),
        })
    }

    /// Reads `root`, waiting at most [`WAIT`]: the reading, or else the newest one so far
    /// (`None` before the first).
    pub fn sample(&mut self, root: &str) -> Option<DiskSample> {
        if !self.pending.contains(root) && self.requests.send(root.to_owned()).is_ok() {
            self.pending.insert(root.to_owned());
        }
        let deadline = Instant::now() + WAIT;
        while self.pending.contains(root) {
            match self
                .results
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok((answered, reading)) => {
                    self.pending.remove(&answered);
                    self.latest.insert(answered, reading);
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        self.latest.get(root).copied()
    }

    /// Drops what is known about `root` once no sensor shows it.
    pub fn forget(&mut self, root: &str) {
        self.latest.remove(root);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::platform::{Capabilities, MonitorInfo, OsAppearance};

    #[test]
    fn absent_drives_are_samples_too() {
        let space = DiskSpace {
            total: 1000,
            free: 1200,
        };
        assert_eq!(
            sample(Some(space)),
            DiskSample {
                present: true,
                total: 1000,
                free: 1000
            }
        );
        assert!(!sample(None).present);
    }

    /// C: answers at once; N: (a network drive) only once it is let go.
    #[derive(Default)]
    struct SlowNetworkDrive {
        hold: Mutex<()>,
    }

    impl crate::platform::Platform for SlowNetworkDrive {
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
        fn disk_space(&self, root: &str) -> Option<DiskSpace> {
            if root == "N:\\" {
                drop(self.hold.lock());
            }
            Some(DiskSpace {
                total: 100,
                free: 40,
            })
        }
    }

    #[test]
    fn a_slow_drive_does_not_hold_up_the_sampler() {
        let platform = Arc::new(SlowNetworkDrive::default());
        let held = platform.hold.lock().expect("lock");
        let mut disks = Disks::new(Arc::clone(&platform) as Arc<dyn Platform>).expect("started");
        assert!(disks.sample("C:\\").is_some_and(|s| s.present));
        let started = Instant::now();
        assert_eq!(disks.sample("N:\\"), None, "not answered yet");
        assert!(started.elapsed() < Duration::from_secs(2));
        // Asked again while the first reading still runs: not queued twice.
        assert_eq!(disks.sample("N:\\"), None);
        assert_eq!(disks.pending.len(), 1);
        drop(held);
        // The answer arrives; the next tick takes it.
        let mut answered = None;
        for _ in 0..50 {
            answered = disks.sample("N:\\");
            if answered.is_some() {
                break;
            }
        }
        assert!(answered.is_some_and(|s| s.present));
        disks.forget("N:\\");
        assert!(!disks.latest.contains_key("N:\\"));
    }
}
