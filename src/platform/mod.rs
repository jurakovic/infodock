//! The OS layer (SPEC §4.6). Everything OS-specific sits behind these traits so other platforms
//! can be added without touching sensors. Windows lives in `windows/`; every other OS gets the
//! portable stub in `stub.rs` (enough to develop and test on Linux).
//!
//! The spec's `Shell`, `Icons`, `Power` and `Devices` services are methods of [`Platform`],
//! added with the milestones that need them (M2–M3); the locale is plain data (`locale.rs`).

pub mod accessible;
pub mod bitmap;
pub mod canvas;
pub mod drag;
pub mod geometry;
pub mod locale;
pub mod notify;
pub mod recording;
#[cfg(not(windows))]
mod stub;
pub mod text;
#[cfg(windows)]
mod windows;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use serde::Serialize;

use crate::config::model::{BarMode, DateLocale, Edge, FloatingRect, Material, Settings, ZOrder};
pub use accessible::{AccessibleAction, AccessibleBar, AccessibleItem};
pub use bitmap::{Bitmap, ImageError, decode_image, load_image};
pub use canvas::{Brush, Canvas, Color, GradientStop, PointF, RectF, Shape, SizeF};
pub use drag::{DragEvent, DragItem, DropEffect, DropEffects};
pub use geometry::{MonitorInfo, Rect};
pub use locale::Locale;
pub use text::{Font, TextEngine, TextLayout, TextStyle};

/// Which sensors and features exist on this system (SPEC §4.6). Flags turn on as their
/// implementations land.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub appbar: bool,
    pub recycle_bin: bool,
    pub start_menu: bool,
    pub wifi: bool,
    pub battery: bool,
    /// The [`PowerAction`] ids this system offers, in menu order.
    pub power_modes: Vec<String>,
    pub eject: bool,
    pub per_core_cpu: bool,
}

impl Capabilities {
    pub fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::RecycleBin => self.recycle_bin,
            Capability::StartMenu => self.start_menu,
            Capability::Wifi => self.wifi,
            Capability::Battery => self.battery,
            Capability::Eject => self.eject,
            Capability::PerCoreCpu => self.per_core_cpu,
        }
    }
}

/// A capability a sensor type can require (SPEC §4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    RecycleBin,
    StartMenu,
    Wifi,
    Battery,
    Eject,
    PerCoreCpu,
}

/// The OS appearance settings the bar follows (SPEC §10.3, §12.1–12.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OsAppearance {
    /// Apps use dark mode.
    pub dark: bool,
    /// "Transparency effects" is on; materials are disabled while it is off.
    pub transparency: bool,
    /// The user's accent colour.
    pub accent: Color,
    /// "Animation effects" is on (SPEC §5.5).
    pub animations: bool,
}

impl Default for OsAppearance {
    fn default() -> Self {
        Self {
            dark: true,
            transparency: true,
            // The Windows default accent.
            accent: Color::rgba(0x00, 0x78, 0xD4, 1.0),
            animations: true,
        }
    }
}

/// Processor times since the system started, in 100 ns units (`GetSystemTimes`); kernel time
/// includes idle time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuTimes {
    pub idle: u64,
    pub kernel: u64,
    pub user: u64,
}

/// One reading of the processors (SPEC Appendix A, `cpu`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CpuReading {
    pub times: CpuTimes,
    /// The busy share of all processors as Task Manager shows it (`% Processor Utility`,
    /// 0–100); `None` on the first reading, which has nothing to compare with, or where the
    /// counter cannot be read.
    pub total: Option<f64>,
    /// The same per logical processor, in processor order; empty where unavailable.
    pub cores: Vec<f64>,
    /// The number of logical processors.
    pub logical: u32,
}

/// Reads the processors. Owned by the sampler thread, so counters are opened once and reused
/// (SPEC §14.1).
pub trait CpuReader: Send {
    /// Why not (the OS's message) if nothing could be read.
    fn read(&mut self) -> Result<CpuReading, String>;
}

/// A page file's size and how much of it is in use, bytes (SPEC Appendix A, `swap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageFile {
    pub allocated: u64,
    pub used: u64,
}

/// What kind of drive a volume is on (SPEC Appendix A, `volumes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Optical,
    Network,
    Ram,
    Other,
}

/// A volume with a drive letter (SPEC §4.5: lists of volumes; Appendix A `volumes`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// `C:\`.
    pub root: String,
    pub kind: DriveKind,
    /// The volume label; empty if it has none or no medium is inserted.
    pub label: String,
    /// It can be ejected: removable and optical drives, and disks on USB, FireWire or SD
    /// buses (SPEC §9.22).
    pub ejectable: bool,
    /// A medium is inserted; always for fixed, network and RAM drives.
    pub media: bool,
}

impl Volume {
    /// A volume of `kind` with a medium, not ejectable unless removable or optical.
    pub fn new(root: impl Into<String>, kind: DriveKind, label: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            kind,
            label: label.into(),
            ejectable: matches!(kind, DriveKind::Removable | DriveKind::Optical),
            media: true,
        }
    }
}

/// Why a drive was not ejected (SPEC §9.22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EjectError {
    /// A program or service has it open; its name when Windows says.
    InUse(Option<String>),
    /// Not a drive that can be ejected (an internal disk), or not connected.
    NotEjectable,
    /// Something else went wrong.
    Failed(String),
}

/// Words an eject's result as a notification: its title and body (SPEC §9.18, §9.22).
pub type EjectReport = Box<dyn FnOnce(&Result<(), EjectError>) -> (String, String) + Send>;

/// A volume's size and free space, bytes (SPEC Appendix A, `disk/<X:\>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiskSpace {
    pub total: u64,
    pub free: u64,
}

/// A network interface (SPEC Appendix A, `net/<id|auto>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetInterface {
    /// A stable id: the interface GUID.
    pub id: String,
    /// The connection's name, e.g. `Wi-Fi`.
    pub alias: String,
    /// The adapter, e.g. `Intel(R) Wi-Fi 6E AX211`.
    pub description: String,
    /// Connected (operational).
    pub up: bool,
    /// Bytes received and sent since the interface started.
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// Link speeds, bits per second.
    pub link_rx: u64,
    pub link_tx: u64,
}

