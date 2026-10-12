//! Shell services (SPEC §4.6): the file and folder pickers Settings uses (`IFileOpenDialog`).

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, PCWSTR, Result};

use crate::platform::PathRequest;

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
