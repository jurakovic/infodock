//! The drive watch (INTERNALS.md, Threads and windows). Windows asks the programs that keep a
//! drive open before it removes, locks or dismounts the drive, and one that does not let go
//! stops it. So the `drives` thread keeps a handle with no access on every local drive but the
//! system drive, registered for the drive's device events on a hidden window. On a request it
//! closes the drive in the gate ([`DRIVES`]), waits for the readings running there, closes its
//! handle and says yes; when the request fails or ends, it watches the drive again. It answers
//! on a thread of its own, whatever the bar thread is doing. Jobs handed to it with [`run`] (the
//! Recycle Bin's readings) run between its answers, unless a call in a job handles messages: an
//! answer that comes then waits for the other threads' readings of the drive, not the job's own.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::{offset_of, size_of};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DBT_CUSTOMEVENT, DBT_DEVICEQUERYREMOVE, DBT_DEVICEQUERYREMOVEFAILED,
    DBT_DEVICEREMOVECOMPLETE, DBT_DEVICEREMOVEPENDING, DBT_DEVTYP_HANDLE, DEV_BROADCAST_HANDLE,
    DEV_BROADCAST_HDR, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DispatchMessageW,
    GUID_IO_VOLUME_DISMOUNT, GUID_IO_VOLUME_DISMOUNT_FAILED, GUID_IO_VOLUME_LOCK,
    GUID_IO_VOLUME_LOCK_FAILED, GUID_IO_VOLUME_MOUNT, GUID_IO_VOLUME_UNLOCK, GetMessageW,
    HDEVNOTIFY, MSG, PostMessageW, RegisterClassExW, RegisterDeviceNotificationW,
    UnregisterDeviceNotification, WM_APP, WM_CLOSE, WM_DEVICECHANGE, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::{GUID, PCWSTR, w};

use super::device::Device;
use super::disk::{self, QuietErrors};
use crate::platform::DriveKind;
use crate::platform::drives::{DriveGate, Entry, drive_letter};
use crate::platform::guard::guard;

/// Not `Infodock.Bar`, which the single-instance check looks for.
const CLASS_NAME: PCWSTR = w!("Infodock.Drives");
/// Private: jobs are waiting ([`run`]).
const WM_JOBS: u32 = WM_APP + 1;
/// How long a request waits for the readings running on its drive.
const READINGS_WAIT: Duration = Duration::from_secs(2);

/// Which drives may be read now. The platform's readings of a drive (`disk.rs`, `icons.rs`, the
/// Recycle Bin in `shell.rs`) enter it through [`enter`]; the actions and the pickers do not.
static DRIVES: DriveGate = DriveGate::new();

/// Held by the tests that lock a drive, and by those that count on every drive being open
/// meanwhile, so they run one at a time.
#[cfg(test)]
pub(super) static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

type Job = Box<dyn FnOnce() + Send>;

/// The drives thread: its window, and the jobs it is handed.
struct Thread {
    hwnd: isize,
    jobs: Sender<Job>,
}

/// Started on first use; `None` if it could not start.
static THREAD: OnceLock<Option<Thread>> = OnceLock::new();

fn thread() -> Option<&'static Thread> {
    THREAD.get_or_init(start).as_ref()
}

/// A reading of `path`'s drive (see [`DriveGate::enter`]); `None` while Windows takes the drive.
/// The first call starts the drives thread.
pub(super) fn enter(path: &str) -> Option<Reading> {
    thread();
    let entry = DRIVES.enter(path)?;
    let letter = drive_letter(path);
    if let Some(letter) = letter {
        HELD.with_borrow_mut(|held| held.push(letter));
    }
    Some(Reading {
        _entry: entry,
        letter,
        _here: PhantomData,
    })
}

/// A reading of a drive in the gate, also counted for its thread (`HELD`) while it lives, so it
/// stays on the thread that entered.
pub(super) struct Reading {
    _entry: Entry<'static>,
    letter: Option<char>,
    _here: PhantomData<*const ()>,
}

impl Drop for Reading {
    fn drop(&mut self) {
        if let Some(letter) = self.letter {
            HELD.with_borrow_mut(|held| {
                if let Some(index) = held.iter().position(|&l| l == letter) {
                    held.swap_remove(index);
                }
            });
        }
    }
}

/// Runs `job` on the drives thread and waits for its answer; `None` if the thread could not
/// start, or the job gave no answer (it panicked). Not for the drives thread itself, which
/// would wait for itself.
pub(super) fn run<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let thread = thread()?;
    let (answer, answered) = mpsc::channel();
    let job: Job = Box::new(move || {
        let _ = answer.send(job());
    });
    thread.jobs.send(job).ok()?;
    // SAFETY: posting to the drives window, which lives as long as the process.
    unsafe {
        PostMessageW(
            Some(HWND(thread.hwnd as *mut c_void)),
            WM_JOBS,
            WPARAM(0),
            LPARAM(0),
        )
    }
    .ok()?;
    answered.recv().ok()
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
    /// The jobs handed to the drives thread, there.
    static JOBS: RefCell<Option<Receiver<Job>>> = const { RefCell::new(None) };
    /// A job runs, so the next one waits for it.
    static IN_JOB: Cell<bool> = const { Cell::new(false) };
    /// The drive letters of this thread's readings ([`Reading`]). On the drives thread they are
    /// a job's, which a request that comes inside the job (a call in it handled messages)
    /// cannot wait for.
    static HELD: RefCell<Vec<char>> = const { RefCell::new(Vec::new()) };
}

