//! This module handles the actual boot and related stuff.
//!
//! This means: lower-level memory management, handling ELF files and video initialization.

use alloc::{vec, vec::Vec};

use core::convert::{identity, TryInto};

use uefi::prelude::*;
use uefi::proto::console::gop::GraphicsOutput;
use uefi::proto::media::file::Directory;

use log::{debug, info, error};

use multiboot::header::{Header, MultibootAddresses};
use multiboot::information::{
    MemoryEntry, Module, Multiboot, MultibootInfo, SIGNATURE_EAX, SymbolType
};

use elfloader::ElfBinary;

use super::config::Entry;
use super::file::File;
use super::mem::{Allocation, MultibootAllocator};

mod elf;
mod video;

use elf::OurElfLoader;

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
    let kernel_vec: Vec<u8> = File::open(&entry.image, volume)?.into();
    let header = Header::from_slice(kernel_vec.as_slice()).ok_or_else(|| {
        error!("invalid Multiboot header");
        Status::LOAD_ERROR
    })?;
    debug!("loaded kernel {:?} to {:?}", header, kernel_vec.as_ptr());
    let (kernel_allocations, addresses, symbols) = match header.get_addresses() {
        Some(addr) => load_kernel_multiboot(kernel_vec, addr, header.header_start),
        None => load_kernel_elf(kernel_vec, &entry.image),
    }?;
    let (symbols_struct, symbols_vec) = match symbols {
        Some((s, v)) => (Some(s), Some(v)),
        None => (None, None),
    };
    
    // Load all modules, fail completely if one fails to load.
    // just always use whole pages, that's easier for us
    let modules_vec: Vec<Allocation> = entry.modules.iter().flat_map(identity).map(|module|
        File::open(&module.image, volume).map(|f| f.into())
    ).collect::<Result<Vec<_>, _>>()?;
    info!("loaded {} modules", modules_vec.len());
    for (index, module) in modules_vec.iter().enumerate() {
        debug!("loaded module {} to {:?}", index, module.as_ptr());
    }
    
    let mut graphics_output = video::setup_video(&header, &systab)?;
    
    let (multiboot_information, multiboot_allocator) = prepare_multiboot_information(
        &entry, &modules_vec, symbols_struct, graphics_output
    );
    
    Ok(PreparedEntry {
        entry, kernel_allocations, addresses, multiboot_information,
        multiboot_allocator, symbols_vec, modules_vec,
    })
}

enum Addresses {
    Multiboot(MultibootAddresses),
    /// the entry address
    Elf(usize),
}


/// Load a kernel which has its addresses specified inside the Multiboot header.
fn load_kernel_multiboot(
    kernel_vec: Vec<u8>, addresses: MultibootAddresses, header_start: u32
) -> Result<(Vec<Allocation>, Addresses, Option<(SymbolType, Vec<u8>)>), Status> {
    // Try to the get symbols from parsing this as an ELF, if it fails, we have no symbols.
    // TODO: Instead add support for AOut symbols?
    let symbols = match ElfBinary::new("", kernel_vec.as_slice()) {
        Ok(binary) => Some(elf::symbols(&binary)),
        Err(_) => None,
    };
    
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
        addresses.load_address.try_into().unwrap(), kernel_length
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
    
    Ok((vec![allocation], Addresses::Multiboot(addresses), symbols))
}

/// Load a kernel which uses ELF semantics.
fn load_kernel_elf(
    kernel_vec: Vec<u8>, name: &str
) -> Result<(Vec<Allocation>, Addresses, Option<(SymbolType, Vec<u8>)>), Status> {
    let binary = ElfBinary::new(name, kernel_vec.as_slice()).map_err(|msg| {
        error!("failed to parse ELF structure of kernel: {}", msg);
        Status::LOAD_ERROR
    })?;
    let mut loader = OurElfLoader::new(binary.entry_point());
    binary.load(&mut loader).map_err(|msg| {
        error!("failed to load kernel: {}", msg);
        Status::LOAD_ERROR
    })?;
    let symbols = Some(elf::symbols(&binary));
    let entry_point = loader.entry_point();
    Ok((loader.into(), Addresses::Elf(entry_point), symbols))
}

