#![no_std]
#![no_main]
#![feature(abi_efiapi)]
#![feature(asm)]
#![feature(result_flattening)] // used in boot/mod.rs

//! towboot – a bootloader for Multiboot kernels on UEFI systems

extern crate rlibc;
extern crate alloc;

use core::str::FromStr;

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::fs::SimpleFileSystem;

use log::{debug, info, warn, error};

mod boot;
// contains several workarounds for bugs in the Rust UEFI targets
mod hacks;
mod config;
mod file;
mod mem;
mod menu;

#[entry]
fn efi_main(image: Handle, mut systab: SystemTable<Boot>) -> Status {
    // Putting this comment above the function breaks the entry annotation.
    //! This is the main function.
    //! Startup happens here.
    uefi_services::init(&mut systab).expect_success("Failed to initialize utilities");
    
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
    
    let config = match config::get_config(&mut volume, &mut systab, load_options) {
        Ok(Some(c)) => c,
        Ok(None) => return Status::SUCCESS,
        Err(e) => {
            error!("failed to get config: {:?}", e);
            return Status::INVALID_PARAMETER
        }
    };
    if let Some(level) = &config.log_level {
        if let Ok(level) = log::LevelFilter::from_str(&level) {
            log::set_max_level(level);
        } else {
            warn!("'{}' is not a valid log level, using default", level);
        }
    }
    debug!("config: {:?}", config);
    let entry_to_boot = menu::choose(&config, &mut systab);
    debug!("okay, trying to load {:?}", entry_to_boot);
    info!("loading {}...", entry_to_boot);
    
    match boot::PreparedEntry::new(&entry_to_boot, &mut volume, &systab) {
        Ok(e) => {
            info!("booting {}...", entry_to_boot);
            e.boot(image, systab);
            unreachable!();
        },
        Err(e) => {
            error!("failed to prepare the entry: {:?}", e);
            return e // give up
            // TODO: perhaps redisplay the menu or something like that
        },
    };
}
