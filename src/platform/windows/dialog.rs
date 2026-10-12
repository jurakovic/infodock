//! Native dialogs (SPEC §6.4): confirmations with `TaskDialogIndirect` (common controls 6,
//! enabled by the application manifest).

use std::mem::size_of;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDCBF_CANCEL_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
    TaskDialogIndirect,
};
use windows::core::{HSTRING, PCWSTR};

/// Asks `message` with a button labelled `action` and Cancel; returns whether `action` was
/// chosen. Runs a modal loop.
pub(super) fn confirm(owner: HWND, title: &str, message: &str, action: &str) -> bool {
    const CONFIRM: i32 = 100;
    let (title, message, action) = (
        HSTRING::from(title),
        HSTRING::from(message),
        HSTRING::from(action),
    );
    let buttons = [TASKDIALOG_BUTTON {
        nButtonID: CONFIRM,
        pszButtonText: PCWSTR(action.as_ptr()),
    }];
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        // Esc and the close button cancel.
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(message.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: CONFIRM,
        ..Default::default()
    };
    let mut pressed = 0;
    // SAFETY: every pointer in `config` refers to locals that outlive the call.
    let shown = unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) };
    if let Err(e) = &shown {
        tracing::warn!("the confirmation dialog could not be shown: {e}");
    }
    shown.is_ok() && pressed == CONFIRM
}
