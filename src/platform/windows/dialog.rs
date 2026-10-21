//! Native dialogs (SPEC §6.4): confirmations, questions and messages with `TaskDialogIndirect`
//! (common controls 6, enabled by the application manifest).

use std::mem::size_of;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOG_FLAGS, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TD_ERROR_ICON,
    TD_INFORMATION_ICON, TD_WARNING_ICON, TDCBF_CANCEL_BUTTON, TDCBF_OK_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_USE_COMMAND_LINKS, TaskDialogIndirect,
};
use windows::core::{HSTRING, PCWSTR};

use crate::platform::{Choice, Message, MessageKind};

/// The id of the first of our own buttons.
const FIRST: i32 = 100;

/// Asks `message` with a button labelled `action` and Cancel; returns whether `action` was
/// chosen. Runs a modal loop.
pub(super) fn confirm(owner: HWND, title: &str, message: &str, action: &str) -> bool {
    let choice = Choice {
        question: message,
        detail: "",
        choices: &[action],
    };
    show(Some(owner), title, &choice, TASKDIALOG_FLAGS(0)) == Some(0)
}

/// Asks `choice` with a command link per choice and Cancel; the chosen index. Runs a modal
/// loop.
pub fn choose(owner: Option<HWND>, title: &str, choice: &Choice) -> Option<usize> {
    show(owner, title, choice, TDF_USE_COMMAND_LINKS)
}

fn show(
    owner: Option<HWND>,
    title: &str,
    choice: &Choice,
    flags: TASKDIALOG_FLAGS,
) -> Option<usize> {
    let (title, question, detail) = (
        HSTRING::from(title),
        HSTRING::from(choice.question),
        HSTRING::from(choice.detail),
    );
    let labels: Vec<HSTRING> = choice.choices.iter().map(|c| HSTRING::from(*c)).collect();
    let buttons: Vec<TASKDIALOG_BUTTON> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| TASKDIALOG_BUTTON {
            nButtonID: FIRST + i as i32,
            pszButtonText: PCWSTR(label.as_ptr()),
        })
        .collect();
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner.unwrap_or_default(),
        // Esc and the close button cancel.
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | flags,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(question.as_ptr()),
        pszContent: content(&detail),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: FIRST,
        ..Default::default()
    };
    let mut pressed = 0;
    // SAFETY: every pointer in `config` refers to locals that outlive the call.
    let shown = unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) };
    if let Err(e) = &shown {
        tracing::warn!("the dialog could not be shown: {e}");
    }
    let index = usize::try_from(pressed - FIRST).ok()?;
    (shown.is_ok() && index < buttons.len()).then_some(index)
}

/// Shows `message` with OK. Runs a modal loop.
pub fn message(owner: Option<HWND>, title: &str, message: &Message) {
    let (title, text, detail) = (
        HSTRING::from(title),
        HSTRING::from(message.text),
        HSTRING::from(message.detail),
    );
    let icon = match message.kind {
        MessageKind::Info => TD_INFORMATION_ICON,
        MessageKind::Warning => TD_WARNING_ICON,
        MessageKind::Error => TD_ERROR_ICON,
    };
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner.unwrap_or_default(),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: TDCBF_OK_BUTTON,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: icon },
        pszMainInstruction: PCWSTR(text.as_ptr()),
        pszContent: content(&detail),
        ..Default::default()
    };
    // SAFETY: every pointer in `config` refers to locals that outlive the call.
    if let Err(e) = unsafe { TaskDialogIndirect(&config, None, None, None) } {
        tracing::warn!("the message could not be shown: {e}");
    }
}

/// The dialog's content text, or none when `detail` is empty.
fn content(detail: &HSTRING) -> PCWSTR {
    if detail.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(detail.as_ptr())
    }
}
