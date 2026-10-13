//! Which topics to sample when (SPEC §4.4): each topic is sampled at the shortest interval its
//! subscribers asked for and keeps its newest sample for subscribers that join later. Pure; the
//! sampler thread supplies the clock.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// No topic is sampled more often than this, whatever a sensor asks for.
pub const MIN_INTERVAL: Duration = Duration::from_millis(100);

struct Entry<S> {
    /// Each subscriber's interval.
    subscribers: HashMap<String, Duration>,
    next: Instant,
    last: Option<Instant>,
    latest: Option<S>,
}

impl<S> Entry<S> {
    fn interval(&self) -> Duration {
        self.subscribers
            .values()
            .copied()
            .min()
            .unwrap_or(MIN_INTERVAL)
    }
}

pub struct Scheduler<K, S> {
    topics: HashMap<K, Entry<S>>,
}

impl<K, S> Default for Scheduler<K, S> {
    fn default() -> Self {
        Self {
            topics: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, S: Clone> Scheduler<K, S> {
    /// Replaces `subscriber`'s requests (an empty list unsubscribes it). Returns the newest
    /// sample of each requested topic, if there is one yet, since the subscriber may be new
    /// (or a re-created sensor with the same id); a new topic is due at once.
    pub fn subscribe(
        &mut self,
        subscriber: &str,
        requests: &[(K, Duration)],
        now: Instant,
    ) -> Vec<(K, S)> {
        // It leaves every topic, then joins the requested ones again.
        for entry in self.topics.values_mut() {
            entry.subscribers.remove(subscriber);
        }
        let mut catch_up = Vec::new();
        for (topic, interval) in requests {
            let interval = (*interval).max(MIN_INTERVAL);
            let entry = self.topics.entry(topic.clone()).or_insert_with(|| Entry {
                subscribers: HashMap::new(),
                next: now,
                last: None,
                latest: None,
            });
            entry
                .subscribers
                .entry(subscriber.to_owned())
                .and_modify(|d| *d = (*d).min(interval))
                .or_insert(interval);
            // A shorter interval applies from the last sample on.
            if let Some(last) = entry.last {
                entry.next = entry.next.min(last + entry.interval());
            }
            if let Some(sample) = &entry.latest
                && !catch_up.iter().any(|(t, _)| t == topic)
            {
                catch_up.push((topic.clone(), sample.clone()));
            }
        }
        self.topics.retain(|_, entry| !entry.subscribers.is_empty());
        catch_up
    }

    /// After the system resumed: every topic is due now, and the cadence starts again from the
    /// next reading (SPEC §4.4).
    pub fn resume(&mut self, now: Instant) {
        for entry in self.topics.values_mut() {
            entry.next = now;
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

    /// Records that `topic` was sampled at `now` (`None`: it could not be read) and schedules
    /// the next sample, keeping the cadence unless the sample came more than an interval late.
    pub fn sampled(&mut self, topic: &K, now: Instant, sample: Option<S>) {
        let Some(entry) = self.topics.get_mut(topic) else {
            return;
        };
        let interval = entry.interval();
        entry.last = Some(now);
        let next = entry.next + interval;
        entry.next = if next > now { next } else { now + interval };
        if sample.is_some() {
            entry.latest = sample;
        }
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
        let mut s: Scheduler<&str, u32> = Scheduler::default();
        assert_eq!(s.next_due(), None);
        assert!(s.subscribe("a", &requests(&[("cpu", 2000)]), t0).is_empty());
        assert!(s.subscribe("b", &requests(&[("cpu", 1000)]), t0).is_empty());
        assert_eq!(s.interval(&"cpu"), Some(SEC));
        // A new topic is due at once.
        assert_eq!(s.due(t0), ["cpu"]);
        s.sampled(&"cpu", t0, Some(1));
        assert_eq!(s.next_due(), Some(t0 + SEC));
        assert!(s.due(t0 + SEC / 2).is_empty());

        // The fast subscriber leaves: the slower interval applies after the next sample.
        s.subscribe("b", &[], t0 + SEC / 2);
        assert_eq!(s.due(t0 + SEC), ["cpu"]);
        s.sampled(&"cpu", t0 + SEC, Some(2));
        assert_eq!(s.next_due(), Some(t0 + 3 * SEC));

        // The last subscriber leaves: nothing is sampled.
        s.subscribe("a", &[], t0 + SEC);
        assert_eq!(s.next_due(), None);
    }

    #[test]
    fn resuming_makes_every_topic_due() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32> = Scheduler::default();
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
        let mut s: Scheduler<&str, u32> = Scheduler::default();
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
        let mut s: Scheduler<&str, u32> = Scheduler::default();
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
    fn intervals_have_a_floor_and_failed_reads_keep_the_old_sample() {
        let t0 = Instant::now();
        let mut s: Scheduler<&str, u32> = Scheduler::default();
        s.subscribe("a", &requests(&[("cpu", 0)]), t0);
        assert_eq!(s.interval(&"cpu"), Some(MIN_INTERVAL));
        s.sampled(&"cpu", t0, Some(1));
        s.sampled(&"cpu", t0 + MIN_INTERVAL, None);
        assert_eq!(s.subscribe("b", &requests(&[("cpu", 0)]), t0), [("cpu", 1)]);
    }
}