/// What a path names (SPEC §9.17: a Shortcut is inactive while its target is missing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Missing,
    File,
    Folder,
}

/// A program, document, folder or URI to start, as a sensor click starts it (SPEC §9.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launch<'a> {
    pub target: &'a str,
    /// Parameters for a program, passed as they are (never interpreted by a shell).
    pub args: &'a str,
    /// The working folder; `None` leaves it to the shell.
    pub dir: Option<&'a str>,
    /// Run as administrator (the `runas` verb), which asks for elevation.
    pub admin: bool,
}

impl<'a> Launch<'a> {
    /// `target` opened as Explorer would, without parameters.
    pub fn open(target: &'a str) -> Self {
        Self {
            target,
            args: "",
            dir: None,
            admin: false,
        }
    }
}

/// What a battery is doing (SPEC §9.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    Charging,
    Discharging,
    /// Full, on AC power.
    Full,
    /// On AC power but held below full, e.g. by a charge limit.
    NotCharging,
    Unknown,
}

/// One battery now (SPEC Appendix A `battery/<i>`).
#[derive(Debug, Clone, PartialEq)]
pub struct BatteryReading {
    /// Its vendor and model as the battery reports them; empty when it says nothing.
    pub name: String,
    /// Its charge, 0–100.
    pub percent: f32,
    pub state: BatteryState,
    /// What the battery estimates from its rate; `None` unless discharging or charging.
    pub time_to_empty: Option<std::time::Duration>,
    pub time_to_full: Option<std::time::Duration>,
}

/// The batteries and the power line (SPEC Appendix A `battery/<i>`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PowerStatus {
    /// Running on AC power.
    pub ac_online: bool,
    /// In the order the battery driver lists them; one that cannot be read keeps its place,
    /// with the driver's message, so the batteries after it keep theirs (SPEC §9.14).
    pub batteries: Vec<Result<BatteryReading, String>>,
}

/// The Wi-Fi connection now (SPEC §9.15, Appendix A `wifi`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WifiStatus {
    /// A wireless interface exists.
    pub available: bool,
    /// Its radio is on (not in airplane mode, not switched off).
    pub radio_on: bool,
    pub connected: bool,
    /// The network name; `None` when not connected, or when Windows withholds it.
    pub ssid: Option<String>,
    /// The signal quality, 0–100.
    pub quality: u32,
    pub rssi_dbm: Option<i32>,
    /// The channel's centre frequency, MHz.
    pub frequency_mhz: Option<u32>,
    /// The link rate, Mbit/s.
    pub link_mbps: Option<u32>,
    /// The 802.11 standard, e.g. `802.11ax`.
    pub phy: Option<&'static str>,
    /// Windows needs location access to tell the network name (Windows 11 24H2 and later).
    pub location_denied: bool,
}

/// Reads the Wi-Fi connection. Owned by the sampler thread, so the WLAN client stays open
/// (and its notifications registered) while the `wifi` topic has subscribers.
pub trait WifiReader: Send {
    /// Why not (the OS's message) if nothing could be read.
    fn read(&mut self) -> Result<WifiStatus, String>;
}

/// What the Recycle Bin holds on all drives (SPEC §9.16, Appendix A `recycle-bin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecycleBin {
    pub items: u64,
    /// The size of the items, bytes.
    pub bytes: u64,
}

/// An entry of a folder (SPEC §9.19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirItem {
    pub name: String,
    pub path: String,
    /// A folder (or a link to one), not a file.
    pub folder: bool,
    /// Hidden, or a system file.
    pub hidden: bool,
}

/// Compares names as Explorer sorts them: ignoring case, with runs of digits by their value
/// (`File 2` before `File 10`). The platform-neutral stand-in for `CompareStringEx`.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        chars.next();
                    }
                    digits.trim_start_matches('0').to_owned()
                };
                let (x, y) = (run(&mut a), run(&mut b));
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(&y));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// A file operation into a folder (SPEC §9.19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOp {
    Copy,
    Move,
    /// Shortcuts (`.lnk`) to the items.
    Link,
}

impl FileOp {
    /// The pointer's effect for this operation.
    pub fn effect(self) -> DropEffect {
        match self {
            Self::Copy => DropEffect::Copy,
            Self::Move => DropEffect::Move,
            Self::Link => DropEffect::Link,
        }
    }
}

/// The root of `path` by its form: `C:\` for `C:\Data`, `\\server\share\` for a UNC path.
pub fn path_root(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Some(format!("{}:\\", path[..1].to_ascii_uppercase()));
    }
    let unc = path.strip_prefix("\\\\")?;
    let mut parts = unc.split('\\').filter(|p| !p.is_empty());
    let (server, share) = (parts.next()?, parts.next()?);
    Some(format!("\\\\{server}\\{share}\\").to_lowercase())
}

/// `stem` + `ext` in `folder`, numbered (`stem (2).ext`) while that name is taken.
pub fn unique_path(folder: &std::path::Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = folder.join(format!("{stem}{ext}"));
    let mut n = 2;
    while path.exists() && n < 1000 {
        path = folder.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    path
}

/// The text of an Internet shortcut (`.url`) to `url`.
pub fn url_file_text(url: &str) -> String {
    format!("[InternetShortcut]\r\nURL={url}\r\n")
}

/// A folder the shell knows by role (SPEC §9.20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownFolder {
    /// The user's Start Menu programs (`FOLDERID_Programs`).
    Programs,
    /// The Start Menu programs of all users (`FOLDERID_CommonPrograms`).
    CommonPrograms,
}

/// What the Power sensor does (SPEC §9.21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Lock,
    SignOut,
    /// The sign-in screen, keeping the session.
    SwitchUser,
    Sleep,
    Hibernate,
    ShutDown,
    Restart,
}

impl PowerAction {
    /// In menu order.
    pub const ALL: [Self; 7] = [
        Self::Lock,
        Self::SignOut,
        Self::SwitchUser,
        Self::Sleep,
        Self::Hibernate,
        Self::ShutDown,
        Self::Restart,
    ];

    /// The id in the config and in [`Capabilities::power_modes`].
    pub fn id(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::SignOut => "signOut",
            Self::SwitchUser => "switchUser",
            Self::Sleep => "sleep",
            Self::Hibernate => "hibernate",
            Self::ShutDown => "shutDown",
            Self::Restart => "restart",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }
}

