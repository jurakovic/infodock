//! Readings that can stall (SPEC §4.4, §14.2): a volume or path on a network drive whose server
//! is away can take the SMB timeout to answer, and the sampler must not stop for it. They are
//! read on a worker thread; the sampler waits [`WAIT`] for a reading, and a later one is used at
//! the topic's next tick.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// How long the sampler waits for an answer.
const WAIT: Duration = Duration::from_millis(200);

/// The worker thread and what it has answered, by key (a volume root, a path).
pub struct SlowReader<V> {
    requests: Sender<String>,
    results: Receiver<(String, V)>,
    /// The newest reading of each key.
    latest: HashMap<String, V>,
    /// Keys asked for and not answered yet.
    pending: HashSet<String>,
}

impl<V: Copy + Send + 'static> SlowReader<V> {
    /// Starts the worker `name`, which reads with `read`; `None` if the thread cannot start.
    pub fn new(name: &str, read: impl Fn(&str) -> V + Send + 'static) -> Option<Self> {
        let (requests, queue) = mpsc::channel::<String>();
        let (answers, results) = mpsc::channel();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                // Ends when the sampler drops its sender.
                for key in queue {
                    let reading = read(&key);
                    if answers.send((key, reading)).is_err() {
                        return;
                    }
                }
            })
            .map_err(|e| tracing::error!("could not start the {name} reader: {e}"))
            .ok()?;
        Some(Self {
            requests,
            results,
            latest: HashMap::new(),
            pending: HashSet::new(),
        })
    }

    /// Reads `key`, waiting at most [`WAIT`]: the reading, or else the newest one so far
    /// (`None` before the first).
    pub fn sample(&mut self, key: &str) -> Option<V> {
        if !self.pending.contains(key) && self.requests.send(key.to_owned()).is_ok() {
            self.pending.insert(key.to_owned());
        }
        let deadline = Instant::now() + WAIT;
        while self.pending.contains(key) {
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
        self.latest.get(key).copied()
    }

    /// Drops what is known about `key` once no sensor shows it.
    pub fn forget(&mut self, key: &str) {
        self.latest.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn a_slow_reading_does_not_hold_up_the_sampler() {
        // `fast` answers at once; `slow` (a network drive) only once it is let go.
        let hold = Arc::new(Mutex::new(()));
        let held = hold.lock().expect("lock");
        let gate = Arc::clone(&hold);
        let mut reader = SlowReader::new("test", move |key| {
            if key == "slow" {
                drop(gate.lock());
            }
            key.len()
        })
        .expect("started");
        assert_eq!(reader.sample("fast"), Some(4));
        let started = Instant::now();
        assert_eq!(reader.sample("slow"), None, "not answered yet");
        assert!(started.elapsed() < Duration::from_secs(2));
        // Asked again while the first reading still runs: not queued twice.
        assert_eq!(reader.sample("slow"), None);
        assert_eq!(reader.pending.len(), 1);
        drop(held);
        // The answer arrives; the next tick takes it.
        let mut answered = None;
        for _ in 0..50 {
            answered = reader.sample("slow");
            if answered.is_some() {
                break;
            }
        }
        assert_eq!(answered, Some(4));
        reader.forget("slow");
        assert!(!reader.latest.contains_key("slow"));
    }
}
