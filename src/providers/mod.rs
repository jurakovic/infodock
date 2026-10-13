//! Providers read the system data behind the topics (SPEC §4.4, Appendix A). They run on the
//! sampler thread and reach the OS only through [`Platform`].

mod cpu;
mod disk;
mod memory;
mod net;
mod swap;
mod uptime;

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crate::platform::Platform;
use crate::sampler::{Sample, Topic};

/// The providers of all topics, with whatever state they keep between readings.
pub struct Providers {
    platform: Arc<dyn Platform>,
    /// Created on first use; `None` inside when the platform has no reader.
    cpu: Option<Option<cpu::Cpu>>,
    /// The disk reader's thread, started on first use; `None` inside if it could not start.
    disks: Option<Option<disk::Disks>>,
    net: net::Net,
}

impl Providers {
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform,
            cpu: None,
            disks: None,
            net: net::Net::default(),
        }
    }

    /// The system resumed from sleep: providers that read deltas start from a new baseline
    /// (the first CPU reading after resume is discarded, network rates start at 0; Appendix
    /// A).
    pub fn resume(&mut self) {
        if let Some(Some(cpu)) = &mut self.cpu {
            cpu.resume();
        }
        self.net.resume();
    }

    /// Nobody subscribes to `topic` any more: what is kept for it goes, so a later subscriber
    /// starts from a fresh baseline (the first CPU reading is discarded again).
    pub fn forget(&mut self, topic: &Topic) {
        match topic {
            Topic::Cpu => {
                if let Some(Some(cpu)) = &mut self.cpu {
                    cpu.resume();
                }
            }
            Topic::Disk(root) => {
                if let Some(Some(disks)) = &mut self.disks {
                    disks.forget(root);
                }
            }
            Topic::Net(id) => self.net.forget(id),
            Topic::Uptime | Topic::Memory | Topic::Swap => {}
        }
    }

    /// Reads `topic` now; `None` if it cannot be read.
    pub fn sample(&mut self, topic: &Topic) -> Option<Sample> {
        match topic {
            Topic::Uptime => Some(Sample::Uptime(uptime::sample(
                self.platform.uptime(),
                SystemTime::now(),
                Instant::now(),
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
            Topic::Disk(root) => {
                let platform = &self.platform;
                let disks = self
                    .disks
                    .get_or_insert_with(|| disk::Disks::new(Arc::clone(platform)));
                disks.as_mut()?.sample(root).map(Sample::Disk)
            }
            Topic::Net(id) => {
                let interface = self.platform.net_interface(id.as_deref());
                Some(Sample::Net(self.net.sample(id, interface, Instant::now())))
            }
        }
    }
}
