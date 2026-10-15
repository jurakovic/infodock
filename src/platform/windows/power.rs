//! Power: whether the computer has a battery (the `battery` capability, SPEC §4.6).

use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// `BatteryFlag` values that mean there is no battery to show.
const NO_SYSTEM_BATTERY: u8 = 128;
const UNKNOWN: u8 = 255;

pub fn has_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is writable.
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok()
        && !matches!(status.BatteryFlag, NO_SYSTEM_BATTERY | UNKNOWN)
}
