//! This module contains structs and functions to load the configuration.
//!
//! The configuration can come from a file or from the command line.
//! The command line options take precedence if they are specified.

use core::convert::TryInto;
use core::fmt::Write;

use alloc::collections::{btree_map::BTreeMap, btree_set::BTreeSet};
use alloc::fmt;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use log::{trace, error};

use uefi::prelude::*;
use uefi::proto::media::file::Directory;

use miniarg::{ArgumentIterator, Key};

use serde::{Deserialize, de::{IntoDeserializer, value}};

use super::file::File;

#[allow(dead_code)]
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
    volume: &mut Directory, systab: &mut SystemTable<Boot>, load_options: Option<&str>
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
fn read_file(volume: &mut Directory, file_name: &str) -> Result<Config, Status> {
    let text: Vec<u8> = File::open(file_name, volume)?.try_into()?;
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
    load_options: &str, systab: &mut SystemTable<Boot>
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
                trace!("option: {} => {}", key, value);
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
                            error!("invalid value for quirk: {}", value);
                            return Err(Status::INVALID_PARAMETER);
                        }
                    },
                    LoadOptionKey::Help => {
                        writeln!(
                            systab.stdout(), "Usage:\n{}", LoadOptionKey::help_text()
                        ).unwrap();
                        return Ok(None)
                    },
                    LoadOptionKey::Version => {
                        writeln!(
                            systab.stdout(),
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
                        ).unwrap();
                        return Ok(None)
                    }
                }
            },
            Err(e) => {
                error!("failed parsing load options: {:?}", e);
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
            entries
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
    Version,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub default: String,
    pub timeout: Option<u8>,
    pub log_level: Option<String>,
    pub entries: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Debug)]
pub struct Entry {
    pub argv: Option<String>,
    pub image: String,
    pub name: Option<String>,
    #[serde(default)]
    pub quirks: BTreeSet<Quirk>,
    #[serde(default)]
    pub modules: Vec<Module>,
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name.as_ref().unwrap_or(&self.image))
    }
}

#[derive(Deserialize, Debug)]
pub struct Module {
    pub argv: Option<String>,
    pub image: String,
}

/// Runtime options to override information in kernel images.
#[derive(Deserialize, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quirk {
    /// Treat the kernel always as an ELF file.
    /// This ignores bit 16 of the kernel's Multiboot header.
    ForceElf,
    /// Ignore the kernel's preferred resolution and just keep the current one.
    KeepResolution,
    /// Place modules below 200 MB.
    ModulesBelow200Mb,
}
