//! The sampler (SPEC §4.4): one thread that reads the topics sensors subscribe to, each at the
//! shortest interval any of its subscribers asked for, and hands every sample to the bar
//! thread. Providers are shared, so ten sensors on one topic cost one reading per tick.

mod scheduler;
pub mod topics;

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use scheduler::Scheduler;
pub use topics::{
    BATTERY_EVERY, BatterySample, Change, CpuSample, Delivery, DiskSample, MemorySample, NetSample,
    PathSample, Sample, Slow, SwapSample, Topic, TopicRequest, UptimeSample, VOLUMES_EVERY,
};

use crate::providers::Providers;

enum Message {
    Subscribe {
        subscriber: String,
        requests: Vec<TopicRequest>,
    },
    /// The system resumed from sleep.
    Resume,
    Changed(Change),
    Shutdown,
}

/// The handle to the sampler thread; cheap to clone, usable from any thread.
#[derive(Clone)]
pub struct Sampler {
    messages: Sender<Message>,
}

/// Reports a [`Change`] from any thread, e.g. from the thread that emptied the Recycle Bin once
/// it is done, so the sampler reads the topics it concerns at once (SPEC §4.4).
#[derive(Clone)]
pub struct ChangeSink(Option<Arc<dyn Fn(Change) + Send + Sync>>);

impl ChangeSink {
    /// Reports nowhere, for tests.
    pub const NONE: Self = Self(None);

    pub fn new(report: impl Fn(Change) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(report)))
    }

    pub fn report(&self, change: Change) {
        if let Some(report) = &self.0 {
            report(change);
        }
    }
}

/// A sampler whose thread has not started yet; subscriptions made meanwhile wait for it.
pub struct Pending {
    queue: Receiver<Message>,
}

impl Sampler {
    pub fn new() -> (Self, Pending) {
        let (messages, queue) = mpsc::channel();
        (Self { messages }, Pending { queue })
    }

    /// Replaces `subscriber`'s topics; an empty list unsubscribes it.
    pub fn subscribe(&self, subscriber: &str, requests: Vec<TopicRequest>) {
        let _ = self.messages.send(Message::Subscribe {
            subscriber: subscriber.to_owned(),
            requests,
        });
    }

    /// The system resumed from sleep: every topic is read again at once, from new baselines
    /// (SPEC §4.4).
    pub fn resume(&self) {
        let _ = self.messages.send(Message::Resume);
    }

    /// Something changed: the topics that follow it are read again at once (SPEC §4.4), e.g.
    /// volumes, disks and paths when a drive arrived or went.
    pub fn changed(&self, change: Change) {
        let _ = self.messages.send(Message::Changed(change));
    }

    /// Ends the thread.
    pub fn shutdown(&self) {
        let _ = self.messages.send(Message::Shutdown);
    }
}

impl Pending {
    /// Starts the thread; `deliver` runs on it for every sample.
    pub fn start(
        self,
        providers: Providers,
        deliver: Box<dyn Fn(Delivery) + Send>,
    ) -> std::io::Result<JoinHandle<()>> {
        std::thread::Builder::new()
            .name("sampler".into())
            .spawn(move || run(&self.queue, providers, deliver.as_ref()))
    }
}

