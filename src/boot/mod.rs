//! This module handles the actual boot and related stuff.
//!
//! This means: loading kernel and modules, handling ELF files, video initialization and jumping

use alloc::{
    collections::btree_set::BTreeSet,
    format,
    vec,
    vec::Vec,
};

use core::arch::asm;
use uefi::prelude::*;
use uefi::proto::console::gop::GraphicsOutput;
use uefi::proto::media::file::Directory;

use log::{debug, info, error};

use multiboot::header::{Header, MultibootAddresses};
use multiboot::information::{
    MemoryEntry, Module, Multiboot, MultibootInfo, SIGNATURE_EAX, SymbolType
};

use goblin::elf::Elf;

use super::config::{Entry, Quirk};
use super::file::File;
use super::mem::{Allocation, MultibootAllocator};

mod elf;
mod video;

use elf::OurElfLoader;

enum Addresses {
    Multiboot(MultibootAddresses),
    /// the entry address
    Elf(usize),
}

/// A kernel loaded into memory
struct LoadedKernel {
    allocations: Vec<Allocation>,
    addresses: Addresses,
    symbols: Option<(SymbolType, Vec<u8>)>,
}

impl LoadedKernel {
    /// Load a kernel from a vector.
    /// This requires that the Multiboot header has already been parsed.
    fn new(
        kernel_vec: Vec<u8>, header: &Header, quirks: &BTreeSet<Quirk>,
    ) -> Result<Self, Status> {
        match (header.get_addresses(), quirks.contains(&Quirk::ForceElf)) {
            (Some(addr), false) => LoadedKernel::new_multiboot(kernel_vec, addr, header.header_start),
            _ => LoadedKernel::new_elf(kernel_vec),
        }
    }
    
    /// Load a kernel which has its addresses specified inside the Multiboot header.
    fn new_multiboot(
        kernel_vec: Vec<u8>, addresses: MultibootAddresses, header_start: u32
    ) -> Result<Self, Status> {
        // TODO: Add support for AOut symbols? Do we really know this binary is AOut at this point?
        
        // Try to allocate the memory where to load the kernel and move the kernel there.
        // In the worst case we might have blocked the destination by loading the file there,
        // but `move_to_where_it_should_be` should fix this later.
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
        
        Ok(Self {
            allocations: vec![allocation],
            addresses: Addresses::Multiboot(addresses),
            symbols: None,
        })
    }
    
    /// Load a kernel which uses ELF semantics.
    fn new_elf(kernel_vec: Vec<u8>) -> Result<Self, Status> {
        let mut binary = Elf::parse(kernel_vec.as_slice()).map_err(|msg| {
            error!("failed to parse ELF structure of kernel: {msg}");
            Status::LOAD_ERROR
        })?;
        let mut loader = OurElfLoader::new(binary.entry);
        loader.load_elf(&binary, kernel_vec.as_slice()).map_err(|msg| {
            error!("failed to load kernel: {msg}");
            Status::LOAD_ERROR
        })?;
        let symbols = Some(elf::symbols(&mut binary, kernel_vec.as_slice()));
        let entry_point = loader.entry_point();
        Ok(Self{
            allocations: loader.into(),
            addresses: Addresses::Elf(entry_point),
            symbols,
        })
    }
    
    /// Get the symbols struct.
    /// This is needed for the Multiboot Information struct.
    fn symbols_struct(&self) -> Option<&SymbolType> {
        self.symbols.as_ref().map(|(s, _v)| s)
    }
}

/// Prepare information for the kernel.
fn prepare_multiboot_information(
    entry: &Entry, modules: &[Allocation], symbols: Option<SymbolType>,
    graphics_output: &mut GraphicsOutput
) -> (MultibootInfo, MultibootAllocator) {
    let mut info = MultibootInfo::default();
    let mut allocator = MultibootAllocator::new();
    let mut multiboot = Multiboot::from_ref(&mut info, &mut allocator);
    
    // We don't have much information about the partition we loaded the kernel from.
    // There's the UEFI Handle, but the kernel probably won't understand that.
    
    multiboot.set_command_line(entry.argv.as_deref());
    let mb_modules: Vec<Module> = modules.iter().zip(entry.modules.iter()).map(|(module, module_entry)| {
        Module::new(
            module.as_ptr() as u64,
            unsafe { module.as_ptr().offset(module.len.try_into().unwrap()) as u64 },
            module_entry.argv.as_deref()
        )
    }).collect();
    multiboot.set_modules(Some(&mb_modules));
    multiboot.set_symbols(symbols);
    
    // Passing memory information happens after exiting BootServices,
    // so we don't accidentally allocate or deallocate, making the data obsolete.
    // TODO: Do we really need to do this? Our allocations don't matter to the kernel.
    // TODO: But do they affect the firmware's allocations?
    
    // We can't ask the BIOS for information about the drives.
    // (We could ask the firmware and convert it to the legacy BIOS format, however.)
    
    // There is no BIOS config table.
    
    multiboot.set_boot_loader_name(Some(&format!(
        "{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")
    )));
    
    // There is no APM config table.
    
    // There is no VBE information.
    
    video::prepare_information(&mut multiboot, graphics_output);
    
    (info, allocator)
}

pub(crate) struct PreparedEntry<'a> {
    entry: &'a Entry,
    loaded_kernel: LoadedKernel,
    multiboot_information: MultibootInfo,
    multiboot_allocator: MultibootAllocator,
    modules_vec: Vec<Allocation>,
}

