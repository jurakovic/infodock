//! The processors' load (SPEC Appendix A, `cpu`): `GetSystemTimes` for user, kernel and idle
//! time, and PDH's `% Processor Utility` for the busy share Task Manager shows, in total and
//! per logical processor. The PDH query is opened once and reused.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW,
};
use windows::Win32::System::Threading::{GetActiveProcessorCount, GetSystemTimes};
use windows::core::{PCWSTR, w};

use crate::platform::{CpuReader, CpuReading, CpuTimes};

/// All processor groups, for `GetActiveProcessorCount`.
const ALL_PROCESSOR_GROUPS: u16 = 0xFFFF;
/// `ERROR_SUCCESS` for PDH calls.
const OK: u32 = 0;
/// A counter value's `CStatus`: `PDH_CSTATUS_VALID_DATA` and `PDH_CSTATUS_NEW_DATA` both carry
/// data.
fn valid(status: u32) -> bool {
    status <= 1
}
/// `PDH_MORE_DATA`: the buffer is too small (the first call asks for its size).
const MORE_DATA: u32 = 0x8000_07D2;

pub struct WinCpu {
    query: PDH_HQUERY,
    total: Option<PDH_HCOUNTER>,
    cores: Option<PDH_HCOUNTER>,
}

// SAFETY: PDH handles are plain handles usable from any thread; the reader is owned and used
// by one thread (the sampler) at a time.
unsafe impl Send for WinCpu {}

impl WinCpu {
    pub fn new() -> Self {
        let mut query = PDH_HQUERY::default();
        // SAFETY: a live query handle is written to `query` on success.
        let opened = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } == OK;
        let add = |path: PCWSTR| {
            let mut counter = PDH_HCOUNTER::default();
            // SAFETY: `query` is open; the path is a static string.
            (opened && unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) } == OK)
                .then_some(counter)
        };
        let total = add(w!(r"\Processor Information(_Total)\% Processor Utility"));
        let cores = add(w!(r"\Processor Information(*)\% Processor Utility"));
        if total.is_none() {
            tracing::warn!("processor utility counters unavailable; using processor times");
        }
        Self {
            query: if opened { query } else { PDH_HQUERY::default() },
            total,
            cores,
        }
    }

    fn total(&self) -> Option<f64> {
        let counter = self.total?;
        let mut value = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: the counter belongs to the open query; `value` is written on success.
        let status =
            unsafe { PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut value) };
        if status == OK && valid(value.CStatus) {
            // SAFETY: PDH_FMT_DOUBLE fills the double member.
            Some(unsafe { value.Anonymous.doubleValue })
        } else {
            None
        }
    }

    /// Per logical processor, in (group, number) order; instances are named `group,number`.
    /// A processor whose value is not valid keeps its place as NaN, so the others keep theirs.
    fn cores(&self) -> Vec<f64> {
        let Some(counter) = self.cores else {
            return Vec::new();
        };
        let (mut size, mut count) = (0u32, 0u32);
        // SAFETY: asks for the buffer size only.
        let status = unsafe {
            PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None)
        };
        if status != MORE_DATA || size == 0 {
            return Vec::new();
        }
        let item = size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        let mut buffer: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> =
            vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); (size as usize).div_ceil(item)];
        // SAFETY: the buffer holds at least `size` bytes; PDH writes `count` items and their
        // names into it.
        let status = unsafe {
            PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(buffer.as_mut_ptr()),
            )
        };
        if status != OK {
            return Vec::new();
        }
        let mut cores: Vec<((u32, u32), f64)> = buffer
            .iter()
            .take(count as usize)
            .filter_map(|item| {
                // SAFETY: the name points into `buffer`, NUL-terminated.
                let name = unsafe { item.szName.to_string() }.ok()?;
                let (group, number) = name.split_once(',')?;
                let key = (group.parse().ok()?, number.parse().ok()?);
                let value = if valid(item.FmtValue.CStatus) {
                    // SAFETY: PDH_FMT_DOUBLE fills the double member.
                    unsafe { item.FmtValue.Anonymous.doubleValue }
                } else {
                    f64::NAN
                };
                Some((key, value))
            })
            .collect();
        cores.sort_by_key(|(key, _)| *key);
        cores.into_iter().map(|(_, value)| value).collect()
    }
}

impl Drop for WinCpu {
    fn drop(&mut self) {
        if !self.query.is_invalid() {
            // SAFETY: closes the query this reader opened.
            unsafe { PdhCloseQuery(self.query) };
        }
    }
}

/// Whether the per-processor utility counters can be read (SPEC §4.6 `per_core_cpu`); they
/// cannot when the performance counters are broken (`lodctr /r` repairs them). Checked once.
pub fn per_processor_counters() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let cpu = WinCpu::new();
        cpu.cores.is_some()
    })
}

/// The logical processors of all processor groups.
pub fn logical_processors() -> u32 {
    // SAFETY: plain query.
    unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) }.max(1)
}

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

impl CpuReader for WinCpu {
    fn read(&mut self) -> Option<CpuReading> {
        let (mut idle, mut kernel, mut user) = Default::default();
        // SAFETY: three writable FILETIMEs.
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
        if !self.query.is_invalid() {
            // SAFETY: collects the open query's counters.
            unsafe { PdhCollectQueryData(self.query) };
        }
        let logical = logical_processors();
        Some(CpuReading {
            times: CpuTimes {
                idle: ticks(idle),
                kernel: ticks(kernel),
                user: ticks(user),
            },
            total: self.total(),
            cores: self.cores(),
            logical,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`).
    #[test]
    #[ignore = "reads this machine's processors"]
    fn reads_the_utility_counters() {
        let mut cpu = WinCpu::new();
        assert!(cpu.total.is_some(), "the total counter");
        // PDH needs two collections for a rate.
        cpu.read().expect("readable");
        std::thread::sleep(std::time::Duration::from_secs(1));
        let reading = cpu.read().expect("readable");
        let total = reading.total.expect("a total");
        assert!((0.0..=1000.0).contains(&total), "{total}");
        assert_eq!(reading.cores.len(), reading.logical as usize, "{reading:?}");
        assert!(reading.cores.iter().all(|c| c.is_finite()), "{reading:?}");
    }
}
