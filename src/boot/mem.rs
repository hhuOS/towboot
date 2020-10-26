//! Memory management
//!
//! In some situations we need to allocate memory at a specific address.
//! Rust's `alloc` can't do this, so we need to use the UEFI API directly.
//! This creates the problem that the allocated memory is not tracked by the borrow checker.
//! We solve this by encapsulating it into a struct that implements `Drop`.

use alloc::alloc::{alloc, Layout};

use uefi::prelude::*;
use uefi::table::boot::{AllocateType, MemoryType};

use log::error;

pub(super) const PAGE_SIZE: usize = 4096;

/// Tracks our own allocations.
pub(super) struct Allocation {
    ptr: u64,
    pages: usize,
}

impl Drop for Allocation {
    /// Free the associated memory.
    fn drop(&mut self) {
        // We can't free memory after we've exited boot services.
        // But this only happens in `PreparedEntry::boot` and this function doesn't return.
        let systab_ptr = uefi_services::system_table();
        let systab = unsafe { systab_ptr.as_ref() };
        systab.boot_services().free_pages(self.ptr, self.pages)
        // let's just panic if we can't free
        .expect("failed to free the allocated memory for the kernel").unwrap();
    }
}

impl Allocation {
    /// Allocate memory at a specific position.
    ///
    /// Note: This will round up to the whole pages.
    /// Also: This memory is not tracked by Rust.
    pub(crate) fn new_at(
        address: usize, size: usize, systab: &SystemTable<Boot>
    ) -> Result<Self, Status>{
        let count_pages = (size / PAGE_SIZE) + 1; // TODO: this may allocate a page too much
        let ptr = systab.boot_services().allocate_pages(
            AllocateType::Address(address),
            MemoryType::LOADER_DATA,
            count_pages
        ).map_err(|e| {
            error!("failed to allocate memory to place the kernel: {:?}", e);
            Status::LOAD_ERROR
        })?.unwrap();
        Ok(Allocation { ptr, pages: count_pages })
    }
    
    /// Return a slice that references the associated memory.
    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr as *mut u8, self.pages * PAGE_SIZE) }
    }
    
    /// Checks whether a part of memory is allocated.
    pub(crate) fn contains(&self, begin: u64, length: usize) -> bool {
        self.ptr <= begin && self.ptr as usize + self.pages * PAGE_SIZE >= begin as usize + length
    }
}

/// Allocate n bytes of memory and return the address.
pub(super) unsafe fn allocate(count: usize) -> *mut u8 {
    let layout = Layout::array::<u8>(count).expect("tried to allocate more than usize");
    let ptr = alloc(layout);
    if ptr as usize >= u32::MAX as usize {
        panic!("couldn't allocate memory below 4GB");
    }
    if ptr.is_null() {
        panic!("failed to allocate memory");
    }
    ptr
}
