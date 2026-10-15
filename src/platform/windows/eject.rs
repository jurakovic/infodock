//! Ejecting drives (SPEC §9.22): a USB disk or card reader is removed as a device, an optical
//! disc is ejected from its drive, and the shell's *Eject* verb is the fallback.
//!
//! Only volumes the drive list marks ejectable (removable and optical drives, disks on USB,
//! FireWire, SD or MMC) are touched: an internal disk is never asked to go.
//!
//! 1. Volume → device: `IOCTL_STORAGE_GET_DEVICE_NUMBER` on the volume, matched against the disk
//!    (or CD-ROM) interfaces SetupAPI lists, gives the disk's device node.
//! 2. From the disk's parent up, the first device node Windows calls removable is ejected with
//!    `CM_Request_Device_EjectW`; a veto names who holds the drive, and is retried a few times
//!    (a file closed a moment ago may still be held).
//! 3. Optical drives (and removable media without a removable device above them): the volume is
//!    locked and dismounted, then `IOCTL_STORAGE_EJECT_MEDIA`.
//! 4. Otherwise the shell's *Eject* verb on the drive (`IContextMenu`).

use std::mem::size_of;
use std::time::Duration;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVCAP_REMOVABLE, CM_DRP_CAPABILITIES, CM_Get_DevNode_Registry_PropertyW, CM_Get_Parent,
    CM_Request_Device_EjectW, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, PNP_VETO_TYPE,
    PNP_VetoTypeUnknown, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SP_DEVINFO_DATA, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Storage::FileSystem::{FILE_DEVICE_CD_ROM, FILE_DEVICE_DISK};
use windows::Win32::System::Ioctl::{
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, GUID_DEVINTERFACE_CDROM, GUID_DEVINTERFACE_DISK,
    IOCTL_STORAGE_EJECT_MEDIA, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER,
};
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CMF_NORMAL, CMINVOKECOMMANDINFO, IContextMenu, IShellItem,
    SHCreateItemFromParsingName,
};
use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, SW_SHOWNORMAL};
use windows::core::{GUID, HSTRING, PCSTR, PCWSTR, s};

use super::device::Device;
use super::disk::QuietErrors;
use super::from_wide;
use crate::platform::{DriveKind, EjectError};

/// Vetoed ejects are tried this often, this far apart.
const ATTEMPTS: u32 = 3;
const RETRY_AFTER: Duration = Duration::from_millis(500);
/// Device nodes above the disk that are looked at for a removable one.
const MAX_DEPTH: u32 = 8;
/// `CMIC_MASK_FLAG_NO_UI`: the shell shows no error dialog of its own.
const NO_UI: u32 = 0x400;

/// Ejects drive `root` (`E:\`); blocks until it is done (seconds), so it runs off the bar
/// thread.
pub fn eject(root: &str) -> Result<(), EjectError> {
    let _quiet = QuietErrors::new();
    super::com::enter();
    let volume = super::disk::volumes()
        .into_iter()
        .find(|v| v.root.eq_ignore_ascii_case(root))
        .ok_or(EjectError::NotEjectable)?;
    if !volume.ejectable {
        return Err(EjectError::NotEjectable);
    }
    let number = device_number(root);
    let optical = volume.kind == DriveKind::Optical
        || number.is_some_and(|n| n.DeviceType == FILE_DEVICE_CD_ROM.0);
    if optical {
        return eject_media(root).or_else(|e| shell_eject(root).map_err(|_| e));
    }
    let removable = number.and_then(disk_node).and_then(removable_above);
    match removable {
        Some(node) => match request_eject(node) {
            Ok(()) => Ok(()),
            // Someone holds it: the shell could not do better.
            Err(e @ EjectError::InUse(_)) => Err(e),
            Err(e) => shell_eject(root).map_err(|_| e),
        },
        // Media in a reader that stays (e.g. an SD slot): out with the medium.
        None => eject_media(root).or_else(|e| shell_eject(root).map_err(|_| e)),
    }
}

