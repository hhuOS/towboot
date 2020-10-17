#![no_std]
#![no_main]
#![feature(abi_efiapi)]
#![feature(global_asm)]

extern crate rlibc;
extern crate alloc;

use core::convert::TryInto;
use core::fmt::Write;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::proto::media::file::{Directory, File, FileAttribute, FileInfo, FileMode, FileType};

// contains several workarounds for bugs in the Rust UEFI targets
mod hacks;
mod config;
mod menu;

#[entry]
fn efi_main(image: Handle, systab: SystemTable<Boot>) -> Status {
    uefi_services::init(&systab).expect_success("Failed to initialize utilities");
    writeln!(systab.stdout(), "Hello, world!").unwrap();
    
    // get information about the way we were loaded
    // the interesting thing here is the partition handle
    let loaded_image = systab.boot_services()
    .handle_protocol::<LoadedImage>(image)
    .expect_success("Failed to open loaded image protocol");
    let loaded_image = unsafe { &mut *loaded_image.get() };
    
    // open the filesystem
    let fs = systab.boot_services()
    .handle_protocol::<SimpleFileSystem>(loaded_image.device())
    .expect_success("Failed to open filesystem");
    let fs = unsafe { &mut *fs.get() };
    let volume = fs.open_volume().expect_success("Failed to open root directory");
    
    let config = config::get_config(volume, &systab).expect("failed to read config");
    writeln!(systab.stdout(), "config: {:?}", config).unwrap();
    let entry_to_boot = menu::choose(&config, &systab);
    writeln!(systab.stdout(), "okay, trying to load {:?}", entry_to_boot).unwrap();
    
    Status::SUCCESS
}

fn read_file(name: &str, mut volume: Directory, systab: &SystemTable<Boot>) -> Result<Vec<u8>, Status> {
    let file_handle = match volume.open(name, FileMode::Read, FileAttribute::READ_ONLY) {
        Ok(file_handle) => file_handle.unwrap(),
        Err(e) => return {
            writeln!(systab.stdout(), "Failed to find file '{}': {:?}", name, e).unwrap();
            Err(Status::NOT_FOUND)
        }
    };
    let mut file = match file_handle.into_type()
    .expect_success(&format!("Failed to open file '{}'", name).to_string()) {
        FileType::Regular(file) => file,
        FileType::Dir(_) => return {
            writeln!(systab.stdout(), "File '{}' is a directory", name).unwrap();
            Err(Status::UNSUPPORTED)
        }
    };
    let mut info_vec = Vec::<u8>::new();
    
    // we try to get the metadata with a zero-sized buffer
    // this should throw BUFFER_TOO_SMALL and give us the needed size
    let info_result = file.get_info::<FileInfo>(info_vec.as_mut_slice());
    assert_eq!(info_result.status(), Status::BUFFER_TOO_SMALL);
    let info_size: usize = info_result.expect_err("metadata is 0 bytes").data()
    .expect("failed to get size of file metadata");
    info_vec.resize(info_size, 0);
    
    let size: usize = file.get_info::<FileInfo>(info_vec.as_mut_slice())
    .expect(&format!("Failed to get metadata of file '{}'", name).to_string())
    .unwrap().file_size().try_into().unwrap();
    writeln!(systab.stdout(), "File opened.").unwrap();
    // Vec::with_size would allocate enough space, but won't fill it with zeros.
    // file.read seems to need this.
    let mut content_vec = Vec::<u8>::new();
    content_vec.resize(size, 0);
    let read_size = file.read(content_vec.as_mut_slice())
    .expect_success(&format!("Failed to read from file '{}'", name).to_string());
    assert_eq!(read_size, size);
    Ok(content_vec)
}
