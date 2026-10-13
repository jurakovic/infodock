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
const NAME_AT: usize = 16;
/// The name is a `UNICODE_STRING`: its length in bytes, then (aligned) the buffer's address.
const NAME_BUFFER_AT: usize = NAME_AT + size_of::<usize>();
const ENTRY_SIZE: usize = NAME_AT + 2 * size_of::<usize>();

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
        let base = buffer.as_ptr() as usize;
        let bytes: Vec<u8> = buffer.iter().flat_map(|w| w.to_ne_bytes()).collect();
        let used = (returned as usize).min(bytes.len());
        return Some(parse(&bytes[..used], base, page));
    }
}

/// The page files in `SYSTEM_PAGEFILE_INFORMATION` entries (sizes in pages of `page` bytes),
/// each `NextEntryOffset` bytes after the one before; nothing for an empty buffer, which is
/// what a system without page files returns. `bytes` were returned at address `base`, which
/// the entries' names point into; `swapfile.sys` (where Windows swaps out whole apps) is not a
/// page file.
fn parse(bytes: &[u8], base: usize, page: u64) -> Vec<PageFile> {
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u32::from_ne_bytes)
    };
    let name_at = |at: usize| {
        let length = bytes
            .get(at + NAME_AT..at + NAME_AT + 2)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u16::from_ne_bytes) as usize;
        let address = bytes
            .get(at + NAME_BUFFER_AT..at + ENTRY_SIZE)
            .and_then(|b| b.try_into().ok())
            .map_or(0, usize::from_ne_bytes);
        // Only a name inside the returned bytes is read.
        let start = address.checked_sub(base)?;
        let text = bytes.get(start..start.checked_add(length)?)?;
        let units: Vec<u16> = text
            .chunks_exact(2)
            .map(|c| u16::from_ne_bytes([c[0], c[1]]))
            .collect();
        Some(String::from_utf16_lossy(&units))
    };
    let mut files = Vec::new();
    let mut at = 0;
    while at + ENTRY_SIZE <= bytes.len() {
        let swap_file =
            name_at(at).is_some_and(|name| name.to_ascii_lowercase().ends_with("\\swapfile.sys"));
        if !swap_file {
            files.push(PageFile {
                allocated: u64::from(u32_at(at + 4)) * page,
                used: u64::from(u32_at(at + 8)) * page,
            });
        }
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

    const BASE: usize = 0x1000;

    /// An entry whose name (`length` bytes) is at offset `name` of the buffer.
    fn entry(next: u32, total: u32, in_use: u32, name: usize, length: u16) -> Vec<u8> {
        let mut e = Vec::new();
        for v in [next, total, in_use, in_use] {
            e.extend(v.to_ne_bytes());
        }
        e.extend(length.to_ne_bytes());
        e.extend(length.to_ne_bytes());
        e.resize(NAME_BUFFER_AT, 0);
        e.extend((BASE + name).to_ne_bytes());
        e
    }

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_ne_bytes).collect()
    }

    #[test]
    fn parses_the_entries() {
        let pagefile = utf16(r"\??\C:\pagefile.sys");
        let swapfile = utf16(r"\??\C:\swapfile.sys");
        let names = 3 * ENTRY_SIZE;
        let mut bytes = entry(ENTRY_SIZE as u32, 1000, 250, names, pagefile.len() as u16);
        bytes.extend(entry(ENTRY_SIZE as u32, 500, 0, 0x7000, 4));
        let swap_name = names + pagefile.len();
        bytes.extend(entry(0, 64, 1, swap_name, swapfile.len() as u16));
        bytes.extend(&pagefile);
        bytes.extend(&swapfile);
        assert_eq!(
            parse(&bytes, BASE, 4096),
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
        assert!(parse(&[], BASE, 4096).is_empty(), "no page file");
        let first = &bytes[..ENTRY_SIZE + 4];
        assert_eq!(parse(first, BASE, 4096).len(), 1, "cut short");
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
