//! `uptime` (Appendix A): the time since the system started, from `GetTickCount64`.

use std::time::{Duration, SystemTime};

use crate::sampler::UptimeSample;

/// The sample for `uptime` read at `now`.
pub fn sample(uptime: Duration, now: SystemTime) -> UptimeSample {
    let boot = now.checked_sub(uptime).unwrap_or(SystemTime::UNIX_EPOCH);
    let boot_unix_ms = boot
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    UptimeSample {
        boot_unix_ms,
        uptime_secs: uptime.as_secs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_time_is_now_minus_uptime() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let uptime = Duration::from_millis(3 * 86_400_000 + 4_500);
        assert_eq!(
            sample(uptime, now),
            UptimeSample {
                boot_unix_ms: 1_800_000_000_000 - 3 * 86_400_000 - 4_500,
                uptime_secs: 3 * 86_400 + 4,
            }
        );
    }
}
