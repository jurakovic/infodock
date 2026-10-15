//! `battery/<i>` (SPEC §9.14, Appendix A): one battery of the power status, by its place in the
//! system's list. A battery that is not there is a sample of its own (`present: false`), which
//! the sensor shows as *No Battery*.

use crate::platform::{BatteryState, PowerStatus};
use crate::sampler::BatterySample;

pub fn sample(status: &PowerStatus, index: u32) -> BatterySample {
    let battery = usize::try_from(index)
        .ok()
        .and_then(|i| status.batteries.get(i));
    match battery {
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
    }
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
            batteries: vec![BatteryReading {
                name: "SMP 5B10".into(),
                percent: 73.0,
                state: BatteryState::Discharging,
                time_to_empty: Some(Duration::from_secs(9660)),
                time_to_full: None,
            }],
        };
        let first = sample(&status, 0);
        assert!(first.present && !first.ac_online);
        assert_eq!((first.name.as_str(), first.percent), ("SMP 5B10", 73.0));
        assert_eq!(first.time_to_empty, Some(Duration::from_secs(9660)));
        let second = sample(&status, 1);
        assert!(!second.present);
        assert_eq!(second.state, BatteryState::Unknown);
    }
}
