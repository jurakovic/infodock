//! Topic ids and sample payloads (SPEC §4.4, Appendix A).

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::platform::{BatteryState, PathKind, RecycleBin, Volume, WifiStatus};

/// System data a sensor can subscribe to. More topics arrive with their providers (M2, M3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    Uptime,
    Cpu,
    Memory,
    Swap,
    /// `disk/<X:\>`: one volume, by its root.
    Disk(String),
    /// `net/<id|auto>`: one network interface by id, or `None` for the one carrying the
    /// default route.
    Net(Option<String>),
    /// `path/<path>`: whether a file or folder exists, e.g. a Shortcut's target (SPEC §9.17).
    Path(String),
    /// `volumes`: the drives; also pushed when one arrives or goes (SPEC §4.4, Appendix A).
    Volumes,
    /// `recycle-bin`: what the Recycle Bin holds on all drives (SPEC §9.16).
    RecycleBin,
    /// `battery/<i>`: one battery by its place in the driver's list (SPEC §9.14).
    Battery(u32),
    /// `batteries`: the batteries' names, for Settings' battery lists (SPEC §8.2).
    Batteries,
    /// `wifi`: the Wi-Fi connection (SPEC §9.15).
    Wifi,
}

/// Something that changed before the topics it concerns are due again: an OS notification or
/// an action of a sensor. The sampler reads those topics at once (SPEC §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// A drive arrived or went, or a medium was inserted or removed.
    Devices,
    /// Items went into the Recycle Bin, or it was emptied.
    RecycleBin,
    /// The power line or a battery's charge changed (`PBT_APMPOWERSTATUSCHANGE`).
    Power,
    /// Wi-Fi connected, disconnected, roamed, or its radio was switched.
    Wifi,
    /// A slow reading came after the sampler stopped waiting for it (SPEC §4.4): the topics it
    /// belongs to are read again at once and take it.
    Answered(Slow),
}

/// The readings done on worker threads, because they can stall (SPEC §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slow {
    Disks,
    Paths,
    Volumes,
    RecycleBin,
    Batteries,
}

impl Topic {
    /// Whether `change` may change this topic, so it is read again at once: a drive arriving
    /// or going changes the drive list, the disks and the paths on them.
    pub fn follows(&self, change: Change) -> bool {
        match change {
            Change::Devices => matches!(self, Self::Volumes | Self::Disk(_) | Self::Path(_)),
            Change::RecycleBin => *self == Self::RecycleBin,
            Change::Power => matches!(self, Self::Battery(_) | Self::Batteries),
            Change::Wifi => *self == Self::Wifi,
            Change::Answered(slow) => match slow {
                Slow::Disks => matches!(self, Self::Disk(_)),
                Slow::Paths => matches!(self, Self::Path(_)),
                Slow::Volumes => *self == Self::Volumes,
                Slow::RecycleBin => *self == Self::RecycleBin,
                Slow::Batteries => matches!(self, Self::Battery(_) | Self::Batteries),
            },
        }
    }
}

/// The drive list is read this often besides when drives arrive or go (SPEC §4.4).
pub const VOLUMES_EVERY: Duration = Duration::from_secs(60);
/// Batteries are read this often besides when the power status changes (SPEC §9.14).
pub const BATTERY_EVERY: Duration = Duration::from_secs(10);

/// A sensor's wish to receive `topic` at least every `interval`.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicRequest {
    pub topic: Topic,
    pub interval: Duration,
}

/// One reading of a topic.
#[derive(Debug, Clone, PartialEq)]
pub enum Sample {
    Uptime(UptimeSample),
    Cpu(CpuSample),
    Memory(MemorySample),
    Swap(SwapSample),
    Disk(DiskSample),
    Net(NetSample),
    Path(PathSample),
    /// The drives with letters, in letter order.
    Volumes(Vec<Volume>),
    RecycleBin(RecycleBin),
    Battery(BatterySample),
    /// The batteries' names in their places; empty for one without a name or that cannot be
    /// read.
    Batteries(Vec<String>),
    Wifi(WifiStatus),
    /// `{ error }`: the topic failed to read a few times in a row; the platform's message
    /// (SPEC §14.2). Sensors draw inactive and show it in their hint until a reading succeeds.
    Error(String),
}

/// `battery/<i>`: one battery and the power line (SPEC §9.14).
#[derive(Debug, Clone, PartialEq)]
pub struct BatterySample {
    /// The battery exists; otherwise only `ac_online` means something.
    pub present: bool,
    /// Its vendor and model; empty when unknown.
    pub name: String,
    /// Its charge, 0–100.
    pub percent: f32,
    /// The computer runs on AC power.
    pub ac_online: bool,
    pub state: BatteryState,
    /// The battery's own estimates, while discharging or charging.
    pub time_to_empty: Option<Duration>,
    pub time_to_full: Option<Duration>,
}

/// `path/<path>`: what the path names now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathSample {
    pub kind: PathKind,
}

/// `cpu`: the processors' load, each 0–100 (SPEC Appendix A).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CpuSample {
    /// The busy share, as Task Manager shows it.
    pub total: f32,
    /// User, kernel (system, without idle) and idle time shares.
    pub user: f32,
    pub kernel: f32,
    pub idle: f32,
    /// The busy share per logical processor, in processor order; NaN for one that could not
    /// be read; empty where unavailable.
    pub cores: Vec<f32>,
    pub logical: u32,
}

/// `memory`: physical memory, bytes (SPEC Appendix A).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MemorySample {
    pub total: u64,
    pub available: u64,
    /// `total − available`.
    pub used: u64,
    /// The share in use as the OS reports it, 0–100.
    pub load: f32,
}

/// `swap`: all page files together, bytes (SPEC Appendix A); a `total` of 0 means there is
/// no page file.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SwapSample {
    pub total: u64,
    pub used: u64,
    /// `used` of `total`, 0–100.
    pub load: f32,
}

/// `disk/<X:\>`: a volume's size and free space, bytes (SPEC Appendix A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiskSample {
    /// The drive exists and has a medium; otherwise the sizes are 0.
    pub present: bool,
    pub total: u64,
    pub free: u64,
}

/// `net/<id|auto>`: a network interface's traffic (SPEC Appendix A).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetSample {
    /// The interface exists (for Automatic: something carries the default route); otherwise
    /// the rest is empty.
    pub present: bool,
    pub id: String,
    pub alias: String,
    pub description: String,
    pub up: bool,
    /// Bytes per second since the previous reading.
    pub rx_rate: f64,
    pub tx_rate: f64,
    /// Bytes since the interface started.
    pub rx_total: u64,
    pub tx_total: u64,
    /// Link speeds, bits per second.
    pub link_rx: u64,
    pub link_tx: u64,
}

/// `uptime`: when the system started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UptimeSample {
    pub boot_unix_ms: i64,
    pub uptime_secs: u64,
    /// When it was read: a subscriber may get it later, as a catch-up (SPEC §4.4).
    pub read_at: Instant,
}

/// A sample on its way to the bar thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub topic: Topic,
    pub sample: Arc<Sample>,
    /// Only this subscriber, which has just subscribed and gets the newest sample; `None` for
    /// every subscriber of the topic.
    pub to: Option<String>,
    /// Read at once because something changed (a [`Change`]), out of the topic's cadence:
    /// news for every subscriber, however recently it took a sample.
    pub pushed: bool,
}
