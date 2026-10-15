//! Power (SPEC §9.21, §4.6): the battery capability, the power modes this computer offers, and
//! the power actions. Actions run on a short-lived thread: sleep returns only after the computer
//! wakes, and the bar thread must keep answering the broadcasts that come before it sleeps.

use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LUID, WPARAM};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Power::{
    GetPwrCapabilities, GetSystemPowerStatus, SYSTEM_POWER_CAPABILITIES, SYSTEM_POWER_STATUS,
    SetSuspendState,
};
use windows::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSDisconnectSession,
};
use windows::Win32::System::Shutdown::{
    EWX_FORCE, EWX_LOGOFF, ExitWindowsEx, InitiateShutdownW, LockWorkStation,
    SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER, SHTDN_REASON_MINOR_OTHER,
    SHUTDOWN_FORCE_OTHERS, SHUTDOWN_POWEROFF, SHUTDOWN_RESTART,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, PostMessageW, SC_MONITORPOWER, WM_SYSCOMMAND,
};
use windows::core::{Error, PCWSTR, Result};

use crate::platform::PowerAction;

/// `BatteryFlag` values that mean there is no battery to show.
const NO_SYSTEM_BATTERY: u8 = 128;
const UNKNOWN: u8 = 255;
/// `SC_MONITORPOWER`: the displays off.
const MONITOR_OFF: isize = 2;

pub fn has_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is writable.
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok()
        && !matches!(status.BatteryFlag, NO_SYSTEM_BATTERY | UNKNOWN)
}

fn power_capabilities() -> Option<SYSTEM_POWER_CAPABILITIES> {
    let mut caps = SYSTEM_POWER_CAPABILITIES::default();
    // SAFETY: `caps` is writable.
    unsafe { GetPwrCapabilities(&mut caps) }.then_some(caps)
}

/// The power modes this computer offers, in menu order: sleep where it can sleep (classic
/// sleep or Modern Standby), hibernate only with a hibernation file.
pub fn modes() -> Vec<String> {
    let caps = power_capabilities();
    let sleep = caps.is_some_and(|c| c.SystemS1 || c.SystemS2 || c.SystemS3 || c.AoAc);
    let hibernate = caps.is_some_and(|c| c.SystemS4 && c.HiberFilePresent);
    PowerAction::ALL
        .into_iter()
        .filter(|a| match a {
            PowerAction::Sleep => sleep,
            PowerAction::Hibernate => hibernate,
            _ => true,
        })
        .map(|a| a.id().to_owned())
        .collect()
}

/// Starts `action` on its own thread.
pub fn power(action: PowerAction, force: bool) {
    let spawned = std::thread::Builder::new()
        .name("power".into())
        .spawn(move || {
            if let Err(e) = act(action, force) {
                tracing::warn!("{action:?} failed: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("could not start {action:?}: {e}");
    }
}

fn act(action: PowerAction, force: bool) -> Result<()> {
    let reason = SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED;
    // SAFETY: plain calls without pointers, unless noted.
    unsafe {
        match action {
            PowerAction::Lock => LockWorkStation(),
            PowerAction::SignOut => {
                let flags = if force {
                    EWX_LOGOFF | EWX_FORCE
                } else {
                    EWX_LOGOFF
                };
                ExitWindowsEx(flags, reason)
            }
            // The sign-in screen; the session stays.
            PowerAction::SwitchUser => {
                WTSDisconnectSession(Some(WTS_CURRENT_SERVER_HANDLE), WTS_CURRENT_SESSION, false)
            }
            PowerAction::Sleep => {
                let classic =
                    power_capabilities().is_some_and(|c| c.SystemS1 || c.SystemS2 || c.SystemS3);
                if classic {
                    suspend(false)
                } else {
                    // Modern Standby has no sleep call: with the displays off, the computer
                    // goes to standby.
                    PostMessageW(
                        Some(HWND_BROADCAST),
                        WM_SYSCOMMAND,
                        WPARAM(SC_MONITORPOWER as usize),
                        LPARAM(MONITOR_OFF),
                    )
                }
            }
            PowerAction::Hibernate => suspend(true),
            PowerAction::ShutDown | PowerAction::Restart => {
                enable_shutdown_privilege()?;
                let mut flags = if action == PowerAction::Restart {
                    SHUTDOWN_RESTART
                } else {
                    SHUTDOWN_POWEROFF
                };
                if force {
                    flags |= SHUTDOWN_FORCE_OTHERS;
                }
                let status = InitiateShutdownW(PCWSTR::null(), PCWSTR::null(), 0, flags, reason);
                windows::Win32::Foundation::WIN32_ERROR(status).ok()
            }
        }
    }
}

/// Sleeps or hibernates; returns once the computer has woken up. `SetSuspendState` needs
/// `SeShutdownPrivilege` enabled too.
fn suspend(hibernate: bool) -> Result<()> {
    if let Err(e) = enable_shutdown_privilege() {
        tracing::info!("the shutdown privilege could not be enabled: {e}");
    }
    // SAFETY: plain call.
    if unsafe { SetSuspendState(hibernate, false, false) } {
        Ok(())
    } else {
        Err(Error::from_thread())
    }
}

/// Shutting down, restarting, sleeping and hibernating need `SeShutdownPrivilege` enabled in
/// the process token.
fn enable_shutdown_privilege() -> Result<()> {
    let mut token = HANDLE::default();
    // SAFETY: the token handle is ours and closed below; the structures are initialised.
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )?;
        let mut luid = LUID::default();
        let result =
            LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut luid).and_then(|()| {
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };
                AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None)
            });
        let _ = CloseHandle(token);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`).
    #[test]
    #[ignore = "reads this machine's power capabilities"]
    fn modes_always_include_lock_and_shut_down() {
        let modes = modes();
        for always in ["lock", "signOut", "switchUser", "shutDown", "restart"] {
            assert!(modes.iter().any(|m| m == always), "{modes:?}");
        }
        let order: Vec<_> = PowerAction::ALL
            .into_iter()
            .map(|a| a.id().to_owned())
            .filter(|id| modes.contains(id))
            .collect();
        assert_eq!(order, modes, "in menu order");
    }
}
