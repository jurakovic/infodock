//! Toast notifications (SPEC §7.4) through WinRT's `ToastNotificationManager`, as the app's
//! AppUserModelID. The installer's Start Menu shortcut carries the id; for the portable build
//! the id is also registered under `HKCU\Software\Classes\AppUserModelId`, which is what lets an
//! app without a shortcut show toasts.

use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegSetValueExW,
};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::core::{HSTRING, PCWSTR, Result, w};

use crate::platform::notify::toast_xml;

/// The AppUserModelID toasts are shown as; set by [`register`].
static APP_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Makes this process `identifier` (taskbar grouping and toasts) and registers the id with
/// `display_name` for toasts.
pub fn register(identifier: &str, display_name: &str) {
    let _ = APP_ID.set(identifier.to_owned());
    // SAFETY: a valid string; it only names this process.
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(identifier)) } {
        tracing::warn!("could not set the AppUserModelID: {e}");
    }
    if let Err(e) = register_id(identifier, display_name) {
        tracing::warn!("could not register the AppUserModelID for notifications: {e}");
    }
}

fn register_id(identifier: &str, display_name: &str) -> Result<()> {
    let path = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{identifier}"));
    let mut key = HKEY::default();
    // SAFETY: creates or opens a key under HKCU; closed below.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &path,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()?;
    }
    let value: Vec<u8> = display_name
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    // SAFETY: an open key and a NUL-terminated UTF-16 string as bytes.
    let set = unsafe { RegSetValueExW(key, w!("DisplayName"), None, REG_SZ, Some(&value)) };
    // SAFETY: the key opened above.
    let _ = unsafe { RegCloseKey(key) };
    set.ok()
}

/// Shows a toast in the background: the first one loads WinRT, which the bar thread must not
/// wait for.
pub fn notify(title: &str, body: &str) {
    let xml = toast_xml(title, body);
    let spawned = std::thread::Builder::new()
        .name("notify".into())
        .spawn(move || {
            super::com::enter();
            if let Err(e) = show(&xml) {
                tracing::warn!("could not show a notification: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("could not start showing a notification: {e}");
    }
}

fn show(xml: &str) -> Result<()> {
    let id = APP_ID
        .get()
        .map_or_else(|| HSTRING::from(crate::IDENTIFIER), HSTRING::from);
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    ToastNotificationManager::CreateToastNotifierWithId(&id)?.Show(&toast)
}
