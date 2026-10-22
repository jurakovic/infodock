//! Which topics to sample when (SPEC §4.4): each topic is sampled at the shortest interval its
//! subscribers asked for and keeps its newest sample for subscribers that join later. All
//! topics keep to one grid, counted from the first sample, so topics with whole-second
//! intervals are read (and drawn) together. A topic that keeps failing turns into an error
//! sample and is retried less and less often (SPEC §14.2). Pure; the sampler thread supplies
//! the clock.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// No topic is sampled more often than this, whatever a sensor asks for.
const MIN_INTERVAL: Duration = Duration::from_millis(100);
/// Failed readings in a row before a topic's sample is an error (SPEC §14.2); until then
/// subscribers keep the previous sample.
const FAILURES_BEFORE_ERROR: u32 = 3;
/// The first retry after the error sample comes this much later, each next one twice as late,
/// up to [`MAX_BACKOFF`] (and never sooner than the topic's interval).
const FIRST_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

struct Entry<S, U> {
    /// Each subscriber's interval.
    subscribers: HashMap<U, Duration>,
    next: Instant,
    last: Option<Instant>,
    latest: Option<S>,
    /// Failed readings since the last success.
    failures: u32,
}

impl<S, U> Entry<S, U> {
    fn interval(&self) -> Duration {
        self.subscribers
            .values()
            .copied()
            .min()
            .unwrap_or(MIN_INTERVAL)
    }
}

/// Topics `K` with samples `S` for subscribers `U`.
pub struct Scheduler<K, S, U> {
    topics: HashMap<K, Entry<S, U>>,
    /// The first sample; every cadence is counted from it.
    epoch: Option<Instant>,
    /// Topics whose last subscriber left, for [`Scheduler::take_removed`].
    removed: Vec<K>,
}

impl<K, S, U> Default for Scheduler<K, S, U> {
    fn default() -> Self {
        Self {
            topics: HashMap::new(),
            epoch: None,
            removed: Vec::new(),
        }
    }
}

/// The first point after `at` of the grid of `interval` from `epoch`.
fn grid_after(epoch: Instant, interval: Duration, at: Instant) -> Instant {
    let step = interval.as_nanos().max(1);
    let steps = at.saturating_duration_since(epoch).as_nanos() / step + 1;
    epoch + Duration::from_nanos(u64::try_from(steps * step).unwrap_or(u64::MAX))
}

/// How long after the `failures`-th failed reading in a row the next try comes, at least:
/// nothing extra before the error sample, then 1 s, 2 s, 4 s … up to 30 s (SPEC §14.2).
fn backoff(failures: u32) -> Duration {
    match failures.checked_sub(FAILURES_BEFORE_ERROR) {
        None => Duration::ZERO,
        Some(n) => FIRST_BACKOFF
            .saturating_mul(1 << n.min(16))
            .min(MAX_BACKOFF),
    }
}

impl<K: Clone + Eq + Hash, S: Clone + PartialEq, U: Eq + Hash> Scheduler<K, S, U> {
    /// Replaces `subscriber`'s requests (an empty list unsubscribes it). Returns the newest
    /// sample of each requested topic, if there is one yet, since the subscriber may be new
    /// (or a re-created sensor with the same id); a new topic is due at once.
    pub fn subscribe(
        &mut self,
        subscriber: U,
        requests: &[(K, Duration)],
        now: Instant,
    ) -> Vec<(K, S)>
    where
        U: Clone,
    {
        // It leaves every topic, then joins the requested ones again.
        for entry in self.topics.values_mut() {
            entry.subscribers.remove(&subscriber);
        }
        let mut catch_up = Vec::new();
        for (topic, interval) in requests {
            let interval = (*interval).max(MIN_INTERVAL);
            let entry = self.topics.entry(topic.clone()).or_insert_with(|| Entry {
                subscribers: HashMap::new(),
                next: now,
                last: None,
                latest: None,
                failures: 0,
            });
            entry
                .subscribers
                .entry(subscriber.clone())
                .and_modify(|d| *d = (*d).min(interval))
                .or_insert(interval);
            // A shorter interval applies from the last sample on.
            if let (Some(last), Some(epoch)) = (entry.last, self.epoch) {
                entry.next = entry.next.min(grid_after(epoch, entry.interval(), last));
            }
            if let Some(sample) = &entry.latest
                && !catch_up.iter().any(|(t, _)| t == topic)
            {
                catch_up.push((topic.clone(), sample.clone()));
            }
        }
        for (topic, entry) in &self.topics {
            if entry.subscribers.is_empty() {
                self.removed.push(topic.clone());
            }
        }
        self.topics.retain(|_, entry| !entry.subscribers.is_empty());
        catch_up
    }

