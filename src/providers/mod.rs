//! Providers read the system data behind the topics (SPEC §4.4, Appendix A). They run on the
//! sampler thread and reach the OS only through [`Platform`].

mod battery;
mod cpu;
mod disk;
mod memory;
mod net;
mod swap;
mod uptime;
mod worker;

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crate::platform::{Platform, PowerStatus, RecycleBin, Volume, WifiReader};
use crate::sampler::{Change, ChangeSink, DiskSample, PathSample, Sample, Slow, Topic};
use worker::SlowReader;

pub use battery::names as battery_names;

/// The providers of all topics, with whatever state they keep between readings.
pub struct Providers {
    platform: Arc<dyn Platform>,
    /// Created on first use; `None` inside when the platform has no reader.
    cpu: Option<Option<cpu::Cpu>>,
    /// The disks, read on workers (one per drive), created on first use.
    disks: Option<SlowReader<DiskSample>>,
    /// The same for paths (`path/<path>`).
    paths: Option<SlowReader<PathSample>>,
    /// The same for the drive list (one key, `""`).
    volumes: Option<SlowReader<Vec<Volume>>>,
    /// The same for the Recycle Bin (one key, `""`), which can take a while to count.
    recycle_bin: Option<SlowReader<Result<RecycleBin, String>>>,
    /// The same for the batteries (one key, `""`), whose firmware can be slow to answer.
    power: Option<SlowReader<Result<PowerStatus, String>>>,
    /// Created on first use and dropped when nobody reads Wi-Fi; `None` inside where there is
    /// no WLAN.
    wifi: Option<Option<Box<dyn WifiReader>>>,
    /// Where OS notifications that change topics go (Wi-Fi's), so they are read at once.
    changes: ChangeSink,
    net: net::Net,
}

impl Providers {
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform,
            cpu: None,
            disks: None,
            paths: None,
            volumes: None,
            recycle_bin: None,
            power: None,
            wifi: None,
            changes: ChangeSink::NONE,
            net: net::Net::default(),
        }
    }

    /// Reports changes the OS notifies (Wi-Fi connecting, disconnecting) to `changes`.
    pub fn with_changes(mut self, changes: ChangeSink) -> Self {
        self.changes = changes;
        self
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
                if let Some(disks) = &mut self.disks {
                    disks.forget(root);
                }
            }
            Topic::Path(path) => {
                if let Some(paths) = &mut self.paths {
                    paths.forget(path);
                }
            }
            Topic::Volumes => {
                if let Some(volumes) = &mut self.volumes {
                    volumes.forget("");
                }
            }
            Topic::RecycleBin => {
                if let Some(bin) = &mut self.recycle_bin {
                    bin.forget("");
                }
            }
            Topic::Battery(_) | Topic::Batteries => {
                if let Some(power) = &mut self.power {
                    power.forget("");
                }
            }
            // Closes the WLAN client and its notifications.
            Topic::Wifi => self.wifi = None,
            Topic::Net(id) => self.net.forget(id),
            Topic::Uptime | Topic::Memory | Topic::Swap => {}
        }
    }

    /// Reads `topic` now: `None` while there is nothing to show yet (a first CPU reading, which
    /// only sets the baseline; a slow reading not answered yet), the platform's message when it
    /// cannot be read (SPEC §14.2).
    pub fn sample(&mut self, topic: &Topic) -> Result<Option<Sample>, String> {
        Ok(Some(match topic {
            Topic::Uptime => Sample::Uptime(uptime::sample(
                self.platform.uptime(),
                SystemTime::now(),
                Instant::now(),
            )),
            Topic::Cpu => {
                let platform = &self.platform;
                let cpu = self
                    .cpu
                    .get_or_insert_with(|| platform.cpu_reader().map(cpu::Cpu::new));
                let Some(cpu) = cpu.as_mut() else {
                    return Ok(None);
                };
                return Ok(cpu.sample()?.map(Sample::Cpu));
            }
            Topic::Memory => Sample::Memory(memory::sample(self.platform.memory()?)),
            Topic::Swap => Sample::Swap(swap::sample(&self.platform.page_files()?)),
            Topic::Disk(root) => {
                let disks = self.disks.get_or_insert_with(|| {
                    let platform = Arc::clone(&self.platform);
                    let changes = self.changes.clone();
                    SlowReader::new(
                        "disks",
                        move |root| disk::sample(platform.disk_space(root)),
                        move || changes.report(Change::Answered(Slow::Disks)),
                    )
                });
                return Ok(disks.sample(root).map(Sample::Disk));
            }
            Topic::Path(path) => {
                let paths = self.paths.get_or_insert_with(|| {
                    let platform = Arc::clone(&self.platform);
                    let changes = self.changes.clone();
                    SlowReader::new(
                        "paths",
                        move |path| PathSample {
                            kind: platform.path_kind(path),
                        },
                        move || changes.report(Change::Answered(Slow::Paths)),
                    )
                });
                return Ok(paths.sample(path).map(Sample::Path));
            }
            // Listing may stall on a drive that does not answer.
            Topic::Volumes => {
                let volumes = self.volumes.get_or_insert_with(|| {
                    let platform = Arc::clone(&self.platform);
                    let changes = self.changes.clone();
                    SlowReader::new(
                        "volumes",
                        move |_| platform.volumes(),
                        move || changes.report(Change::Answered(Slow::Volumes)),
                    )
                });
                return Ok(volumes.sample("").map(Sample::Volumes));
            }
            Topic::RecycleBin => {
                let bin = self.recycle_bin.get_or_insert_with(|| {
                    let platform = Arc::clone(&self.platform);
                    let changes = self.changes.clone();
                    SlowReader::new(
                        "recycle-bin",
                        move |_| platform.recycle_bin(),
                        move || changes.report(Change::Answered(Slow::RecycleBin)),
                    )
                });
                return Ok(bin.sample("").transpose()?.map(Sample::RecycleBin));
            }
            Topic::Battery(_) | Topic::Batteries => {
                let power = self.power.get_or_insert_with(|| {
                    let platform = Arc::clone(&self.platform);
                    let changes = self.changes.clone();
                    SlowReader::new(
                        "batteries",
                        move |_| platform.power_status(),
                        move || changes.report(Change::Answered(Slow::Batteries)),
                    )
                });
                let Some(status) = power.sample("").transpose()? else {
                    return Ok(None);
                };
                match topic {
                    Topic::Battery(index) => Sample::Battery(battery::sample(&status, *index)?),
                    _ => Sample::Batteries(battery::names(&status)),
                }
            }
            Topic::Wifi => {
                let (platform, changes) = (&self.platform, &self.changes);
                let wifi = self.wifi.get_or_insert_with(|| {
                    let changes = changes.clone();
                    platform.wifi_reader(Box::new(move || changes.report(Change::Wifi)))
                });
                let Some(wifi) = wifi.as_mut() else {
                    return Ok(None);
                };
                Sample::Wifi(wifi.read()?)
            }
            Topic::Net(id) => {
                let interface = self.platform.net_interface(id.as_deref());
                Sample::Net(self.net.sample(id, interface, Instant::now()))
            }
        }))
    }
}
