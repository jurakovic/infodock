//! Starting at sign-in (SPEC §10.1): a value of the per-user `Run` key, and Task Manager's mark
//! for it under `StartupApproved\Run` (a missing mark means enabled).

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::platform::AutostartChange;

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const APPROVED: PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");

/// The command of the `Run` value `name`, if it is a string.
pub fn read(name: &str) -> Option<String> {
    let name = HSTRING::from(name);
    super::read_string(HKEY_CURRENT_USER, RUN, PCWSTR(name.as_ptr()))
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
