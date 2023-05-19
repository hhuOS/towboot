#![no_std]
#![no_main]
#![feature(naked_functions)]

//! towboot – a bootloader for Multiboot kernels on UEFI systems

extern crate alloc;

use core::str::FromStr;
use alloc::string::ToString;

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::loaded_image::LoadOptionsError;
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
    uefi_services::init(&mut systab).expect("Failed to initialize utilities");
    
    // blocks are so cool, I wish the borrow checker was real
    //
    // So what's happening here is that `open_protocol` returns a
    // `ScopedProtocol` which requires us to (read this as "enforces that we")
    // properly close (eg. drop) all protocols we open before exiting
    // Boot Services. `exit_boot_services` also consumes our `SystemTable` and
    // spits out a new one with less functionality.
    // This closure exists so that we can use protocols inside and they're all
    // gone after.
    //
    // This seems safe and sound but it's not, since we can (and do!) get the
    // opened root volume outside of this. (And I'm pretty sure we are not
    // allowed to use it after exiting Boot Services.)
    // There's also the global singleton `uefi_services::system_table`,
    // but this panics at least if we've exited the Boot Services.
    // (That's why we must never hold a reference to its return value!)
    let (config, mut volume) = {
        // get information about the way we were loaded
        // the interesting thing here is the partition handle
        let loaded_image = systab
            .boot_services()
            .open_protocol_exclusive::<LoadedImage>(image)
            .expect("Failed to open loaded image protocol");
        
        // get the load options
        let load_options = match loaded_image.load_options_as_cstr16() {
            Ok(s) => {
                debug!("got load options: {s:}");
                Some(s.to_string())
            },
            Err(LoadOptionsError::NotSet) => {
                debug!("got no load options");
                None
            },
            Err(e) => {
                warn!("failed to get load options: {e:?}");
                warn!("assuming there were none");
                None
            },
        };
        
        // open the filesystem
        let mut fs = systab
            .boot_services()
            .open_protocol_exclusive::<SimpleFileSystem>(loaded_image.device())
            .expect("Failed to open filesystem");
        let mut volume = fs.open_volume().expect("Failed to open root directory");
        
        let config = match config::get(
            &mut volume, load_options.as_deref(),
        ) {
            Ok(Some(c)) => c,
            Ok(None) => return Status::SUCCESS,
            Err(e) => {
                error!("failed to get config: {e:?}");
                return Status::INVALID_PARAMETER
            }
        };
        if let Some(level) = &config.log_level {
            if let Ok(level) = log::LevelFilter::from_str(level) {
                log::set_max_level(level);
            } else {
                warn!("'{level}' is not a valid log level, using default");
            }
        }
        debug!("config: {config:?}");
        (config, volume)
    };
    let entry_to_boot = menu::choose(&config, &mut systab);
    debug!("okay, trying to load {entry_to_boot:?}");
    info!("loading {entry_to_boot}...");
    
    match boot::PreparedEntry::new(entry_to_boot, image, &mut volume, &systab) {
        Ok(e) => {
            info!("booting {entry_to_boot}...");
            e.boot(image, systab);
            unreachable!();
        },
        Err(e) => {
            error!("failed to prepare the entry: {e:?}");
            return e // give up
            // TODO: perhaps redisplay the menu or something like that
        },
    };
}
