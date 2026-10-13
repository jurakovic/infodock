//! `net/<id|auto>` (SPEC Appendix A): a network interface's traffic. Rates come from the
//! change in its byte counters since the previous reading of the same topic, timed with the
//! monotonic clock; a counter that went backwards (a reset), another interface behind
//! Automatic, the first reading and the first after resume all give 0 and a new baseline.

use std::collections::HashMap;
use std::time::Instant;

use crate::platform::NetInterface;
use crate::sampler::NetSample;

/// The counters of the previous reading.
#[derive(Debug, Clone)]
struct Baseline {
    id: String,
    at: Instant,
    rx: u64,
    tx: u64,
}

/// The baselines per topic (`None`: Automatic).
#[derive(Default)]
pub struct Net {
    baselines: HashMap<Option<String>, Baseline>,
}

impl Net {
    /// The sample for the topic `key` from `interface` as read `now`; an interface that is
    /// not there gives an empty sample.
    pub fn sample(
        &mut self,
        key: &Option<String>,
        interface: Option<NetInterface>,
        now: Instant,
    ) -> NetSample {
        let Some(i) = interface else {
            self.baselines.remove(key);
            return NetSample::default();
        };
        let baseline = Baseline {
            id: i.id.clone(),
            at: now,
            rx: i.rx_bytes,
            tx: i.tx_bytes,
        };
        let (rx_rate, tx_rate) = self
            .baselines
            .insert(key.clone(), baseline)
            .filter(|previous| previous.id == i.id)
            .map_or((0.0, 0.0), |previous| rates(&previous, &i, now));
        NetSample {
            present: true,
            id: i.id,
            alias: i.alias,
            description: i.description,
            up: i.up,
            rx_rate,
            tx_rate,
            rx_total: i.rx_bytes,
            tx_total: i.tx_bytes,
            link_rx: i.link_rx,
            link_tx: i.link_tx,
        }
    }

    /// The next reading of every topic starts a new baseline (SPEC §4.4: after resume).
    pub fn resume(&mut self) {
        self.baselines.clear();
    }
}

/// Bytes per second received and sent since `previous`; 0 for a counter that went back.
fn rates(previous: &Baseline, now_counters: &NetInterface, now: Instant) -> (f64, f64) {
    let seconds = now.saturating_duration_since(previous.at).as_secs_f64();
    if seconds <= 0.0 {
        return (0.0, 0.0);
    }
    let rate = |before: u64, after: u64| {
        after
            .checked_sub(before)
            .map_or(0.0, |delta| delta as f64 / seconds)
    };
    (
        rate(previous.rx, now_counters.rx_bytes),
        rate(previous.tx, now_counters.tx_bytes),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn wifi(rx: u64, tx: u64) -> NetInterface {
        NetInterface {
            id: "WIFI".into(),
            alias: "Wi-Fi".into(),
            up: true,
            rx_bytes: rx,
            tx_bytes: tx,
            ..NetInterface::default()
        }
    }

    #[test]
    fn rates_from_counter_deltas() {
        let mut net = Net::default();
        let t0 = Instant::now();
        let auto = None;
        let first = net.sample(&auto, Some(wifi(1000, 500)), t0);
        assert!(first.present && first.up);
        assert_eq!(
            (first.rx_rate, first.tx_rate),
            (0.0, 0.0),
            "the first reading"
        );
        let s = net.sample(&auto, Some(wifi(3000, 1500)), t0 + Duration::from_secs(2));
        assert_eq!((s.rx_rate, s.tx_rate), (1000.0, 500.0));
        assert_eq!((s.rx_total, s.alias.as_str()), (3000, "Wi-Fi"));
    }

    #[test]
    fn resets_resume_and_other_interfaces_start_again() {
        let mut net = Net::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let auto = None;
        net.sample(&auto, Some(wifi(5000, 5000)), at(0));
        let reset = net.sample(&auto, Some(wifi(100, 6000)), at(1));
        assert_eq!(
            (reset.rx_rate, reset.tx_rate),
            (0.0, 1000.0),
            "rx went back"
        );
        let after = net.sample(&auto, Some(wifi(1100, 6000)), at(2));
        assert_eq!(after.rx_rate, 1000.0, "from the new baseline");

        let ethernet = NetInterface {
            id: "ETH".into(),
            rx_bytes: 9_000_000,
            ..wifi(0, 0)
        };
        let switched = net.sample(&auto, Some(ethernet), at(3));
        assert_eq!(
            switched.rx_rate, 0.0,
            "Automatic moved to another interface"
        );

        net.resume();
        let resumed = net.sample(&auto, Some(wifi(2100, 7000)), at(4));
        assert_eq!(resumed.rx_rate, 0.0, "a new baseline after resume");

        let gone = net.sample(&auto, None, at(5));
        assert_eq!(gone, NetSample::default());
    }
}
