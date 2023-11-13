//! This module handles the actual boot and related stuff.
//!
//! This means: loading kernel and modules, handling ELF files, video initialization and jumping

use alloc::{
    collections::btree_set::BTreeSet,
    format,
    vec,
    vec::Vec,
};
#[cfg(target_arch = "x86_64")]
use x86::{
    dtables::DescriptorTablePointer,
    segmentation::{
        BuildDescriptor, CodeSegmentType, DataSegmentType, Descriptor,
        DescriptorBuilder, SegmentDescriptorBuilder,
    },
};

use core::arch::asm;
use core::ffi::c_void;
use core::ptr::NonNull;
use uefi::prelude::*;
use uefi::proto::console::gop::GraphicsOutput;
use uefi::proto::media::file::Directory;
use uefi::table::boot::{ScopedProtocol, MemoryType};
use uefi::table::cfg::ConfigTableEntry;

use log::{debug, info, error, warn};

use multiboot12::header::Header;
use multiboot12::information::{
    Module, InfoBuilder, Symbols
};

use goblin::elf::Elf;
use uefi_services::system_table;

use super::config::{Entry, Quirk};
use super::file::File;
use super::mem::Allocation;

mod config_tables;
mod elf;
mod video;

use elf::OurElfLoader;

/// A kernel loaded into memory
struct LoadedKernel {
    allocations: Vec<Allocation>,
    entry_point: EntryPoint,
    load_base_address: Option<u32>,
    should_exit_boot_services: bool,
    symbols: Option<(Symbols, Vec<u8>)>,
}

impl LoadedKernel {
    /// Load a kernel from a vector.
    /// This requires that the Multiboot header has already been parsed.
    fn new(
        kernel_vec: Vec<u8>, header: &Header, quirks: &BTreeSet<Quirk>,
    ) -> Result<Self, Status> {
        if header.get_load_addresses().is_some() && !quirks.contains(&Quirk::ForceElf) {
            LoadedKernel::new_multiboot(kernel_vec, header, quirks)
        } else {
            LoadedKernel::new_elf(header, kernel_vec, quirks)
        }
    }
    
    /// Load a kernel which has its addresses specified inside the Multiboot header.
    fn new_multiboot(
        kernel_vec: Vec<u8>, header: &Header, quirks: &BTreeSet<Quirk>,
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

        let entry_point = get_kernel_uefi_entry(header, quirks)
            .or(header.get_entry_address().map(
                |e| EntryPoint::Multiboot(e as usize)
            ))
            .unwrap();
        let should_exit_boot_services = !quirks.contains(&Quirk::DontExitBootServices) && header.should_exit_boot_services();
        
        Ok(Self {
            allocations: vec![allocation],
            entry_point,
            load_base_address: Some(addresses.load_addr()),
            should_exit_boot_services,
            symbols: None,
        })
    }
    
