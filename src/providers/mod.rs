//! Providers read the system data behind the topics (SPEC §4.4, Appendix A). They run on the
//! sampler thread and reach the OS only through [`Platform`].

mod uptime;

use std::sync::Arc;
use std::time::SystemTime;

use crate::platform::Platform;
use crate::sampler::{Sample, Topic};

/// The providers of all topics, with whatever state they keep between readings.
pub struct Providers {
    platform: Arc<dyn Platform>,
}

impl Providers {
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self { platform }
    }

    /// Reads `topic` now; `None` if it cannot be read.
    pub fn sample(&mut self, topic: &Topic) -> Option<Sample> {
        match topic {
            Topic::Uptime => Some(Sample::Uptime(uptime::sample(
                self.platform.uptime(),
                SystemTime::now(),
            ))),
        }
    }
}
