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

use multiboot12::header::{Header, Addresses as MultibootAddresses};
use multiboot12::information::{
    Module, InfoBuilder, Symbols
};

use goblin::elf::Elf;

use super::config::{Entry, Quirk};
use super::file::File;
use super::mem::Allocation;

mod elf;
mod video;

use elf::OurElfLoader;

/// A kernel loaded into memory
struct LoadedKernel {
    allocations: Vec<Allocation>,
    entry_address: usize,
    symbols: Option<(Symbols, Vec<u8>)>,
}

impl LoadedKernel {
    /// Load a kernel from a vector.
    /// This requires that the Multiboot header has already been parsed.
    fn new(
        kernel_vec: Vec<u8>, header: &Header, quirks: &BTreeSet<Quirk>,
    ) -> Result<Self, Status> {
        if header.get_load_addresses().is_some() && !quirks.contains(&Quirk::ForceElf) {
            LoadedKernel::new_multiboot(kernel_vec, header)
        } else {
            LoadedKernel::new_elf(header, kernel_vec)
        }
    }
    
    /// Load a kernel which has its addresses specified inside the Multiboot header.
    fn new_multiboot(
        kernel_vec: Vec<u8>, header: &Header,
    ) -> Result<Self, Status> {
        // TODO: Add support for AOut symbols? Do we really know this binary is AOut at this point?
        let addresses = header.get_load_addresses().unwrap();
        
        // Try to allocate the memory where to load the kernel and move the kernel there.
        // In the worst case we might have blocked the destination by loading the file there,
        // but `move_to_where_it_should_be` should fix this later.
        info!("moving the kernel to its desired location...");
        let load_offset = addresses.compute_load_offset(header.header_start());
        // allocate
        let kernel_length: usize = addresses.compute_kernel_length(
            kernel_vec.len().try_into().unwrap()
        ).try_into().unwrap();
        let mut allocation = Allocation::new_at(
            addresses.load_addr().try_into().unwrap(), kernel_length
        )?;
        let kernel_buf = allocation.as_mut_slice();
        // copy from beginning of text to end of data segment and fill the rest with zeroes
        kernel_buf.iter_mut().zip(
            kernel_vec.iter()
            .skip(load_offset.try_into().unwrap())
            .take(kernel_length)
            .chain(core::iter::repeat(&0))
        )
        .for_each(|(dst,src)| *dst = *src);
        // drop the old vector
        core::mem::drop(kernel_vec);
        
        Ok(Self {
            allocations: vec![allocation],
            entry_address: header.get_entry_address().expect(
                "kernels that specify a load address to also specify an entry address"
            ).try_into().unwrap(),
            symbols: None,
        })
    }
    
    /// Load a kernel which uses ELF semantics.
    fn new_elf(header: &Header, kernel_vec: Vec<u8>) -> Result<Self, Status> {
        let mut binary = Elf::parse(kernel_vec.as_slice()).map_err(|msg| {
            error!("failed to parse ELF structure of kernel: {msg}");
            Status::LOAD_ERROR
        })?;
        let mut loader = OurElfLoader::new(binary.entry);
        loader.load_elf(&binary, kernel_vec.as_slice()).map_err(|msg| {
            error!("failed to load kernel: {msg}");
            Status::LOAD_ERROR
        })?;
        let symbols = Some(elf::symbols(header, &mut binary, kernel_vec.as_slice()));
        let entry_address = match header.get_entry_address() {
            Some(a) => a.try_into().unwrap(),
            None => loader.entry_point(),
        };
        Ok(Self {
            allocations: loader.into(), entry_address, symbols,
        })
    }
    
    /// Get the symbols struct.
    /// This is needed for the Multiboot Information struct.
    /// This leaks the allocated memory.
    fn symbols_struct(&mut self) -> Option<Symbols> {
        self.symbols.take().map(|(s, v)| {
            core::mem::forget(v);
            s
        })
    }
}