impl<'a> PreparedEntry<'a> {
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
    pub(crate) fn new(
        entry: &'a Entry, volume: &mut Directory, systab: &SystemTable<Boot>
    ) -> Result<PreparedEntry<'a>, Status> {
        let kernel_vec: Vec<u8> = File::open(&entry.image, volume)?.try_into()?;
        let header = Header::from_slice(kernel_vec.as_slice()).ok_or_else(|| {
            error!("invalid Multiboot header");
            Status::LOAD_ERROR
        })?;
        debug!("loaded kernel {:?} to {:?}", header, kernel_vec.as_ptr());
        let loaded_kernel = LoadedKernel::new(kernel_vec, &header, &entry.quirks)?;
        info!("kernel is loaded and bootable");
        
        // Load all modules, fail completely if one fails to load.
        // just always use whole pages, that's easier for us
        let modules_vec: Vec<Allocation> = entry.modules.iter().map(|module|
            File::open(&module.image, volume)
            .and_then(|f| f.try_into_allocation(&entry.quirks))
        ).collect::<Result<Vec<_>, _>>()?;
        info!("loaded {} modules", modules_vec.len());
        for (index, module) in modules_vec.iter().enumerate() {
            debug!("loaded module {} to {:?}", index, module.as_ptr());
        }
        
        let graphics_output = video::setup_video(&header, systab, &entry.quirks)?;
        
        let (multiboot_information, multiboot_allocator) = prepare_multiboot_information(
            entry, &modules_vec, loaded_kernel.symbols_struct().copied(),
            graphics_output,
        );
        
        Ok(PreparedEntry {
            entry, loaded_kernel, multiboot_information,
            multiboot_allocator, modules_vec,
        })
    }
    
    /// Actually boot an entry.
    ///
    /// What this means:
    /// 1. exit `BootServices`
    /// 2. pass the memory map to the kernel
    /// 3. copy the kernel to its desired location (if needed)
    /// 4. bring the machine in the correct state
    /// 5. jump!
    ///
    /// This function won't return.
    pub(crate) fn boot(mut self, image: Handle, systab: SystemTable<Boot>) {
        // allocate memory for the memory map
        // also, keep a bit of room
        info!("exiting boot services...");
        let mut mmap_vec = Vec::<u8>::new();
        let mut mb_mmap_vec = Vec::<MemoryEntry>::new();
        // Leave a bit of room at the end, we only have one chance.
        let estimated_size = systab.boot_services().memory_map_size().map_size + 100;
        mmap_vec.resize(estimated_size, 0);
        mb_mmap_vec.resize(estimated_size, MemoryEntry::default());
        let (_systab, mmap_iter) = systab.exit_boot_services(image, mmap_vec.as_mut_slice())
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
        
        for allocation in &mut self.loaded_kernel.allocations {
            // It could be possible that we failed to allocate memory for the kernel in the correct
            // place before. Just copy it now to where is belongs.
            // This is *really* unsafe, please see the documentation comment for details.
            unsafe { allocation.move_to_where_it_should_be(mb_mmap) };
        }
        // The kernel will need its code and data, so make sure it stays around indefinitely.
        core::mem::forget(self.loaded_kernel.allocations);
        // The kernel is going to need the modules, so make sure they stay.
        core::mem::forget(self.modules_vec);
        // The kernel is going to need the section headers and symbols.
        core::mem::forget(self.loaded_kernel.symbols);
        
        let entry_address = match &self.loaded_kernel.addresses {
            Addresses::Multiboot(addr) => addr.entry_address as usize,
            Addresses::Elf(e) => *e,
        };
        
        unsafe {
            asm!(
                // The jump to the kernel has to happen in protected mode.
                // If we're built for i686, we already are in protected mode,
                // so this has no effect.
                // If we're built for x86_64, this brings us to compatibility mode.
                ".code32",
                
                // 3.2 Machine state says:
                
                // > ‘CS’: Must be a 32-bit read/execute code segment with an offset of ‘0’
                // > and a limit of ‘0xFFFFFFFF’. The exact value is undefined.
                // TODO: Maybe set this?
                // > 'DS’, 'ES’, ‘FS’, ‘GS’, ‘SS’: Must be a 32-bit read/write data segment with an
                // > offset of ‘0’ and a limit of ‘0xFFFFFFFF’. The exact values are all undefined.
                // TODO: Maybe set this?
                
                // > ‘EFLAGS’: Bit 17 (VM) must be cleared. Bit 9 (IF) must be cleared.
                // > Other bits are all undefined. 
                // disable interrupts (should have been enabled)
                "cli",
                // virtual 8086 mode can't be set, as we're 32 or 64 bit code
                // (and changing that flag is rather difficult)

                // Writing to RBX (and thus EBX) using in("ebx") is forbidden,
                // since this register is used internally by LLVM.
                // Thus, we need to write the mulitboot information address to EAX
                // and copy it into EBX here.
                "mov ebx, eax",

                // > ‘CR0’ Bit 31 (PG) must be cleared. Bit 0 (PE) must be set.
                // > Other bits are all undefined.
                "mov ecx, cr0",
                // disable paging (it should have been enabled)
                "and ecx, ~(1<<31)",
                // enable protected mode (it should have already been enabled)
                "or ecx, 1",
                "mov cr0, ecx",
                
                // The spec doesn't say anything about cr4, but let's do it anyway.
                "mov ecx, cr4",
                // disable PAE
                "and ecx, ~(1<<5)",
                "mov cr4, ecx",
                
                // TODO: Only do this on x86_64?
                // x86_64: switch from compatibility mode to protected mode
                // get the EFER
                "mov ecx, 0xC0000080",
                "rdmsr",
                // disable long mode
                "and eax, ~(1<<8)",
                "wrmsr",
                
                // write the signature to EAX
                "mov eax, {}",
                // finally jump to the kernel
                "jmp edi",
                
                const SIGNATURE_EAX,
                in("eax") &self.multiboot_information,
                in("edi") entry_address,
                options(noreturn),
            );
        }
    }
}