    /// Load a kernel which uses ELF semantics.
    fn new_elf(
        header: &Header, kernel_vec: Vec<u8>, quirks: &BTreeSet<Quirk>,
    ) -> Result<Self, Status> {
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
        let entry_point = get_kernel_uefi_entry(header, quirks)
            .or(header.get_entry_address().map(
                |e| EntryPoint::Multiboot(e as usize)
            ))
            .unwrap_or(EntryPoint::Multiboot(loader.entry_point()));
        let should_exit_boot_services = !quirks.contains(&Quirk::DontExitBootServices) && header.should_exit_boot_services();
        Ok(Self {
            allocations: loader.into(), entry_point, load_base_address: None,
            should_exit_boot_services, symbols,
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

/// Check whether the kernel is compatible to the firmware we are running on.
#[cfg(target_arch = "x86")]
fn get_kernel_uefi_entry(
    header: &Header, quirks: &BTreeSet<Quirk>,
) -> Option<EntryPoint> {
    if let Some(uefi_entry) = header.get_efi32_entry_address() {
        if header.should_exit_boot_services() && !quirks.contains(&Quirk::DontExitBootServices) {
            warn!("The kernel seems to be UEFI-aware but doesn't want us to exit Boot Services.");
            debug!("(The Boot Services tag is missing.)");
            warn!("This is at odds with the Multiboot specification.");
            warn!("So, let's just pretend it isn't UEFI-aware.");
            warn!("(Pass the `DontExitBootServices` quirk to override this.)");
            None
        } else {
            Some(EntryPoint::Uefi(uefi_entry as usize))
        }
    } else {
        None
    }
}

/// Check whether the kernel is compatible to the firmware we are running on.
#[cfg(target_arch = "x86_64")]
fn get_kernel_uefi_entry(
    header: &Header, quirks: &BTreeSet<Quirk>,
) -> Option<EntryPoint> {
    if let Some(uefi_entry) = header.get_efi64_entry_address() {
        if header.should_exit_boot_services() && !quirks.contains(&Quirk::DontExitBootServices) {
            warn!("The kernel seems to be UEFI-aware but doesn't want us to exit Boot Services.");
            debug!("(The Boot Services tag is missing.)");
            warn!("This is at odds with the Multiboot specification.");
            warn!("So, let's just pretend it isn't UEFI-aware.");
            warn!("(Pass the `DontExitBootServices` quirk to override this.)");
            None
        } else {
            Some(EntryPoint::Uefi(uefi_entry as usize))
        }
    } else {
        None
    }
}

/// Prepare information for the kernel.
fn prepare_multiboot_information(
    entry: &Entry, header: Header, load_base_address: Option<u32>,
    modules: &[Allocation], symbols: Option<Symbols>,
    graphics_output: Option<ScopedProtocol<GraphicsOutput>>, image: Handle,
    config_tables: &[ConfigTableEntry], boot_services_exited: bool,
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
    
    if let Some(go) = graphics_output {
        video::prepare_information(&mut info_builder, go);
    }

    // This only has an effect on Multiboot2.
    // TODO: Does this stay valid when we exit Boot Services?
    let systab_ptr = system_table().as_ptr();
    let image_handle_ptr = (unsafe {
        core::mem::transmute::<_, NonNull<c_void>>(image)
    }).as_ptr();
    if cfg!(target_arch = "x86") {
        info_builder.set_system_table_ia32(Some(
            (systab_ptr as usize).try_into().unwrap()
        ));
        info_builder.set_efi_image_handle32(
            (image_handle_ptr as usize).try_into().unwrap()
        );
    } else if cfg!(target_arch = "x86_64") {
        info_builder.set_system_table_x64(Some(
            (systab_ptr as usize).try_into().unwrap()
        ));
        info_builder.set_efi_image_handle64(
            (image_handle_ptr as usize).try_into().unwrap()
        );
    } else {
        warn!("don't know how to pass the UEFI data on this target");
    }

    config_tables::parse_for_multiboot(config_tables, &mut info_builder);

    if !boot_services_exited {
        info_builder.set_boot_services_not_exited();
    }

    if let Some(addr) = load_base_address {
        info_builder.set_image_load_addr(addr);
    }
    
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
        entry: &'a Entry, image: Handle, volume: &mut Directory,
        systab: &SystemTable<Boot>,
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
        
        let graphics_output = video::setup_video(&header, systab, &entry.quirks);
        
        let multiboot_information = prepare_multiboot_information(
            entry, header, loaded_kernel.load_base_address, &modules_vec,
            loaded_kernel.symbols_struct(), graphics_output, image,
            systab.config_table(),
            !entry.quirks.contains(&Quirk::DontExitBootServices),
        );
        
        Ok(PreparedEntry {
            entry, loaded_kernel, multiboot_information, modules_vec,
        })
    }
    
    /// Actually boot an entry.
    ///
    /// What this means:
    /// 1. exit `BootServices` (if needed)
    /// 2. pass the memory map to the kernel
    /// 3. copy the kernel to its desired location (if needed)
    /// 4. bring the machine in the correct state (if needed)
    /// 5. jump!
    ///
    /// This function won't return.
    pub(crate) fn boot(mut self, systab: SystemTable<Boot>) {
        // Estimate the number of memory sections.
        let map_size = systab.boot_services().memory_map_size();
        let estimated_size = map_size.map_size + 500;
        let estimated_count = estimated_size / map_size.entry_size;
        debug!("expecting {estimated_count} memory areas");
        // You may ask yourself why we're not passing map_size.entry_size here.
        // That's because we're passing a slice of uefi.rs' MemoryDescriptors
        // (which hopefully are the same as multiboot2's EFIMemoryDescs),
        // and not the ones the firmware provides us with.
        // (That's also why we can't set the version.)
        // In the future, if uefi.rs might allow us to directly access
        // the returned memory map (including the version!),
        // we might want to pass that instead.
        let mut mb_efi_mmap_vec = self.multiboot_information
            .allocate_efi_memory_map_vec(estimated_count);
        let mut mb_mmap_vec = self.multiboot_information
            .allocate_memory_map_vec(estimated_count);
        self.multiboot_information.set_memory_bounds(Some((0, 0)));
        let (
            mut info, signature, update_memory_info,
        ) = self.multiboot_information.build();
        debug!("passing signature {signature:x} to kernel...");
        let mut mmap_vec = Vec::<u8>::new();
        let memory_map = if self.loaded_kernel.should_exit_boot_services {
            info!("exiting boot services...");
            let (_systab, mut memory_map) = systab.exit_boot_services(MemoryType::LOADER_DATA);
            memory_map.sort();
            // now, write! won't work anymore. Also, we can't allocate any memory.
            memory_map
        } else {
            mmap_vec.resize(estimated_size, 0);
            let memory_map = systab.boot_services().memory_map(mmap_vec.as_mut_slice()).unwrap();
            debug!("got {} memory areas", memory_map.entries().len());
            memory_map
        };
        super::mem::prepare_information(
            &mut info, update_memory_info, &memory_map,
            &mut mb_mmap_vec, &mut mb_efi_mmap_vec,
            self.loaded_kernel.should_exit_boot_services,
        );
        
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
        
        self.loaded_kernel.entry_point.jump(signature, info)
    }
}

/// How to give execution to the kernel
/// 
/// Currently, there are two options: UEFI and Multiboot
enum EntryPoint {
    /// Uefi machine state
    /// 
    /// This is pretty simple: Keep the current state and just pass the
    /// information struct.
    Uefi(usize),
    /// Multiboot machine state
    /// This is pretty complicated (see below).
    Multiboot(usize),
}

impl EntryPoint {
    fn jump(self, signature: u32, info: Vec<u8>) {
        if let Self::Uefi(entry_address) = self {
            self.jump_uefi(entry_address, signature, info)
        } else if let Self::Multiboot(entry_address) = self {
            self.jump_multiboot(entry_address, signature, info)
        } else {
            panic!("invalid entry point")
        }
    }

    fn jump_uefi(self, entry_address: usize, signature: u32, info: Vec<u8>) {
        debug!("jumping to 0x{:x}", entry_address);
        unsafe {
            // TODO: The spec mentions 32 bit registers, even on 64 bit.
            // Do we need to zero the beginning?

            // LLVM needs some registers (https://github.com/rust-lang/rust/blob/1.67.1/compiler/rustc_target/src/asm/x86.rs#L206)
            asm!(
                "mov ebx, ecx",
                "jmp {}",
                in(reg) entry_address,
                in("eax") signature,
                in("ecx") &info.as_slice()[0],
                options(noreturn),
            );
        }
    }

    /// i686-specific part of the Multiboot machine state.
    #[cfg(target_arch = "x86")]
    fn jump_multiboot(self, entry_address: usize, signature: u32, info: Vec<u8>) {
        debug!(
            "preparing machine state and jumping to 0x{:x}", entry_address,
        );

        // 3.2 Machine state says:
        // > ‘EFLAGS’: Bit 17 (VM) must be cleared. Bit 9 (IF) must be cleared.
        // > Other bits are all undefined. 
        // disable interrupts (should have been enabled)
        unsafe { x86::irq::disable() };
        // virtual 8086 mode can't be set, as we're 32 or 64 bit code
        // (and changing that flag is rather difficult)

        // > ‘CS’: Must be a 32-bit read/execute code segment with an offset of ‘0’
        // > and a limit of ‘0xFFFFFFFF’. The exact value is undefined.
        // > 'DS’, 'ES’, ‘FS’, ‘GS’, ‘SS’: Must be a 32-bit read/write data segment with an
        // > offset of ‘0’ and a limit of ‘0xFFFFFFFF’. The exact values are all undefined.
        // We don't set them here as we should already be in the correct state
        // (as opposed to x86_64).


        unsafe {
            asm!(
                // copy the signature
                "mov ebp, eax",
                // copy the struct address
                "mov esi, ecx",
                "jmp {}",
                
                sym Self::jump_multiboot_common,
                // LLVM needs some registers (https://github.com/rust-lang/rust/blob/1.67.1/compiler/rustc_target/src/asm/x86.rs#L206)
                in("eax") signature,
                in("ecx") &info.as_slice()[0],
                in("edi") entry_address,
                options(noreturn),
            );
        }
    }

    /// x86_64-specific part of the Multiboot machine state.
    #[cfg(target_arch = "x86_64")]
    fn jump_multiboot(self, entry_address: usize, signature: u32, info: Vec<u8>) {
        debug!(
            "preparing machine state and jumping to 0x{:x}", entry_address,
        );

        // 3.2 Machine state says:
        // > ‘EFLAGS’: Bit 17 (VM) must be cleared. Bit 9 (IF) must be cleared.
        // > Other bits are all undefined. 
        // disable interrupts (should have been enabled)
        unsafe { x86::irq::disable() };
        // virtual 8086 mode can't be set, as we're 32 or 64 bit code
        // (and changing that flag is rather difficult)

        // > ‘CS’: Must be a 32-bit read/execute code segment with an offset of ‘0’
        // > and a limit of ‘0xFFFFFFFF’. The exact value is undefined.
        // To archieve that, we'll have to set a new GDT and reload
        // the code segment.
        let code_segment_builder: DescriptorBuilder = SegmentDescriptorBuilder::code_descriptor(
            0, u32::MAX, CodeSegmentType::ExecuteRead,
        );
        let code_segment: Descriptor = code_segment_builder
            .present()
            .limit_granularity_4kb()
            .db() // 32 bit
            .finish();
        let data_segment_builder: DescriptorBuilder = SegmentDescriptorBuilder::data_descriptor(
            0, u32::MAX, DataSegmentType::ReadWrite,
        );
        let data_segment: Descriptor = data_segment_builder
            .present()
            .limit_granularity_4kb()
            .db() // 32bit
            .finish();
        let gdt = DescriptorTablePointer::new_from_slice(
            &[Descriptor::NULL, code_segment, data_segment]
        );

        unsafe {
            x86::dtables::lgdt(&gdt);
            // This IDT is invalid (but that's no problem as we already
            // disabled interrupts).
            x86::dtables::lidt::<u32>(&DescriptorTablePointer::default());

            asm!(
                // copy the signature
                "mov ebp, eax",
                // copy the struct address
                "mov esi, ecx",
                
                "push 0x08", // code segment
                "lea rbx, [2f]",
                "push rbx",
                // This "return" allows us to overwrite CS.
                "retfq",

                // We're now in compatibility mode, yay.
                "2:",
                ".code32",
                
                // > 'DS’, 'ES’, ‘FS’, ‘GS’, ‘SS’: Must be a 32-bit read/write data segment with an
                // > offset of ‘0’ and a limit of ‘0xFFFFFFFF’. The exact values are all undefined.
                "mov eax, 0x10", // data segment
                "mov ds, eax",
                "mov es, eax",
                "mov fs, eax",
                "mov gs, eax",
                "mov ss, eax",

                "jmp {}",
                
                sym Self::jump_multiboot_common,
                // LLVM needs some registers (https://github.com/rust-lang/rust/blob/1.67.1/compiler/rustc_target/src/asm/x86.rs#L206)
                in("eax") signature,
                in("ecx") &info.as_slice()[0],
                in("edi") entry_address,
                options(noreturn),
            );
        }
    }

    /// This last part is common for i686 and x86_64.
    #[naked]
    extern "stdcall" fn jump_multiboot_common() {
        unsafe {
            asm!(
                ".code32",
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
                options(noreturn),
            );
        }
    }
}
