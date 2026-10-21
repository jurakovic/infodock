//! Starting at sign-in (SPEC §10.1): a value of the per-user `Run` key, and Task Manager's mark
//! for it under `StartupApproved\Run` (a missing mark means enabled).

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::platform::AutostartChange;

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const APPROVED: PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");

/// The command of the `Run` value `name`, if it is a string.
pub fn read(name: &str) -> Option<String> {
    let name = HSTRING::from(name);
    let mut size = 0u32;
    // SAFETY: no buffer: asks for the size in bytes only.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            &name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: `buf` holds `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then(|| super::from_wide(&buf))
}

/// Writes or removes the `Run` value `name`.
pub fn change(name: &str, change: &AutostartChange) -> Result<(), String> {
    let name = HSTRING::from(name);
    match change {
        AutostartChange::Enable(command) | AutostartChange::Update(command) => {
            let data: Vec<u8> = command
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            // SAFETY: a NUL-terminated UTF-16 string of `data.len()` bytes; the key is created
            // if missing.
            let status = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    RUN,
                    &name,
                    REG_SZ.0,
                    Some(data.as_ptr().cast()),
                    data.len() as u32,
                )
            };
            check(status)?;
            if matches!(change, AutostartChange::Enable(_)) {
                remove(APPROVED, &name)?;
            }
            Ok(())
        }
        AutostartChange::Remove => {
            remove(RUN, &name)?;
            remove(APPROVED, &name)
        }
    }
}

/// Removes a value; one that is not there is removed already.
fn remove(key: PCWSTR, name: &HSTRING) -> Result<(), String> {
    // SAFETY: plain strings; nothing is returned.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key, name) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status)
}

fn check(status: WIN32_ERROR) -> Result<(), String> {
    status.ok().map_err(|e| e.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes, reads and removes a `Run` value of its own (`cargo test -- --ignored` on Windows).
    #[test]
    #[ignore = "writes to this user's Run key"]
    fn run_values_are_written_read_and_removed() {
        let name = "Infodock test entry";
        let command = r#""C:\Infodock test\infodock.exe""#;
        change(name, &AutostartChange::Enable(command)).expect("written");
        assert_eq!(read(name).as_deref(), Some(command));
        let moved = r#""D:\Infodock test\infodock.exe""#;
        change(name, &AutostartChange::Update(moved)).expect("written");
        assert_eq!(read(name).as_deref(), Some(moved));
        change(name, &AutostartChange::Remove).expect("removed");
        assert_eq!(read(name), None);
        // Removing what is not there is no error.
        change(name, &AutostartChange::Remove).expect("removed");
    }
}