/// Physical memory (SPEC Appendix A, `memory`; `GlobalMemoryStatusEx`), bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryStatus {
    pub total: u64,
    pub available: u64,
    /// The share in use as the OS reports it (`dwMemoryLoad`), 0–100.
    pub load: u32,
}

/// A change to the per-user `Run` entry that starts Infodock at sign-in (SPEC §10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutostartChange<'a> {
    /// The user turned autostart on: write the command, and clear the mark with which Task
    /// Manager's Startup apps keeps an entry from starting.
    Enable(&'a str),
    /// The exe moved: write its command, keeping that mark.
    Update(&'a str),
    /// Remove the entry, and its mark.
    Remove,
}

/// Why a reading failed where the platform has no reader for it (SPEC §14.2); the bar runs
/// only on Windows, so no user sees it.
pub const UNSUPPORTED: &str = "not supported on this platform";

pub trait Platform: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    /// The capabilities if they are known already, without waiting for a probe still running:
    /// the bar's first frame does not wait for it (SPEC §14.1).
    fn probed_capabilities(&self) -> Option<Capabilities> {
        Some(self.capabilities())
    }
    /// All active monitors, physical pixels.
    fn monitors(&self) -> Vec<MonitorInfo>;
    fn appearance(&self) -> OsAppearance;
    /// The time since the system started (the `uptime` topic, SPEC Appendix A).
    fn uptime(&self) -> std::time::Duration;
    /// The installed font families, sorted by name (SPEC §4.5: lists of fonts); none where
    /// the platform cannot list them.
    fn font_families(&self) -> Vec<String> {
        Vec::new()
    }
    /// The number of logical processors, e.g. for the CPU dual graph's series (SPEC §9.9).
    fn logical_processors(&self) -> u32 {
        std::thread::available_parallelism().map_or(1, |n| n.get() as u32)
    }
    /// A reader of the processors' load, for the `cpu` topic; `None` where there is none.
    fn cpu_reader(&self) -> Option<Box<dyn CpuReader>> {
        None
    }
    /// Physical memory now, for the `memory` topic; why not (the OS's message) when it cannot
    /// be read.
    fn memory(&self) -> Result<MemoryStatus, String> {
        Err(UNSUPPORTED.into())
    }
    /// The page files now, for the `swap` topic: empty when there is none; why not when they
    /// cannot be read.
    fn page_files(&self) -> Result<Vec<PageFile>, String> {
        Err(UNSUPPORTED.into())
    }
    /// The volumes with drive letters, in letter order.
    fn volumes(&self) -> Vec<Volume> {
        Vec::new()
    }
    /// The size and free space of the volume at `root` (`C:\`), for the `disk/<X:\>` topic;
    /// `None` when there is none (no such drive, no medium inserted).
    fn disk_space(&self, root: &str) -> Option<DiskSpace> {
        let _ = root;
        None
    }
    /// The network connections users know (no loopback or filter drivers), by name (SPEC §4.5:
    /// lists of network adapters).
    fn net_interfaces(&self) -> Vec<NetInterface> {
        Vec::new()
    }
    /// The interface `id` now, or with `None` the one carrying the default route (SPEC §9.13
    /// Automatic), for the `net/<id|auto>` topic; `None` when there is none.
    fn net_interface(&self, id: Option<&str>) -> Option<NetInterface> {
        let _ = id;
        None
    }
    /// The local wall-clock time; UTC where the platform does not know the time zone.
    fn local_time(&self) -> crate::sensors::dates::LocalTime {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        crate::sensors::dates::LocalTime::from_unix(
            i64::try_from(now.as_secs()).unwrap_or(0),
            now.subsec_millis() as u16,
        )
    }
    /// Opens `target` as Explorer would, for sensor clicks (SPEC §9): a program such as
    /// `taskmgr.exe`, a file or folder, or a URI such as `ms-settings:dateandtime`. It runs in
    /// the background; failures are logged (and the shell may explain them).
    fn open(&self, target: &str) {
        self.launch(&Launch::open(target));
    }
    /// Starts a program, document, folder or URI with parameters, a working folder and
    /// optionally elevated (SPEC §9.17), in the background like [`Platform::open`].
    fn launch(&self, launch: &Launch) {
        tracing::info!("starting {launch:?} is not supported on this platform");
    }
    /// What `path` names now. It may wait for a network server, so it runs off the bar
    /// thread (the `path/<path>` topic).
    fn path_kind(&self, path: &str) -> PathKind {
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => PathKind::Folder,
            Ok(_) => PathKind::File,
            Err(_) => PathKind::Missing,
        }
    }
    /// The entries of folder `path`, unsorted; why not when it cannot be read. It may wait
    /// for a network server, so it runs off the bar thread (SPEC §9.19).
    fn list_dir(&self, path: &str) -> Result<Vec<DirItem>, String> {
        let entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        Ok(entries
            .filter_map(Result::ok)
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path();
                DirItem {
                    folder: path.is_dir(),
                    hidden: name.starts_with('.'),
                    path: path.to_string_lossy().into_owned(),
                    name,
                }
            })
            .collect())
    }
    /// Compares two names as the shell sorts them (SPEC §9.19).
    fn compare_names(&self, a: &str, b: &str) -> std::cmp::Ordering {
        natural_cmp(a, b)
    }
    /// Where the shell keeps `folder`; `None` where there is no such folder.
    fn known_folder(&self, folder: KnownFolder) -> Option<String> {
        let _ = folder;
        None
    }
    /// The volume `path` lies on (`C:\`, `\\server\share\`), to choose between moving and
    /// copying a drop (SPEC §9.19); `None` where it cannot tell. By the path's form, so a drag
    /// touches no drive: a volume mounted in a folder counts as the one holding the folder.
    fn volume_of(&self, path: &str) -> Option<String> {
        path_root(path)
    }
    /// Copies or moves `paths` into `folder`, or makes shortcuts (`.lnk`) to them there, in the
    /// background with the shell's progress and conflict dialogs (SPEC §9.19); then calls
    /// `done`, whatever happened.
    fn file_operation(
        &self,
        op: FileOp,
        paths: Vec<String>,
        folder: String,
        done: Box<dyn FnOnce() + Send>,
    ) {
        tracing::info!("{op:?} {paths:?} into {folder} is not supported on this platform");
        done();
    }
    /// Moves `paths` to the Recycle Bin in the background, asking as the OS settings say (SPEC
    /// §9.16); then calls `done`, whatever happened.
    fn recycle(&self, paths: Vec<String>, done: Box<dyn FnOnce() + Send>) {
        tracing::info!("recycling {paths:?} is not supported on this platform");
        done();
    }
    /// Writes an Internet shortcut to `url` into `folder` as `stem.url` (numbered if that name
    /// is taken), in the background (SPEC §9.19); failures are logged.
    fn create_url_file(&self, folder: &str, stem: &str, url: &str) {
        let (folder, stem, url) = (PathBuf::from(folder), stem.to_owned(), url.to_owned());
        let spawned = std::thread::Builder::new()
            .name("url-file".into())
            .spawn(move || {
                let path = unique_path(&folder, &stem, ".url");
                if let Err(e) = std::fs::write(&path, url_file_text(&url)) {
                    tracing::warn!("could not write {}: {e}", path.display());
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("could not start writing a link file: {e}");
        }
    }
    /// Shows `path` selected in its folder in Explorer, in the background (SPEC §9.19).
    fn reveal(&self, path: &str) {
        tracing::info!("revealing {path:?} is not supported on this platform");
    }
    /// The shell's icon for a file, folder or drive, `px` physical px square (a `.lnk` shows
    /// its target's icon); `None` where it has none or the platform has no shell. It may be
    /// slow, so the bar asks for it on a worker thread.
    fn shell_icon(&self, path: &str, px: u32) -> Option<Bitmap> {
        let _ = (path, px);
        None
    }
    /// The batteries and whether the computer runs on AC power, for the `battery/<i>` topics
    /// and Settings' battery lists; why not (the OS's message) when it cannot be read.
    /// Batteries can be slow to answer, so it is read off the sampler thread.
    fn power_status(&self) -> Result<PowerStatus, String> {
        Err(UNSUPPORTED.into())
    }
    /// A reader of the Wi-Fi connection for the `wifi` topic; `changed` is called from another
    /// thread when Windows reports a connection, a disconnection or a radio change (SPEC
    /// §9.15). `None` where there is no WLAN.
    fn wifi_reader(&self, changed: Box<dyn Fn() + Send + Sync>) -> Option<Box<dyn WifiReader>> {
        let _ = changed;
        None
    }
    /// What the Recycle Bin holds on all drives, for the `recycle-bin` topic; why not (the
    /// shell's message) when it cannot be read. It may take a while with many items, so it is
    /// read off the sampler thread.
    fn recycle_bin(&self) -> Result<RecycleBin, String> {
        Err(UNSUPPORTED.into())
    }
    /// Empties the Recycle Bin in the background, with the OS confirmation, progress and
    /// sound (SPEC §9.16), then calls `done`, whether it was emptied or not.
    fn empty_recycle_bin(&self, done: Box<dyn FnOnce() + Send>) {
        tracing::info!("emptying the Recycle Bin is not supported on this platform");
        done();
    }
    /// Locks, signs out, switches user, sleeps, hibernates, shuts down or restarts (SPEC
    /// §9.21), in the background; `force` closes apps without asking (sign out, shut down,
    /// restart). Failures are logged.
    fn power(&self, action: PowerAction, force: bool) {
        tracing::info!("{action:?} (force: {force}) is not supported on this platform");
    }
    /// Ejects drive `root` (`E:\`) in the background (SPEC §9.22) and shows the notification
    /// `report` makes of the result.
    fn eject(&self, root: &str, report: EjectReport) {
        tracing::info!("ejecting {root} is not supported on this platform");
        let (title, body) = report(&Err(EjectError::NotEjectable));
        self.notify(&title, &body);
    }
    /// Shows a notification from Infodock (SPEC §7.4): a toast with `title` (e.g. the
    /// sensor's) and `body`, in the background; failures are logged.
    fn notify(&self, title: &str, body: &str) {
        tracing::info!("notification: {title}: {body}");
    }
    /// Plays a WAV sound in the background, e.g. the Time sensor's chime (SPEC §9.5).
    fn play_sound(&self, wav: &'static [u8]) {
        let _ = wav;
    }
    /// The command of the per-user `Run` entry `name` (SPEC §10.1), as written; `None` without
    /// one, or where nothing starts at sign-in.
    fn autostart(&self, name: &str) -> Option<String> {
        let _ = name;
        None
    }
    /// Changes the `Run` entry `name`; the error is the OS's message. Does nothing where
    /// nothing starts at sign-in.
    fn set_autostart(&self, name: &str, change: &AutostartChange) -> Result<(), String> {
        let _ = (name, change);
        Ok(())
    }
    /// `bytes` in the Windows code page `code_page`, or the system's ANSI code page with `None`
    /// (the legacy import, SPEC §11.4); `None` where the platform cannot convert them.
    fn decode_code_page(&self, bytes: &[u8], code_page: Option<u32>) -> Option<String> {
        let _ = (bytes, code_page);
        None
    }
    /// The locale for dates and numbers (SPEC §8.6): the OS regional format for `System`, the
    /// UI language's for `Ui`. English (United States) where the platform cannot tell.
    fn locale(&self, which: DateLocale) -> Locale {
        let _ = which;
        Locale::en_us()
    }
}

/// Where and how the bar should sit (SPEC §5.1–5.3), derived from the settings.
#[derive(Debug, Clone, PartialEq)]
pub struct DockRequest {
    pub visible: bool,
    pub edge: Edge,
    pub mode: BarMode,
    /// `"primary"` or a [`MonitorInfo::key`].
    pub monitor: String,
    /// Logical px.
    pub height: u32,
    pub reserve_space: bool,
    pub z_order: ZOrder,
    pub hide_on_fullscreen: bool,
    /// Where a floating bar is (SPEC §5.1); `None` until it was moved or sized.
    pub floating: Option<FloatingRect>,
}

impl DockRequest {
    pub fn from_settings(s: &Settings, visible: bool) -> Self {
        Self {
            visible,
            edge: s.bar.edge,
            mode: s.bar.mode,
            monitor: s.bar.monitor.clone(),
            height: s.bar_height(),
            reserve_space: s.bar.reserve_space,
            z_order: s.bar.z_order,
            hide_on_fullscreen: s.bar.hide_on_fullscreen,
            floating: s.bar.floating,
        }
    }
}

/// The bar's geometry after placement.
#[derive(Debug, Clone, PartialEq)]
pub struct BarGeometry {
    /// Physical pixels.
    pub rect: Rect,
    pub edge: Edge,
    pub monitor: MonitorInfo,
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppBarEvent {
    Placed(BarGeometry),
    /// A full-screen app opened (`true`) or closed on the bar's monitor (SPEC §5.3).
    Fullscreen(bool),
}

/// Receives AppBar events on the bar thread. Platform-internal: the bar window forwards them to
/// the handler as [`UiEvent`]s.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) type AppBarSink = Box<dyn Fn(AppBarEvent) + Send + Sync>;

/// Docks the bar window and reserves screen space (SPEC §5.2). Implementations are thread-safe;
/// changes are applied asynchronously on the window's UI thread.
pub trait AppBar: Send + Sync {
    /// Applies a new placement. Repeating the current request is cheap.
    fn configure(&self, request: DockRequest);
    /// Recomputes the placement, e.g. after the monitor layout changed.
    fn refresh(&self);
    /// Brings the bar to the front and focuses it. With z-order `bottom` it stays in front until
    /// it loses focus (SPEC §5.3).
    fn raise(&self);
    /// Releases the reserved screen space. Safe to call repeatedly and from the panic hook.
    fn release(&self);
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0}")]
    Os(String),
}

/// The platform for this OS.
pub fn current() -> Arc<dyn Platform> {
    #[cfg(windows)]
    return Arc::new(windows::WindowsPlatform);
    #[cfg(not(windows))]
    return Arc::new(stub::StubPlatform);
}

// ---------------------------------------------------------------------------------------------
// Drawing: text and images (SPEC §4.6)
// ---------------------------------------------------------------------------------------------

/// The text engine for this thread's windows: DirectWrite on Windows, fixed metrics elsewhere.
pub fn text_engine() -> Rc<dyn TextEngine> {
    #[cfg(windows)]
    match windows::text::DirectWrite::new() {
        Ok(engine) => return Rc::new(engine),
        Err(e) => tracing::error!("text rendering is unavailable: {e}"),
    }
    Rc::new(recording::FixedMetrics)
}

/// Decodes a raster image (everything but SVG); see [`decode_image`].
fn decode_raster(
    bytes: &[u8],
    format: bitmap::ImageFormat,
    height: Option<u32>,
) -> Result<Bitmap, ImageError> {
    #[cfg(windows)]
    return windows::image::decode(bytes, format, height);
    #[cfg(not(windows))]
    {
        let _ = (bytes, format, height);
        Err(ImageError::Unsupported)
    }
}

/// What a file or folder picker asks for (SPEC §4.6 `IFileOpenDialog`).
pub struct PathRequest<'a> {
    pub title: &'a str,
    /// Pick a folder instead of a file.
    pub folder: bool,
    /// The file types offered first, e.g. `["png", "jpg"]`, under `types_name`; "All files"
    /// (`all_files`) is always offered too.
    pub extensions: &'a [&'a str],
    pub types_name: &'a str,
    pub all_files: &'a str,
}

