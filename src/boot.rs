//! This module handles the actual boot.

use alloc::{vec, vec::Vec};

use core::convert::{identity, TryInto};

use uefi::prelude::*;
use uefi::proto::media::file::Directory;
use uefi::table::boot::{AllocateType, MemoryType};

use log::{trace, debug, info, error};

use multiboot1::{Addresses, Metadata, MultibootAddresses};

use elfloader::{ElfBinary, ElfLoader, Flags, LoadableHeaders, P64, Rela, VAddr};

use crate::config::Entry;

/// Prepare an entry for boot.
///
/// What this means:
/// 1. load the kernel into memory
/// 2. try to parse the Multiboot information
/// 3. move the kernel to where it wants to be
/// 4. load the modules
/// 5. make the framebuffer ready
/// 6. create the Multiboot information for the kernel
///
/// Return a `PreparedEntry` which can be used to actually boot.
/// This is non-destructive and will always return.
pub(crate) fn prepare_entry<'a>(
    entry: &'a Entry, volume: &mut Directory, systab: &SystemTable<Boot>
) -> Result<PreparedEntry<'a>, Status> {
    let kernel_vec = crate::read_file(&entry.image, volume)?;
    let metadata = multiboot1::parse(kernel_vec.as_slice()).map_err(|e| {
        error!("invalid Multiboot header: {:?}", e);
        Status::LOAD_ERROR
    })?;
    debug!("loaded kernel: {:?}", metadata);
    let kernel_allocations = match &metadata.addresses {
        Addresses::Multiboot(addr) => load_kernel_multiboot(kernel_vec, addr, &systab),
        Addresses::Elf(addr) => load_kernel_elf(kernel_vec, &entry.image, &systab),
    }?;
    
    // Load all modules, fail completely if one fails to load.
    let modules_vec: Vec<Vec<u8>> = entry.modules.iter().flat_map(identity).map(|module|
        crate::read_file(&module.image, volume)
    ).collect::<Result<Vec<_>, _>>()?;
    info!("loaded {} modules", modules_vec.len());
    
    
    // TODO: Steps 5 and 6
    Ok(PreparedEntry { entry, kernel_allocations, metadata, modules_vec })
}


/// Load a kernel which has its addresses specified inside the Multiboot header.
fn load_kernel_multiboot(
    kernel_vec: Vec<u8>, addresses: &MultibootAddresses, systab: &SystemTable<Boot>
) -> Result<Vec<Allocation>, Status> {
    // try to allocate the memory where to load the kernel and move the kernel there
    // TODO: maybe optimize this so that we at first read just the beginning of the kernel
    // and then read the whole kernel into the right place directly
    // The current implementation is fast enough
    // (we're copying just a few megabytes through memory),
    // but in some cases we could block the destination with the source and this would be bad.
    info!("moving the kernel to its desired location...");
    // allocate
    let kernel_length: usize = {
        if addresses.bss_end_address == 0 {addresses.load_end_address - addresses.load_address}
        else {addresses.bss_end_address - addresses.load_address}
    }.try_into().unwrap();
    let mut allocation = allocate_at(
        addresses.load_address.try_into().unwrap(), kernel_length, &systab
    )?;
    let kernel_buf = allocation.as_mut_slice();
    // copy from beginning of text to end of data segment and fill the rest with zeroes
    kernel_buf.iter_mut().zip(
        kernel_vec.iter()
        .skip(addresses.load_offset.try_into().unwrap())
        .take((addresses.load_end_address - addresses.load_address).try_into().unwrap())
        .chain(core::iter::repeat(&0))
    )
    .for_each(|(dst,src)| *dst = *src);
    // drop the old vector
    core::mem::drop(kernel_vec);
    Ok(vec![allocation])
}

/// Load a kernel which uses ELF semantics.
fn load_kernel_elf(
    kernel_vec: Vec<u8>, name: &str, systab: &SystemTable<Boot>
) -> Result<Vec<Allocation>, Status> {
    let binary = ElfBinary::new(name, kernel_vec.as_slice()).map_err(|msg| {
        error!("failed to parse ELF structure of kernel: {}", msg);
        Status::LOAD_ERROR
    })?;
    let mut loader = OurElfLoader::new(systab);
    binary.load(&mut loader).map_err(|msg| {
        error!("failed to load kernel: {}", msg);
        Status::LOAD_ERROR
    })?;
    Ok(loader.allocations)
}

