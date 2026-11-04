//! Network interfaces (SPEC Appendix A, `net/<id|auto>`): `GetIfTable2` lists them,
//! `GetIfEntry2` reads one interface's counters each tick, and `GetBestInterfaceEx` finds the
//! one carrying the default route (a route lookup; nothing is sent).

use std::collections::HashSet;

use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceGuidToLuid, FreeMibTable, GetBestInterfaceEx, GetIfEntry2, GetIfTable2,
    IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_ROW2, MIB_IF_TABLE2,
};
use windows::Win32::NetworkManagement::Ndis::{IfOperStatusUp, NET_LUID_LH};
use windows::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, IN_ADDR, IN_ADDR_0, IN6_ADDR, IN6_ADDR_0, SOCKADDR, SOCKADDR_IN,
    SOCKADDR_IN6,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
};
use windows::core::{GUID, PCWSTR, PWSTR, w};

use super::from_wide;
use crate::platform::NetInterface;

/// `InterfaceAndOperStatusFlags`: a hardware adapter, and a filter driver's view of one.
const HARDWARE_INTERFACE: u8 = 1;
const FILTER_INTERFACE: u8 = 2;
/// The network adapters' device class; each numbered subkey is an adapter, with its
/// interface's GUID (`NetCfgInstanceId`) and `Characteristics`.
const ADAPTER_CLASS: PCWSTR =
    w!(r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}");
/// `Characteristics`: an adapter hidden from the user (the WAN Miniports, Hyper-V's switch
/// extensions), which Network Connections leaves out too.
const NCF_HIDDEN: u32 = 0x8;

/// A GUID as `GUID::try_from` reads it back: `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`.
fn guid_text(guid: &GUID) -> String {
    let d = guid.data4;
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        guid.data1, guid.data2, guid.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

fn read(row: &MIB_IF_ROW2) -> NetInterface {
    NetInterface {
        id: guid_text(&row.InterfaceGuid),
        alias: from_wide(&row.Alias),
        description: from_wide(&row.Description),
        up: row.OperStatus == IfOperStatusUp,
        rx_bytes: row.InOctets,
        tx_bytes: row.OutOctets,
        link_rx: row.ReceiveLinkSpeed,
        link_tx: row.TransmitLinkSpeed,
    }
}

/// A connection users know: no loopback, no filter driver's view and no adapter hidden from the
/// user (`hidden`, by GUID); a hardware adapter, or another one while it is connected (e.g. a
/// VPN).
fn listed(row: &MIB_IF_ROW2, hidden: &HashSet<String>) -> bool {
    let flags = row.InterfaceAndOperStatusFlags._bitfield;
    row.Type != IF_TYPE_SOFTWARE_LOOPBACK
        && flags & FILTER_INTERFACE == 0
        && (flags & HARDWARE_INTERFACE != 0 || row.OperStatus == IfOperStatusUp)
        && !hidden.contains(&guid_text(&row.InterfaceGuid))
}

/// The GUIDs ([`guid_text`]) of the adapters hidden from the user (`NCF_HIDDEN`); none where the
/// class cannot be read.
fn hidden_adapters() -> HashSet<String> {
    let mut hidden = HashSet::new();
    let mut class = HKEY::default();
    // SAFETY: opens the class key for reading; it is closed below.
    if unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            ADAPTER_CLASS,
            None,
            KEY_READ,
            &mut class,
        )
    } != ERROR_SUCCESS
    {
        return hidden;
    }
    for index in 0u32.. {
        let mut name = [0u16; 64];
        let mut length = name.len() as u32;
        // SAFETY: `name` holds `length` characters; the subkey's name is NUL-terminated.
        let status = unsafe {
            RegEnumKeyExW(
                class,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut length,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            continue;
        }
        let adapter = PCWSTR(name.as_ptr());
        let characteristics = super::read_dword_in(class, adapter, w!("Characteristics"));
        if characteristics.is_some_and(|c| c & NCF_HIDDEN != 0)
            && let Some(id) = super::read_string(class, adapter, w!("NetCfgInstanceId"))
        {
            hidden.insert(id.trim_matches(['{', '}']).to_uppercase());
        }
    }
    // SAFETY: closes the key opened above.
    let _ = unsafe { RegCloseKey(class) };
    hidden
}

