//! Readings that can stall (SPEC §4.4, §14.2): a volume or path on a network drive whose server
//! is away can take the SMB timeout to answer, and the sampler must not stop for it. Each key (a
//! volume root, a path) is read on a worker thread of its own, so one that stalls holds up
//! nothing else. The sampler waits [`WAIT`] for a reading it asks for; a key still being read
//! from an earlier tick is not waited for again. A reading that comes after the wait calls the
//! reader's `late` hook, so its topic is read again at once and takes it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// How long the sampler waits for an answer.
const WAIT: Duration = Duration::from_millis(200);

type Read<V> = Arc<dyn Fn(&str) -> V + Send + Sync>;
type Late = Arc<dyn Fn() + Send + Sync>;

/// A key's worker: its requests, and whether the sampler still waits for its answer.
struct Worker {
    requests: Sender<()>,
    waiting: Arc<AtomicBool>,
}

/// The workers of one kind of reading and what they have answered, by key.
pub struct SlowReader<V> {
    name: String,
    read: Read<V>,
    late: Late,
    workers: HashMap<String, Worker>,
    answers: Sender<(String, V)>,
    results: Receiver<(String, V)>,
    /// The newest reading of each key.
    latest: HashMap<String, V>,
    /// Keys asked for and not answered yet.
    pending: HashSet<String>,
    /// Keys whose answer came after the wait and has not been taken yet.
    fresh: HashSet<String>,
}

impl<V: Clone + Send + 'static> SlowReader<V> {
    /// A reader whose workers are named `name` and read with `read`; `late` is called (on a
    /// worker) when an answer comes after the sampler stopped waiting for it.
    pub fn new(
        name: &str,
        read: impl Fn(&str) -> V + Send + Sync + 'static,
        late: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (answers, results) = mpsc::channel();
        Self {
            name: name.to_owned(),
            read: Arc::new(read),
            late: Arc::new(late),
            workers: HashMap::new(),
            answers,
            results,
            latest: HashMap::new(),
            pending: HashSet::new(),
            fresh: HashSet::new(),
        }
    }

    /// `key`'s worker, started on first use; `None` if its thread cannot start.
    fn worker(&mut self, key: &str) -> Option<&Worker> {
        if !self.workers.contains_key(key) {
            let (requests, queue) = mpsc::channel::<()>();
            let waiting = Arc::new(AtomicBool::new(false));
            let (read, late, answers) = (
                Arc::clone(&self.read),
                Arc::clone(&self.late),
                self.answers.clone(),
            );
            let (owned, still) = (key.to_owned(), Arc::clone(&waiting));
            std::thread::Builder::new()
                .name(self.name.clone())
                .spawn(move || {
                    // Ends when the reader drops the key's sender.
                    for () in queue {
                        let reading = read(&owned);
                        if answers.send((owned.clone(), reading)).is_err() {
                            return;
                        }
                        // After the answer is sent: the sampler stops waiting before it looks
                        // for answers a last time, so one of the two always sees it.
                        if !still.load(Ordering::SeqCst) {
                            late();
                        }
                    }
                })
                .map_err(|e| tracing::error!("could not start a {} reader: {e}", self.name))
                .ok()?;
            self.workers
                .insert(key.to_owned(), Worker { requests, waiting });
        }
        self.workers.get(key)
    }

    /// Takes an answer as the newest reading of its key; `fresh` when it came after the wait.
    fn take(&mut self, key: String, reading: V, fresh: bool) {
        // A key forgotten while it was being read stays forgotten.
        if self.workers.contains_key(&key) {
            self.pending.remove(&key);
            if fresh {
                self.fresh.insert(key.clone());
            }
            self.latest.insert(key, reading);
        }
    }

    /// Takes the answers that came meanwhile; they are fresh.
    fn collect(&mut self) {
        while let Ok((key, reading)) = self.results.try_recv() {
            self.take(key, reading, true);
        }
    }

    /// Reads `key`: an answer that came late and was not taken yet, or else a new reading,
    /// waiting at most [`WAIT`] for it (not at all while an earlier reading still runs). `None`
    /// before the first reading.
    pub fn sample(&mut self, key: &str) -> Option<V> {
        self.collect();
        if self.fresh.remove(key) || self.pending.contains(key) {
            return self.latest.get(key).cloned();
        }
        let waiting = {
            let worker = self.worker(key)?;
            worker.waiting.store(true, Ordering::SeqCst);
            if worker.requests.send(()).is_err() {
                return self.latest.get(key).cloned();
            }
            Arc::clone(&worker.waiting)
        };
        self.pending.insert(key.to_owned());
        let deadline = Instant::now() + WAIT;
        while self.pending.contains(key) {
            match self
                .results
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok((answered, reading)) => {
                    // Other keys' answers came after their own wait.
                    let fresh = answered != key;
                    self.take(answered, reading, fresh);
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        if self.pending.contains(key) {
            waiting.store(false, Ordering::SeqCst);
            // It may have come just now.
            self.collect();
            self.fresh.remove(key);
        }
        self.latest.get(key).cloned()
    }

    /// Drops what is known about `key` once no sensor shows it; its worker ends after any
    /// reading it is still doing.
    pub fn forget(&mut self, key: &str) {
        self.workers.remove(key);
        self.latest.remove(key);
        self.pending.remove(key);
        self.fresh.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use super::*;

    /// A reader whose `slow` key answers only once let go; `late` counts the late answers.
    fn reader() -> (SlowReader<usize>, Arc<Mutex<()>>, Arc<AtomicUsize>) {
        let hold = Arc::new(Mutex::new(()));
        let gate = Arc::clone(&hold);
        let lates = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&lates);
        let reader = SlowReader::new(
            "test",
            move |key: &str| {
                if key == "slow" {
                    drop(gate.lock());
                }
                key.len()
            },
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
            },
        );
        (reader, hold, lates)
    }

    #[test]
    fn a_stalled_key_holds_up_nothing_else() {
        let (mut reader, hold, lates) = reader();
        let held = hold.lock().expect("lock");
        assert_eq!(reader.sample("fast"), Some(4));
        let started = Instant::now();
        assert_eq!(reader.sample("slow"), None, "not answered yet");
        assert!(started.elapsed() >= WAIT);
        // Still stalled: neither waited for again nor asked twice; other keys answer.
        let started = Instant::now();
        assert_eq!(reader.sample("slow"), None);
        assert!(started.elapsed() < WAIT, "{:?}", started.elapsed());
        assert_eq!(reader.sample("other"), Some(5));
        assert_eq!(reader.pending.len(), 1);
        drop(held);
        // The late answer calls the hook; the next sample takes it without asking again.
        let deadline = Instant::now() + Duration::from_secs(5);
        while lates.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(lates.load(Ordering::SeqCst), 1);
        assert_eq!(reader.sample("slow"), Some(4));
        assert!(reader.fresh.is_empty());
        // Asked again: answered within the wait, so no late call.
        assert_eq!(reader.sample("slow"), Some(4));
        assert_eq!(lates.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_forgotten_key_stays_forgotten() {
        let (mut reader, hold, _) = reader();
        let held = hold.lock().expect("lock");
        assert_eq!(reader.sample("slow"), None);
        reader.forget("slow");
        drop(held);
        // Its answer comes after it was forgotten.
        std::thread::sleep(Duration::from_millis(50));
        reader.collect();
        assert!(!reader.latest.contains_key("slow"));
        assert!(reader.fresh.is_empty() && reader.workers.is_empty());
    }
}