/// Starts the drives thread and returns it once its window is created.
fn start() -> Option<Thread> {
    let (jobs, queue) = mpsc::channel::<Job>();
    let (ready, created) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("drives".into())
        .spawn(move || {
            // Its jobs call into the shell.
            super::com::enter();
            // A drive without a medium brings up no dialog.
            let _quiet = QuietErrors::new();
            JOBS.with_borrow_mut(|jobs| *jobs = Some(queue));
            let hwnd = match create_window() {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    tracing::warn!("drives are not watched: no window: {e}");
                    let _ = ready.send(None);
                    return;
                }
            };
            let _ = ready.send(Some(hwnd.0 as isize));
            super::close::watch_for_bar();
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
    let hwnd = created.recv().ok().flatten()?;
    Some(Thread { hwnd, jobs })
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
            WM_JOBS => {
                run_jobs();
                LRESULT(0)
            }
            WM_DEVICECHANGE => {
                device_change(hwnd, wparam, lparam);
                // TRUE: a removal Windows asks about may go on.
                LRESULT(1)
            }
            // The window lives as long as the process. A posted WM_CLOSE (`taskkill`) went to
            // the bar window instead (`close.rs`), and one sent here is ignored.
            WM_CLOSE => LRESULT(0),
            // SAFETY: default processing for our own window.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        },
    )
}

/// Runs the jobs handed over, one after the other. A job that handles messages is not
/// interrupted by the next: this loop takes it when the job is done.
fn run_jobs() {
    if IN_JOB.replace(true) {
        return;
    }
    while let Some(job) = JOBS.with_borrow(|jobs| jobs.as_ref()?.try_recv().ok()) {
        guard("a job on the drives thread", || (), job);
    }
    IN_JOB.set(false);
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

/// Lets go of the drive of `notify` for `why`: closes it in the gate, waits for the other
/// threads' readings of it and closes the handle, before the answer lets Windows go on. A
/// reading of it by the job this call came inside still runs then. The registration stays, to
/// hear how it ends, as Windows asks.
fn let_go(notify: HDEVNOTIFY, why: &str) {
    let found = WATCHED.with_borrow_mut(|watched| {
        let drive = watched.iter_mut().find(|drive| drive.notify == notify)?;
        Some((drive.letter, drive.device.take()))
    });
    let Some((letter, device)) = found else {
        return;
    };
    // A job's readings of the drive, below this call on the same thread, cannot end while it
    // waits; the other threads' can.
    let own = HELD.with_borrow(|held| held.iter().filter(|&&l| l == letter).count());
    let finished = DRIVES.close(letter, own, READINGS_WAIT) && own == 0;
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
    use std::time::Instant;

    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::System::Ioctl::{FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME};

    use super::*;

    /// The watched drives, read on the drives thread: each letter, whether its handle is open,
    /// and its registration.
    fn watched() -> Vec<(char, bool, usize)> {
        run(|| {
            WATCHED.with_borrow(|watched| {
                watched
                    .iter()
                    .map(|d| (d.letter, d.device.is_some(), d.notify.0 as usize))
                    .collect()
            })
        })
        .expect("the drives thread answers")
    }

    fn system_letter() -> char {
        let drive = std::env::var("SystemDrive").expect("SystemDrive");
        drive_letter(&drive).expect("a drive letter")
    }

    /// The fixed drives but the system drive.
    fn other_fixed() -> Vec<char> {
        disk::roots()
            .iter()
            .filter(|root| disk::kind(root) == DriveKind::Fixed && !disk::is_system(root))
            .filter_map(|root| drive_letter(root))
            .collect()
    }

    /// Runs on the Windows CI runner (`cargo test -- --ignored`), which has `D:` beside the
    /// system drive.
    #[test]
    #[ignore = "watches this machine's drives"]
    fn every_fixed_drive_but_the_system_drive_is_watched() {
        let _alone = ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let others = other_fixed();
        assert!(!others.is_empty(), "a fixed drive besides the system drive");
        let watched = watched();
        for letter in &others {
            assert!(
                watched.iter().any(|d| d.0 == *letter && d.1),
                "{letter}: in {watched:?}"
            );
        }
        let system = system_letter();
        assert!(watched.iter().all(|d| d.0 != system), "{watched:?}");
    }

    /// Runs on the Windows CI runner as an administrator, which may open a volume to lock it.
    /// The runner's checkout is on `D:`, so the lock fails: the watch lets go of the drive
    /// before the lock is tried, waiting for a reading that ends 300 ms later, and the failure
    /// watches the drive again, as an unlock would.
    #[test]
    #[ignore = "locks a drive of this machine"]
    fn a_lock_lets_go_of_the_drive_and_its_end_watches_it_again() {
        let _alone = ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let letter = *other_fixed()
            .first()
            .expect("a fixed drive besides the system drive");
        let root = format!("{letter}:\\");
        let before = watched();
        assert!(
            before.iter().any(|d| d.0 == letter && d.1),
            "{letter}: is watched: {before:?}"
        );
        let volume =
            Device::open_volume(&root, GENERIC_READ.0 | GENERIC_WRITE.0).expect("the volume opens");
        let reading = DRIVES.enter(&root).expect("open");
        let (locked, took) = std::thread::scope(|s| {
            s.spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                drop(reading);
            });
            let started = Instant::now();
            let locked = volume.control(FSCTL_LOCK_VOLUME, None, None);
            (locked, started.elapsed())
        });
        if locked {
            volume.control(FSCTL_UNLOCK_VOLUME, None, None);
        }
        drop(volume);
        assert!(
            took >= Duration::from_millis(250),
            "the lock waited for the reading on {letter}: only {took:?} (locked: {locked})"
        );
        // Let go of meanwhile, so watched again: the handle open, the gate too.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let now = watched();
            if now.iter().any(|d| d.0 == letter && d.1) && DRIVES.enter(&root).is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{letter}: not watched again (locked: {locked}): {now:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

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
