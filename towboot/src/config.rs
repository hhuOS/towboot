//! This module contains structs and functions to load the configuration.
//!
//! The configuration can come from a file or from the command line.
//! The command line options take precedence if they are specified.
//! 
//! Be aware that is module is also used by `xtask build` to generate a
//! configuration file from the runtime args.

use alloc::collections::{btree_map::BTreeMap, btree_set::BTreeSet};
use alloc::fmt;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use log::{trace, error};

#[cfg(target_os = "uefi")]
use {
    uefi::prelude::*,
    uefi_services::println,
};

#[cfg(not(target_os = "uefi"))]
use super::file::{
    Boot, Handle, SystemTable, Status
};

use miniarg::{ArgumentIterator, Key};

use serde::Deserialize;
use serde::de::{IntoDeserializer, value};

pub(super) use towboot_config::{Config, Entry, Module, Quirk};

use super::file::File;

#[allow(dead_code)]
#[cfg(target_os = "uefi")]
mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

const CONFIG_FILE: &str = "\\towboot.toml";

/// Get the config.
/// If we were called with command line options, try them first.
/// Otherwise, read and parse a configuration file.
///
/// Returns None if just a help text has been displayed.
pub fn get(
    image_fs_handle: Handle, load_options: Option<&str>, systab: &SystemTable<Boot>
) -> Result<Option<Config>, Status> {
    let config_source: ConfigSource = match load_options {
        Some(lo) => match parse_load_options(lo)? {
            Some(cs) => cs,
            None => return Ok(None),
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

/// Parse the command line options.
///
/// See [`LoadOptionKey`] for available options.
///
/// This function errors, if the command line options are not valid.
/// That is:
/// * general reasons
/// * keys without values
/// * values without keys
/// * invalid keys
///
/// [`LoadOptionKey`]: enum.LoadOptionKey.html
fn parse_load_options(
    load_options: &str,
) -> Result<Option<ConfigSource>, Status> {
    let options = LoadOptionKey::parse(load_options);
    let mut config_file = None;
    let mut kernel = None;
    let mut log_level = None;
    let mut modules = Vec::<&str>::new();
    let mut quirks = BTreeSet::<Quirk>::new();
    for option in options {
        match option {
            Ok((key, value)) => {
                trace!("option: {key} => {value}");
                match key {
                    LoadOptionKey::Config => config_file = Some(value),
                    LoadOptionKey::Kernel => kernel = Some(value),
                    LoadOptionKey::LogLevel => log_level = Some(value),
                    LoadOptionKey::Module => modules.push(value),
                    LoadOptionKey::Quirk => {
                        let parsed: Result<Quirk, value::Error> = Quirk::deserialize(
                            value.into_deserializer()
                        );
                        if let Ok(parsed) = parsed {
                            quirks.insert(parsed);
                        } else {
                            error!("invalid value for quirk: {value}");
                            return Err(Status::INVALID_PARAMETER);
                        }
                    },
                    LoadOptionKey::Help => {
                        println!("Usage:\n{}", LoadOptionKey::help_text());
                        return Ok(None)
                    }
                    #[cfg(target_os = "uefi")]
                    LoadOptionKey::Version => {
                        println!(
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
                        );
                        return Ok(None)
                    }
                }
            },
            Err(e) => {
                error!("failed parsing load options: {e:?}");
                return Err(Status::INVALID_PARAMETER)
            },
        }
    }
    if let Some(kernel) = kernel {
        let modules = modules.iter().map(|m| {
            let (image, argv) = m.split_once(' ').unwrap_or((m, ""));
            Module {
                image: image.to_string(),
                argv: Some(argv.to_string()),
            }
        }).collect();
        let (kernel_image, kernel_argv) = kernel.split_once(' ').unwrap_or((kernel, ""));
        let mut entries = BTreeMap::new();
        entries.insert("cli".to_string(), Entry {
            argv: Some(kernel_argv.to_string()),
            image: kernel_image.to_string(),
            name: None,
            quirks,
            modules,
        });
        Ok(Some(ConfigSource::Given(Config {
            default: "cli".to_string(),
            timeout: Some(0),
            log_level: log_level.map(ToString::to_string),
            entries,
            src: ".".to_string(), // TODO: put the CWD here
        })))
    } else if let Some(c) = config_file {
        Ok(Some(ConfigSource::File(c.to_string())))
    } else {
        Ok(Some(ConfigSource::File(CONFIG_FILE.to_string())))
    }
}

enum ConfigSource {
    File(String),
    Given(Config),
}

/// Available options.
#[derive(Debug, Key)]
enum LoadOptionKey {
    /// Load the specified configuration file instead of the default one.
    Config,
    /// Don't load a configuration file, instead boot the specified kernel.
    Kernel,
    /// Set the log level. (This only applies if `-kernel` is specified.)
    LogLevel,
    /// Load a module with the given args. Can be specified multiple times.
    Module,
    /// Enable a specific quirk. (Only applies when loading a kernel.)
    Quirk,
    /// Displays all available options and how to use them.
    Help,
    /// Displays the version of towboot
    #[cfg(target_os = "uefi")]
    Version,
}
