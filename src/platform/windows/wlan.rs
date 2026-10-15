//! Wi-Fi through the WLAN API (SPEC §9.15, Appendix A). `wlanapi.dll` is loaded only when it is
//! used: it is missing on Windows editions without the wireless LAN feature, and a static import
//! would keep Infodock from starting there.

use std::ffi::c_void;

use windows::Win32::Foundation::{FreeLibrary, HANDLE, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{PCSTR, s, w};

type OpenHandle = unsafe extern "system" fn(u32, *const c_void, *mut u32, *mut HANDLE) -> u32;
type EnumInterfaces = unsafe extern "system" fn(HANDLE, *const c_void, *mut *mut u32) -> u32;
type FreeMemory = unsafe extern "system" fn(*const c_void);
type CloseHandle = unsafe extern "system" fn(HANDLE, *const c_void) -> u32;

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
