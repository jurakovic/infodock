//! The drive watch (INTERNALS.md, Threads and windows). Windows asks the programs that keep a
//! drive open before it removes, locks or dismounts the drive, and one that does not let go
//! stops it. So the `drives` thread keeps a handle with no access on every local drive but the
//! system drive, registered for the drive's device events on a hidden window. On a request it
//! closes the drive in the gate ([`DRIVES`]), waits for the readings running there, closes its
//! handle and says yes; when the request fails or ends, it watches the drive again. It answers
//! on a thread of its own, whatever the bar thread is doing.

use std::cell::RefCell;
use std::mem::{offset_of, size_of};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DBT_CUSTOMEVENT, DBT_DEVICEQUERYREMOVE, DBT_DEVICEQUERYREMOVEFAILED,
    DBT_DEVICEREMOVECOMPLETE, DBT_DEVICEREMOVEPENDING, DBT_DEVTYP_HANDLE, DEV_BROADCAST_HANDLE,
    DEV_BROADCAST_HDR, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DispatchMessageW,
    GUID_IO_VOLUME_DISMOUNT, GUID_IO_VOLUME_DISMOUNT_FAILED, GUID_IO_VOLUME_LOCK,
    GUID_IO_VOLUME_LOCK_FAILED, GUID_IO_VOLUME_MOUNT, GUID_IO_VOLUME_UNLOCK, GetMessageW,
    HDEVNOTIFY, MSG, RegisterClassExW, RegisterDeviceNotificationW, UnregisterDeviceNotification,
    WM_CLOSE, WM_DEVICECHANGE, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{GUID, PCWSTR, w};

use super::device::Device;
use super::disk::{self, QuietErrors};
use crate::platform::DriveKind;
use crate::platform::drives::{DriveGate, Entry, drive_letter};
use crate::platform::guard::guard;

/// Not `Infodock.Bar`, which the single-instance check looks for.
const CLASS_NAME: PCWSTR = w!("Infodock.Drives");
/// How long a request waits for the readings running on its drive.
const READINGS_WAIT: Duration = Duration::from_secs(2);

/// Which drives may be read now. Every reading of a drive in `platform/windows` enters it
/// through [`enter`].
static DRIVES: DriveGate = DriveGate::new();

/// The drives thread's window, once it is created; `None` if it could not be.
static WINDOW: OnceLock<Option<isize>> = OnceLock::new();

/// A reading of `path`'s drive (see [`DriveGate::enter`]); `None` while Windows takes the drive.
/// The first call starts the drives thread.
pub(super) fn enter(path: &str) -> Option<Entry<'static>> {
    WINDOW.get_or_init(start);
    DRIVES.enter(path)
}

/// A watched drive.
struct Watched {
    letter: char,
    /// The handle with no access; `None` while Windows takes the drive.
    device: Option<Device>,
    /// Its registration for the handle's device events, undone when it is dropped.
    notify: HDEVNOTIFY,
}

impl Drop for Watched {
    fn drop(&mut self) {
        // SAFETY: the registration made in `register`, undone once.
        let _ = unsafe { UnregisterDeviceNotification(self.notify) };
    }
}

thread_local! {
    /// The watched drives, on the drives thread.
    static WATCHED: RefCell<Vec<Watched>> = const { RefCell::new(Vec::new()) };
}

