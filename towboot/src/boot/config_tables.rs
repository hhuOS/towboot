//! Handle UEFI config tables.
use alloc::slice;
use alloc::vec::Vec;

use log::{debug, warn};
use multiboot12::information::InfoBuilder;
use acpi::rsdp::Rsdp;
use smbioslib::{SMBiosEntryPoint32, SMBiosEntryPoint64};
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
    let config_tables: Vec<ConfigTableEntry> = with_config_table(|s|
        s.iter().cloned().collect()
    );
    debug!("going through configuration tables...");
    for table in config_tables {
        match table.guid {
            ACPI_GUID => handle_acpi(&table, info_builder),
            ACPI2_GUID => handle_acpi(&table, info_builder),
            DEBUG_IMAGE_INFO_GUID => debug!("ignoring image debug info"),
            DXE_SERVICES_GUID => debug!("ignoring dxe services table"),
            HAND_OFF_BLOCK_LIST_GUID => debug!("ignoring hand-off block list"),
            LZMA_COMPRESS_GUID => debug!("ignoring lzma filesystem"),
            MEMORY_STATUS_CODE_RECORD_GUID => debug!("ignoring early memory info"),
            MEMORY_TYPE_INFORMATION_GUID => debug!("ignoring early memory info"),
            SMBIOS_GUID => handle_smbios(&table, info_builder),
            SMBIOS3_GUID => handle_smbios(&table, info_builder),
            guid => debug!("ignoring table {guid}"),
        }
    }
}

/// Parse the ACPI RSDP and create the Multiboot struct for it.
fn handle_acpi(table: &ConfigTableEntry, info_builder: &mut InfoBuilder) {
    debug!("handling ACPI RSDP");
    let rsdp = unsafe { *(table.address as *const Rsdp) };
    if rsdp.validate().is_err() {
        warn!("the RSDP is invalid");
        return;
    }
    if rsdp.revision() == 0 {
        info_builder.set_rsdp_v1(
            rsdp.signature(), rsdp.checksum(),
            rsdp.oem_id().as_bytes()[0..6].try_into().unwrap(),
            rsdp.revision(), rsdp.rsdt_address(),
        );
    } else {
        info_builder.set_rsdp_v2(
            rsdp.signature(), rsdp.checksum(),
            rsdp.oem_id().as_bytes()[0..6].try_into().unwrap(),
            rsdp.revision(), rsdp.rsdt_address(), rsdp.length(),
            rsdp.xsdt_address(), rsdp.ext_checksum(),
        );
    }
}

/// The entry point for SMBIOS.
enum EntryPoint {
    SMBIOS2(SMBiosEntryPoint32),
    SMBIOS3(SMBiosEntryPoint64),
}

impl EntryPoint {
    fn major_version(&self) -> u8 {
        match self {
            Self::SMBIOS2(e) => e.major_version(),
            Self::SMBIOS3(e) => e.major_version(),
        }
    }

    fn minor_version(&self) -> u8 {
        match self {
            Self::SMBIOS2(e) => e.minor_version(),
            Self::SMBIOS3(e) => e.minor_version(),
        }
    }

    fn entry_point_length(&self) -> u8 {
        match self {
            Self::SMBIOS2(e) => e.entry_point_length(),
            Self::SMBIOS3(e) => e.entry_point_length(),
        }
    }

    fn structure_table_address(&self) -> u64 {
        match self {
            Self::SMBIOS2(e) => e.structure_table_address().into(),
            Self::SMBIOS3(e) => e.structure_table_address(),
        }
    }

    fn structure_table_length(&self) -> u32 {
        match self {
            Self::SMBIOS2(e) => e.structure_table_length().into(),
            Self::SMBIOS3(e) => e.structure_table_maximum_size(),
        }
    }
}


/// Copy the SMBIOS tables.
/// This is a copy of the Entry Point and the Structure Table.
/// Caveat: The Structure Table pointer in the Entry Point is not adjusted.
fn handle_smbios(table: &ConfigTableEntry, info_builder: &mut InfoBuilder) {
    debug!("handling SMBIOS table");
    let bigger_slice = unsafe { slice::from_raw_parts(
        table.address as *const u8, 100 + match table.guid {
            SMBIOS_GUID => SMBiosEntryPoint32::MINIMUM_SIZE,
            SMBIOS3_GUID => SMBiosEntryPoint64::MINIMUM_SIZE,
            guid => panic!("{guid} is not a SMBIOS table"),
        }
    ) };
    let entry_point = match table.guid {
        SMBIOS_GUID => EntryPoint::SMBIOS2(
            SMBiosEntryPoint32::try_scan_from_raw(bigger_slice)
            .expect("the 32 bit SMBIOS table to be parseable")
        ),
        SMBIOS3_GUID => EntryPoint::SMBIOS3(
            SMBiosEntryPoint64::try_scan_from_raw(bigger_slice)
            .expect("the 64 bit SMBIOS table to be parseable")
        ),
        guid => panic!("{guid} is not a SMBIOS table"),
    };
    let mut bytes = bigger_slice[0..entry_point.entry_point_length().into()].to_vec();
    // TODO: replace structure_table_address afterwards
    let structure_table_address: usize = entry_point.structure_table_address().try_into().unwrap();
    bytes.extend_from_slice(unsafe { slice::from_raw_parts(
        structure_table_address as *const u8,
        entry_point.structure_table_length().try_into().unwrap(),
    ) });
    info_builder.add_smbios_tag(
        entry_point.major_version(), entry_point.minor_version(), bytes.as_slice(),
    );
}