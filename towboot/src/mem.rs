//! Memory management
//!
//! In some situations we need to allocate memory at a specific address.
//! Rust's `alloc` can't do this, so we need to use the UEFI API directly.
//! This creates the problem that the allocated memory is not tracked by the borrow checker.
//! We solve this by encapsulating it into a struct that implements `Drop`.
//!
//! Also, gathering memory map information for the kernel happens here.

use core::mem::size_of;
use core::ptr::NonNull;

use alloc::boxed::Box;
use alloc::collections::btree_set::BTreeSet;
use alloc::vec::Vec;

use uefi::prelude::*;
use uefi::boot::{AllocateType, allocate_pages, free_pages, memory_map};
use uefi::mem::memory_map::{
    MemoryDescriptor, MemoryMap, MemoryMapMut, MemoryMapOwned, MemoryType
};

use log::{debug, warn, error};

use towboot_config::Quirk;

// no multiboot import here as some of the types have the same name as the UEFI ones

pub(super) const PAGE_SIZE: usize = 4096;

/// Tracks our own allocations.
#[derive(Debug)]
pub(super) struct Allocation {
    /// the actual start of the allocation; this will be the beginning of a page
    ptr: NonNull<u8>,
    /// how far into the page the allocation starts
    offset: usize,
    /// the length that was requested
    pub len: usize,
    pages: usize,
    /// the address of memory where it should have been allocated
    /// (only when it differs from ptr)
    should_be_at: Option<u64>,
}

impl Drop for Allocation {
    /// Free the associated memory.
    fn drop(&mut self) {
        // We can't free memory after we've exited boot services.
        // But this only happens in `PreparedEntry::boot` and this function doesn't return.
        unsafe { free_pages(self.ptr, self.pages) }
        // let's just panic if we can't free
        .expect("failed to free the allocated memory");
    }
}

impl Allocation {
    /// Allocate memory at a specific position.
    ///
    /// Note: This will round up to whole pages.
    ///
    /// If the memory can't be allocated at the specified address,
    /// it will print a warning and allocate it somewhere else instead.
    /// You can move the allocated memory later to the correct address by calling
    /// [`move_to_where_it_should_be`], but please keep its safety implications in mind.
    /// This only works for our code and data by default, but this can be
    /// overridden with the `ForceOverwrite` quirk.
    ///
    /// [`move_to_where_it_should_be`]: struct.Allocation.html#method.move_to_where_it_should_be
    pub(crate) fn new_at(
        address: usize,
        size: usize,
        quirks: &BTreeSet<Quirk>,
        should_exit_boot_services: bool,
    ) -> Result<Self, Status>{
        let page_offset = address % PAGE_SIZE;
        if page_offset != 0 {
            debug!("wasting {page_offset} bytes because the allocation is not page-aligned");
        }
        let page_start = address - page_offset;
        let count_pages = (size + page_offset).div_ceil(PAGE_SIZE);
        match allocate_pages(
            AllocateType::Address(page_start.try_into().unwrap()),
            MemoryType::LOADER_DATA,
            count_pages,
        ) {
            Ok(ptr) => Ok(Self {
                ptr,
                offset: page_offset,
                len: size,
                pages: count_pages,
                should_be_at: None,
            }),
            Err(e) => {
                warn!("failed to allocate 0x{size:x} bytes of memory at 0x{address:x}: {e:?}");
                // find out why that part of memory is occupied
                let memory_map = get_memory_map();
                let mut types_in_the_way = BTreeSet::new();
                warn!("the following sections are in the way:");
                for entry in memory_map.entries() {
                    // if it's after the space we need, ignore it
                    if entry.phys_start > (address + size).try_into().unwrap() {
                        continue;
                    }
                    // if it's before the space we need, ignore it
                    if entry.phys_start + entry.page_count * 4096 < address.try_into().unwrap() {
                        continue;
                    }
                    // if it's empty, ignore it
                    if entry.ty == MemoryType::CONVENTIONAL {
                        continue;
                    }
                    warn!("{entry:x?}");
                    types_in_the_way.insert(entry.ty);
                }
                // if the allocation is only blocked by our code or data,
                // allocate it somewhere else and move later
                // This also applies to allocations of the Boot Services,
                // but we need to check if we're going to exit them.
                types_in_the_way.remove(&MemoryType::LOADER_CODE);
                types_in_the_way.remove(&MemoryType::LOADER_DATA);
                if should_exit_boot_services {
                    types_in_the_way.remove(&MemoryType::BOOT_SERVICES_CODE);
                    types_in_the_way.remove(&MemoryType::BOOT_SERVICES_DATA);
                }
                if types_in_the_way.is_empty() || quirks.contains(&Quirk::ForceOverwrite) {
                    warn!("going to allocate it somewhere else and try to move it later");
                    warn!("this might fail without notice");
                    Self::new_under_4gb(size, &BTreeSet::default()).map(|mut allocation| {
                        allocation.should_be_at = Some(address.try_into().unwrap());
                        allocation
                    })
                } else {
                    error!("Cannot allocate memory for the kernel, it might be too big.");
                    warn!(
                        "If you're in a virtual machine, you could try passing the ForceOverwrite quirk."
                    );
                    Err(Status::LOAD_ERROR)
                }
            }
        }
    }
    
