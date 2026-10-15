//! Volume devices (SPEC Appendix A `volumes`, §9.22): the bus a disk hangs on, which tells a
//! USB disk (ejectable) from an internal one, and whether a removable or optical drive holds a
//! medium. Both are asked of the device without reading the medium, so nothing spins up.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    BusType1394, BusTypeMmc, BusTypeSd, BusTypeUsb, CreateFileW, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, STORAGE_BUS_TYPE,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    IOCTL_STORAGE_CHECK_VERIFY2, IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};
use windows::core::HSTRING;

/// An open device handle, closed on drop.
pub(super) struct Device(pub(super) HANDLE);

impl Device {
    /// Opens the volume of the drive `root` (`E:\`) as a device, with `access` (0 to query
    /// only).
    pub(super) fn open_volume(root: &str, access: u32) -> Option<Self> {
        let letter = root.trim_end_matches('\\');
        let path = HSTRING::from(format!(r"\\.\{letter}"));
        // SAFETY: a valid path; the handle is closed on drop.
        unsafe {
            CreateFileW(
                &path,
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .ok()
        .map(Self)
    }

    /// Sends `code` with `input` and an `output` buffer; whether the device answered.
    pub(super) fn control(
        &self,
        code: u32,
        input: Option<&[u8]>,
        output: Option<&mut [u8]>,
    ) -> bool {
        let mut returned = 0u32;
        let (in_ptr, in_len) = input.map_or((None, 0), |i| {
            (Some(i.as_ptr().cast::<c_void>()), i.len() as u32)
        });
        let (out_ptr, out_len) = match output {
            Some(o) => (Some(o.as_mut_ptr().cast::<c_void>()), o.len() as u32),
            None => (None, 0),
        };
        // SAFETY: the buffers are valid for the lengths given; no overlapped I/O.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                in_ptr,
                in_len,
                out_ptr,
                out_len,
                Some(&mut returned),
                None,
            )
        }
        .is_ok()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: the handle opened in `open_volume`.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// The bus the disk of drive `root` hangs on; `None` if the device does not say.
fn bus(root: &str) -> Option<STORAGE_BUS_TYPE> {
    let device = Device::open_volume(root, 0)?;
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        ..Default::default()
    };
    // SAFETY: STORAGE_PROPERTY_QUERY is plain data; viewed as its bytes for the call.
    let input = unsafe {
        std::slice::from_raw_parts(
            (&raw const query).cast::<u8>(),
            size_of::<STORAGE_PROPERTY_QUERY>(),
        )
    };
    // The descriptor's fixed part is enough for the bus type; the rest (strings) may not fit.
    let mut output = [0u8; 1024];
    if !device.control(IOCTL_STORAGE_QUERY_PROPERTY, Some(input), Some(&mut output)) {
        return None;
    }
    // SAFETY: the buffer is larger than the descriptor's fixed part and aligned for reading
    // it unaligned.
    let descriptor =
        unsafe { std::ptr::read_unaligned(output.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>()) };
    Some(descriptor.BusType)
}

/// Whether the disk of drive `root` is on a bus whose disks can be unplugged: USB, FireWire,
/// SD or MMC (an external disk, which Windows reports as fixed).
pub fn on_removable_bus(root: &str) -> bool {
    bus(root).is_some_and(|b| [BusTypeUsb, BusType1394, BusTypeSd, BusTypeMmc].contains(&b))
}

/// Whether removable or optical drive `root` holds a medium, without reading it.
pub fn has_media(root: &str) -> bool {
    Device::open_volume(root, FILE_READ_ATTRIBUTES.0)
        .is_some_and(|d| d.control(IOCTL_STORAGE_CHECK_VERIFY2, None, None))
}