/// Starts the drives thread and returns its window once it is created.
fn start() -> Option<isize> {
    let (ready, created) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("drives".into())
        .spawn(move || {
            // A drive without a medium brings up no dialog.
            let _quiet = QuietErrors::new();
            let hwnd = match create_window() {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    tracing::warn!("drives are not watched: no window: {e}");
                    let _ = ready.send(None);
                    return;
                }
            };
            let _ = ready.send(Some(hwnd.0 as isize));
            watch(hwnd);
            let mut msg = MSG::default();
            // SAFETY: the standard message loop for this thread's window.
            unsafe {
                // 0 = WM_QUIT, -1 = error.
                while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                    DispatchMessageW(&msg);
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("drives are not watched: the thread did not start: {e}");
        return None;
    }
    created.recv().ok().flatten()
}

/// The hidden window: top-level, so volume broadcasts reach it, and never shown.
fn create_window() -> windows::core::Result<HWND> {
    // SAFETY: plain module and class registration calls.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }?.into();
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised; a second registration fails harmlessly.
    unsafe { RegisterClassExW(&class) };
    // SAFETY: creates our own window, with no state attached.
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS_NAME,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    guard(
        "the drives window's procedure",
        // SAFETY: default processing for the message whose handling panicked.
        || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        || match msg {
            WM_DEVICECHANGE => {
                device_change(hwnd, wparam, lparam);
                // TRUE: a removal Windows asks about may go on.
                LRESULT(1)
            }
            // The window lives as long as the process: the WM_CLOSE `taskkill` sends to every
            // top-level window does not end it (the bar's ends the app).
            WM_CLOSE => LRESULT(0),
            // SAFETY: default processing for our own window.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        },
    )
}

/// What Windows says of a watched drive.
#[derive(Debug, PartialEq, Eq)]
enum Event {
    /// It is about to remove, lock or dismount the drive.
    LetGo(&'static str),
    /// That failed or is over, and the drive is still there.
    WatchAgain(&'static str),
    /// The drive is going or gone.
    Gone,
}

/// The event of a `WM_DEVICECHANGE` with `code` about a watched handle; `guid` is the custom
/// event's.
fn event(code: u32, guid: GUID) -> Option<Event> {
    Some(match code {
        DBT_DEVICEQUERYREMOVE => Event::LetGo("its removal"),
        DBT_DEVICEQUERYREMOVEFAILED => Event::WatchAgain("its removal failed"),
        DBT_DEVICEREMOVEPENDING | DBT_DEVICEREMOVECOMPLETE => Event::Gone,
        DBT_CUSTOMEVENT if guid == GUID_IO_VOLUME_LOCK => Event::LetGo("a lock"),
        DBT_CUSTOMEVENT if guid == GUID_IO_VOLUME_DISMOUNT => Event::LetGo("a dismount"),
        DBT_CUSTOMEVENT if guid == GUID_IO_VOLUME_LOCK_FAILED => {
            Event::WatchAgain("the lock failed")
        }
        DBT_CUSTOMEVENT if guid == GUID_IO_VOLUME_UNLOCK => Event::WatchAgain("it was unlocked"),
        DBT_CUSTOMEVENT if guid == GUID_IO_VOLUME_DISMOUNT_FAILED => {
            Event::WatchAgain("the dismount failed")
        }
        DBT_CUSTOMEVENT if guid == GUID_IO_VOLUME_MOUNT => Event::WatchAgain("it was mounted"),
        _ => return None,
    })
}

fn device_change(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    if super::window::is_volume_change(wparam, lparam) {
        watch(hwnd);
        return;
    }
    let code = wparam.0 as u32;
    let Some((notify, guid)) = handle_event(code, lparam) else {
        return;
    };
    match event(code, guid) {
        Some(Event::LetGo(why)) => let_go(notify, why),
        Some(Event::WatchAgain(why)) => watch_again(hwnd, notify, why),
        Some(Event::Gone) => gone(notify),
        None => {}
    }
}

/// The registration a `WM_DEVICECHANGE` with `code` is about, and its custom event's GUID (zero
/// for other events); `None` when it is not about a handle.
fn handle_event(code: u32, lparam: LPARAM) -> Option<(HDEVNOTIFY, GUID)> {
    if lparam.0 == 0 {
        return None;
    }
    let header = lparam.0 as *const DEV_BROADCAST_HDR;
    // SAFETY: a non-null lParam of WM_DEVICECHANGE points to a DEV_BROADCAST_HDR.
    let (size, kind) = unsafe { ((*header).dbch_size as usize, (*header).dbch_devicetype) };
    if kind != DBT_DEVTYP_HANDLE || size < offset_of!(DEV_BROADCAST_HANDLE, dbch_eventguid) {
        return None;
    }
    let event = lparam.0 as *const DEV_BROADCAST_HANDLE;
    // SAFETY: a DEV_BROADCAST_HANDLE at least as long as the fields read.
    let notify = unsafe { (&raw const (*event).dbch_hdevnotify).read_unaligned() };
    let custom =
        code == DBT_CUSTOMEVENT && size >= offset_of!(DEV_BROADCAST_HANDLE, dbch_nameoffset);
    // SAFETY: as above; the GUID is there for a custom event.
    let guid = if custom {
        unsafe { (&raw const (*event).dbch_eventguid).read_unaligned() }
    } else {
        GUID::zeroed()
    };
    Some((notify, guid))
}

/// The local drives to watch: fixed, removable and optical, but not the system drive.
fn listed() -> Vec<char> {
    disk::roots()
        .iter()
        .filter(|root| {
            matches!(
                disk::kind(root),
                DriveKind::Fixed | DriveKind::Removable | DriveKind::Optical
            ) && !disk::is_system(root)
        })
        .filter_map(|root| drive_letter(root))
        .collect()
}

/// Watches the drives listed now and forgets those no longer listed; at start and on each
/// volume broadcast.
fn watch(hwnd: HWND) {
    let listed = listed();
    let gone: Vec<Watched> = WATCHED.with_borrow_mut(|watched| {
        let (kept, gone) = std::mem::take(watched)
            .into_iter()
            .partition(|drive| listed.contains(&drive.letter));
        *watched = kept;
        gone
    });
    // Dropped outside the borrow: undoing a registration may wait for Windows.
    drop(gone);
    let new: Vec<char> = WATCHED.with_borrow(|watched| {
        listed
            .iter()
            .copied()
            .filter(|&letter| !watched.iter().any(|drive| drive.letter == letter))
            .collect()
    });
    for letter in new {
        if let Some(drive) = register(hwnd, letter) {
            WATCHED.with_borrow_mut(|watched| watched.push(drive));
        }
    }
    // Every drive Windows is not taking is open, so one whose drive went is read again once
    // another drive takes its letter.
    let taken: Vec<char> = WATCHED.with_borrow(|watched| {
        watched
            .iter()
            .filter(|drive| drive.device.is_none())
            .map(|drive| drive.letter)
            .collect()
    });
    for letter in 'A'..='Z' {
        if !taken.contains(&letter) {
            DRIVES.open(letter);
        }
    }
    let watched: Vec<String> = WATCHED.with_borrow(|watched| {
        watched
            .iter()
            .map(|drive| format!("{}:", drive.letter))
            .collect()
    });
    tracing::debug!("watching drives {}", watched.join(" "));
}

/// Opens drive `letter` with no access and registers the handle for its device events.
fn register(hwnd: HWND, letter: char) -> Option<Watched> {
    let Some(device) = Device::open_volume(&format!("{letter}:\\"), 0) else {
        tracing::debug!("{letter}: is not watched: it did not open");
        return None;
    };
    let filter = DEV_BROADCAST_HANDLE {
        dbch_size: size_of::<DEV_BROADCAST_HANDLE>() as u32,
        dbch_devicetype: DBT_DEVTYP_HANDLE.0,
        dbch_handle: device.0,
        ..Default::default()
    };
    // SAFETY: a DEV_BROADCAST_HANDLE for an open handle, and our own window; the registration
    // is undone when the `Watched` drops.
    let registered = unsafe {
        RegisterDeviceNotificationW(
            HANDLE(hwnd.0),
            (&raw const filter).cast(),
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    match registered {
        Ok(notify) => Some(Watched {
            letter,
            device: Some(device),
            notify,
        }),
        Err(e) => {
            tracing::debug!("{letter}: is not watched: {e}");
            None
        }
    }
}

/// Lets go of the drive of `notify` for `why`: closes it in the gate, waits for its readings and
/// closes the handle, before the answer lets Windows go on. The registration stays, to hear how
/// it ends, as Windows asks.
fn let_go(notify: HDEVNOTIFY, why: &str) {
    let found = WATCHED.with_borrow_mut(|watched| {
        let drive = watched.iter_mut().find(|drive| drive.notify == notify)?;
        Some((drive.letter, drive.device.take()))
    });
    let Some((letter, device)) = found else {
        return;
    };
    let finished = DRIVES.close(letter, READINGS_WAIT);
    drop(device);
    if finished {
        tracing::info!("let go of {letter}: for {why}");
    } else {
        tracing::warn!("let go of {letter}: for {why}, though a reading of it still runs");
    }
}

/// Watches the drive of `notify` again, once what it was let go for failed or is over: a new
/// handle and registration, and the gate open.
fn watch_again(hwnd: HWND, notify: HDEVNOTIFY, why: &str) {
    let found = WATCHED.with_borrow_mut(|watched| {
        let index = watched.iter().position(|drive| drive.notify == notify)?;
        // Still watched (a mount, or a lock this window did not hear of): nothing to do.
        watched[index]
            .device
            .is_none()
            .then(|| watched.remove(index))
    });
    let Some(drive) = found else {
        return;
    };
    let letter = drive.letter;
    drop(drive);
    DRIVES.open(letter);
    match register(hwnd, letter) {
        Some(drive) => {
            WATCHED.with_borrow_mut(|watched| watched.push(drive));
            tracing::info!("watching {letter}: again: {why}");
        }
        None => tracing::info!("{letter}: is read again, unwatched: {why}"),
    }
}

/// Forgets the drive of `notify`, which is going. A gate closed for it opens with the volume
/// broadcast that follows (`watch`).
fn gone(notify: HDEVNOTIFY) {
    let found = WATCHED.with_borrow_mut(|watched| {
        let index = watched.iter().position(|drive| drive.notify == notify)?;
        Some(watched.remove(index))
    });
    if let Some(drive) = found {
        tracing::debug!("{}: is going", drive.letter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_each_event_asks() {
        let none = GUID::zeroed();
        assert_eq!(
            event(DBT_DEVICEQUERYREMOVE, none),
            Some(Event::LetGo("its removal"))
        );
        assert!(matches!(
            event(DBT_DEVICEQUERYREMOVEFAILED, none),
            Some(Event::WatchAgain(_))
        ));
        assert_eq!(event(DBT_DEVICEREMOVEPENDING, none), Some(Event::Gone));
        assert_eq!(event(DBT_DEVICEREMOVECOMPLETE, none), Some(Event::Gone));
        for guid in [GUID_IO_VOLUME_LOCK, GUID_IO_VOLUME_DISMOUNT] {
            assert!(matches!(
                event(DBT_CUSTOMEVENT, guid),
                Some(Event::LetGo(_))
            ));
        }
        for guid in [
            GUID_IO_VOLUME_LOCK_FAILED,
            GUID_IO_VOLUME_UNLOCK,
            GUID_IO_VOLUME_DISMOUNT_FAILED,
            GUID_IO_VOLUME_MOUNT,
        ] {
            assert!(matches!(
                event(DBT_CUSTOMEVENT, guid),
                Some(Event::WatchAgain(_))
            ));
        }
        assert_eq!(event(DBT_CUSTOMEVENT, none), None, "another custom event");
        assert_eq!(event(0x0007, none), None, "DBT_DEVNODES_CHANGED");
    }
}