/// The device number of the disk (or CD-ROM) behind drive `root`.
fn device_number(root: &str) -> Option<STORAGE_DEVICE_NUMBER> {
    let device = Device::open_volume(root, 0)?;
    query_number(&device)
}

fn query_number(device: &Device) -> Option<STORAGE_DEVICE_NUMBER> {
    let mut output = [0u8; size_of::<STORAGE_DEVICE_NUMBER>()];
    if !device.control(IOCTL_STORAGE_GET_DEVICE_NUMBER, None, Some(&mut output)) {
        return None;
    }
    // SAFETY: the buffer holds a STORAGE_DEVICE_NUMBER, read unaligned.
    Some(unsafe { std::ptr::read_unaligned(output.as_ptr().cast::<STORAGE_DEVICE_NUMBER>()) })
}

/// The device node of the disk (or CD-ROM) with `number`, among the present device interfaces.
fn disk_node(number: STORAGE_DEVICE_NUMBER) -> Option<u32> {
    let class: GUID = if number.DeviceType == FILE_DEVICE_CD_ROM.0 {
        GUID_DEVINTERFACE_CDROM
    } else if number.DeviceType == FILE_DEVICE_DISK.0 {
        GUID_DEVINTERFACE_DISK
    } else {
        return None;
    };
    // SAFETY: the list is destroyed below; every structure carries its size; the detail
    // buffer is u64-aligned and as large as SetupAPI asked for.
    unsafe {
        let set = SetupDiGetClassDevsW(
            Some(&class),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .ok()?;
        let mut found = None;
        for index in 0.. {
            let mut interface = SP_DEVICE_INTERFACE_DATA {
                cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInterfaces(set, None, &class, index, &mut interface).is_err() {
                break;
            }
            let mut required = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(
                set,
                &interface,
                None,
                0,
                Some(&mut required),
                None,
            );
            if (required as usize) < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() {
                continue;
            }
            let mut buffer = vec![0u64; (required as usize).div_ceil(8)];
            let detail = buffer
                .as_mut_ptr()
                .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            let mut info = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiGetDeviceInterfaceDetailW(
                set,
                &interface,
                Some(detail),
                required,
                None,
                Some(&mut info),
            )
            .is_err()
            {
                continue;
            }
            // The path runs from DevicePath to the end of the buffer, NUL-terminated.
            let offset = std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
            let chars = (buffer.len() * 8 - offset) / 2;
            let path = std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>().add(offset).cast::<u16>(),
                chars,
            );
            let same = Device::open(&from_wide(path), 0)
                .and_then(|d| query_number(&d))
                .is_some_and(|n| {
                    n.DeviceType == number.DeviceType && n.DeviceNumber == number.DeviceNumber
                });
            if same {
                found = Some(info.DevInst);
                break;
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        found
    }
}

/// Whether Windows calls device node `node` removable.
fn is_removable(node: u32) -> bool {
    let mut capabilities = 0u32;
    let mut length = size_of::<u32>() as u32;
    // SAFETY: a u32 buffer of the length given.
    let status = unsafe {
        CM_Get_DevNode_Registry_PropertyW(
            node,
            CM_DRP_CAPABILITIES,
            None,
            Some((&raw mut capabilities).cast()),
            &mut length,
            0,
        )
    };
    status == CR_SUCCESS && capabilities & CM_DEVCAP_REMOVABLE.0 != 0
}

/// The first removable device node above the disk `node` (e.g. the USB mass storage device).
fn removable_above(node: u32) -> Option<u32> {
    let mut current = node;
    for _ in 0..MAX_DEPTH {
        let mut parent = 0u32;
        // SAFETY: plain query into a u32.
        if unsafe { CM_Get_Parent(&mut parent, current, 0) } != CR_SUCCESS {
            return None;
        }
        if is_removable(parent) {
            return Some(parent);
        }
        current = parent;
    }
    None
}

/// Asks Windows to eject device node `node`, as *Safely Remove Hardware* does.
fn request_eject(node: u32) -> Result<(), EjectError> {
    let mut last = EjectError::Failed(String::new());
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(RETRY_AFTER);
        }
        let mut veto = PNP_VETO_TYPE::default();
        let mut name = [0u16; 260];
        // SAFETY: the veto type and name buffers are writable; with a name buffer, Windows
        // shows no dialog of its own.
        let status = unsafe { CM_Request_Device_EjectW(node, Some(&mut veto), Some(&mut name), 0) };
        if status == CR_SUCCESS && veto == PNP_VetoTypeUnknown {
            return Ok(());
        }
        last = if veto == PNP_VetoTypeUnknown {
            EjectError::Failed(format!("configuration manager error {}", status.0))
        } else {
            EjectError::InUse(holder(&from_wide(&name)))
        };
        tracing::info!(
            "eject of device node {node} refused: {last:?} (veto {})",
            veto.0
        );
    }
    Err(last)
}

/// A veto name that names a program (not a device path such as `\Device\HarddiskVolume5`).
fn holder(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && !name.starts_with('\\')).then(|| name.to_owned())
}

