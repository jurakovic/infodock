//! `memory` (SPEC Appendix A): physical memory from `GlobalMemoryStatusEx`; the load is the
//! OS's own figure (`dwMemoryLoad`), as Task Manager shows it.

use crate::platform::MemoryStatus;
use crate::sampler::MemorySample;

/// The sample for `status`; what is available never exceeds the total.
pub fn sample(status: MemoryStatus) -> MemorySample {
    let available = status.available.min(status.total);
    MemorySample {
        total: status.total,
        available,
        used: status.total - available,
        load: status.load.min(100) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn used_is_what_is_not_available() {
        let s = sample(MemoryStatus {
            total: 32 * GB,
            available: 18 * GB,
            load: 43,
        });
        assert_eq!(
            (s.total, s.available, s.used, s.load),
            (32 * GB, 18 * GB, 14 * GB, 43.0)
        );
        let odd = sample(MemoryStatus {
            total: GB,
            available: 2 * GB,
            load: 120,
        });
        assert_eq!((odd.available, odd.used, odd.load), (GB, 0, 100.0));
    }
}