    /// Allocate memory page-aligned below 4GB.
    ///
    /// Note: This will round up to whole pages.
    pub(crate) fn new_under_4gb(size: usize, quirks: &BTreeSet<Quirk>) -> Result<Self, Status> {
        let count_pages = size.div_ceil(PAGE_SIZE);
        let ptr = allocate_pages(
                AllocateType::MaxAddress(if quirks.contains(&Quirk::ModulesBelow200Mb) {
                    200 * 1024 * 1024
                } else {
                    u32::MAX.into()
                }),
                MemoryType::LOADER_DATA,
                count_pages
            )
            .map_err(|e| {
                error!("failed to allocate {size} bytes of memory: {e:?}");
                get_memory_map();
                Status::LOAD_ERROR
            })?;
        Ok(Self {
            ptr, offset: 0, len: size, pages: count_pages, should_be_at: None,
        })
    }
    
    /// Return a slice that references the associated memory.
    pub(crate) const fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(
            self.ptr.as_ptr().add(self.offset),
            self.len,
        ) }
    }
    
    /// Get the pointer inside.
    pub(crate) fn as_ptr(&self) -> *const u8 {
        assert!(self.offset < self.len);
        unsafe { self.ptr.as_ptr().add(self.offset) }
    }
    
    /// Move to the desired location.
    ///
    /// This is unsafe: In the worst case we could overwrite ourselves, our
    /// variables, firmware code, firmware data, ACPI memory, the Multiboot info
    /// struct or anything referenced therein.
    /// 
    /// See the checks in [`new_at`].
    pub(crate) unsafe fn move_to_where_it_should_be(&mut self) {
        if let Some(a) = self.should_be_at {
            debug!("trying to write {self:?}...");
            // checks already happened in new_at
            let dest: usize = a.try_into().unwrap();
            unsafe {
                core::ptr::copy(
                    self.ptr.as_ptr().add(self.offset),
                    dest as *mut u8,
                    self.len,
                );
            }
            self.ptr = unsafe {
                NonNull::new(a as *mut u8).unwrap().sub(self.offset)
            };
            self.should_be_at = None;
        }
    }
}

/// Get the current memory map.
/// 
/// If the log level is set to debug, the memory map is also logged.
fn get_memory_map() -> MemoryMapOwned {
    debug!("memory map:");
    let mut memory_map = memory_map(MemoryType::LOADER_DATA).expect("failed to get memory map");
    memory_map.sort();
    for descriptor in memory_map.entries() {
        debug!("{descriptor:x?}");
    }
    memory_map
}