pub fn interfaces() -> Vec<NetInterface> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: on success `table` points to a table the call allocated, freed below.
    if unsafe { GetIfTable2(&mut table) } != NO_ERROR || table.is_null() {
        return Vec::new();
    }
    // SAFETY: the table holds `NumEntries` rows from `Table` on; the pointer to them is taken
    // without borrowing the declared one-row array.
    let rows = unsafe {
        let first = (&raw const (*table).Table).cast::<MIB_IF_ROW2>();
        std::slice::from_raw_parts(first, (*table).NumEntries as usize)
    };
    let hidden = hidden_adapters();
    let mut list: Vec<NetInterface> = Vec::new();
    for row in rows.iter().filter(|row| listed(row, &hidden)) {
        let interface = read(row);
        if !list.iter().any(|i| i.id == interface.id) {
            list.push(interface);
        }
    }
    // SAFETY: frees the table GetIfTable2 allocated; `rows` is not used after this.
    unsafe { FreeMibTable(table.cast()) };
    list.sort_by_cached_key(|i| i.alias.to_lowercase());
    list
}

pub fn interface(id: Option<&str>) -> Option<NetInterface> {
    let mut row = MIB_IF_ROW2::default();
    match id {
        Some(id) => {
            let guid = GUID::try_from(id).ok()?;
            let mut luid = NET_LUID_LH::default();
            // SAFETY: reads the GUID, writes the LUID.
            if unsafe { ConvertInterfaceGuidToLuid(&guid, &mut luid) } != NO_ERROR {
                return None;
            }
            row.InterfaceLuid = luid;
        }
        None => row.InterfaceIndex = default_route()?,
    }
    // SAFETY: the row names the interface by LUID or index; the call fills in the rest.
    (unsafe { GetIfEntry2(&mut row) } == NO_ERROR).then(|| read(&row))
}

/// The index of the interface that carries traffic to the internet: the best route to a
/// public address, over IPv4 or else IPv6.
fn default_route() -> Option<u32> {
    let v4 = SOCKADDR_IN {
        sin_family: AF_INET,
        sin_addr: IN_ADDR {
            S_un: IN_ADDR_0 {
                S_addr: u32::from_ne_bytes([8, 8, 8, 8]),
            },
        },
        ..Default::default()
    };
    let v6 = SOCKADDR_IN6 {
        sin6_family: AF_INET6,
        sin6_addr: IN6_ADDR {
            u: IN6_ADDR_0 {
                Byte: [
                    0x20, 0x01, 0x48, 0x60, 0x48, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0x88, 0x88,
                ],
            },
        },
        ..Default::default()
    };
    let addresses: [*const SOCKADDR; 2] = [(&raw const v4).cast(), (&raw const v6).cast()];
    addresses.into_iter().find_map(|address| {
        let mut index = 0u32;
        // SAFETY: `address` points to a whole socket address of its family.
        (unsafe { GetBestInterfaceEx(address, &mut index) } == NO_ERROR.0).then_some(index)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_round_trip() {
        let guid = GUID::from_u128(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
        let text = guid_text(&guid);
        assert_eq!(text, "01234567-89AB-CDEF-0011-223344556677");
        assert_eq!(GUID::try_from(text.as_str()).ok(), Some(guid));
    }

    /// Runs on the Windows CI runner (`cargo test -- --ignored`).
    #[test]
    #[ignore = "reads this machine's network"]
    fn lists_and_reads_interfaces() {
        let list = interfaces();
        assert!(!list.is_empty(), "no interfaces");
        // Hidden adapters (the runner has WAN Miniports) are not listed.
        assert!(
            !list
                .iter()
                .any(|i| i.description.starts_with("WAN Miniport")),
            "{list:?}"
        );
        let auto = interface(None).expect("a default route");
        assert!(auto.up, "{auto:?}");
        let again = interface(Some(&auto.id)).expect("found by id");
        assert_eq!(again.id, auto.id);
        assert!(again.rx_bytes >= auto.rx_bytes, "{auto:?} → {again:?}");
        assert!(interface(Some("00000000-0000-0000-0000-000000000000")).is_none());
    }
}
