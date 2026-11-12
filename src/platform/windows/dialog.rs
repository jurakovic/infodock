//! Native dialogs (SPEC §6.4): confirmations, questions and messages with `TaskDialogIndirect`
//! (common controls 6, enabled by the application manifest), and the colour dialog
//! (`ChooseColorW`, §10.7).

use std::mem::size_of;
use std::sync::{Mutex, PoisonError};

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOG_FLAGS, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TD_ERROR_ICON,
    TD_INFORMATION_ICON, TD_WARNING_ICON, TDCBF_CANCEL_BUTTON, TDCBF_OK_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_USE_COMMAND_LINKS, TaskDialogIndirect,
};
use windows::core::{HSTRING, PCWSTR};

use crate::platform::{Choice, Color, Message, MessageKind};

/// The id of the first of our own buttons.
const FIRST: i32 = 100;

/// The colour dialog's 16 custom colours, kept while the app runs; white until mixed.
static CUSTOM_COLORS: Mutex<[COLORREF; 16]> = Mutex::new([COLORREF(0x00FF_FFFF); 16]);

/// Windows' colour dialog, opened on `initial` with its colour mixer shown: the colour chosen
/// with OK, `None` for Cancel. Runs a modal loop.
pub fn pick_color(owner: Option<HWND>, initial: Color) -> Option<Color> {
    // A copy, so no lock is held across the modal loop.
    let mut custom = *CUSTOM_COLORS.lock().unwrap_or_else(PoisonError::into_inner);
    let channel = |v: f32| u32::from((v.clamp(0.0, 1.0) * 255.0).round() as u8);
    let mut dialog = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner.unwrap_or_default(),
        // COLORREF is 0x00BBGGRR.
        rgbResult: COLORREF(
            channel(initial.r) | channel(initial.g) << 8 | channel(initial.b) << 16,
        ),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    // SAFETY: the struct and the custom colours it points to outlive the call.
    let chosen = unsafe { ChooseColorW(&mut dialog) }.as_bool();
    *CUSTOM_COLORS.lock().unwrap_or_else(PoisonError::into_inner) = custom;
    chosen.then(|| {
        let [r, g, b, _] = dialog.rgbResult.0.to_le_bytes();
        Color::rgba(r, g, b, 1.0)
    })
}

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