    /// The topics nobody subscribes to any more since the last call, so their providers can
    /// drop what they keep between readings.
    pub fn take_removed(&mut self) -> Vec<K> {
        let subscribed = &self.topics;
        let mut removed = std::mem::take(&mut self.removed);
        // Left and joined again meanwhile: still in use.
        removed.retain(|topic| !subscribed.contains_key(topic));
        removed
    }

    /// After the system resumed: every topic is due now, and the cadence starts again from the
    /// next reading (SPEC §4.4).
    pub fn resume(&mut self, now: Instant) {
        for entry in self.topics.values_mut() {
            entry.next = now;
        }
    }

    /// The topics `which` picks are due now, e.g. after a drive arrived (SPEC §4.4); the
    /// cadence goes on from that reading.
    pub fn refresh(&mut self, now: Instant, which: impl Fn(&K) -> bool) {
        for (topic, entry) in &mut self.topics {
            if which(topic) {
                entry.next = now;
            }
        }
    }

    /// The topics to sample now.
    pub fn due(&self, now: Instant) -> Vec<K> {
        self.topics
            .iter()
            .filter(|(_, entry)| entry.next <= now)
            .map(|(topic, _)| topic.clone())
            .collect()
    }

    /// Records that `topic` was read at `now` (`None`: nothing to show yet, e.g. a first
    /// reading that only sets a baseline) and schedules the next sample at the grid's next
    /// point for its interval. Returns whether the topic had failed until now.
    pub fn sampled(&mut self, topic: &K, now: Instant, sample: Option<S>) -> bool {
        let epoch = *self.epoch.get_or_insert(now);
        let Some(entry) = self.topics.get_mut(topic) else {
            return false;
        };
        entry.last = Some(now);
        entry.next = grid_after(epoch, entry.interval(), now);
        let failed = entry.failures >= FAILURES_BEFORE_ERROR;
        if sample.is_some() {
            entry.latest = sample;
            entry.failures = 0;
        }
        failed && entry.failures == 0
    }

    /// Records that `topic` could not be read at `now` (SPEC §14.2). Until the third failure
    /// in a row its subscribers keep the previous sample and the next try comes at the next
    /// tick; from then on its sample is `error`, returned when it differs from the one
    /// delivered last, and the tries back off from 1 s to 30 s.
    pub fn failed(&mut self, topic: &K, now: Instant, error: S) -> Option<S> {
        let epoch = *self.epoch.get_or_insert(now);
        let entry = self.topics.get_mut(topic)?;
        entry.last = Some(now);
        entry.failures = entry.failures.saturating_add(1);
        entry.next = grid_after(epoch, entry.interval(), now).max(now + backoff(entry.failures));
        if entry.failures < FAILURES_BEFORE_ERROR || entry.latest.as_ref() == Some(&error) {
            return None;
        }
        entry.latest = Some(error.clone());
        Some(error)
    }

    /// When the next topic is due; `None` without subscribers.
    pub fn next_due(&self) -> Option<Instant> {
        self.topics.values().map(|entry| entry.next).min()
    }

