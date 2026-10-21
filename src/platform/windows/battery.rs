//! Batteries (SPEC §9.14, Appendix A `battery/<i>`): charge, state, times and names from
//! `starship-battery` (the battery class driver's IOCTLs), the power line from
//! `GetSystemPowerStatus`. A battery the crate cannot read (one that reports its capacity
//! only relatively) is taken from `GetSystemPowerStatus` alone.

use std::time::Duration;

use starship_battery::units::{ratio, time};
use starship_battery::{Battery, Manager, State};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::platform::{BatteryReading, BatteryState, PowerStatus};

/// `ACLineStatus`: on AC power.
const AC_ONLINE: u8 = 1;
/// `BatteryFlag` bits.
const CHARGING: u8 = 8;
const NO_SYSTEM_BATTERY: u8 = 128;
const UNKNOWN_STATUS: u8 = 255;
/// `BatteryLifePercent` and `BatteryLifeTime` when unknown.
const UNKNOWN_PERCENT: u8 = 255;
const UNKNOWN_TIME: u32 = u32::MAX;

/// The power line and the batteries now; Windows' message when it cannot say.
pub fn status() -> Result<PowerStatus, String> {
    let mut system = SYSTEM_POWER_STATUS::default();
    // SAFETY: `system` is writable.
    unsafe { GetSystemPowerStatus(&mut system) }.map_err(|e| e.message())?;
    let ac_online = system.ACLineStatus == AC_ONLINE;
    let has_battery = !matches!(system.BatteryFlag, NO_SYSTEM_BATTERY | UNKNOWN_STATUS)
        && system.BatteryLifePercent != UNKNOWN_PERCENT;
    Ok(PowerStatus {
        ac_online,
        batteries: with_system(
            read_batteries(),
            has_battery.then(|| from_system(&system, ac_online)),
        ),
    })
}

/// The batteries the class driver lists, each in its place, one that cannot be read with the
/// driver's message; none (logged) if they cannot be enumerated.
fn read_batteries() -> Vec<Result<BatteryReading, String>> {
    let list = Manager::new().and_then(|manager| manager.batteries());
    match list {
        Ok(batteries) => batteries
            .map(|b| b.map(|b| reading(&b)).map_err(|e| e.to_string()))
            .collect(),
        Err(e) => {
            tracing::warn!("the batteries cannot be listed: {e}");
            Vec::new()
        }
    }
}

fn reading(battery: &Battery) -> BatteryReading {
    let name = [battery.vendor(), battery.model()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let duration = |t: Option<starship_battery::units::Time>| {
        t.map(|t| t.get::<time::second>())
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(Duration::from_secs_f32)
    };
    BatteryReading {
        name,
        percent: battery
            .state_of_charge()
            .get::<ratio::percent>()
            .clamp(0.0, 100.0),
        state: match battery.state() {
            State::Charging => BatteryState::Charging,
            State::Discharging | State::Empty => BatteryState::Discharging,
            State::Full => BatteryState::Full,
            State::Paused => BatteryState::NotCharging,
            State::Unknown => BatteryState::Unknown,
        },
        time_to_empty: duration(battery.time_to_empty()),
        time_to_full: duration(battery.time_to_full()),
    }
}

/// `batteries` with Windows' own view (`system`, if it sees a battery) in the first place
/// when the driver gives none that can be read: none listed (one that reports its capacity
/// only relatively), or each failing.
fn with_system(
    mut batteries: Vec<Result<BatteryReading, String>>,
    system: Option<BatteryReading>,
) -> Vec<Result<BatteryReading, String>> {
    if let Some(system) = system
        && !batteries.iter().any(Result::is_ok)
    {
        match batteries.first_mut() {
            Some(first) => *first = Ok(system),
            None => batteries.push(Ok(system)),
        }
    }
    batteries
}

/// The system battery as `GetSystemPowerStatus` sees it: charge, charging, and Windows'
/// estimate of the time left.
fn from_system(system: &SYSTEM_POWER_STATUS, ac_online: bool) -> BatteryReading {
    let percent = f32::from(system.BatteryLifePercent.min(100));
    let state = if system.BatteryFlag & CHARGING != 0 {
        BatteryState::Charging
    } else if !ac_online {
        BatteryState::Discharging
    } else if percent >= 100.0 {
        BatteryState::Full
    } else {
        BatteryState::NotCharging
    };
    BatteryReading {
        name: String::new(),
        percent,
        state,
        time_to_empty: (state == BatteryState::Discharging
            && system.BatteryLifeTime != UNKNOWN_TIME)
            .then(|| Duration::from_secs(u64::from(system.BatteryLifeTime))),
        time_to_full: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`): a server without a battery,
    /// on AC power.
    #[test]
    #[ignore = "reads this machine's power status"]
    fn power_status_of_this_machine() {
        let status = status().expect("readable");
        assert!(status.ac_online, "{status:?}");
        for battery in status.batteries.iter().flatten() {
            assert!((0.0..=100.0).contains(&battery.percent), "{battery:?}");
        }
    }

    fn battery(name: &str) -> BatteryReading {
        BatteryReading {
            name: name.into(),
            percent: 50.0,
            state: BatteryState::Discharging,
            time_to_empty: None,
            time_to_full: None,
        }
    }

    #[test]
    fn windows_view_fills_in_only_when_no_battery_can_be_read() {
        let system = || Some(battery("system"));
        let failed = || Err::<BatteryReading, String>("The device is not ready.".into());
        // None listed, or none readable: Windows' view takes the first place.
        assert_eq!(with_system(Vec::new(), system()), [Ok(battery("system"))]);
        assert_eq!(
            with_system(vec![failed(), failed()], system()),
            [Ok(battery("system")), failed()]
        );
        // One readable: every battery keeps its place, the failing one too.
        assert_eq!(
            with_system(vec![failed(), Ok(battery("second"))], system()),
            [failed(), Ok(battery("second"))]
        );
        // Windows sees no battery either.
        assert_eq!(with_system(vec![failed()], None), [failed()]);
    }
}
