//! This module handles the actual boot.

use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use core::convert::{identity, TryInto};
use core::fmt::Write;

use uefi::prelude::*;
use uefi::proto::media::file::Directory;
use uefi::table::boot::{AllocateType, MemoryType};

use multiboot1::Addresses;

use crate::config::Entry;

/// Boot an entry.
///
/// What this means:
/// 1. load the kernel into memory
/// 2. try to parse the Multiboot information
/// 3. move the kernel to where it wants to be
/// 4. load the modules
/// 5. make the framebuffer ready
/// 6. create the Multiboot information for the kernel
/// 7. exit BootServices
/// 8. when on x64_64: switch to x86
/// 9. jump!
pub fn boot_entry(entry: &Entry, volume: &mut Directory, image: Handle, systab: SystemTable<Boot>) -> Result<(), ()> {
    let kernel_vec = crate::read_file(&entry.image, volume, &systab)
    .expect("failed to load image");
    let metadata = multiboot1::parse(kernel_vec.as_slice()).expect("invalid Multiboot header");
    writeln!(systab.stdout(), "loaded kernel: {:?}", metadata).unwrap();
    let addresses = match &metadata.addresses {
        Addresses::Multiboot(addr) => addr,
        Addresses::Elf(elf) => todo!("handle ELF addresses")
    };
    
    // try to allocate the memory where to load the kernel and move the kernel there
    // TODO: maybe optimize this so that we at first read just the beginning of the kernel
    // and then read the whole kernel into the right place directly
    // The current implementation is fast enough
    // (we're copying just a few megabytes through memory),
    // but in some cases we could block the destination with the source and this would be bad.
    writeln!(systab.stdout(), "moving the kernel to its desired location...").unwrap();
    // allocate
    let kernel_length: usize = {
        if addresses.bss_end_address == 0 {addresses.load_end_address - addresses.load_address}
        else {addresses.bss_end_address - addresses.load_address}
    }.try_into().unwrap();
    let kernel_ptr = systab.boot_services().allocate_pages(
        AllocateType::Address(addresses.load_address.try_into().unwrap()),
        MemoryType::LOADER_DATA,
        // TODO: this may allocate a page too much
        ((kernel_length / 4096) + 1).try_into().unwrap() // page size
    ).expect("failed to allocate memory to place the kernel").unwrap();
    let kernel_buf = unsafe {
        core::slice::from_raw_parts_mut(kernel_ptr as *mut u8, kernel_length)
    };
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
    // TODO: Don't we lose metadata here? Or did we copy that before?
    
    let modules_vec: Vec<Vec<u8>> = entry.modules.iter().flat_map(identity).map(|module|
        crate::read_file(&module.image, volume, &systab)
        .expect(&format!("failed to load module '{}", module.image).to_string())
    ).collect();
    writeln!(systab.stdout(), "loaded {} modules", modules_vec.len()).unwrap();
    
    
    // TODO: Steps 5 and 6
    
    // allocate memory for the memory map
    // also, keep a bit of room
    let mut mmap_vec = Vec::<u8>::new();
    mmap_vec.resize(systab.boot_services().memory_map_size() + 100, 0);
    let (systab, mmap_iter) = systab.exit_boot_services(image, mmap_vec.as_mut_slice())
    .expect("failed to exit boot services").unwrap();
    // now, write! won't work anymore. Also, we can't allocate any memory.
    
    // TODO: Step 8
    
    // TODO: Not sure whether this works. We don't get any errors.
    let entry_ptr = unsafe {core::mem::transmute::<u32, fn()>(addresses.entry_address)};
    entry_ptr();
    unreachable!();
}
