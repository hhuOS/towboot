//! Memory management
//!
//! In some situations we need to allocate memory at a specific address.
//! Rust's `alloc` can't do this, so we need to use the UEFI API directly.
//! This creates the problem that the allocated memory is not tracked by the borrow checker.
//! We solve this by encapsulating it into a struct that implements `Drop`.
//!
//! Also, gathering memory map information for the kernel happens here.

use core::convert::TryInto;

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::vec::Vec;

use uefi::prelude::*;
use uefi::table::boot::{AllocateType, MemoryDescriptor, MemoryType};
use uefi_services::system_table;

use log::{debug, warn, error};

use hashbrown::HashMap;

// no multiboot import here as some of the types have the same name as the UEFI ones

pub(super) const PAGE_SIZE: usize = 4096;

/// Tracks our own allocations.
pub(super) struct Allocation {
    ptr: u64,
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
        unsafe { system_table().as_ref() }.boot_services().free_pages(self.ptr, self.pages)
        // let's just panic if we can't free
        .expect("failed to free the allocated memory for the kernel").unwrap();
    }
}

impl Allocation {
    /// Allocate memory at a specific position.
    ///
    /// Note: This will round up to the whole pages.
    /// Also: This memory is not tracked by Rust.
    pub(crate) fn new_at(address: usize, size: usize) -> Result<Self, Status>{
        let count_pages = (size / PAGE_SIZE) + 1; // TODO: this may allocate a page too much
        match unsafe { system_table().as_ref() }.boot_services().allocate_pages(
            AllocateType::Address(address),
            MemoryType::LOADER_DATA,
            count_pages
        ).log_warning() {
            Ok(ptr) => Ok(Allocation { ptr, len: size, pages: count_pages, should_be_at: None }),
            Err(e) => {
                warn!("failed to allocate {} bytes of memory at {:x}: {:?}", size, address, e);
                dump_memory_map();
                warn!("going to allocate it somewhere else and try to move it later");
                warn!("this might fail without notice");
                match Self::new_under_4gb(size).map(|mut allocation| {
                    allocation.should_be_at = Some(address.try_into().unwrap());
                    allocation
                }) {
                    Ok(a) => return Ok(a),
                    Err(e) => Err(e),
                }
            }
        }
    }
    
    /// Allocate memory page-aligned below 4GB.
    ///
    /// Note: This will round up to the whole pages.
    /// Also: This memory is not tracked by Rust.
    pub(crate) fn new_under_4gb(size: usize) -> Result<Self, Status> {
        let count_pages = (size / PAGE_SIZE) + 1; // TODO: this may allocate a page too much
        let ptr = unsafe { system_table().as_ref() }.boot_services().allocate_pages(
            AllocateType::MaxAddress(u32::MAX as usize),
            MemoryType::LOADER_DATA,
            count_pages
        ).map_err(|e| {
            error!("failed to allocate {} bytes of memory: {:?}", size, e);
            dump_memory_map();
            Status::LOAD_ERROR
        })?.unwrap();
        Ok(Allocation { ptr, len:size, pages: count_pages, should_be_at: None })
    }
    
    /// Return a slice that references the associated memory.
    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr as *mut u8, self.pages * PAGE_SIZE) }
    }
    
    /// Checks whether a part of memory is allocated.
    pub(crate) fn contains(&self, begin: u64, length: usize) -> bool {
        self.ptr <= begin && self.ptr as usize + self.pages * PAGE_SIZE >= begin as usize + length
    }
    
    /// Get the pointer inside.
    pub(crate) fn as_ptr(&self) -> *const u8 {
        self.ptr as *const u8
    }
    
    /// Move to the desired location.
    ///
    /// This is incredibly unsafe: We don't allocate the memory there, we just write.
    pub(crate) unsafe fn move_to_where_it_should_be(&mut self) {
        // TODO: we *really* might want to check the memory map,
        // so we at least don't write to memory-mapped IO or ACPI tables or stuff like that
        match self.should_be_at {
            None => return,
            Some(a) => {
                let dest: usize = a.try_into().unwrap();
                let src: usize = self.ptr.try_into().unwrap();
                core::ptr::copy(src as *mut u8, dest as *mut u8, self.len);
                self.ptr = a;
                self.should_be_at = None;
            }
        }
    }
}

/// Show the current memory map.
fn dump_memory_map() {
    debug!("memory map:");
    let mut buf = Vec::new();
    buf.resize(unsafe { system_table().as_ref() }.boot_services().memory_map_size(), 0);
    let (key, iterator) = unsafe { system_table().as_ref() }.boot_services()
    .memory_map(buf.as_mut_slice()).log_warning().expect("failed to get memory map");
    for descriptor in iterator {
        debug!("{:?}", descriptor);
    }
}


/// Proxy Rust's allocator to the multiboot crate.
pub(super) struct MultibootAllocator {
    allocations: HashMap<multiboot::information::PAddr, Layout>
}