/// Shows the OS file or folder picker, modal to the native window `owner`, on the calling
/// thread. `None` if the user cancelled, or where there is no picker.
pub fn pick_path(owner: Option<isize>, request: &PathRequest) -> Option<String> {
    #[cfg(windows)]
    return windows::shell::pick_path(owner, request);
    #[cfg(not(windows))]
    {
        let _ = (owner, request);
        None
    }
}

/// What the save picker asks for (SPEC §4.6 `IFileSaveDialog`).
pub struct SaveRequest<'a> {
    pub title: &'a str,
    /// The file name suggested, without its extension.
    pub file_name: &'a str,
    /// The file types offered, each its name and extension (e.g. `infodock.json`); the first is
    /// chosen at first, and its extension is added to a name typed without one.
    pub types: &'a [(&'a str, &'a str)],
}

/// Shows the OS save picker, modal to the native window `owner`, on the calling thread: the
/// chosen path and the index of the chosen file type. `None` if the user cancelled, or where
/// there is no picker.
pub fn save_path(owner: Option<isize>, request: &SaveRequest) -> Option<(String, usize)> {
    #[cfg(windows)]
    return windows::shell::save_path(owner, request);
    #[cfg(not(windows))]
    {
        let _ = (owner, request);
        None
    }
}

/// A question answered with one of `choices` or Cancel (SPEC §6.4).
pub struct Choice<'a> {
    pub question: &'a str,
    /// Shown under the question; may be empty.
    pub detail: &'a str,
    /// The first is the default.
    pub choices: &'a [&'a str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Warning,
    Error,
}

