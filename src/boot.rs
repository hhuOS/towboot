//! This module handles the actual boot.

use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use core::convert::identity;
use core::fmt::Write;

use uefi::prelude::*;
use uefi::proto::media::file::Directory;

use crate::config::Entry;

/// Boot an entry.
///
/// What this means:
/// 1. load the kernel into memory
/// 2. try to parse the Multiboot information
/// 3. load the modules
/// 4. make the framebuffer ready
/// 5. create the Multiboot information for the kernel
/// 6. exit BootServices
/// 7. when on x64_64: switch to x86
/// 8. jump!
pub fn boot_entry(entry: &Entry, volume: &mut Directory, image: Handle, systab: SystemTable<Boot>) -> Result<(), ()> {
    let kernel_vec = crate::read_file(&entry.image, volume, &systab)
    .expect("failed to load image");
    let metadata = multiboot1::parse(kernel_vec.as_slice()).expect("invalid Multiboot header");
    writeln!(systab.stdout(), "loaded kernel: {:?}", metadata).unwrap();
    
    let modules_vec: Vec<Vec<u8>> = entry.modules.iter().flat_map(identity).map(|module|
        crate::read_file(&module.image, volume, &systab)
        .expect(&format!("failed to load module '{}", module.image).to_string())
    ).collect();
    writeln!(systab.stdout(), "loaded {} modules", modules_vec.len()).unwrap();
    
    
    // TODO: Steps 4 and 5
    
    // allocate memory for the memory map
    // also, keep a bit of room
    let mut mmap_vec = Vec::<u8>::new();
    mmap_vec.resize(systab.boot_services().memory_map_size() + 100, 0);
    let (systab, mmap_iter) = systab.exit_boot_services(image, mmap_vec.as_mut_slice())
    .expect("failed to exit boot services").unwrap();
    // now, write! won't work anymore. Also, we can't allocate any memory.
    
    // TODO: Step 7 and 8
    Ok(())
}