impl MultibootAllocator {
    /// Initialize the allocator.
    pub(super) fn new() -> Self {
        MultibootAllocator { allocations: HashMap::new() }
    }
}

impl multiboot::information::MemoryManagement for MultibootAllocator {
    /// Get a slice to the memory referenced by the pointer.
    unsafe fn paddr_to_slice(
        &self, addr: multiboot::information::PAddr, length: usize
    ) -> Option<&'static [u8]> {
        // TODO: Does this check make the function safe?
        // Or is it even too strict?
        self.allocations.get(&addr).map(|layout|
            core::slice::from_raw_parts(addr as *const u8, layout.size())
        )
    }

    /// Allocate n bytes of memory and return the address.
    unsafe fn allocate(
        &mut self, length: usize
    ) -> Option<(multiboot::information::PAddr, &mut [u8])> {
        let layout = Layout::array::<u8>(length).expect("tried to allocate more than usize");
        let ptr = alloc(layout);
        if ptr as usize >= u32::MAX as usize {
            error!("couldn't allocate memory below 4GB");
            return None
        }
        if ptr.is_null() {
            error!("failed to allocate memory");
            None
        } else {
            self.allocations.insert(ptr as multiboot::information::PAddr, layout);
            Some((
                ptr as multiboot::information::PAddr, core::slice::from_raw_parts_mut(ptr, length)
            ))
        }
    }
    
    /// Free the previously allocated memory.
    unsafe fn deallocate(&mut self, addr: multiboot::information::PAddr) {
        if addr == 0 {
            return;
        }
        match self.allocations.remove(&addr) {
            None => panic!(
                "couldn't free memory that has not been previously allocated: {}", addr
            ),
            Some(layout) => dealloc(addr as *mut u8, layout)
        }
    }
}

/// Pass the memory map to the kernel.
///
/// This needs to have a buffer to write to because we can't allocate memory anymore.
/// (The buffer may be too large.)
pub(super) fn prepare_information<'a, I>(
    multiboot: &mut multiboot::information::Multiboot, mmap_iter: I,
    mb_mmap_buf: &'static mut[multiboot::information::MemoryEntry]
) where I: ExactSizeIterator<Item = &'a MemoryDescriptor> {
    // Descriptors are the ones from UEFI, Entries are the ones from Multiboot.
    let mut count = 0;
    let mut entry_iter = mb_mmap_buf.iter_mut();
    let mut current_entry = entry_iter.next().unwrap();
    for descriptor in mmap_iter {
        let next_entry = multiboot::information::MemoryEntry::new(
            descriptor.phys_start, descriptor.page_count * PAGE_SIZE as u64, match descriptor.ty {
                // after we've started the kernel, no-one needs our code or data
                MemoryType::LOADER_CODE | MemoryType::LOADER_DATA
                | MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA
                => multiboot::information::MemoryType::Available,
                // the kernel may want to use UEFI Runtime Services
                MemoryType::RUNTIME_SERVICES_CODE | MemoryType::RUNTIME_SERVICES_DATA
                => multiboot::information::MemoryType::Reserved,
                // it's free memory!
                MemoryType::CONVENTIONAL => multiboot::information::MemoryType::Available,
                MemoryType::UNUSABLE => multiboot::information::MemoryType::Defect,
                MemoryType::ACPI_RECLAIM => multiboot::information::MemoryType::ACPI,
                MemoryType::ACPI_NON_VOLATILE => multiboot::information::MemoryType::NVS,
                // TODO: Are these correct?
                MemoryType::MMIO | MemoryType::MMIO_PORT_SPACE | MemoryType::PAL_CODE
                => multiboot::information::MemoryType::Reserved,
                MemoryType::PERSISTENT_MEMORY => multiboot::information::MemoryType::Available,
                _ => multiboot::information::MemoryType::Reserved, // better be safe than sorry
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
                *current_entry = multiboot::information::MemoryEntry::new(
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
    
    // "Lower" and "upper" memory as understood by a BIOS in kilobytes.
    // This means:
    // Lower memory is the part of the memory from beginning to the first memory hole,
    // adressable by just 20 bits (because the 8086's address bus had just 20 pins).
    // Upper memory is the part of the memory from 1 MB to the next memory hole
    // (usually a few megabytes).
    // We assume (and assert) that the table we got from the firmware is ordered.
    let lower = 640; // If we had less than 640KB, we wouldn't fit into memory.
    let upper = mb_mmap_buf.iter().find(|e| e.base_address() == 1024 * 1024)
    .unwrap().length() / 1024;
    multiboot.set_memory_bounds(Some((lower.try_into().unwrap(), upper.try_into().unwrap())));
    
    multiboot.set_memory_regions(Some((
        mb_mmap_buf.as_ptr() as multiboot::information::PAddr, count
    )));
}