/// A message with OK (SPEC §6.4).
pub struct Message<'a> {
    pub kind: MessageKind,
    pub text: &'a str,
    /// Shown under the text; may be empty.
    pub detail: &'a str,
}

/// The native dialogs, for flows that ask in steps (an import: the file, then Replace or
/// Merge, then the report). Tests answer with a script.
pub trait Dialogs {
    /// The file or folder picker; see [`pick_path`].
    fn open_path(&self, request: &PathRequest) -> Option<String>;
    /// The save picker; see [`save_path`].
    fn save_path(&self, request: &SaveRequest) -> Option<(String, usize)>;
    /// The index of the chosen answer; `None` for Cancel, or where there are no dialogs.
    fn choose(&self, choice: &Choice) -> Option<usize>;
    fn show_message(&self, message: &Message);
}

/// The OS dialogs (`TaskDialogIndirect`, the pickers), modal to the native window `owner`, on
/// the calling thread, each running its own modal loop. Without them (other OSes), nothing is
/// asked and every question is cancelled.
pub struct NativeDialogs {
    pub owner: Option<isize>,
}

impl Dialogs for NativeDialogs {
    fn open_path(&self, request: &PathRequest) -> Option<String> {
        pick_path(self.owner, request)
    }

    fn save_path(&self, request: &SaveRequest) -> Option<(String, usize)> {
        save_path(self.owner, request)
    }

