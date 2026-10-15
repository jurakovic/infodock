//! File operations for drops (SPEC §9.16, §9.19): copying and moving into a folder and moving
//! to the Recycle Bin with `IFileOperation` (the shell's progress, conflict and confirmation
//! dialogs, undo) and shortcuts (`.lnk`) with `IShellLinkW`.

use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile,
};
use windows::Win32::UI::Shell::{
    FOF_ALLOWUNDO, FOF_NO_UI, FOF_NOCONFIRMMKDIR, FOFX_RECYCLEONDELETE, FileOperation,
    IFileOperation, IFileOperationProgressSink, IShellItem, IShellLinkW,
    SHCreateItemFromParsingName, ShellLink,
};
use windows::core::{HSTRING, Interface, PCWSTR, Result};

use crate::platform::{FileOp, unique_path};

/// Runs `op` for `paths` into `folder` on a short-lived thread (the shell's dialogs may wait
/// for the user), then calls `done`. Failures and cancelled operations are logged.
pub fn file_operation(
    op: FileOp,
    paths: Vec<String>,
    folder: String,
    done: Box<dyn FnOnce() + Send>,
) {
    spawn("file-operation", move || {
        let result = match op {
            FileOp::Copy | FileOp::Move => transfer(op, &paths, &folder, false),
            FileOp::Link => link(&paths, &folder),
        };
        if let Err(e) = result {
            tracing::warn!(
                "{op:?} of {} items into {folder:?} failed: {e}",
                paths.len()
            );
        }
        done();
    });
}

/// Moves `paths` to the Recycle Bin on a short-lived thread, confirming as the Recycle Bin's
/// settings say, then calls `done`.
pub fn recycle(paths: Vec<String>, done: Box<dyn FnOnce() + Send>) {
    spawn("recycle", move || {
        if let Err(e) = delete(&paths) {
            tracing::warn!(
                "moving {} items to the Recycle Bin failed: {e}",
                paths.len()
            );
        }
        done();
    });
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            super::com::enter();
            work();
        });
    if let Err(e) = spawned {
        tracing::warn!("could not start a file operation: {e}");
    }
}

fn item(path: &str) -> Result<IShellItem> {
    // SAFETY: a NUL-terminated path; the shell parses it.
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None) }
}

fn operation() -> Result<IFileOperation> {
    // SAFETY: the shell's file operation object, in this thread's apartment.
    unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }
}

/// Copies or moves with one `IFileOperation`: one progress dialog for all items (none when
/// `quiet`, for tests).
fn transfer(op: FileOp, paths: &[String], folder: &str, quiet: bool) -> Result<()> {
    let operation = operation()?;
    let destination = item(folder)?;
    let flags = if quiet {
        FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOF_NO_UI
    } else {
        FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR
    };
    // SAFETY: COM calls on objects of this thread; the items live until PerformOperations.
    unsafe {
        operation.SetOperationFlags(flags)?;
        for path in paths {
            let source = item(path)?;
            match op {
                FileOp::Move => operation.MoveItem(
                    &source,
                    &destination,
                    PCWSTR::null(),
                    None::<&IFileOperationProgressSink>,
                )?,
                _ => operation.CopyItem(
                    &source,
                    &destination,
                    PCWSTR::null(),
                    None::<&IFileOperationProgressSink>,
                )?,
            }
        }
        operation.PerformOperations()
    }
}

/// Deletes to the Recycle Bin with undo (SPEC §9.16).
fn delete(paths: &[String]) -> Result<()> {
    let operation = operation()?;
    // SAFETY: as in `transfer`.
    unsafe {
        operation.SetOperationFlags(FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE)?;
        for path in paths {
            operation.DeleteItem(&item(path)?, None::<&IFileOperationProgressSink>)?;
        }
        operation.PerformOperations()
    }
}

/// Makes a shortcut (`name.ext.lnk`, numbered if taken) to each of `paths` in `folder`.
fn link(paths: &[String], folder: &str) -> Result<()> {
    for path in paths {
        let name = crate::sensors::shortcut::file_name(path).trim_end_matches(':');
        let file = unique_path(std::path::Path::new(folder), name, ".lnk");
        // SAFETY: the shell's link object in this thread's apartment; strings outlive the
        // calls.
        unsafe {
            let shortcut: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shortcut.SetPath(&HSTRING::from(path.as_str()))?;
            let persist: IPersistFile = shortcut.cast()?;
            persist.Save(&HSTRING::from(file.as_os_str()), true)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`): copies, moves and links
    /// files between temporary folders without the shell's dialogs.
    #[test]
    #[ignore = "uses the Windows shell"]
    fn copies_moves_and_links() {
        super::super::com::enter();
        let from = tempfile::tempdir().expect("a folder");
        let to = tempfile::tempdir().expect("a folder");
        let file = |name: &str| {
            let path = from.path().join(name);
            std::fs::write(&path, b"x").expect("a file");
            path.to_string_lossy().into_owned()
        };
        let folder = to.path().to_string_lossy().into_owned();
        let (a, b, c) = (file("a.txt"), file("b.txt"), file("c.txt"));
        transfer(FileOp::Copy, std::slice::from_ref(&a), &folder, true).expect("copied");
        assert!(to.path().join("a.txt").exists() && from.path().join("a.txt").exists());
        transfer(FileOp::Move, &[b], &folder, true).expect("moved");
        assert!(to.path().join("b.txt").exists() && !from.path().join("b.txt").exists());
        link(&[c.clone(), c], &folder).expect("linked");
        assert!(to.path().join("c.txt.lnk").exists());
        assert!(to.path().join("c.txt (2).lnk").exists());
    }
}
