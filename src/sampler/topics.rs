//! Topic ids and sample payloads (SPEC §4.4, Appendix A).

use std::sync::Arc;
use std::time::Duration;

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
}

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
    /// The busy share per logical processor; empty where unavailable.
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
}

/// A sample on its way to the bar thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub topic: Topic,
    pub sample: Arc<Sample>,
    /// Only this subscriber, which has just subscribed and gets the newest sample; `None` for
    /// every subscriber of the topic.
    pub to: Option<String>,
}