/// Prepare information for the kernel.
fn prepare_multiboot_information(
    entry: &Entry, header: Header, modules: &[Allocation],
    symbols: Option<Symbols>, graphics_output: &mut GraphicsOutput,
) -> InfoBuilder {
    let mut info_builder = header.info_builder();
    
    // We don't have much information about the partition we loaded the kernel from.
    // There's the UEFI Handle, but the kernel probably won't understand that.
    
    info_builder.set_command_line(entry.argv.as_deref());
    let mb_modules: Vec<Module> = modules.iter().zip(entry.modules.iter()).map(|(module, module_entry)| {
        info_builder.new_module(
            (module.as_ptr() as usize).try_into().unwrap(),
            (unsafe {
                module.as_ptr().offset(module.len.try_into().unwrap())
            } as usize ).try_into().unwrap(),
            module_entry.argv.as_deref()
        )
    }).collect();
    info_builder.set_modules(Some(mb_modules));
    info_builder.set_symbols(symbols);
    
    // Passing memory information happens after exiting BootServices,
    // so we don't accidentally allocate or deallocate, making the data obsolete.
    // TODO: Do we really need to do this? Our allocations don't matter to the kernel.
    // TODO: But do they affect the firmware's allocations?
    
    // We can't ask the BIOS for information about the drives.
    // (We could ask the firmware and convert it to the legacy BIOS format, however.)
    
    // There is no BIOS config table.
    
    info_builder.set_boot_loader_name(Some(&format!(
        "{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")
    )));
    
    // There is no APM config table.
    
    // There is no VBE information.
    
    video::prepare_information(&mut info_builder, graphics_output);
    
    info_builder
}

pub(crate) struct PreparedEntry<'a> {
    entry: &'a Entry,
    loaded_kernel: LoadedKernel,
    multiboot_information: InfoBuilder,
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
        let mut loaded_kernel = LoadedKernel::new(kernel_vec, &header, &entry.quirks)?;
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
        
        let multiboot_information = prepare_multiboot_information(
            entry, header, &modules_vec,
            loaded_kernel.symbols_struct(), graphics_output,
        );
        
        Ok(PreparedEntry {
            entry, loaded_kernel, multiboot_information, modules_vec,
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
        let mut mmap_vec = Vec::<u8>::new();
        // Leave a bit of room at the end, we only have one chance.
        let map_size = systab.boot_services().memory_map_size();
        let estimated_size = map_size.map_size + 200;
        let estimated_count = estimated_size / map_size.entry_size;
        debug!("expecting {estimated_count} memory areas");
        // these are just placeholders
        mmap_vec.resize(estimated_size, 0);
        let mut mb_mmap_vec = self.multiboot_information
            .allocate_memory_map_vec(estimated_count);
        self.multiboot_information.set_memory_bounds(Some((0, 0)));
        let (
            mut info, signature, update_memory_info,
        ) = self.multiboot_information.build();
        debug!("passing {} to kernel...", signature);
        if !self.entry.quirks.contains(&Quirk::DontExitBootServices) {
            info!("exiting boot services...");
            let (_systab, mmap_iter) = systab.exit_boot_services(image, mmap_vec.as_mut_slice())
                .expect("failed to exit boot services");
            // now, write! won't work anymore. Also, we can't allocate any memory.
            
            // Passing the memory map has to happen here,
            // since we can't allocate or deallocate anymore.
            super::mem::prepare_information(
                &mut info, update_memory_info,
                mmap_iter, &mut mb_mmap_vec, true,
            );
        } else {
            let (_key, mmap_iter) = systab.boot_services().memory_map(mmap_vec.as_mut_slice()).unwrap();
            debug!("got {} memory areas", mmap_iter.len());
            super::mem::prepare_information(
                &mut info, update_memory_info,
                mmap_iter, &mut mb_mmap_vec, false,
            );
        }
        
        for allocation in &mut self.loaded_kernel.allocations {
            // It could be possible that we failed to allocate memory for the kernel in the correct
            // place before. Just copy it now to where is belongs.
            // This is *really* unsafe, please see the documentation comment for details.
            unsafe { allocation.move_to_where_it_should_be(
                &mb_mmap_vec, &self.entry.quirks,
            ) };
        }
        // The kernel will need its code and data, so make sure it stays around indefinitely.
        core::mem::forget(self.loaded_kernel.allocations);
        // The kernel is going to need the modules, so make sure they stay.
        core::mem::forget(self.modules_vec);
        // The kernel is going to need the section headers and symbols.
        core::mem::forget(self.loaded_kernel.symbols);
        
        debug!(
            "preparing machine state and jumping to 0x{:x}",
            self.loaded_kernel.entry_address,
        );

        unsafe {
            asm!(
                // The jump to the kernel has to happen in protected mode.
                // If we're built for i686, we already are in protected mode,
                // so this has no effect.
                // If we're built for x86_64, this brings us to compatibility mode.
                ".code32",

                // copy the signature
                "mov ebp, eax",
                // copy the struct address
                "mov esi, ecx",

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
                "mov eax, ebp",
                // write the struct address to EBX
                "mov ebx, esi",
                // finally jump to the kernel
                "jmp edi",
                
                // LLVM needs some registers (https://github.com/rust-lang/rust/blob/1.67.1/compiler/rustc_target/src/asm/x86.rs#L206)
                in("eax") signature,
                in("ecx") &info.as_slice()[0],
                in("edi") self.loaded_kernel.entry_address,
                options(noreturn),
            );
        }
    }
}
