//! Wi-Fi through the WLAN API (SPEC §9.15, Appendix A). `wlanapi.dll` is loaded only when it is
//! used: it is missing on Windows editions without the wireless LAN feature, and a static import
//! would keep Infodock from starting there. Only its types come from the `windows` crate; its
//! functions are looked up in the loaded library.

use std::ffi::c_void;

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, FreeLibrary, HANDLE, HMODULE, WIN32_ERROR};
use windows::Win32::NetworkManagement::WiFi::{
    DOT11_PHY_TYPE, DOT11_SSID, L2_NOTIFICATION_DATA, WLAN_BSS_LIST, WLAN_CONNECTION_ATTRIBUTES,
    WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST, WLAN_NOTIFICATION_SOURCE_ACM,
    WLAN_NOTIFICATION_SOURCE_MSM, WLAN_NOTIFICATION_SOURCE_NONE, WLAN_RADIO_STATE,
    dot11_BSS_type_infrastructure, dot11_radio_state_on, wlan_interface_state_connected,
    wlan_intf_opcode_current_connection, wlan_intf_opcode_radio_state, wlan_intf_opcode_rssi,
    wlan_notification_acm_connection_complete, wlan_notification_acm_disconnected,
    wlan_notification_acm_interface_arrival, wlan_notification_acm_interface_removal,
    wlan_notification_msm_connected, wlan_notification_msm_disconnected,
    wlan_notification_msm_radio_state_change, wlan_notification_msm_roaming_end,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{GUID, PCSTR, s, w};

use crate::platform::{WifiReader, WifiStatus};

type OpenHandle = unsafe extern "system" fn(u32, *const c_void, *mut u32, *mut HANDLE) -> u32;
type EnumInterfaces = unsafe extern "system" fn(HANDLE, *const c_void, *mut *mut u32) -> u32;
type FreeMemory = unsafe extern "system" fn(*const c_void);
type CloseHandle = unsafe extern "system" fn(HANDLE, *const c_void) -> u32;
type QueryInterface = unsafe extern "system" fn(
    HANDLE,
    *const GUID,
    i32,
    *const c_void,
    *mut u32,
    *mut *mut c_void,
    *mut i32,
) -> u32;
type GetNetworkBssList = unsafe extern "system" fn(
    HANDLE,
    *const GUID,
    *const DOT11_SSID,
    i32,
    i32,
    *const c_void,
    *mut *mut WLAN_BSS_LIST,
) -> u32;
type Callback = unsafe extern "system" fn(*mut L2_NOTIFICATION_DATA, *mut c_void);
type RegisterNotification = unsafe extern "system" fn(
    HANDLE,
    u32,
    i32,
    Option<Callback>,
    *const c_void,
    *const c_void,
    *mut u32,
) -> u32;

/// The client version for Windows Vista and later.
const CLIENT_VERSION: u32 = 2;

/// `wlanapi.dll` while loaded.
struct Library(HMODULE);

impl Library {
    fn load() -> Option<Self> {
        // SAFETY: a system DLL from System32 only.
        unsafe { LoadLibraryExW(w!("wlanapi.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .ok()
            .map(Self)
    }

    /// The function `name` as `T`, which must be its signature.
    unsafe fn get<T>(&self, name: PCSTR) -> Option<T> {
        // SAFETY: the module is loaded; the caller names the function's real type.
        unsafe {
            let f = GetProcAddress(self.0, name)?;
            Some(std::mem::transmute_copy(&f))
        }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: the module loaded in `load`.
        let _ = unsafe { FreeLibrary(self.0) };
    }
}

/// Whether the WLAN service runs and knows at least one wireless interface (the `wifi`
/// capability, SPEC §4.6).
pub fn has_interface() -> bool {
    let Some(lib) = Library::load() else {
        return false;
    };
    // SAFETY: the signatures of wlanapi.h; the handle and list are released below.
    unsafe {
        let (Some(open), Some(enumerate), Some(free), Some(close)) = (
            lib.get::<OpenHandle>(s!("WlanOpenHandle")),
            lib.get::<EnumInterfaces>(s!("WlanEnumInterfaces")),
            lib.get::<FreeMemory>(s!("WlanFreeMemory")),
            lib.get::<CloseHandle>(s!("WlanCloseHandle")),
        ) else {
            return false;
        };
        let (mut version, mut handle) = (0u32, HANDLE::default());
        if open(CLIENT_VERSION, std::ptr::null(), &mut version, &mut handle) != 0 {
            // Typically: the WLAN AutoConfig service does not run (no wireless hardware).
            return false;
        }
        let mut list: *mut u32 = std::ptr::null_mut();
        // A WLAN_INTERFACE_INFO_LIST starts with its number of items.
        let any =
            enumerate(handle, std::ptr::null(), &mut list) == 0 && !list.is_null() && *list > 0;
        if !list.is_null() {
            free(list.cast());
        }
        close(handle, std::ptr::null());
        any
    }
}

/// What a notification callback calls; boxed twice so its address is thin and stable.
type Changed = Box<dyn Fn() + Send + Sync>;

/// An open WLAN client with its notifications registered.
pub struct WinWifi {
    handle: HANDLE,
    enumerate: EnumInterfaces,
    query: QueryInterface,
    bss_list: GetNetworkBssList,
    free: FreeMemory,
    close: CloseHandle,
    register: RegisterNotification,
    /// The callback's context, freed after the handle is closed.
    changed: *mut Changed,
    /// Kept loaded while the functions above are used; dropped last.
    _lib: Library,
}

// SAFETY: the WLAN handle and functions may be used from any thread; the context is only read,
// by the notification callback, and freed once the handle (and with it the callbacks) is gone.
unsafe impl Send for WinWifi {}

impl WinWifi {
    /// Opens a client that calls `changed` when Wi-Fi connects, disconnects, roams or its radio
    /// is switched; `None` (logged) where there is no WLAN service.
    pub fn open(changed: Changed) -> Option<Self> {
        let lib = Library::load()?;
        // SAFETY: the signatures of wlanapi.h.
        let (open, enumerate, query, bss_list, free, close, register) = unsafe {
            (
                lib.get::<OpenHandle>(s!("WlanOpenHandle"))?,
                lib.get::<EnumInterfaces>(s!("WlanEnumInterfaces"))?,
                lib.get::<QueryInterface>(s!("WlanQueryInterface"))?,
                lib.get::<GetNetworkBssList>(s!("WlanGetNetworkBssList"))?,
                lib.get::<FreeMemory>(s!("WlanFreeMemory"))?,
                lib.get::<CloseHandle>(s!("WlanCloseHandle"))?,
                lib.get::<RegisterNotification>(s!("WlanRegisterNotification"))?,
            )
        };
        let (mut version, mut handle) = (0u32, HANDLE::default());
        // SAFETY: out parameters for the version and handle.
        let status = unsafe { open(CLIENT_VERSION, std::ptr::null(), &mut version, &mut handle) };
        if status != 0 {
            tracing::warn!("the WLAN service cannot be opened: error {status}");
            return None;
        }
        let changed = Box::into_raw(Box::new(changed));
        let sources = WLAN_NOTIFICATION_SOURCE_ACM.0 | WLAN_NOTIFICATION_SOURCE_MSM.0;
        // SAFETY: the context lives until after the handle is closed (Drop).
        let status = unsafe {
            register(
                handle,
                sources,
                1,
                Some(on_notification),
                changed as *const c_void,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            // Still readable, only not pushed.
            tracing::warn!("WLAN notifications are not available: error {status}");
        }
        Some(Self {
            handle,
            enumerate,
            query,
            bss_list,
            free,
            close,
            register,
            changed,
            _lib: lib,
        })
    }

    /// Queries `opcode` of `interface`; the data (freed by the caller with `free`) or the
    /// error code.
    unsafe fn query(&self, interface: &GUID, opcode: i32) -> Result<*mut c_void, u32> {
        let (mut size, mut data, mut kind) = (0u32, std::ptr::null_mut(), 0i32);
        // SAFETY: out parameters; the caller frees the data.
        let status = unsafe {
            (self.query)(
                self.handle,
                interface,
                opcode,
                std::ptr::null(),
                &mut size,
                &mut data,
                &mut kind,
            )
        };
        if status != 0 || data.is_null() {
            return Err(status);
        }
        Ok(data)
    }

    /// The interfaces, connected ones first.
    fn interfaces(&self) -> Result<Vec<WLAN_INTERFACE_INFO>, String> {
        let mut list: *mut u32 = std::ptr::null_mut();
        // SAFETY: the list is freed below; its items are copied out first.
        unsafe {
            let code = (self.enumerate)(self.handle, std::ptr::null(), &mut list);
            if code != 0 {
                return Err(message(code));
            }
            if list.is_null() {
                return Ok(Vec::new());
            }
            let info = list.cast::<WLAN_INTERFACE_INFO_LIST>();
            let count = (*info).dwNumberOfItems as usize;
            let first = (&raw const (*info).InterfaceInfo).cast::<WLAN_INTERFACE_INFO>();
            let mut items: Vec<WLAN_INTERFACE_INFO> = (0..count).map(|i| *first.add(i)).collect();
            (self.free)(list.cast());
            items.sort_by_key(|i| i.isState != wlan_interface_state_connected);
            Ok(items)
        }
    }

    /// Whether the radio of `interface` is on: some PHY is on in software and hardware.
    fn radio_on(&self, interface: &GUID) -> bool {
        // SAFETY: the data is a WLAN_RADIO_STATE, freed after reading.
        unsafe {
            let Ok(data) = self.query(interface, wlan_intf_opcode_radio_state.0) else {
                // Unknown: rather on than a false "off".
                return true;
            };
            let state = &*data.cast::<WLAN_RADIO_STATE>();
            let phys = &state.PhyRadioState[..(state.dwNumberOfPhys as usize).min(64)];
            let on = phys.is_empty()
                || phys.iter().any(|p| {
                    p.dot11SoftwareRadioState == dot11_radio_state_on
                        && p.dot11HardwareRadioState == dot11_radio_state_on
                });
            (self.free)(data);
            on
        }
    }

    /// The connected BSS's signal (dBm) and centre frequency (kHz), from the BSS list.
    fn bss(
        &self,
        interface: &GUID,
        ssid: &DOT11_SSID,
        bssid: [u8; 6],
        secure: bool,
    ) -> Option<(i32, u32)> {
        let mut list: *mut WLAN_BSS_LIST = std::ptr::null_mut();
        // SAFETY: the list is freed below; the entry is copied out first.
        unsafe {
            let status = (self.bss_list)(
                self.handle,
                interface,
                ssid,
                dot11_BSS_type_infrastructure.0,
                i32::from(secure),
                std::ptr::null(),
                &mut list,
            );
            if status != 0 || list.is_null() {
                return None;
            }
            let count = (*list).dwNumberOfItems as usize;
            let first = (&raw const (*list).wlanBssEntries)
                .cast::<windows::Win32::NetworkManagement::WiFi::WLAN_BSS_ENTRY>();
            let found = (0..count)
                .map(|i| &*first.add(i))
                .find(|e| e.dot11Bssid == bssid)
                .map(|e| (e.lRssi, e.ulChCenterFrequency));
            (self.free)(list.cast());
            found
        }
    }
}

/// The system's message for a WLAN API error code (a Win32 error).
fn message(code: u32) -> String {
    windows::core::Error::from_hresult(WIN32_ERROR(code).to_hresult()).message()
}

impl WifiReader for WinWifi {
    fn read(&mut self) -> Result<WifiStatus, String> {
        let interfaces = self.interfaces()?;
        let Some(interface) = interfaces.first() else {
            return Ok(WifiStatus::default());
        };
        let guid = interface.InterfaceGuid;
        let mut status = WifiStatus {
            available: true,
            radio_on: self.radio_on(&guid),
            connected: interface.isState == wlan_interface_state_connected,
            ..WifiStatus::default()
        };
        if !status.connected {
            return Ok(status);
        }
        // SAFETY: the data is a WLAN_CONNECTION_ATTRIBUTES (or a LONG for the RSSI), freed
        // after reading.
        unsafe {
            match self.query(&guid, wlan_intf_opcode_current_connection.0) {
                Ok(data) => {
                    let attributes = &*data.cast::<WLAN_CONNECTION_ATTRIBUTES>();
                    let association = &attributes.wlanAssociationAttributes;
                    let ssid = association.dot11Ssid;
                    let len = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
                    status.ssid = Some(String::from_utf8_lossy(&ssid.ucSSID[..len]).into_owned());
                    status.quality = association.wlanSignalQuality.min(100);
                    let rate = association.ulRxRate.max(association.ulTxRate);
                    status.link_mbps = (rate > 0).then_some(rate / 1000);
                    status.phy = phy_name(association.dot11PhyType);
                    let secure = attributes.wlanSecurityAttributes.bSecurityEnabled.as_bool();
                    let bssid = association.dot11Bssid;
                    (self.free)(data);
                    if let Some((rssi, khz)) = self.bss(&guid, &ssid, bssid, secure) {
                        status.rssi_dbm = Some(rssi);
                        status.frequency_mhz = (khz > 0).then_some(khz / 1000);
                    }
                }
                // Windows 11 24H2: the network name needs location access; the signal does not.
                Err(code) if code == ERROR_ACCESS_DENIED.0 => {
                    status.location_denied = true;
                    if let Ok(data) = self.query(&guid, wlan_intf_opcode_rssi.0) {
                        let rssi = *data.cast::<i32>();
                        (self.free)(data);
                        status.rssi_dbm = Some(rssi);
                        status.quality = WifiStatus::quality_from_rssi(rssi);
                    }
                }
                Err(code) => return Err(message(code)),
            }
        }
        Ok(status)
    }
}

impl Drop for WinWifi {
    fn drop(&mut self) {
        // SAFETY: unregisters, then closes the handle, which waits for running callbacks;
        // only then is the context freed.
        unsafe {
            (self.register)(
                self.handle,
                WLAN_NOTIFICATION_SOURCE_NONE.0,
                0,
                None,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            (self.close)(self.handle, std::ptr::null());
            drop(Box::from_raw(self.changed));
        }
    }
}

/// Connecting, disconnecting, roaming and radio switches reach the sampler at once.
unsafe extern "system" fn on_notification(data: *mut L2_NOTIFICATION_DATA, context: *mut c_void) {
    // SAFETY: Windows passes the notification and our context, valid during the call.
    let (Some(data), Some(changed)) = (unsafe { data.as_ref() }, unsafe {
        context.cast::<Changed>().as_ref()
    }) else {
        return;
    };
    let code = data.NotificationCode as i32;
    let relevant = match data.NotificationSource {
        source if source == WLAN_NOTIFICATION_SOURCE_ACM => [
            wlan_notification_acm_connection_complete.0,
            wlan_notification_acm_disconnected.0,
            wlan_notification_acm_interface_arrival.0,
            wlan_notification_acm_interface_removal.0,
        ]
        .contains(&code),
        source if source == WLAN_NOTIFICATION_SOURCE_MSM => [
            wlan_notification_msm_connected.0,
            wlan_notification_msm_disconnected.0,
            wlan_notification_msm_roaming_end.0,
            wlan_notification_msm_radio_state_change.0,
        ]
        .contains(&code),
        _ => false,
    };
    if relevant {
        changed();
    }
}

/// The 802.11 standard of a PHY type.
fn phy_name(phy: DOT11_PHY_TYPE) -> Option<&'static str> {
    Some(match phy.0 {
        4 => "802.11a",
        5 => "802.11b",
        6 => "802.11g",
        7 => "802.11n",
        8 => "802.11ac",
        9 => "802.11ad",
        10 => "802.11ax",
        11 => "802.11be",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on the Windows CI runner (`cargo test -- --ignored`), which has no WLAN service: no
    /// reader. Where there is one, it reads and closes cleanly.
    #[test]
    #[ignore = "reads this machine's Wi-Fi"]
    fn reads_wifi_where_there_is_any() {
        let Some(mut reader) = WinWifi::open(Box::new(|| {})) else {
            assert!(!has_interface());
            return;
        };
        let status = reader.read().expect("readable");
        assert!(status.quality <= 100, "{status:?}");
        drop(reader);
    }
}