    fn choose(&self, choice: &Choice) -> Option<usize> {
        #[cfg(windows)]
        return windows::dialog::choose(
            owner_hwnd(self.owner),
            &crate::i18n::t("app.name"),
            choice,
        );
        #[cfg(not(windows))]
        {
            let _ = choice;
            None
        }
    }

    fn show_message(&self, message: &Message) {
        #[cfg(windows)]
        windows::dialog::message(owner_hwnd(self.owner), &crate::i18n::t("app.name"), message);
        #[cfg(not(windows))]
        tracing::info!("message: {} {}", message.text, message.detail);
    }
}

#[cfg(windows)]
fn owner_hwnd(owner: Option<isize>) -> Option<::windows::Win32::Foundation::HWND> {
    owner.map(|h| ::windows::Win32::Foundation::HWND(h as *mut std::ffi::c_void))
}

/// Renders `width`×`height` physical px at `scale` with the bar's renderer, e.g. for previews
/// and the screenshots CI publishes (SPEC §12.4, §16). Windows only for now.
pub fn render_offscreen(
    width: u32,
    height: u32,
    scale: f32,
    paint: &mut dyn FnMut(&mut dyn Canvas),
) -> Result<Bitmap, PlatformError> {
    #[cfg(windows)]
    return windows::render::render_offscreen(width, height, scale, paint);
    #[cfg(not(windows))]
    {
        let _ = (width, height, scale, paint);
        Err(PlatformError::Os("no renderer on this OS".into()))
    }
}

/// A renderer kept across frames, for the budget tests (SPEC §14.1).
#[cfg(all(windows, test))]
pub use windows::render::FrameRenderer;

// ---------------------------------------------------------------------------------------------
// The bar thread (SPEC §4.1)
// ---------------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------------
// App start-up: data folders and the single-instance lock (SPEC §11.1, §6.5)
// ---------------------------------------------------------------------------------------------

/// Where the app keeps its files (SPEC §11.1).
#[derive(Debug, Clone, PartialEq)]
pub struct AppDirs {
    /// Config and skin packs: `%APPDATA%\<identifier>` on Windows.
    pub config: PathBuf,
    /// Log files: `%LOCALAPPDATA%\<identifier>\logs` on Windows.
    pub logs: PathBuf,
}

impl AppDirs {
    /// Skin packs, one folder each (SPEC §12.4).
    pub fn skins(&self) -> PathBuf {
        self.config.join("skins")
    }
}

/// The per-user folders for the app with this identifier.
pub fn app_dirs(identifier: &str) -> AppDirs {
    #[cfg(windows)]
    return windows::instance::app_dirs(identifier);
    #[cfg(not(windows))]
    return stub::app_dirs(identifier);
}

/// Held for the app's lifetime; dropping it lets another instance start.
pub struct InstanceGuard(#[allow(dead_code)] Box<dyn Send>);

impl InstanceGuard {
    fn new(inner: impl Send + 'static) -> Self {
        Self(Box::new(inner))
    }
}

/// The result of claiming the single-instance lock.
pub enum Instance {
    /// This is the only instance.
    Primary(InstanceGuard),
    /// Another instance runs already; ask it to show itself with [`activate_running_instance`].
    Secondary,
}

/// Claims the per-user single-instance lock. If the lock cannot be created, this instance runs
/// as the primary one.
pub fn single_instance(identifier: &str) -> Instance {
    #[cfg(windows)]
    return windows::instance::single_instance(identifier);
    #[cfg(not(windows))]
    return stub::single_instance(identifier);
}

/// Makes this process the app `identifier` for the OS (taskbar grouping, notifications;
/// SPEC §7.4), named `display_name` in notifications.
pub fn register_app_id(identifier: &str, display_name: &str) {
    #[cfg(windows)]
    windows::toast::register(identifier, display_name);
    #[cfg(not(windows))]
    let _ = (identifier, display_name);
}

/// Asks the running instance to show the bar and open Settings (SPEC §6.5). Waits up to a few
/// seconds for its bar window to exist.
pub fn activate_running_instance() {
    #[cfg(windows)]
    windows::instance::activate_running_instance();
}

/// Events for the bar logic, delivered on the bar thread.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    /// The bar window exists; sent once before any other event.
    Started,
    /// Work was queued for the bar thread ([`UiWaker::wake`]).
    Wake,
    /// The bar was placed or moved.
    Placed(BarGeometry),
    /// A full-screen app opened (`true`) or closed on the bar's monitor (SPEC §5.3).
    Fullscreen(bool),
    /// Light/dark mode, accent colour or transparency effects may have changed.
    AppearanceChanged,
    /// The OS asks the app to close (graceful `taskkill`, Restart Manager).
    CloseRequested,
    /// The system time or time zone changed (`WM_TIMECHANGE`).
    TimeChanged,
    /// The OS regional format changed (`WM_SETTINGCHANGE` "intl").
    LocaleChanged,
    /// The system resumed from sleep or hibernation.
    Resumed,
    /// A drive arrived or went: a disk plugged in or removed, a disc inserted or ejected, a
    /// network drive mapped or disconnected.
    DevicesChanged,
    /// The power line or a battery's charge changed (SPEC §9.14).
    PowerChanged,
    /// A second launch asked this instance to show itself (SPEC §6.5).
    Activate,
    /// The tray icon was clicked or selected with the keyboard (SPEC §6.5).
    TrayClick,
    /// The tray icon's menu was requested (right click, menu key).
    TrayMenu,
    /// A timer set with [`BarWindow::set_timer`] is due.
    Timer(TimerId),
    /// The pointer moved over the window (or anywhere while captured); logical px from the
    /// window's top-left corner.
    PointerMove(PointF),
    /// The pointer left the window.
    PointerLeave,
    PointerDown {
        button: PointerButton,
        at: PointF,
        modifiers: Modifiers,
    },
    PointerUp {
        button: PointerButton,
        at: PointF,
        modifiers: Modifiers,
    },
    /// Another window took the pointer capture.
    CaptureLost,
    /// Input in the popup window (SPEC §7.2).
    Popup(PopupInput),
    /// The popup window lost the activation: another window was clicked or activated, or it
    /// was asked to close (Alt+F4). It is hidden already.
    PopupDeactivated,
    /// A key pressed while the bar window has the keyboard focus (SPEC §14.4).
    Key { key: Key, modifiers: Modifiers },
    /// A screen reader asks something of a sensor through UI Automation (SPEC §14.4).
    Accessible(AccessibleAction),
    /// The bar window got the keyboard focus.
    FocusGained,
    /// The bar window lost the keyboard focus; `within_app`: to another window of this app on
    /// the bar thread (its popup, a confirmation), so it may come back.
    FocusLost { within_app: bool },
}