    /// The interval `topic` is sampled at.
    #[cfg(test)]
    fn interval(&self, topic: &K) -> Option<Duration> {
        self.topics.get(topic).map(Entry::interval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: Duration = Duration::from_secs(1);

    fn requests(list: &[(&'static str, u64)]) -> Vec<(&'static str, Duration)> {
        list.iter()
            .map(|(k, ms)| (*k, Duration::from_millis(*ms)))
            .collect()
    }

    #[test]
    fn samples_each_topic_at_its_shortest_interval() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        assert_eq!(s.next_due(), None);
        assert!(s.subscribe("a", &requests(&[("cpu", 2000)]), t0).is_empty());
        assert!(s.subscribe("b", &requests(&[("cpu", 1000)]), t0).is_empty());
        assert_eq!(s.interval(&"cpu"), Some(SEC));
        // A new topic is due at once.
        assert_eq!(s.due(t0), ["cpu"]);
        s.sampled(&"cpu", t0, Some(1));
        assert_eq!(s.next_due(), Some(t0 + SEC));
        assert!(s.due(t0 + SEC / 2).is_empty());

        // The fast subscriber leaves: the slower interval applies after the next sample, on
        // the grid from the first sample.
        s.subscribe("b", &[], t0 + SEC / 2);
        assert_eq!(s.due(t0 + SEC), ["cpu"]);
        s.sampled(&"cpu", t0 + SEC, Some(2));
        assert_eq!(s.next_due(), Some(t0 + 2 * SEC));
        assert!(s.take_removed().is_empty());

        // The last subscriber leaves: nothing is sampled, and the topic is reported once.
        s.subscribe("a", &[], t0 + SEC);
        assert_eq!(s.next_due(), None);
        assert_eq!(s.take_removed(), ["cpu"]);
        assert!(s.take_removed().is_empty());
    }

    #[test]
    fn settings_and_a_sensor_called_settings_are_two_subscribers() {
        use crate::sampler::Subscriber;
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, Subscriber> = Scheduler::default();
        let sensor = Subscriber::Sensor("settings".into());
        s.subscribe(sensor.clone(), &requests(&[("volumes", 60_000)]), t0);
        s.subscribe(Subscriber::Settings, &requests(&[("volumes", 1000)]), t0);
        assert_eq!(s.interval(&"volumes"), Some(SEC));
        // Settings leaves; the sensor still reads the topic at its own interval.
        s.subscribe(Subscriber::Settings, &[], t0);
        assert!(s.take_removed().is_empty());
        assert_eq!(s.interval(&"volumes"), Some(60 * SEC));
        s.subscribe(sensor, &[], t0);
        assert_eq!(s.take_removed(), ["volumes"]);
    }

    #[test]
    fn refreshed_topics_are_due_at_once() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        s.subscribe("a", &requests(&[("volumes", 60_000), ("cpu", 1000)]), t0);
        for topic in s.due(t0) {
            s.sampled(&topic, t0, Some(1));
        }
        let later = t0 + SEC / 2;
        assert!(s.due(later).is_empty());
        s.refresh(later, |t| *t == "volumes");
        assert_eq!(s.due(later), ["volumes"]);
        s.sampled(&"volumes", later, Some(2));
        assert_eq!(s.due(later), Vec::<&str>::new());
        // Nobody subscribes to a disk: nothing to refresh.
        s.refresh(later, |t| *t == "disk");
        assert!(s.due(later).is_empty());
    }

    #[test]
    fn topics_keep_to_one_grid() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        s.subscribe("a", &requests(&[("cpu", 1000)]), t0);
        s.sampled(&"cpu", t0, Some(1));
        // Memory joins 300 ms later (opening the CPU counters took that long): read at once,
        // then on the same whole seconds as the CPU.
        let later = t0 + Duration::from_millis(300);
        s.subscribe("b", &requests(&[("memory", 2000)]), later);
        assert_eq!(s.due(later), ["memory"]);
        s.sampled(&"memory", later, Some(1));
        assert_eq!(s.next_due(), Some(t0 + SEC));
        s.sampled(&"cpu", t0 + SEC, Some(2));
        let mut due = s.due(t0 + 2 * SEC);
        due.sort_unstable();
        assert_eq!(due, ["cpu", "memory"]);
        // Leaving and joining again before anyone asked: not removed.
        s.subscribe("b", &[], later);
        s.subscribe("b", &requests(&[("memory", 2000)]), later);
        assert!(s.take_removed().is_empty());
    }

    #[test]
    fn resuming_makes_every_topic_due() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        s.subscribe("a", &requests(&[("uptime", 60_000), ("cpu", 1000)]), t0);
        for topic in s.due(t0) {
            s.sampled(&topic, t0, Some(1));
        }
        assert!(s.due(t0 + SEC / 2).is_empty());
        s.resume(t0 + SEC / 2);
        let mut due = s.due(t0 + SEC / 2);
        due.sort_unstable();
        assert_eq!(due, ["cpu", "uptime"]);
    }

