//! Handle UEFI config tables.
use alloc::slice;
use alloc::vec::Vec;

use log::{debug, warn, error};
use multiboot12::information::InfoBuilder;
use acpi::rsdp::Rsdp;
use dmidecode::EntryPoint;
use uefi::system::with_config_table;
use uefi::table::cfg::{
    ConfigTableEntry, ACPI_GUID, ACPI2_GUID, DEBUG_IMAGE_INFO_GUID,
    DXE_SERVICES_GUID, HAND_OFF_BLOCK_LIST_GUID, LZMA_COMPRESS_GUID,
    MEMORY_STATUS_CODE_RECORD_GUID, MEMORY_TYPE_INFORMATION_GUID, SMBIOS_GUID,
    SMBIOS3_GUID,
};

/// Go through all of the configuration tables.
/// Some of them are interesting for Multiboot2.
pub(super) fn parse_for_multiboot(info_builder: &mut InfoBuilder) {
    // first, copy all config table pointers
    // TODO: remove this when with_config_table takes a FnMut
    let config_tables: Vec<ConfigTableEntry> = with_config_table(<[ConfigTableEntry]>::to_vec);
    debug!("going through configuration tables...");
    for table in config_tables {
        match table.guid {
            ACPI_GUID | ACPI2_GUID => handle_acpi(&table, info_builder),
            DEBUG_IMAGE_INFO_GUID => debug!("ignoring image debug info"),
            DXE_SERVICES_GUID => debug!("ignoring dxe services table"),
            HAND_OFF_BLOCK_LIST_GUID => debug!("ignoring hand-off block list"),
            LZMA_COMPRESS_GUID => debug!("ignoring lzma filesystem"),
            MEMORY_STATUS_CODE_RECORD_GUID | MEMORY_TYPE_INFORMATION_GUID => debug!("ignoring early memory info"),
            SMBIOS_GUID | SMBIOS3_GUID => handle_smbios(&table, info_builder),
            guid => debug!("ignoring table {guid}"),
        }
    }
}

/// Parse the ACPI RSDP and create the Multiboot struct for it.
fn handle_acpi(table: &ConfigTableEntry, info_builder: &mut InfoBuilder) {
    debug!("handling ACPI RSDP");
    let rsdp: Rsdp = unsafe { *(table.address.cast()) };
    if rsdp.validate().is_err() {
        warn!("the RSDP is invalid");
        return;
    }

    match table.guid {
        ACPI_GUID => {
            if rsdp.revision() != 0 {
                warn!("expected RSDP version 0, but got {}", rsdp.revision());
            }
            info_builder.set_rsdp_v1(
                rsdp.signature(), rsdp.checksum(),
                rsdp.oem_id().as_bytes()[0..6].try_into().unwrap(),
                rsdp.revision(), rsdp.rsdt_address(),
            );
        }
        ACPI2_GUID => {
            if rsdp.revision() != 2 {
                warn!("expected RSDP version 2, but got {}", rsdp.revision());
            }
            if rsdp.revision() == 0 {
                // some u-boot versions do this
                warn!("RSDP revision is 0, forcing v1");
                info_builder.set_rsdp_v1(
                    rsdp.signature(), rsdp.checksum(),
                    rsdp.oem_id().as_bytes()[0..6].try_into().unwrap(),
                    rsdp.revision(), rsdp.rsdt_address(),
                );
                return;
            }
            info_builder.set_rsdp_v2(
                rsdp.signature(), rsdp.checksum(),
                rsdp.oem_id().as_bytes()[0..6].try_into().unwrap(),
                rsdp.revision(), rsdp.rsdt_address(), rsdp.length(),
                rsdp.xsdt_address(), rsdp.ext_checksum(),
            );
        }
        _ => panic!("'handle_acpi()' called with wrong config table entry")
    }
}

/// Copy the SMBIOS tables.
/// This is a copy of the Entry Point and the Structure Table.
/// Caveat: The Structure Table pointer in the Entry Point is not adjusted.
fn handle_smbios(table: &ConfigTableEntry, info_builder: &mut InfoBuilder) {
    debug!("handling SMBIOS table");
    let bigger_slice = unsafe { slice::from_raw_parts(table.address.cast(), 128) };
    match EntryPoint::search(bigger_slice) {
        Ok(entry_point) => {
            let version = entry_point.to_version();
            let should_be_version = match table.guid {
                SMBIOS_GUID => 2,
                SMBIOS3_GUID => 3,
                _ => panic!("'handle_smbios()' called with wrong config table entry")
            };
            if version.major != should_be_version {
                warn!(
                    "expected SMBIOS entry point version {}, but got {}",
                    should_be_version, version.major,
                );
            }
            let mut bytes = bigger_slice[0..entry_point.len().into()].to_vec();
            // TODO: replace structure_table_address afterwards
            let structure_table_address: usize = entry_point.smbios_address().try_into().unwrap();
            bytes.extend_from_slice(unsafe { slice::from_raw_parts(
                structure_table_address as *const u8,
                entry_point.smbios_len().try_into().unwrap(),
            ) });
            info_builder.add_smbios_tag(
                version.major, version.minor, bytes.as_slice(),
            );
        },
        Err(e) => error!("failed to parse SMBIOS entry point: {e:?}"),
    }
}