/// A key the popup window (SPEC §9.19) or the focused bar (§14.4) passes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Tab,
    Space,
    /// The context-menu key.
    Menu,
    F10,
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Backspace,
    Home,
    End,
    PageUp,
    PageDown,
}

/// Input in the popup window, logical px from its top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub enum PopupInput {
    PointerMove(PointF),
    PointerLeave,
    PointerDown {
        button: PointerButton,
        at: PointF,
        modifiers: Modifiers,
    },
    PointerUp {
        button: PointerButton,
        at: PointF,
        modifiers: Modifiers,
    },
    /// The wheel turned by `lines` text lines (the system's lines per notch; positive scrolls
    /// towards the top), with the pointer at `at`.
    Wheel {
        lines: f32,
        at: PointF,
    },
    Key {
        key: Key,
        modifiers: Modifiers,
    },
    /// A character typed, e.g. for jumping to a name (never a control character).
    Char(char),
    /// Another window took the pointer capture the popup asked for.
    CaptureLost,
}

/// The pointer's shape over the popup window, or the bar's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupCursor {
    #[default]
    Arrow,
    /// Over a grip that changes the width.
    ResizeWidth,
    /// Over a grip that changes the height.
    ResizeHeight,
}

/// The bar logic's timers; each runs independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerId {
    /// The sensors' earliest wake-up (SPEC §4.4).
    Wake,
    /// The hint's show or hide delay (SPEC §7.1).
    Hint,
    /// Polls Esc while a reorder drag runs (SPEC §6.2).
    Drag,
    /// The next frame of a bar animation (SPEC §5.5).
    Frame,
    /// The open popup's next wake-up: an animation frame, or work it waits for (SPEC §7.2).
    Popup,
    /// Checks whether the capability probe has answered, so the sensors that need one join
    /// (SPEC §14.1).
    Probe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

/// Modifier keys held during an input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

/// An entry of a native menu: the tray menu, and the context menus of SPEC §6.4.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuEntry {
    Item {
        id: u32,
        label: String,
        checked: bool,
        enabled: bool,
    },
    Separator,
    Submenu {
        label: String,
        entries: Vec<MenuEntry>,
    },
}

impl MenuEntry {
    pub fn item(id: u32, label: impl Into<String>) -> Self {
        Self::Item {
            id,
            label: label.into(),
            checked: false,
            enabled: true,
        }
    }

    pub fn check(id: u32, label: impl Into<String>, checked: bool) -> Self {
        Self::Item {
            id,
            label: label.into(),
            checked,
            enabled: true,
        }
    }

    /// The same item, greyed out unless `enabled`.
    pub fn enabled(mut self, enabled: bool) -> Self {
        if let Self::Item { enabled: e, .. } = &mut self {
            *e = enabled;
        }
        self
    }

    pub fn submenu(label: impl Into<String>, entries: Vec<MenuEntry>) -> Self {
        Self::Submenu {
            label: label.into(),
            entries,
        }
    }
}

/// The bar logic. Called by the native bar window, always on the bar thread.
pub trait BarHandler {
    fn on_event(&mut self, window: &dyn BarWindow, event: UiEvent);
    /// A drag from another app over the bar (SPEC §6.3); returns what dropping would do, or on
    /// [`DragEvent::Drop`] what it did.
    fn on_drag(&mut self, window: &dyn BarWindow, event: DragEvent) -> DropEffect {
        let _ = (window, event);
        DropEffect::None
    }
    /// Draws the bar into a cleared canvas of `size` logical px.
    fn paint(&mut self, canvas: &mut dyn Canvas, size: SizeF);
    /// The skin's shadow band or the bar's shadow to show beside the bar (SPEC §5.1, §5.5),
    /// asked after each frame.
    fn band(&self) -> Option<Band> {
        None
    }
    /// The bar as screen readers see it (SPEC §14.4), asked after each frame once a UI
    /// Automation client asked for the bar.
    fn accessible(&self) -> AccessibleBar {
        AccessibleBar::default()
    }
}

/// A skin's shadow band or the bar's shadow (SPEC §5.1, §5.5, §12.4): a transparent shadow as
/// wide as the bar, beside it on the side facing the screen centre. The platform shows it in a
/// click-through window of its own that follows the bar's window, outside the reserved work
/// area.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    /// Physical px; its rows nearest the bar are the darkest.
    pub bitmap: Bitmap,
    /// Above the bar (one at the bottom of the screen), or below it.
    pub above: bool,
}

impl Band {
    /// The same band: the same pixels on the same side. Cheap, for checking every frame.
    pub fn same(&self, other: &Self) -> bool {
        self.above == other.above
            && Arc::ptr_eq(self.bitmap.shared_pixels(), other.bitmap.shared_pixels())
    }
}

