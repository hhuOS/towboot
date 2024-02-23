#![no_std]
#![no_main]
#![feature(let_chains)]
#![feature(naked_functions)]

//! towboot – a bootloader for Multiboot kernels on UEFI systems

extern crate alloc;

use core::str::FromStr;
use alloc::string::ToString;

use uefi::fs::PathBuf;
use uefi::data_types::CString16;
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::loaded_image::LoadOptionsError;

use log::{debug, info, warn, error};

mod boot;
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
    log::set_max_level(log::LevelFilter::Info);
    
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
    // This may seem safe and sound but I'm not sure whether it actually is.
    // There's also the global singleton `uefi_services::system_table`,
    // but this panics at least if we've exited the Boot Services.
    // (That's why we must never hold a reference to its return value!)
    let (config, image_fs_handle) = {
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
        
        // get the filesystem
        let image_fs_handle = loaded_image.device().expect("the image to be loaded from a device");
        
        let mut config = match config::get(
            image_fs_handle, load_options.as_deref().unwrap_or_default(), &systab,
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
        // resolve paths relative to the config file itself
        if let Some(config_parent) = PathBuf::from(
            CString16::try_from(config.src.as_str())
                .expect("paths to be valid strings")
        ).parent() {
            for path in config.needed_files() {
                if path.starts_with('\\') {
                    continue
                }
                let mut buf = config_parent.clone();
                buf.push(PathBuf::from(CString16::try_from(path.as_str())
                    .expect("paths to be valid strings")
                ));
                *path = buf.to_string();
            }
        }
        debug!("config: {config:?}");
        (config, image_fs_handle)
    };
    let entry_to_boot = menu::choose(&config, &mut systab);
    debug!("okay, trying to load {entry_to_boot:?}");
    info!("loading {entry_to_boot}...");
    
    match boot::PreparedEntry::new(entry_to_boot, image, image_fs_handle, &systab) {
        Ok(e) => {
            info!("booting {entry_to_boot}...");
            e.boot(systab);
            unreachable!();
        },
        Err(e) => {
            error!("failed to prepare the entry: {e:?}");
            e // give up
            // TODO: perhaps redisplay the menu or something like that
        },
    }
}