/// Prepare information for the kernel.
fn prepare_multiboot_information(
    entry: &Entry, modules: &Vec<Allocation>, symbols: Option<SymbolType>,
    graphics_output: &mut GraphicsOutput
) -> (MultibootInfo, MultibootAllocator) {
    let mut info = MultibootInfo::default();
    let mut allocator = MultibootAllocator::new();
    let mut multiboot = Multiboot::from_ref(&mut info, &mut allocator);
    
    multiboot.set_command_line(match &entry.argv {
        None => None,
        Some(s) => Some(&s),
    });
    
    let mb_modules: Vec<Module> = modules.iter().zip(entry.modules.iter().flatten()).map(|(module, module_entry)| {
        Module::new(
            module.as_ptr() as u64,
            unsafe { module.as_ptr().offset(module.len.try_into().unwrap()) as u64 },
            match &module_entry.argv {
                None => None,
                Some(s) => Some(&s),
            }
        )
    }).collect();
    multiboot.set_modules(Some(&mb_modules));
    
    // Passing memory information happens after exiting BootServices,
    // so we don't accidentally allocate or deallocate, making the data obsolete.
    // TODO: Do we really need to do this? Our allocations don't matter to the kernel.
    // TODO: But do they affect the firmware's allocations?
    
    multiboot.set_symbols(symbols);
    
    video::prepare_information(&mut multiboot, graphics_output);
    
    // TODO: the rest
    (info, allocator)
}

pub(crate) struct PreparedEntry<'a> {
    entry: &'a Entry,
    kernel_allocations: Vec<Allocation>,
    addresses: Addresses,
    multiboot_information: MultibootInfo,
    multiboot_allocator: MultibootAllocator,
    symbols_vec: Option<Vec<u8>>,
    modules_vec: Vec<Allocation>,
}

impl PreparedEntry<'_> {
    /// Actuelly boot an entry.
    ///
    /// What this means:
    /// 1. exit BootServices
    /// 2. pass the memory map to the kernel
    /// 3. copy the kernel to its desired location (if needed)
    /// 4. when on x64_64: switch to x86
    /// 5. jump!
    ///
    /// This function won't return.
    pub(crate) fn boot(mut self, image: Handle, systab: SystemTable<Boot>) {
        match &self.entry.name {
            Some(n) => info!("booting '{}'...", n),
            None => info!("booting..."),
        }
        
        // allocate memory for the memory map
        // also, keep a bit of room
        info!("exiting boot services...");
        let mut mmap_vec = Vec::<u8>::new();
        let mut mb_mmap_vec = Vec::<MemoryEntry>::new();
        // Leave a bit of room at the end, we only have one chance.
        let estimated_size = systab.boot_services().memory_map_size() + 100;
        mmap_vec.resize(estimated_size, 0);
        mb_mmap_vec.resize(estimated_size, MemoryEntry::default());
        let (systab, mmap_iter) = systab.exit_boot_services(image, mmap_vec.as_mut_slice())
        .expect("failed to exit boot services").unwrap();
        // now, write! won't work anymore. Also, we can't allocate any memory.
        
        // Passing the memory map has to happen here,
        // since we can't allocate or deallocate anymore.
        let mut multiboot = Multiboot::from_ref(
            &mut self.multiboot_information, &mut self.multiboot_allocator
        );
        let mb_mmap = super::mem::prepare_information(
            &mut multiboot, mmap_iter, mb_mmap_vec.leak()
        );
        
        for mut allocation in &mut self.kernel_allocations {
            // It could be possible that we failed to allocate memory for the kernel in the correct
            // place before. Just copy it now to where is belongs.
            // This is *really* unsafe, please see the documentation comment for details.
            unsafe { allocation.move_to_where_it_should_be(&mb_mmap) };
            // We are going to jump into it, so make sure it stays around indefinitely.
            core::mem::forget(allocation);
        }
        // The kernel is going to need the modules, so make sure they stay.
        for allocation in &self.modules_vec {
            core::mem::forget(allocation);
        }
        // The kernel is going to need the section headers and symbols.
        core::mem::forget(self.symbols_vec);
        
        // TODO: Step 4
        
        let entry_address = match &self.addresses {
            Addresses::Multiboot(addr) => addr.entry_address as usize,
            Addresses::Elf(e) => *e,
        };
        
        unsafe {
            asm!(
                // 3.2 Machine state says:
                
                // > ‘EFLAGS’: Bit 17 (VM) must be cleared. Bit 9 (IF) must be cleared.
                // > Other bits are all undefined. 
                // disable interrupts (should have been enabled)
                "cli",
                // virtual 8086 mode can't be set, as we're 32 or 64 bit code
                // (and changing that flag is rather difficult)
                
                // > ‘CR0’ Bit 31 (PG) must be cleared. Bit 0 (PE) must be set.
                // > Other bits are all undefined.
                "mov edx, cr0",
                // disable paging (it should have been enabled)
                "and edx, ~(1<<31)",
                // enable protected mode (it should have already been enabled)
                "or edx, 1",
                "mov cr0, edx",
                
                // finally jump to the kernel
                "jmp ecx",
                
                in("eax") SIGNATURE_EAX,
                in("ebx") &self.multiboot_information,
                in("ecx") entry_address,
            );
        }
        unreachable!();
    }
}
