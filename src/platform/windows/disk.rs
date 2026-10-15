//! Volumes and their free space (SPEC Appendix A, `disk/<X:\>` and `volumes`):
//! `GetLogicalDriveStringsW`, `GetDriveTypeW`, `GetVolumeInformationW` and
//! `GetDiskFreeSpaceExW`. A drive without a medium (an empty card reader or optical drive)
//! fails quietly: the calls run with critical-error dialogs turned off for the thread. Labels
//! are read only from local drives that answer at once: a network drive whose server is away
//! would block for the SMB timeout, and an optical drive may spin up. Folder listings (SPEC
//! §9.19) and path checks run the same way.

use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM, GetDiskFreeSpaceExW,
    GetDriveTypeW, GetFileAttributesW, GetLogicalDriveStringsW, GetVolumeInformationW,
    INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX, SetThreadErrorMode, THREAD_ERROR_MODE,
};
use windows::core::PCWSTR;

use super::from_wide;
use crate::platform::{DirItem, DiskSpace, DriveKind, PathKind, Volume};

/// `GetDriveTypeW` results.
const DRIVE_NO_ROOT_DIR: u32 = 1;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

/// Turns off the "insert a disk" dialogs for this thread while it lives.
pub(super) struct QuietErrors(THREAD_ERROR_MODE);

impl QuietErrors {
    pub(super) fn new() -> Self {
        let mut old = THREAD_ERROR_MODE::default();
        // SAFETY: `old` receives the previous mode, restored on drop.
        let _ = unsafe {
            SetThreadErrorMode(
                SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX,
                Some(&mut old),
            )
        };
        Self(old)
    }
}

impl Drop for QuietErrors {
    fn drop(&mut self) {
        // SAFETY: restores the mode `new` replaced.
        let _ = unsafe { SetThreadErrorMode(self.0, None) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub fn volumes() -> Vec<Volume> {
    let _quiet = QuietErrors::new();
    let mut buffer = [0u16; 512];
    // SAFETY: the buffer is writable; the result is the length used, or more if it is short.
    let length = unsafe { GetLogicalDriveStringsW(Some(&mut buffer)) } as usize;
    if length == 0 || length > buffer.len() {
        return Vec::new();
    }
    buffer[..length]
        .split(|&c| c == 0)
        .filter(|root| !root.is_empty())
        .map(|root| {
            let root = String::from_utf16_lossy(root);
            let path = wide(&root);
            // SAFETY: a NUL-terminated root path.
            let kind = match unsafe { GetDriveTypeW(PCWSTR(path.as_ptr())) } {
                DRIVE_FIXED => DriveKind::Fixed,
                DRIVE_REMOVABLE => DriveKind::Removable,
                DRIVE_CDROM => DriveKind::Optical,
                DRIVE_REMOTE => DriveKind::Network,
                DRIVE_RAMDISK => DriveKind::Ram,
                _ => DriveKind::Other,
            };
            let local = matches!(
                kind,
                DriveKind::Fixed | DriveKind::Removable | DriveKind::Ram
            );
            let label = if local { label(&path) } else { String::new() };
            let (ejectable, media) = match kind {
                DriveKind::Removable | DriveKind::Optical => {
                    (!is_system(&root), super::device::has_media(&root))
                }
                DriveKind::Fixed => (
                    !is_system(&root) && super::device::on_removable_bus(&root),
                    true,
                ),
                DriveKind::Network | DriveKind::Ram | DriveKind::Other => (false, true),
            };
            Volume {
                root,
                kind,
                label,
                ejectable,
                media,
            }
        })
        .collect()
}

/// The label of the volume at the NUL-terminated `path`; empty if it has none or no medium.
fn label(path: &[u16]) -> String {
    let mut name = [0u16; 261];
    // SAFETY: a NUL-terminated root path and a writable name buffer; the rest is not asked
    // for.
    unsafe {
        GetVolumeInformationW(
            PCWSTR(path.as_ptr()),
            Some(&mut name),
            None,
            None,
            None,
            None,
        )
    }
    .map_or_else(|_| String::new(), |()| from_wide(&name))
}

/// What `path` names, without a dialog for a drive that has no medium.
pub fn path_kind(path: &str) -> PathKind {
    let _quiet = QuietErrors::new();
    let wide = wide(path);
    // SAFETY: a NUL-terminated path.
    let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        PathKind::Missing
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
        PathKind::Folder
    } else {
        PathKind::File
    }
}

/// The entries of folder `path` with their attributes (no "insert a disk" dialogs): links to
/// folders (junctions) count as folders.
pub fn list_dir(path: &str) -> Result<Vec<DirItem>, String> {
    use std::os::windows::fs::MetadataExt;
    let _quiet = QuietErrors::new();
    let entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            // From the directory listing itself: no further call per entry.
            let attributes = entry.metadata().ok()?.file_attributes();
            Some(DirItem {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path().to_string_lossy().into_owned(),
                folder: attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
                hidden: attributes & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0,
            })
        })
        .collect())
}

pub fn space(root: &str) -> Option<DiskSpace> {
    let _quiet = QuietErrors::new();
    let path = wide(root);
    // SAFETY: a NUL-terminated root path.
    if unsafe { GetDriveTypeW(PCWSTR(path.as_ptr())) } == DRIVE_NO_ROOT_DIR {
        return None;
    }
    let (mut total, mut free) = (0u64, 0u64);
    // SAFETY: a NUL-terminated root path and two writable counters.
    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(path.as_ptr()),
            None,
            Some(&mut total),
            Some(&mut free),
        )
    }
    .ok()?;
    Some(DiskSpace { total, free })
}

/// Whether `root` is the drive Windows runs from (`SystemDrive`), which is never ejected.
fn is_system(root: &str) -> bool {
    std::env::var("SystemDrive").is_ok_and(|drive| {
        let drive = drive.trim_end_matches('\\');
        !drive.is_empty() && root.trim_end_matches('\\').eq_ignore_ascii_case(drive)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`).
    #[test]
    #[ignore = "reads this machine's drives"]
    fn the_system_drive_has_space() {
        let volumes = volumes();
        let system = volumes
            .iter()
            .find(|v| v.kind == DriveKind::Fixed)
            .expect("a fixed drive");
        assert!(system.root.ends_with(":\\"), "{volumes:?}");
        let free = space(&system.root).expect("readable");
        assert!(free.total > 0 && free.free <= free.total, "{free:?}");
        assert!(space("Q:\\").is_none() || volumes.iter().any(|v| v.root == "Q:\\"));
        assert_eq!(path_kind(&system.root), PathKind::Folder);
        assert!(system.media && !system.ejectable, "{system:?}");
    }

    #[test]
    fn path_kinds() {
        let dir = std::env::temp_dir();
        let file = dir.join("infodock-path-kind.txt");
        std::fs::write(&file, b"x").expect("written");
        assert_eq!(path_kind(&file.to_string_lossy()), PathKind::File);
        assert_eq!(path_kind(&dir.to_string_lossy()), PathKind::Folder);
        let _ = std::fs::remove_file(&file);
        assert_eq!(path_kind(&file.to_string_lossy()), PathKind::Missing);
    }
}
