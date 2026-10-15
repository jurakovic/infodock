//! Shell services (SPEC §4.6): starting programs, files and URIs for sensor clicks
//! (`ShellExecuteExW`, with parameters, a working folder and the `runas` verb), and the file
//! and folder pickers Settings uses (`IFileOpenDialog`).

use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOLDERID_RecycleBinFolder, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog,
    IFileOpenDialog, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, SHGetKnownFolderIDList,
    SIGDN_FILESYSPATH, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::platform::{Launch, PathRequest};

/// Starts `launch` as Explorer would, on a short-lived thread: the shell can take a moment (and
/// show its own message when the target is missing, or ask for elevation), and the bar thread
/// must not wait.
pub fn launch(launch: &Launch) {
    let target = launch.target.to_owned();
    let args = launch.args.to_owned();
    let dir = launch.dir.map(str::to_owned);
    let admin = launch.admin;
    let spawned = std::thread::Builder::new()
        .name("open".into())
        .spawn(move || {
            // Shell extensions may use COM.
            // SAFETY: per-thread initialisation, balanced below.
            let initialised =
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
                    .is_ok();
            let file = HSTRING::from(target.as_str());
            let params = HSTRING::from(args.as_str());
            let directory = dir.as_deref().map(HSTRING::from);
            let optional = |s: &HSTRING| {
                if s.is_empty() {
                    PCWSTR::null()
                } else {
                    PCWSTR(s.as_ptr())
                }
            };
            let mut info = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                // The thread ends after the call, so the shell must finish before it returns.
                fMask: SEE_MASK_NOASYNC,
                lpVerb: if admin { w!("runas") } else { PCWSTR::null() },
                lpFile: PCWSTR(file.as_ptr()),
                lpParameters: optional(&params),
                lpDirectory: directory.as_ref().map_or(PCWSTR::null(), optional),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            // SAFETY: `info` is initialised and the strings outlive the call.
            match unsafe { ShellExecuteExW(&mut info) } {
                Ok(()) => {}
                // The user declined the elevation prompt.
                Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => {
                    tracing::info!("starting {target:?} was cancelled");
                }
                Err(e) => tracing::warn!("could not open {target:?}: {e}"),
            }
            if initialised {
                // SAFETY: balances the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("could not start opening a target: {e}");
    }
}

/// Whether the shell has a Recycle Bin (the `recycle_bin` capability, SPEC §4.6); asked of the
/// shell's namespace, so no drive is touched.
pub fn has_recycle_bin() -> bool {
    // SAFETY: the returned ID list is ours and freed with CoTaskMemFree.
    unsafe {
        match SHGetKnownFolderIDList(&FOLDERID_RecycleBinFolder, 0, None) {
            Ok(list) => {
                CoTaskMemFree(Some(list.cast_const().cast()));
                true
            }
            Err(_) => false,
        }
    }
}

/// Shows the picker modal to `owner`; `None` if the user cancelled or it failed (logged).
pub fn pick_path(owner: Option<isize>, request: &PathRequest) -> Option<String> {
    // The dialog needs COM in a single-threaded apartment; balanced below when this call
    // initialised it.
    // SAFETY: plain per-call initialisation.
    let initialised =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let result = show(owner, request);
    if initialised {
        // SAFETY: balances the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
    match result {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!("the file picker failed: {e}");
            None
        }
    }
}

fn show(owner: Option<isize>, request: &PathRequest) -> Result<Option<String>> {
    // Kept alive until the dialog has closed: the filter specs point into them.
    let types_name = HSTRING::from(request.types_name);
    let types_spec = HSTRING::from(
        request
            .extensions
            .iter()
            .map(|e| format!("*.{e}"))
            .collect::<Vec<_>>()
            .join(";"),
    );
    let all_name = HSTRING::from(request.all_files);
    let all_spec = HSTRING::from("*.*");
    let title = HSTRING::from(request.title);
    // SAFETY: plain COM calls; every string outlives the dialog, and the returned path is freed
    // with CoTaskMemFree.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(&title)?;
        if request.folder {
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
        } else {
            let mut specs = Vec::new();
            if !request.extensions.is_empty() {
                specs.push(COMDLG_FILTERSPEC {
                    pszName: PCWSTR(types_name.as_ptr()),
                    pszSpec: PCWSTR(types_spec.as_ptr()),
                });
            }
            specs.push(COMDLG_FILTERSPEC {
                pszName: PCWSTR(all_name.as_ptr()),
                pszSpec: PCWSTR(all_spec.as_ptr()),
            });
            dialog.SetFileTypes(&specs)?;
            dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM)?;
        }
        let owner = owner.map(|h| HWND(h as *mut std::ffi::c_void));
        if dialog.Show(owner).is_err() {
            // Cancelled (or closed): nothing chosen.
            return Ok(None);
        }
        let item = dialog.GetResult()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const std::ffi::c_void));
        Ok(text)
    }
}
