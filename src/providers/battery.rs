//! `battery/<i>` (SPEC §9.14, Appendix A): one battery of the power status, by its place in the
//! driver's list. A battery that is not there is a sample of its own (`present: false`), which
//! the sensor shows as *No Battery*; one that cannot be read is a failed reading (§14.2), and
//! the batteries after it keep their places. `batteries`: their names, for Settings.

use crate::platform::{BatteryState, PowerStatus};
use crate::sampler::BatterySample;

/// Battery `index`; the driver's message if it cannot be read.
pub fn sample(status: &PowerStatus, index: u32) -> Result<BatterySample, String> {
    let battery = usize::try_from(index)
        .ok()
        .and_then(|i| status.batteries.get(i))
        .map(|b| b.as_ref().map_err(Clone::clone))
        .transpose()?;
    Ok(match battery {
        Some(b) => BatterySample {
            present: true,
            name: b.name.clone(),
            percent: b.percent,
            ac_online: status.ac_online,
            state: b.state,
            time_to_empty: b.time_to_empty,
            time_to_full: b.time_to_full,
        },
        None => BatterySample {
            present: false,
            name: String::new(),
            percent: 0.0,
            ac_online: status.ac_online,
            state: BatteryState::Unknown,
            time_to_empty: None,
            time_to_full: None,
        },
    })
}

/// The batteries' names in their places, for Settings' battery lists (SPEC §8.2); empty for
/// one without a name or that cannot be read.
pub fn names(status: &PowerStatus) -> Vec<String> {
    status
        .batteries
        .iter()
        .map(|b| b.as_ref().map(|b| b.name.clone()).unwrap_or_default())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::platform::BatteryReading;

    #[test]
    fn picks_the_battery_by_its_place() {
        let status = PowerStatus {
            ac_online: false,
            batteries: vec![Ok(BatteryReading {
                name: "SMP 5B10".into(),
                percent: 73.0,
                state: BatteryState::Discharging,
                time_to_empty: Some(Duration::from_secs(9660)),
                time_to_full: None,
            })],
        };
        let first = sample(&status, 0).expect("readable");
        assert!(first.present && !first.ac_online);
        assert_eq!((first.name.as_str(), first.percent), ("SMP 5B10", 73.0));
        assert_eq!(first.time_to_empty, Some(Duration::from_secs(9660)));
        let second = sample(&status, 1).expect("readable");
        assert!(!second.present);
        assert_eq!(second.state, BatteryState::Unknown);
    }

    #[test]
    fn a_battery_that_cannot_be_read_keeps_its_place() {
        let status = PowerStatus {
            ac_online: true,
            batteries: vec![
                Err("The device is not ready.".into()),
                Ok(BatteryReading {
                    name: "Second".into(),
                    percent: 40.0,
                    state: BatteryState::Charging,
                    time_to_empty: None,
                    time_to_full: None,
                }),
            ],
        };
        assert_eq!(sample(&status, 0), Err("The device is not ready.".into()));
        let second = sample(&status, 1).expect("readable");
        assert_eq!((second.name.as_str(), second.percent), ("Second", 40.0));
        assert_eq!(names(&status), ["", "Second"]);
    }
}