/// Pass the memory map to the kernel.
///
/// This needs to have a buffer to write to because we can't allocate memory anymore.
/// (The buffer may be too large.)
pub(super) fn prepare_information(
    info_bytes: &mut [u8],
    mut update_memory_info: Box<dyn FnMut(
        &mut [u8], u32, u32, &[multiboot12::information::MemoryEntry],
        Option<&[multiboot12::information::EfiMemoryDescriptor]>,
    )>,
    efi_mmap: &uefi::mem::memory_map::MemoryMapOwned,
    mb_mmap_vec: &mut Vec<multiboot12::information::MemoryEntry>,
    mb_efi_mmap_vec: &mut Vec<multiboot12::information::EfiMemoryDescriptor>,
    boot_services_exited: bool,
) {
    // Descriptors are the ones from UEFI, Entries are the ones from Multiboot.
    let empty_entry = mb_mmap_vec[0].clone();
    let mut count = 0;
    let mut entry_iter = mb_mmap_vec.iter_mut();
    let mut current_entry = entry_iter.next().unwrap();
    for descriptor in efi_mmap.entries() {
        let next_entry = empty_entry.with(
            descriptor.phys_start, descriptor.page_count * PAGE_SIZE as u64, match descriptor.ty {
                // after we've started the kernel, no-one needs our code or data
                MemoryType::LOADER_CODE | MemoryType::LOADER_DATA
                => multiboot12::information::MemoryType::Available,
                // have Boot Services been exited?
                MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA
                => match boot_services_exited {
                    true => multiboot12::information::MemoryType::Available,
                    false => multiboot12::information::MemoryType::Reserved,
                },
                // the kernel may want to use UEFI Runtime Services
                MemoryType::RUNTIME_SERVICES_CODE | MemoryType::RUNTIME_SERVICES_DATA
                => multiboot12::information::MemoryType::Reserved,
                // it's free memory!
                MemoryType::CONVENTIONAL => multiboot12::information::MemoryType::Available,
                MemoryType::UNUSABLE => multiboot12::information::MemoryType::Defective,
                MemoryType::ACPI_RECLAIM => multiboot12::information::MemoryType::AcpiAvailable,
                MemoryType::ACPI_NON_VOLATILE => multiboot12::information::MemoryType::ReservedHibernate,
                MemoryType::MMIO | MemoryType::MMIO_PORT_SPACE | MemoryType::PAL_CODE
                => multiboot12::information::MemoryType::Reserved,
                MemoryType::PERSISTENT_MEMORY => multiboot12::information::MemoryType::Available,
                _ => multiboot12::information::MemoryType::Reserved, // better be safe than sorry
            }
        );
        if count == 0 {
            *current_entry = next_entry;
            count += 1;
        } else {
            // join adjacent entries of the same type
            if (
                next_entry.memory_type() == current_entry.memory_type()
            ) && (
                next_entry.base_address() == (
                    current_entry.base_address() + current_entry.length()
                )
            ) {
                *current_entry = empty_entry.with(
                    current_entry.base_address(),
                    current_entry.length() + next_entry.length(),
                    current_entry.memory_type(),
                );
            } else {
                current_entry = entry_iter.next().unwrap();
                *current_entry = next_entry;
                count += 1;
            }
        }
    }
    debug!("shrunk memory areas down to {count}");
    mb_mmap_vec.truncate(count);
    assert_eq!(mb_mmap_vec.len(), count);
    
    // "Lower" and "upper" memory as understood by a BIOS in kilobytes.
    // This means:
    // Lower memory is the part of the memory from beginning to the first memory hole,
    // adressable by just 20 bits (because the 8086's address bus had just 20 pins).
    // Upper memory is the part of the memory from 1 MB to the next memory hole
    // (usually a few megabytes).
    let lower = 640; // If we had less than 640KB, we wouldn't fit into memory.
    let upper = mb_mmap_vec
        .iter()
        // find the area starting at 1MB and get its length
        .find(|e| e.base_address() == 1024 * 1024)
        // if there is none, it's 0KB
        .map_or(0, |e| e.length()) / 1024;

    // When updating either uefi.rs or multiboot2, make sure that the types
    // still match.
    // We can at least check whether they have the same size.
    assert_eq!(
        size_of::<MemoryDescriptor>(),
        size_of::<multiboot12::information::EfiMemoryDescriptor>(),
    );
    // We need to copy all entries, because we can't access `efi_mmap.buf`.
    // It might be safer to create new `EFIMemoryDesc`s instead of transmuting.
    efi_mmap.entries().zip(mb_efi_mmap_vec.iter_mut())
        .for_each(
            |(src, dst)|
            *dst = unsafe { core::mem::transmute::<MemoryDescriptor, multiboot12::information::EfiMemoryDescriptor>(*src) }
        );
    
    update_memory_info(
        info_bytes, lower.try_into().unwrap(), upper.try_into().unwrap(),
        mb_mmap_vec.as_slice(), Some(mb_efi_mmap_vec.as_slice()),
    );
    // dropping this box breaks on Multiboot1, when Boot Services have been exited
    if boot_services_exited {
        core::mem::forget(update_memory_info);
    }
}
