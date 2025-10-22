#![no_std]
#![no_main]

//! towboot – a bootloader for Multiboot kernels on UEFI systems

extern crate alloc;

use core::str::FromStr;
use core::time::Duration;
use alloc::string::ToString;

use uefi::prelude::*;
use uefi::boot::{image_handle, open_protocol_exclusive, stall};
use uefi::fs::PathBuf;
use uefi::data_types::CString16;
use uefi::proto::loaded_image::{LoadedImage, LoadOptionsError};

use log::{debug, info, warn, error};

mod boot;
mod config;
mod file;
mod mem;
mod menu;

#[entry]
/// This is the main function. Startup happens here.
fn main() -> Status {
    uefi::helpers::init().expect("Failed to initialize utilities");
    log::set_max_level(log::LevelFilter::Info);

    // get information about the way we were loaded
    // the interesting thing here is the partition handle
    let loaded_image = open_protocol_exclusive::<LoadedImage>(image_handle())
        .expect("Failed to open loaded image protocol");

    // get the load options
    let load_options = match loaded_image.load_options_as_cstr16() {
        Ok(s) => {
            debug!("got load options: {s:}");
            Some(s.to_string())
        }
        Err(LoadOptionsError::NotSet) => {
            debug!("got no load options");
            None
        }
        Err(e) => {
            warn!("failed to get load options: {e:?}");
            warn!("assuming there were none");
            None
        }
    };

    // get the filesystem
    let image_fs_handle = loaded_image.device().expect("the image to be loaded from a device");

    let mut config = match config::get(
        image_fs_handle, load_options.as_deref().unwrap_or_default(),
    ) {
        Ok(Some(c)) => c,
        Ok(None) => return Status::SUCCESS,
        Err(e) => {
            error!("failed to get config: {e:?}");
            return Status::INVALID_PARAMETER;
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
                continue;
            }
            let mut buf = config_parent.clone();
            buf.push(PathBuf::from(CString16::try_from(path.as_str())
                .expect("paths to be valid strings")
            ));
            *path = buf.to_string();
        }
    }
    debug!("config: {config:?}");
    let entry_to_boot = menu::choose(&config);
    debug!("okay, trying to load {entry_to_boot:?}");
    info!("loading {entry_to_boot}...");
    
    match boot::PreparedEntry::new(entry_to_boot, image_fs_handle) {
        Ok(e) => {
            info!("booting {entry_to_boot}...");
            e.boot();
        },
        Err(e) => {
            error!("failed to prepare the entry: {e:?}");
            stall(Duration::from_secs(10));
            e // give up
            // TODO: perhaps redisplay the menu or something like that
        },
    }
}
