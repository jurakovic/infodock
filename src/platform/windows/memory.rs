//! Physical memory (SPEC Appendix A, `memory`): `GlobalMemoryStatusEx`.

use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use crate::platform::MemoryStatus;

pub fn status() -> Option<MemoryStatus> {
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is writable and its length field is set, as the call requires.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(MemoryStatus {
        total: status.ullTotalPhys,
        available: status.ullAvailPhys,
        load: status.dwMemoryLoad,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`).
    #[test]
    #[ignore = "reads this machine's memory"]
    fn reads_sane_values() {
        let status = status().expect("readable");
        assert!(status.total >= 1 << 30, "{status:?}");
        assert!(status.available <= status.total, "{status:?}");
        assert!(status.load <= 100, "{status:?}");
    }
}
