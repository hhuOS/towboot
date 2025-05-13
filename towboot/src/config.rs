//! This module contains functions to load the configuration.
//!
//! The configuration can come from a file or from the command line.
//! The command line options take precedence if they are specified.
//! 
//! Most of the actual structs can be found in the [`towboot_config`] crate.
//! The towbootctl package has its own config.rs.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use log::error;
use uefi::prelude::*;

use towboot_config::{Config, ConfigSource, parse_load_options};

use super::file::File;

/// Generate the output for `-version`.
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
    image_fs_handle: Handle, load_options: &str,
) -> Result<Option<Config>, Status> {
    match parse_load_options(load_options, &version_info()) {
        Ok(Some(ConfigSource::File(s))) => Ok(Some(read_file(image_fs_handle, &s)?)),
        Ok(Some(ConfigSource::Given(c))) => Ok(Some(c)),
        Ok(None) => Ok(None),
        Err(()) => Err(Status::INVALID_PARAMETER),
    }
}

/// Try to read and parse the configuration from the given file.
fn read_file(image_fs_handle: Handle, file_name: &str) -> Result<Config, Status> {
    let bytes: Vec<u8> = File::open(file_name, image_fs_handle)?.try_into()?;
    let text = str::from_utf8(&bytes).map_err(|e| {
        error!("configuration file contains invalid bytes: {e:?}");
        Status::UNSUPPORTED
    })?;
    let mut config: Config = toml::from_str(text).map_err(|e| {
        error!("configuration file could not be parsed: {e:?}");
        Status::UNSUPPORTED
    })?;
    config.src = file_name.to_string();
    Ok(config)
}