    #[test]
    fn a_late_subscriber_gets_the_newest_sample() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        s.subscribe("a", &requests(&[("uptime", 60_000)]), t0);
        s.sampled(&"uptime", t0, Some(7));
        let joined = s.subscribe("b", &requests(&[("uptime", 60_000)]), t0 + SEC);
        assert_eq!(joined, [("uptime", 7)]);
        // Subscribing again (a re-created sensor keeps its id) catches up again.
        assert_eq!(
            s.subscribe("b", &requests(&[("uptime", 30_000)]), t0 + SEC),
            [("uptime", 7)]
        );
        // A shorter interval applies from the last sample on.
        assert_eq!(s.next_due(), Some(t0 + 30 * SEC));
    }

    #[test]
    fn keeps_the_cadence_and_restarts_after_a_long_delay() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        s.subscribe("a", &requests(&[("cpu", 1000)]), t0);
        s.sampled(&"cpu", t0, Some(1));
        // A little late: the next one stays on the grid.
        s.sampled(&"cpu", t0 + SEC + SEC / 10, Some(2));
        assert_eq!(s.next_due(), Some(t0 + 2 * SEC));
        // Much later (e.g. after sleep): one interval from now.
        let late = t0 + 10 * SEC;
        s.sampled(&"cpu", late, Some(3));
        assert_eq!(s.next_due(), Some(late + SEC));
    }

    #[test]
    fn intervals_have_a_floor_and_empty_reads_keep_the_old_sample() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        s.subscribe("a", &requests(&[("cpu", 0)]), t0);
        assert_eq!(s.interval(&"cpu"), Some(MIN_INTERVAL));
        s.sampled(&"cpu", t0, Some(1));
        s.sampled(&"cpu", t0 + MIN_INTERVAL, None);
        assert_eq!(s.subscribe("b", &requests(&[("cpu", 0)]), t0), [("cpu", 1)]);
    }

    #[test]
    fn the_third_failure_in_a_row_is_an_error_and_retries_back_off() {
        const ERROR: u32 = 99;
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        s.subscribe("a", &requests(&[("memory", 1000)]), t0);
        s.sampled(&"memory", t0, Some(1));
        // Two failures: the old sample stays and the next tries come at the next ticks.
        let mut at = t0 + SEC;
        for _ in 0..2 {
            assert_eq!(s.failed(&"memory", at, ERROR), None);
            assert_eq!(s.next_due(), Some(at + SEC));
            at += SEC;
        }
        assert_eq!(
            s.subscribe("b", &requests(&[("memory", 1000)]), at),
            [("memory", 1)]
        );
        // The third: the error is the sample now, for late subscribers too.
        assert_eq!(s.failed(&"memory", at, ERROR), Some(ERROR));
        assert_eq!(
            s.subscribe("c", &requests(&[("memory", 1000)]), at),
            [("memory", ERROR)]
        );
        // Retries back off: 1 s, 2 s, 4 s … 30 s; the same error is not delivered again.
        let mut waits = Vec::new();
        for _ in 0..7 {
            let next = s.next_due().expect("due");
            waits.push((next - at).as_secs());
            at = next;
            assert_eq!(s.failed(&"memory", at, ERROR), None);
        }
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        // Another message is news.
        assert_eq!(s.failed(&"memory", at, ERROR + 1), Some(ERROR + 1));
        // The first success: the sample is back, and so is the topic's interval on its grid.
        let back = s.next_due().expect("due");
        assert!(s.sampled(&"memory", back, Some(2)));
        assert_eq!(s.next_due(), Some(grid_after(t0, SEC, back)));
        assert_eq!(
            s.subscribe("d", &requests(&[("memory", 1000)]), back),
            [("memory", 2)]
        );
        // And the count starts again.
        assert!(!s.sampled(&"memory", back + SEC, Some(3)));
        assert_eq!(s.failed(&"memory", back + 2 * SEC, ERROR), None);
    }

    #[test]
    fn a_slow_topic_is_not_retried_sooner_than_its_interval() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32, &str> = Scheduler::default();
        s.subscribe("a", &requests(&[("volumes", 60_000)]), t0);
        for n in 0..4 {
            let at = t0 + 60 * n * SEC;
            s.failed(&"volumes", at, 0);
            assert_eq!(s.next_due(), Some(at + 60 * SEC));
        }
        // A change still reads it at once.
        let later = t0 + 200 * SEC;
        s.refresh(later, |_| true);
        assert_eq!(s.due(later), ["volumes"]);
    }
}