fn run(queue: &Receiver<Message>, mut providers: Providers, deliver: &dyn Fn(Delivery)) {
    let mut scheduler: Scheduler<Topic, Arc<Sample>> = Scheduler::default();
    // What changed since the last readings: the topics they concern are read now, pushed.
    let mut changes: Vec<Change> = Vec::new();
    loop {
        let message = match scheduler.next_due() {
            Some(at) => queue.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => queue.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match message {
            Ok(Message::Subscribe {
                subscriber,
                requests,
            }) => {
                let requests: Vec<(Topic, Duration)> = requests
                    .into_iter()
                    .map(|r| (r.topic, r.interval))
                    .collect();
                for (topic, sample) in scheduler.subscribe(&subscriber, &requests, Instant::now()) {
                    deliver(Delivery {
                        topic,
                        sample,
                        to: Some(subscriber.clone()),
                        pushed: false,
                    });
                }
                for topic in scheduler.take_removed() {
                    providers.forget(&topic);
                }
            }
            Ok(Message::Resume) => {
                providers.resume();
                scheduler.resume(Instant::now());
            }
            Ok(Message::Changed(change)) => {
                scheduler.refresh(Instant::now(), |topic| topic.follows(change));
                changes.push(change);
            }
            Ok(Message::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        for topic in scheduler.due(now) {
            let deliver = |sample: &Arc<Sample>| {
                deliver(Delivery {
                    topic: topic.clone(),
                    sample: Arc::clone(sample),
                    to: None,
                    pushed: changes.iter().any(|&c| topic.follows(c)),
                });
            };
            match providers.sample(&topic) {
                Ok(sample) => {
                    let sample = sample.map(Arc::new);
                    if let Some(sample) = &sample {
                        deliver(sample);
                    }
                    if scheduler.sampled(&topic, now, sample) {
                        tracing::info!("{topic:?} can be read again");
                    }
                }
                // After a few in a row, the error is the topic's sample (SPEC §14.2).
                Err(message) => {
                    let error = Arc::new(Sample::Error(message.clone()));
                    if let Some(error) = scheduler.failed(&topic, now, error) {
                        tracing::warn!("{topic:?} cannot be read: {message}");
                        deliver(&error);
                    }
                }
            }
        }
        changes.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn delivers_samples_to_subscribers_until_shut_down() {
        let (sampler, pending) = Sampler::new();
        let request = TopicRequest {
            topic: Topic::Uptime,
            interval: Duration::from_millis(100),
        };
        // Queued before the thread starts.
        sampler.subscribe("s_1", vec![request.clone()]);
        let (tx, rx) = mpsc::channel();
        let thread = pending
            .start(
                Providers::new(crate::platform::current()),
                Box::new(move |d| {
                    let _ = tx.send(d);
                }),
            )
            .expect("the thread starts");
        let wait = Duration::from_secs(5);
        let first = rx.recv_timeout(wait).expect("a sample");
        assert_eq!(
            (first.topic.clone(), first.to.clone()),
            (Topic::Uptime, None)
        );
        assert!(matches!(*first.sample, Sample::Uptime(_)));

        // A second subscriber gets the newest sample at once, addressed to it.
        sampler.subscribe("s_2", vec![request]);
        let catch_up = loop {
            let d = rx.recv_timeout(wait).expect("a delivery");
            if d.to.is_some() {
                break d;
            }
        };
        assert_eq!(catch_up.to.as_deref(), Some("s_2"));

        sampler.subscribe("s_1", Vec::new());
        sampler.subscribe("s_2", Vec::new());
        sampler.shutdown();
        thread.join().expect("the thread ends");
    }
    /// The sampler's cost per tick (SPEC §14.1, ≤ 2 ms): the topics the first-run layout's
    /// sensors subscribe to, read on this machine's providers as a tick reads them, a tick
    /// every 100 ms; the first, which opens the providers, is one of them. CI's budget check
    /// reads the median.
    #[cfg(windows)]
    #[test]
    #[ignore = "measures a budget; run with --ignored"]
    fn budget_sampler_tick() {
        use crate::test_util::{median, record_budget};

        const TICKS: usize = 50;
        let platform = crate::platform::current();
        let caps = platform.capabilities();
        let registry = crate::sensors::registry::registry();
        let mut topics: Vec<Topic> = Vec::new();
        for entry in crate::config::Config::first_run("1.0.0").layout.all() {
            let sensor = registry.create(entry, &crate::sensors::State::new(), &caps);
            for request in sensor.topics() {
                if !topics.contains(&request.topic) {
                    topics.push(request.topic);
                }
            }
        }
        assert!(topics.contains(&Topic::Cpu), "{topics:?}");
        let mut providers = Providers::new(platform);
        let mut times = Vec::with_capacity(TICKS);
        for _ in 0..TICKS {
            let began = Instant::now();
            for topic in &topics {
                let _ = providers.sample(topic);
            }
            times.push(began.elapsed());
            std::thread::sleep(Duration::from_millis(100));
        }
        record_budget(
            "sampler",
            "Sampler cost per tick",
            median(&mut times),
            Duration::from_millis(2),
            TICKS,
        );
    }

    #[test]
    fn a_device_change_reads_the_drive_list_again_at_once() {
        let (sampler, pending) = Sampler::new();
        sampler.subscribe(
            "d",
            vec![TopicRequest {
                topic: Topic::Volumes,
                interval: VOLUMES_EVERY,
            }],
        );
        let (tx, rx) = mpsc::channel();
        let thread = pending
            .start(
                Providers::new(crate::platform::current()),
                Box::new(move |d| {
                    let _ = tx.send(d);
                }),
            )
            .expect("the thread starts");
        let wait = Duration::from_secs(5);
        let first = rx.recv_timeout(wait).expect("the first list");
        assert!(matches!(*first.sample, Sample::Volumes(_)));
        // Not due for a minute, but a drive arrived.
        sampler.changed(Change::Devices);
        let again = rx.recv_timeout(wait).expect("read again");
        assert_eq!((again.topic, again.to), (Topic::Volumes, None));
        assert!(again.pushed, "news for every subscriber");
        assert!(!first.pushed);
        sampler.shutdown();
        thread.join().expect("the thread ends");
    }
}