/// Ejects the medium of drive `root`: lock, dismount, eject.
fn eject_media(root: &str) -> Result<(), EjectError> {
    let device = Device::open_volume(root, GENERIC_READ.0)
        .ok_or_else(|| EjectError::Failed("the drive cannot be opened".into()))?;
    // Fails while files on it are open.
    if !device.control(FSCTL_LOCK_VOLUME, None, None) {
        return Err(EjectError::InUse(None));
    }
    let _ = device.control(FSCTL_DISMOUNT_VOLUME, None, None);
    if device.control(IOCTL_STORAGE_EJECT_MEDIA, None, None) {
        Ok(())
    } else {
        Err(EjectError::Failed("the drive did not eject".into()))
    }
}

/// The shell's *Eject* verb on drive `root`.
fn shell_eject(root: &str) -> Result<(), EjectError> {
    let failed = |e: windows::core::Error| EjectError::Failed(e.message());
    let verb = s!("eject");
    // SAFETY: plain COM calls; the menu is destroyed below and the verb is a static string.
    unsafe {
        let item: IShellItem =
            SHCreateItemFromParsingName(&HSTRING::from(root), None).map_err(failed)?;
        let menu: IContextMenu = item.BindToHandler(None, &BHID_SFUIObject).map_err(failed)?;
        let popup = CreatePopupMenu().map_err(failed)?;
        let result = menu
            .QueryContextMenu(popup, 0, 1, 0x7FFF, CMF_NORMAL)
            .ok()
            .and_then(|()| {
                menu.InvokeCommand(&CMINVOKECOMMANDINFO {
                    cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                    fMask: NO_UI,
                    lpVerb: PCSTR(verb.as_ptr()),
                    nShow: SW_SHOWNORMAL.0,
                    ..Default::default()
                })
            });
        let _ = DestroyMenu(popup);
        result.map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_veto_names_a_program_not_a_device() {
        assert_eq!(holder("Explorer.EXE"), Some("Explorer.EXE".into()));
        assert_eq!(holder(r"\Device\HarddiskVolume5"), None);
        assert_eq!(holder("  "), None);
    }

    /// Runs on the Windows CI runner (`cargo test -- --ignored`): finds the system disk's
    /// device node without ejecting anything, and refuses a drive that is not there or not
    /// ejectable.
    #[test]
    #[ignore = "reads this machine's devices"]
    fn finds_the_system_disk_and_ejects_nothing_fixed() {
        let system = super::super::disk::volumes()
            .into_iter()
            .find(|v| v.kind == DriveKind::Fixed && !v.ejectable)
            .expect("a fixed drive");
        let number = device_number(&system.root).expect("a device number");
        assert!(disk_node(number).is_some(), "{number:?}");
        assert_eq!(eject(&system.root), Err(EjectError::NotEjectable));
        assert_eq!(eject("Q:\\"), Err(EjectError::NotEjectable));
    }
}
