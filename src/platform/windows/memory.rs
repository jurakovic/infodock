//! Physical memory and page files (SPEC Appendix A, `memory` and `swap`):
//! `GlobalMemoryStatusEx`, and `NtQuerySystemInformation(SystemPageFileInformation)`, which
//! gives every page file's size and use in one cheap call (no WMI).

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS};
use windows::Win32::System::SystemInformation::{
    GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};

use crate::platform::{MemoryStatus, PageFile};

/// `SystemPageFileInformation`: one `SYSTEM_PAGEFILE_INFORMATION` per page file.
const PAGE_FILE_INFORMATION: SYSTEM_INFORMATION_CLASS = SYSTEM_INFORMATION_CLASS(18);
/// `STATUS_INFO_LENGTH_MISMATCH`: the buffer is too small.
const LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
/// The largest buffer tried, bytes; a few page files need a few hundred.
const MAX_BUFFER: usize = 64 * 1024;
/// `NextEntryOffset`, `TotalSize`, `TotalInUse` and `PeakUsage` (the ULONGs before the name).
const ENTRY_HEAD: usize = 16;

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

pub fn page_files() -> Option<Vec<PageFile>> {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: `info` is writable.
    unsafe { GetSystemInfo(&mut info) };
    let page = u64::from(info.dwPageSize.max(1));
    // u64 elements keep the buffer aligned for the entries' pointers.
    let mut buffer = vec![0u64; 512];
    loop {
        let size = buffer.len() * size_of::<u64>();
        let mut returned = 0u32;
        // SAFETY: the buffer is writable for `size` bytes; `returned` receives the length used.
        let status = unsafe {
            NtQuerySystemInformation(
                PAGE_FILE_INFORMATION,
                buffer.as_mut_ptr().cast(),
                size as u32,
                &mut returned,
            )
        };
        if status.0 == LENGTH_MISMATCH && size < MAX_BUFFER {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status.is_err() {
            return None;
        }
        let bytes: Vec<u8> = buffer.iter().flat_map(|w| w.to_ne_bytes()).collect();
        let used = (returned as usize).min(bytes.len());
        return Some(parse(&bytes[..used], page));
    }
}

/// The page files in `SYSTEM_PAGEFILE_INFORMATION` entries (sizes in pages of `page` bytes),
/// each `NextEntryOffset` bytes after the one before; nothing for an empty buffer, which is
/// what a system without page files returns.
fn parse(bytes: &[u8], page: u64) -> Vec<PageFile> {
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u32::from_ne_bytes)
    };
    let mut files = Vec::new();
    let mut at = 0;
    while at + ENTRY_HEAD <= bytes.len() {
        files.push(PageFile {
            allocated: u64::from(u32_at(at + 4)) * page,
            used: u64::from(u32_at(at + 8)) * page,
        });
        let next = u32_at(at) as usize;
        if next == 0 {
            break;
        }
        at += next;
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(next: u32, total: u32, in_use: u32) -> Vec<u8> {
        let mut e = Vec::new();
        for v in [next, total, in_use, in_use] {
            e.extend(v.to_ne_bytes());
        }
        // The UNICODE_STRING of the name (not read).
        e.extend([0u8; 16]);
        e
    }

    #[test]
    fn parses_the_entries() {
        let mut bytes = entry(32, 1000, 250);
        bytes.extend(entry(0, 500, 0));
        assert_eq!(
            parse(&bytes, 4096),
            [
                PageFile {
                    allocated: 1000 * 4096,
                    used: 250 * 4096
                },
                PageFile {
                    allocated: 500 * 4096,
                    used: 0
                }
            ]
        );
        assert!(parse(&[], 4096).is_empty(), "no page file");
        assert_eq!(parse(&bytes[..20], 4096).len(), 1, "cut short");
    }

    /// Runs on the Windows CI runner (`cargo test -- --ignored`).
    #[test]
    #[ignore = "reads this machine's memory"]
    fn reads_sane_values() {
        let status = status().expect("readable");
        assert!(status.total >= 1 << 30, "{status:?}");
        assert!(status.available <= status.total, "{status:?}");
        assert!(status.load <= 100, "{status:?}");
        let files = page_files().expect("readable");
        for file in &files {
            assert!(file.used <= file.allocated, "{files:?}");
        }
    }
}
