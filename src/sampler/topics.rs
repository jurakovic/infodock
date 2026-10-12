//! Topic ids and sample payloads (SPEC §4.4, Appendix A).

use std::sync::Arc;
use std::time::Duration;

/// System data a sensor can subscribe to. More topics arrive with their providers (M2, M3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    Uptime,
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
