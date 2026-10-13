//! `cpu` (SPEC Appendix A): the processors' load. User, kernel and idle shares come from the
//! change in processor times since the previous reading; the busy share from the platform's
//! utility counter where it has one (as Task Manager), else from the same times. The first
//! reading after a start or resume only sets the baseline.

use crate::platform::{CpuReader, CpuReading, CpuTimes};
use crate::sampler::CpuSample;

pub struct Cpu {
    reader: Box<dyn CpuReader>,
    previous: Option<CpuTimes>,
}

impl Cpu {
    pub fn new(reader: Box<dyn CpuReader>) -> Self {
        Self {
            reader,
            previous: None,
        }
    }

    /// Reads the processors; `None` for the first reading (it only sets the baseline) and when
    /// nothing could be read.
    pub fn sample(&mut self) -> Option<CpuSample> {
        let reading = self.reader.read()?;
        let previous = self.previous.replace(reading.times)?;
        Some(sample(previous, &reading))
    }

    /// The next reading starts a new baseline (SPEC §4.4: after resume).
    pub fn resume(&mut self) {
        self.previous = None;
    }
}

/// The sample from the processor times at the previous reading and `now`.
pub fn sample(previous: CpuTimes, now: &CpuReading) -> CpuSample {
    let delta = |before: u64, after: u64| after.saturating_sub(before) as f64;
    let idle = delta(previous.idle, now.times.idle);
    // Kernel time includes idle time.
    let kernel = delta(previous.kernel, now.times.kernel);
    let user = delta(previous.user, now.times.user);
    let all = kernel + user;
    let share = |part: f64| {
        if all > 0.0 {
            (100.0 * part / all).clamp(0.0, 100.0) as f32
        } else {
            0.0
        }
    };
    let busy = share(all - idle);
    // NaN (a processor that could not be read) stays NaN and keeps its place.
    let percent = |value: f64| value.clamp(0.0, 100.0) as f32;
    CpuSample {
        total: now.total.map_or(busy, percent),
        user: share(user),
        kernel: share((kernel - idle).max(0.0)),
        idle: if all > 0.0 { share(idle) } else { 100.0 },
        cores: now.cores.iter().copied().map(percent).collect(),
        logical: now.logical.max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(idle: u64, kernel: u64, user: u64) -> CpuTimes {
        CpuTimes { idle, kernel, user }
    }

    #[test]
    fn shares_from_time_deltas() {
        // 1000 ticks passed: 600 idle, 150 kernel (750 − 600), 250 user.
        let now = CpuReading {
            times: times(1600, 1750, 1250),
            total: None,
            cores: Vec::new(),
            logical: 8,
        };
        let s = sample(times(1000, 1000, 1000), &now);
        assert_eq!(
            (s.total, s.user, s.kernel, s.idle),
            (40.0, 25.0, 15.0, 60.0)
        );
        assert_eq!(s.logical, 8);
    }

    #[test]
    fn utility_counter_wins_and_is_clamped() {
        let now = CpuReading {
            times: times(10, 10, 10),
            total: Some(123.0),
            cores: vec![12.5, -1.0, 150.0],
            logical: 0,
        };
        let s = sample(times(0, 0, 0), &now);
        assert_eq!(s.total, 100.0);
        assert_eq!(s.cores, [12.5, 0.0, 100.0]);
        assert_eq!(s.logical, 1);
        // No time passed (or the counters went backwards): idle.
        let s = sample(times(10, 10, 10), &CpuReading::default());
        assert_eq!((s.total, s.user, s.kernel, s.idle), (0.0, 0.0, 0.0, 100.0));
    }

    struct Fixed(Vec<CpuReading>);

    impl CpuReader for Fixed {
        fn read(&mut self) -> Option<CpuReading> {
            (!self.0.is_empty()).then(|| self.0.remove(0))
        }
    }

    #[test]
    fn the_first_reading_sets_the_baseline() {
        let reading = |t: u64| CpuReading {
            times: times(t, 2 * t, t),
            ..CpuReading::default()
        };
        let mut cpu = Cpu::new(Box::new(Fixed(vec![
            reading(0),
            reading(100),
            reading(200),
            reading(300),
        ])));
        assert!(cpu.sample().is_none(), "discarded");
        assert!(cpu.sample().is_some());
        cpu.resume();
        assert!(cpu.sample().is_none(), "a new baseline after resume");
        assert!(cpu.sample().is_some());
        assert!(cpu.sample().is_none(), "nothing more to read");
    }
}