/// The native bar window as the bar logic sees it (bar thread only).
pub trait BarWindow {
    /// Schedules a repaint.
    fn invalidate(&self);
    /// Applies a window material; [`Material::None`] removes it.
    fn set_material(&self, material: Material);
    /// Shows the notification-area icon with this tooltip, or removes it with `None`.
    fn set_tray_icon(&self, tooltip: Option<&str>);
    /// Shows a menu and returns the chosen item's id: at the pointer, or beside `near`
    /// (physical screen px, e.g. the focused sensor) without covering it. Runs the menu's modal
    /// loop; events that arrive meanwhile are delivered after the current handler call.
    fn show_menu(&self, entries: &[MenuEntry], near: Option<Rect>) -> Option<u32>;
    /// Delivers [`UiEvent::Timer`] once after `delay` (it may come a few ms early), replacing the
    /// previous timer with this id; `None` cancels it.
    fn set_timer(&self, id: TimerId, delay: Option<std::time::Duration>);
    /// Shows the hint window (SPEC §7.1) at `rect` (physical screen px) with what `paint` draws
    /// at `scale`: fading in when it was hidden, updated in place when it was shown. `name` is
    /// its title, which screen readers read (§14.4).
    fn show_hint(&self, rect: Rect, scale: f32, name: &str, paint: &mut dyn FnMut(&mut dyn Canvas));
    fn hide_hint(&self);
    /// Shows the popup window (SPEC §7.2) at `rect` (physical screen px) with what `paint`
    /// draws at `scale` and gives it the keyboard focus: fading in when it was hidden, updated
    /// in place (and moved) when it was shown. Its input arrives as [`UiEvent::Popup`]. `name`
    /// is its title, which screen readers read (§14.4).
    fn show_popup(
        &self,
        rect: Rect,
        scale: f32,
        name: &str,
        paint: &mut dyn FnMut(&mut dyn Canvas),
    );
    /// Hides the popup window; no [`UiEvent::PopupDeactivated`] follows.
    fn hide_popup(&self);
    /// The pointer's shape over the popup window, e.g. over a resize grip.
    fn set_popup_cursor(&self, cursor: PopupCursor);
    /// While captured, the popup's pointer events keep coming when the pointer leaves it
    /// (dragging a grip); [`PopupInput::CaptureLost`] says when another window takes it.
    fn capture_popup_pointer(&self, capture: bool);
    /// While captured, pointer events keep coming when the pointer leaves the window.
    fn capture_pointer(&self, capture: bool);
    /// Whether Esc is held now, read without focus (clicks never give the bar the focus; SPEC
    /// §6.2).
    fn escape_down(&self) -> bool;
    /// Gives the bar window the keyboard focus (the hotkey, SPEC §14.4), remembering which
    /// window of another app had it; returns whether the bar has it now.
    fn take_focus(&self) -> bool;
    /// Gives the keyboard focus back to the window that had it before [`Self::take_focus`].
    fn restore_focus(&self);
    /// Asks a yes/no question in a native dialog (SPEC §6.4): `message` as the main text and
    /// `action` on the confirming button. Returns whether the user confirmed.
    fn confirm(&self, message: &str, action: &str) -> bool;
    /// Moves a floating bar by physical px (SPEC §5.1) and returns where it is now; `None` when
    /// the bar is docked or not shown.
    fn move_by(&self, dx: i32, dy: i32) -> Option<Rect>;
    /// Puts a floating bar at `rect` (physical screen px) while one of its ends is dragged
    /// (SPEC §5.1); `width` is its width in logical px. Returns where it is now; `None` when
    /// the bar is docked or not shown.
    fn resize_to(&self, rect: Rect, width: u32) -> Option<Rect>;
    /// The pointer's shape over the bar window: the resize cursor over a floating bar's ends.
    fn set_cursor(&self, cursor: PopupCursor);
    /// Shows the bar `dy` physical px below (positive) or above its place, for the rise
    /// animation (SPEC §5.5); 0 puts it back.
    fn slide(&self, dy: i32);
}

/// Removes the tray icon from any thread, e.g. on exit.
pub trait TrayIcon: Send + Sync {
    fn remove(&self);
}

/// Wakes the bar thread from any thread; its handler then receives [`UiEvent::Wake`]. Wakes
/// that arrive before the handler ran are coalesced.
#[derive(Clone)]
pub struct UiWaker(Arc<dyn Fn() + Send + Sync>);

impl UiWaker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(wake))
    }

    pub fn wake(&self) {
        (self.0)()
    }
}

/// Builds the bar logic on the bar thread, with a waker its own workers can use (e.g. the icon
/// loader).
pub type HandlerFactory = Box<dyn FnOnce(UiWaker) -> Box<dyn BarHandler> + Send>;

/// Handles to a running bar thread, usable from any thread.
pub struct BarThread {
    pub appbar: Arc<dyn AppBar>,
    pub tray: Arc<dyn TrayIcon>,
    pub waker: UiWaker,
}

/// Starts the bar thread: creates the bar window and its AppBar, builds the handler with
/// `make_handler` on that thread and runs the message loop. Returns once the window exists; the
/// window becomes visible with the first [`AppBar::configure`].
pub fn start_bar_thread(make_handler: HandlerFactory) -> Result<BarThread, PlatformError> {
    #[cfg(windows)]
    return windows::window::start_bar_thread(make_handler);
    #[cfg(not(windows))]
    return stub::start_bar_thread(make_handler);
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::{natural_cmp, path_root, unique_path, url_file_text};

    #[test]
    fn roots_of_paths() {
        assert_eq!(path_root("c:\\Data\\a.txt").as_deref(), Some("C:\\"));
        assert_eq!(path_root("E:").as_deref(), Some("E:\\"));
        assert_eq!(
            path_root("\\\\Server\\Share\\Docs\\a").as_deref(),
            Some("\\\\server\\share\\")
        );
        assert_eq!(path_root("\\\\server"), None);
        assert_eq!(path_root("relative\\a"), None);
        let dir = tempfile::tempdir().expect("a temporary folder");
        assert_eq!(
            unique_path(dir.path(), "a", ".url"),
            dir.path().join("a.url")
        );
        std::fs::write(dir.path().join("a.url"), b"x").expect("a file");
        assert_eq!(
            unique_path(dir.path(), "a", ".url"),
            dir.path().join("a (2).url")
        );
        assert_eq!(
            url_file_text("https://example.com/"),
            "[InternetShortcut]\r\nURL=https://example.com/\r\n"
        );
    }

    #[test]
    fn names_sort_as_explorer_sorts_them() {
        let mut names = vec![
            "file10", "File2", "file1", "alpha", "Beta", "file02b", "file2a",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "alpha", "Beta", "file1", "File2", "file2a", "file02b", "file10"
            ]
        );
        assert_eq!(natural_cmp("Report", "report"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "ab"), Ordering::Less);
        assert_eq!(natural_cmp("v1.10", "v1.9"), Ordering::Greater);
    }
}
