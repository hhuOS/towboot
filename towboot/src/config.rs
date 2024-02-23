//! This module contains functions to load the configuration.
//!
//! The configuration can come from a file or from the command line.
//! The command line options take precedence if they are specified.
//! 
//! Most of the actual structs can be found in the `towboot_config` crate.
//! The xtask package has its own config.rs.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use uefi::prelude::*;

use towboot_config::{CONFIG_FILE, Config, ConfigSource, parse_load_options};

use super::file::File;

fn version_info() -> String {
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
