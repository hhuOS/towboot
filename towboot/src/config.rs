//! This module contains structs and functions to load the configuration.
//!
//! The configuration can come from a file or from the command line.
//! The command line options take precedence if they are specified.
//! 
//! Be aware that is module is also used by `xtask build` to generate a
//! configuration file from the runtime args.
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[cfg(target_os = "uefi")]
use uefi::prelude::*;

#[cfg(not(target_os = "uefi"))]
use super::file::{
    Boot, Handle, SystemTable, Status
};

pub(super) use towboot_config::Config;
use towboot_config::{CONFIG_FILE, ConfigSource, parse_load_options};

use super::file::File;

#[cfg(target_os = "uefi")]
fn version_info() -> String {
    use alloc::format;
    #[allow(dead_code)]
    mod built_info {
        include!(concat!(env!("OUT_DIR"), "/built.rs"));
    }
    format!(
        "This is {} {}{}, built as {} for {} on {}. It is licensed under the {}.",
        built_info::PKG_NAME,
        built_info::GIT_VERSION.unwrap(),
        if built_info::GIT_DIRTY.unwrap() {
            " (dirty)"
        } else {
            ""
        },
        built_info::PROFILE,
        built_info::TARGET,
        built_info::HOST,
        built_info::PKG_LICENSE,
    )
}
#[cfg(not(target_os = "uefi"))]
fn version_info() -> String {
    "(unknown)".to_string()
}

/// Get the config.
/// If we were called with command line options, try them first.
/// Otherwise, read and parse a configuration file.
///
/// Returns None if just a help text has been displayed.
pub fn get(
    image_fs_handle: Handle, load_options: Option<&str>, systab: &SystemTable<Boot>
) -> Result<Option<Config>, Status> {
    let config_source: ConfigSource = match load_options {
        Some(lo) => match parse_load_options(lo, &version_info()) {
            Ok(Some(cs)) => cs,
            Ok(None) => return Ok(None),
            Err(()) => return Err(Status::INVALID_PARAMETER),
        },
        // fall back to the hardcoded config file
        None => ConfigSource::File(CONFIG_FILE.to_string()),
    };
    Ok(Some(match config_source {
        ConfigSource::File(s) => read_file(image_fs_handle, &s, systab)?,
        ConfigSource::Given(c) => c,
    }))
}

/// Try to read and parse the configuration from the given file.
fn read_file(image_fs_handle: Handle, file_name: &str, systab: &SystemTable<Boot>) -> Result<Config, Status> {
    let text: Vec<u8> = File::open(file_name, image_fs_handle, systab)?.try_into()?;
    let mut config: Config = toml::from_slice(text.as_slice()).expect("failed to parse config file");
    config.src = file_name.to_string();
    Ok(config)
}
