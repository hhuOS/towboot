//! This module contains structs and functions to load the configuration.
//!
//! The configuration can come from a file or from the command line.
//! The command line options take precedence if they are specified.

use core::fmt::Write;

use alloc::fmt;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use log::{trace, error};

use uefi::prelude::*;
use uefi::proto::media::file::Directory;

use hashbrown::hash_map::HashMap;

use miniarg::{ArgumentIterator, Key};

use serde::Deserialize;

use super::file::File;

const CONFIG_FILE: &str = "\\bootloader.toml";

/// Get the config.
/// If we were called with command line options, try them first.
/// Otherwise, read and parse a configuration file.
///
/// Returns None if just a help text has been displayed.
pub fn get_config(
    volume: &mut Directory, systab: &SystemTable<Boot>, load_options: Option<&str>
) -> Result<Option<Config>, Status> {
    let config_source: ConfigSource = match load_options {
        Some(lo) => match parse_load_options(lo, systab)? {
            Some(cs) => cs,
            None => return Ok(None),
        },
        // fall back to the hardcoded config file
        None => ConfigSource::File(CONFIG_FILE.to_string()),
    };
    Ok(Some(match config_source {
        ConfigSource::File(s) => read_file(volume, &s)?,
        ConfigSource::Given(c) => c,
    }))
}

/// Try to read and parse the configuration from the given file.
fn read_file<'a>(volume: &mut Directory, file_name: &'a str) -> Result<Config, Status> {
    let text: Vec<u8> = File::open(file_name, volume)?.into();
    Ok(toml::from_slice(text.as_slice()).expect("failed to parse config file"))
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
    load_options: &str, systab: &SystemTable<Boot>
) -> Result<Option<ConfigSource>, Status> {
    let options = LoadOptionKey::parse(&load_options);
    let mut config_file = None;
    let mut kernel = None;
    let mut log_level = None;
    let mut modules = Vec::<&str>::new();
    for option in options {
        match option {
            Ok((key, value)) => {
                trace!("option: {} => {}", key, value);
                match key {
                    LoadOptionKey::Config => config_file = Some(value),
                    LoadOptionKey::Kernel => kernel = Some(value),
                    LoadOptionKey::LogLevel => log_level = Some(value),
                    LoadOptionKey::Module => modules.push(value),
                    LoadOptionKey::Help => {
                        writeln!(
                            systab.stdout(), "Usage:\n{}", LoadOptionKey::help_text()
                        ).unwrap();
                        return Ok(None)
                    },
                }
            },
            Err(e) => {
                error!("failed parsing load options: {:?}", e);
                return Err(Status::INVALID_PARAMETER)
            },
        }
    }
    if let Some(kernel) = kernel {
        let mods = modules.iter().map(|m| {
            let (image, argv) = m.split_once(" ").unwrap_or((m, ""));
            Module {
                image: image.to_string(),
                argv: Some(argv.to_string()),
            }
        }).collect();
        let (kernel_image, kernel_argv) = kernel.split_once(" ").unwrap_or((kernel, ""));
        let mut entries = HashMap::new();
        entries.insert("cli".to_string(), Entry {
            argv: Some(kernel_argv.to_string()),
            image: kernel_image.to_string(),
            name: None,
            modules: Some(mods),
        });
        Ok(Some(ConfigSource::Given(Config {
            default: "cli".to_string(),
            timeout: Some(0),
            log_level: log_level.map(|l| l.to_string()),
            entries
        })))
    } else if config_file.is_some() {
        Ok(Some(ConfigSource::File(config_file.unwrap().to_string())))
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
    /// Displays all available options and how to use them.
    Help,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub default: String,
    pub timeout: Option<u8>,
    pub log_level: Option<String>,
    pub entries: HashMap<String, Entry>,
}

#[derive(Deserialize, Debug)]
pub struct Entry {
    pub argv: Option<String>,
    pub image: String,
    pub name: Option<String>,
    pub modules: Option<Vec<Module>>,
}

#[derive(Deserialize, Debug)]
pub struct Module {
    pub argv: Option<String>,
    pub image: String,
}
