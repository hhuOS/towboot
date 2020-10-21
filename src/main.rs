#![no_std]
#![no_main]
#![feature(abi_efiapi)]
#![feature(global_asm)]
#![feature(str_split_once)] // used in config.rs

//! a bootloader for Multiboot kernels on UEFI systems

extern crate rlibc;
extern crate alloc;

use core::convert::TryInto;
use core::str::FromStr;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::proto::media::file::{Directory, File, FileAttribute, FileInfo, FileMode, FileType};

use log::{debug, info, warn, error};

mod boot;
// contains several workarounds for bugs in the Rust UEFI targets
mod hacks;
mod config;
mod menu;

#[entry]
fn efi_main(image: Handle, systab: SystemTable<Boot>) -> Status {
    // Putting this comment above the function breaks the entry annotation.
    //! This is the main function.
    //! Startup happens here.
    uefi_services::init(&systab).expect_success("Failed to initialize utilities");
    
    // get information about the way we were loaded
    // the interesting thing here is the partition handle
    let loaded_image = systab.boot_services()
    .handle_protocol::<LoadedImage>(image)
    .expect_success("Failed to open loaded image protocol");
    let loaded_image = unsafe { &mut *loaded_image.get() };
    
    // get the load options
    let mut load_options_buf: [u8; 2048] = [0; 2048];
    let load_options = match loaded_image.load_options(&mut load_options_buf) {
        Ok(s) => {
            debug!("got load options: {:}", s);
            Some(s)
        },
        Err(e) => {
            warn!("failed to get load options: {:?}", e);
            warn!("assuming there were none");
            None
        },
    };
    
    // open the filesystem
    let fs = systab.boot_services()
    .handle_protocol::<SimpleFileSystem>(loaded_image.device())
    .expect_success("Failed to open filesystem");
    let fs = unsafe { &mut *fs.get() };
    let mut volume = fs.open_volume().expect_success("Failed to open root directory");
    
    let config = config::get_config(&mut volume, load_options).expect("failed to read config");
    if let Some(level) = &config.log_level {
        match log::LevelFilter::from_str(&level) {
            Ok(l) => log::set_max_level(l),
            Err(_) => warn!("'{}' is not a valid log level, using default", level),
        }
    }
    debug!("config: {:?}", config);
    let entry_to_boot = menu::choose(&config, &systab);
    debug!("okay, trying to load {:?}", entry_to_boot);
    
    boot::boot_entry(&entry_to_boot, &mut volume, image, systab).expect("failed to boot the entry");
    // TODO: redisplay the menu or something like that if we end up here again
    
    // We've booted the kernel (or we panicked before), so we aren't here.
    unreachable!();
}


/// Read a whole file into memory and return the resulting byte vector.
///
/// The path is relative to the volume we're loaded from.
///
/// Possible errors:
/// * `Status::NOT_FOUND`: the file does not exist
/// * `Status::UNSUPPORTED`: the given path does exist, but it's a directory
fn read_file(name: &str, volume: &mut Directory) -> Result<Vec<u8>, Status> {
    info!("loading file '{}'...", name);
    let file_handle = match volume.open(name, FileMode::Read, FileAttribute::READ_ONLY) {
        Ok(file_handle) => file_handle.unwrap(),
        Err(e) => return {
            error!("Failed to find file '{}': {:?}", name, e);
            Err(Status::NOT_FOUND)
        }
    };
    let mut file = match file_handle.into_type()
    .expect_success(&format!("Failed to open file '{}'", name).to_string()) {
        FileType::Regular(file) => file,
        FileType::Dir(_) => return {
            error!("File '{}' is a directory", name);
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
    // Vec::with_size would allocate enough space, but won't fill it with zeros.
    // file.read seems to need this.
    let mut content_vec = Vec::<u8>::new();
    content_vec.resize(size, 0);
    let read_size = file.read(content_vec.as_mut_slice())
    .expect_success(&format!("Failed to read from file '{}'", name).to_string());
    assert_eq!(read_size, size);
    Ok(content_vec)
}
