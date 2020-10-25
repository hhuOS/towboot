//! This module handles the actual boot and related stuff.
//!
//! This means: lower-level memory management, handling ELF files and video initialization.

use alloc::{vec, vec::Vec};

use core::convert::{identity, TryInto};

use uefi::prelude::*;
use uefi::proto::console::gop::GraphicsOutput;
use uefi::proto::media::file::Directory;

use log::{debug, info, error};

use multiboot::{Header, Multiboot, MultibootAddresses, MultibootInfo, SIGNATURE_EAX};

use elfloader::ElfBinary;

use crate::config::Entry;

mod elf;
mod mem;
mod video;

use elf::OurElfLoader;
use mem::Allocation;

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
    let header = Header::from_slice(kernel_vec.as_slice()).ok_or_else(|| {
        error!("invalid Multiboot header");
        Status::LOAD_ERROR
    })?;
    debug!("loaded kernel: {:?}", header);
    let (kernel_allocations, addresses) = match header.get_addresses() {
        Some(addr) => load_kernel_multiboot(kernel_vec, addr, header.header_start, &systab),
        None => load_kernel_elf(kernel_vec, &entry.image, &systab),
    }?;
    
    // Load all modules, fail completely if one fails to load.
    let modules_vec: Vec<Vec<u8>> = entry.modules.iter().flat_map(identity).map(|module|
        crate::read_file(&module.image, volume)
    ).collect::<Result<Vec<_>, _>>()?;
    info!("loaded {} modules", modules_vec.len());
    
    let mut graphics_output = video::setup_video(&header, &systab)?;
    
    let multiboot_information = prepare_multiboot_information(graphics_output);
    
    Ok(PreparedEntry { entry, kernel_allocations, header, addresses, multiboot_information, modules_vec })
}

enum Addresses {
    Multiboot(MultibootAddresses),
    /// the entry address
    Elf(usize),
}


/// Load a kernel which has its addresses specified inside the Multiboot header.
fn load_kernel_multiboot(
    kernel_vec: Vec<u8>, addresses: MultibootAddresses,
    header_start: u32, systab: &SystemTable<Boot>
) -> Result<(Vec<Allocation>, Addresses), Status> {
    // try to allocate the memory where to load the kernel and move the kernel there
    // TODO: maybe optimize this so that we at first read just the beginning of the kernel
    // and then read the whole kernel into the right place directly
    // The current implementation is fast enough
    // (we're copying just a few megabytes through memory),
    // but in some cases we could block the destination with the source and this would be bad.
    info!("moving the kernel to its desired location...");
    let load_offset = addresses.compute_load_offset(header_start);
    // allocate
    let kernel_length: usize = {
        if addresses.bss_end_address == 0 {addresses.load_end_address - addresses.load_address}
        else {addresses.bss_end_address - addresses.load_address}
    }.try_into().unwrap();
    let mut allocation = Allocation::new_at(
        addresses.load_address.try_into().unwrap(), kernel_length, &systab
    )?;
    let kernel_buf = allocation.as_mut_slice();
    // copy from beginning of text to end of data segment and fill the rest with zeroes
    kernel_buf.iter_mut().zip(
        kernel_vec.iter()
        .skip(load_offset.try_into().unwrap())
        .take((addresses.load_end_address - addresses.load_address).try_into().unwrap())
        .chain(core::iter::repeat(&0))
    )
    .for_each(|(dst,src)| *dst = *src);
    // drop the old vector
    core::mem::drop(kernel_vec);
    Ok((vec![allocation], Addresses::Multiboot(addresses)))
}

/// Load a kernel which uses ELF semantics.
fn load_kernel_elf(
    kernel_vec: Vec<u8>, name: &str, systab: &SystemTable<Boot>
) -> Result<(Vec<Allocation>, Addresses), Status> {
    let binary = ElfBinary::new(name, kernel_vec.as_slice()).map_err(|msg| {
        error!("failed to parse ELF structure of kernel: {}", msg);
        Status::LOAD_ERROR
    })?;
    let mut loader = OurElfLoader::new(systab);
    binary.load(&mut loader).map_err(|msg| {
        error!("failed to load kernel: {}", msg);
        Status::LOAD_ERROR
    })?;
    Ok((loader.allocations, Addresses::Elf(binary.entry_point() as usize)))
}

/// Prepare information for the kernel.
fn prepare_multiboot_information(graphics_output: &mut GraphicsOutput) -> MultibootInfo {
    let mut info = MultibootInfo::default();
    let mut multiboot = Multiboot::from_ref(&mut info);
    
    video::prepare_information(&mut multiboot, graphics_output);
    
    // TODO: the rest
    info
}

pub(crate) struct PreparedEntry<'a> {
    entry: &'a Entry,
    // this has been allocated via allocate_pages(), so it's not tracked by Rust
    // we have to explicitly take care of disposing this if a boot fails
    kernel_allocations: Vec<Allocation>,
    header: Header,
    addresses: Addresses,
    multiboot_information: MultibootInfo,
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
        
        let entry_address = match &self.addresses {
            Addresses::Multiboot(addr) => addr.entry_address as usize,
            Addresses::Elf(e) => *e,
        };
        
        unsafe {
            asm!(
                // cr0 and eflags should be correct, already
                "jmp {}",
                in(reg) entry_address,
                in("eax") SIGNATURE_EAX,
                in("ebx") &self.multiboot_information,
            );
        }
        unreachable!();
    }
}