struct OurElfLoader<'a> {
    // be careful, they have to be freed!
    allocations: Vec<Allocation>,
    systab: &'a SystemTable<Boot>
}

impl<'a> OurElfLoader<'a> {
    fn new(systab: &'a SystemTable<Boot>) -> Self {
        OurElfLoader {
            allocations: Vec::new(),
            systab
        }
    }
}

impl<'a> ElfLoader for OurElfLoader<'a> {
    fn allocate(&mut self, load_headers: LoadableHeaders) -> Result<(), &'static str> {
        for header in load_headers {
            if header.virtual_addr() != header.physical_addr() {
                todo!("support loading ELFs where virtual_addr != physical_addr")
            }
            trace!("header: {:?}", header);
            debug!(
                "allocating {} {} bytes at {:#x}",
                header.mem_size(), header.flags(), header.physical_addr()
            );
            let mut allocation = allocate_at(
                header.physical_addr().try_into().unwrap(),
                header.mem_size().try_into().unwrap(),
                &self.systab
            ).map_err(|e| "failed to allocate memory for the kernel")?;
            let mut mem_slice = allocation.as_mut_slice();
            mem_slice.fill(0);
            self.allocations.push(allocation);
        }
        Ok(())
    }

    fn relocate(&mut self, entry: &Rela<P64>) -> Result<(), &'static str> {
        unimplemented!("no support for ELF relocations");
    }

    fn load(&mut self, flags: Flags, base: VAddr, region: &[u8]) -> Result<(), &'static str> {
        // check whether we actually allocated this
        if !self.allocations.iter().any(|a| a.ptr == base && a.pages * 4096 >= region.len()) {
            panic!("we didn't allocate {:#x}, but tried to write to it o.O", base);
        }
        debug!("load {} bytes into {:#x}", region.len(), base);
        let mut mem_slice = unsafe {
            core::slice::from_raw_parts_mut(base as *mut u8, region.len())
        };
        mem_slice.clone_from_slice(region);
        Ok(())
    }
}

/// Allocate memory at a specific position.
///
/// Note: This will round up to the whole pages.
/// Also: This memory is not tracked by Rust.
fn allocate_at(
    address: usize, size: usize, systab: &SystemTable<Boot>
) -> Result<Allocation, Status>{
    let count_pages = (size / 4096) + 1; // TODO: this may allocate a page too much
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

/// Tracks our own allocations.
struct Allocation {
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
    /// Return a slice that references the associated memory.
    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr as *mut u8, self.pages * 4096) }
    }
}

pub(crate) struct PreparedEntry<'a> {
    entry: &'a Entry,
    // this has been allocated via allocate_pages(), so it's not tracked by Rust
    // we have to explicitly take care of disposing this if a boot fails
    kernel_allocations: Vec<Allocation>,
    metadata: Metadata,
    modules_vec: Vec<Vec<u8>>,
    // TODO: framebuffer and Multiboot information
}

impl Drop for PreparedEntry<'_> {
    /// Abort the boot.
    ///
    /// Disposes the loaded kernel and modules and restores the framebuffer.
    fn drop(&mut self) {
        // TODO: restore the framebuffer
    }
}

impl PreparedEntry<'_> {
    /// Actuelly boot an entry.
    ///
    /// What this means:
    /// 1. exit BootServices
    /// 2. when on x64_64: switch to x86
    /// 3. jump!
    ///
    /// This function won't return.
    pub(crate) fn boot(&self, image: Handle, systab: SystemTable<Boot>) {
        match &self.entry.name {
            Some(n) => info!("booting '{}'...", n),
            None => info!("booting..."),
        }
        
        // allocate memory for the memory map
        // also, keep a bit of room
        info!("exiting boot services...");
        let mut mmap_vec = Vec::<u8>::new();
        mmap_vec.resize(systab.boot_services().memory_map_size() + 100, 0);
        let (systab, mmap_iter) = systab.exit_boot_services(image, mmap_vec.as_mut_slice())
        .expect("failed to exit boot services").unwrap();
        // now, write! won't work anymore. Also, we can't allocate any memory.
        
        // TODO: Step 2
        
        let entry_address = match &self.metadata.addresses {
            Addresses::Multiboot(addr) => addr.entry_address as usize,
            Addresses::Elf(e) => *e as usize,
        };
        // TODO: Not sure whether this works. We don't get any errors.
        let entry_ptr = unsafe {core::mem::transmute::<_, fn()>(entry_address)};
        entry_ptr();
        unreachable!();
    }
}
