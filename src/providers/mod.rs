//! Providers read the system data behind the topics (SPEC §4.4, Appendix A). They run on the
//! sampler thread and reach the OS only through [`Platform`].

mod cpu;
mod memory;
mod swap;
mod uptime;

use std::sync::Arc;
use std::time::SystemTime;

use crate::platform::Platform;
use crate::sampler::{Sample, Topic};

/// The providers of all topics, with whatever state they keep between readings.
pub struct Providers {
    platform: Arc<dyn Platform>,
    /// Created on first use; `None` inside when the platform has no reader.
    cpu: Option<Option<cpu::Cpu>>,
}

impl Providers {
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform,
            cpu: None,
        }
    }

    /// The system resumed from sleep: providers that read deltas start from a new baseline
    /// (the first CPU reading after resume is discarded, Appendix A).
    pub fn resume(&mut self) {
        if let Some(Some(cpu)) = &mut self.cpu {
            cpu.resume();
        }
    }

    /// Reads `topic` now; `None` if it cannot be read.
    pub fn sample(&mut self, topic: &Topic) -> Option<Sample> {
        match topic {
            Topic::Uptime => Some(Sample::Uptime(uptime::sample(
                self.platform.uptime(),
                SystemTime::now(),
            ))),
            Topic::Cpu => {
                let platform = &self.platform;
                let cpu = self
                    .cpu
                    .get_or_insert_with(|| platform.cpu_reader().map(cpu::Cpu::new));
                cpu.as_mut()?.sample().map(Sample::Cpu)
            }
            Topic::Memory => self
                .platform
                .memory()
                .map(|status| Sample::Memory(memory::sample(status))),
            Topic::Swap => self
                .platform
                .page_files()
                .map(|files| Sample::Swap(swap::sample(&files))),
        }
    }
}
