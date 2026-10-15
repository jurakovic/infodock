//! `disk/<X:\>` (SPEC Appendix A): a volume's size and free space. A drive that is missing or
//! has no medium is a sample too (`present: false`), so sensors can show it as absent. Volumes
//! are read on a worker thread (`worker.rs`), since a network drive can stall.

use crate::platform::DiskSpace;
use crate::sampler::DiskSample;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
