//! `swap` (SPEC Appendix A): all page files together, as the platform reports their
//! allocated sizes and use.

use crate::platform::PageFile;
use crate::sampler::SwapSample;

/// The sample for these page files; none gives a total of 0 (no page file, SPEC §9.11).
pub fn sample(files: &[PageFile]) -> SwapSample {
    let total: u64 = files.iter().map(|f| f.allocated).sum();
    let used: u64 = files.iter().map(|f| f.used.min(f.allocated)).sum();
    let load = if total > 0 {
        (100.0 * used as f64 / total as f64) as f32
    } else {
        0.0
    };
    SwapSample { total, used, load }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1 << 20;

    #[test]
    fn page_files_add_up() {
        let files = [
            PageFile {
                allocated: 6 * 1024 * MB,
                used: 400 * MB,
            },
            PageFile {
                allocated: 2 * 1024 * MB,
                used: 3 * 1024 * MB,
            },
        ];
        let s = sample(&files);
        assert_eq!((s.total, s.used), (8 * 1024 * MB, 2448 * MB));
        assert!((s.load - 29.88).abs() < 0.01, "{}", s.load);
        assert_eq!(sample(&[]), SwapSample::default(), "no page file");
    }
}
