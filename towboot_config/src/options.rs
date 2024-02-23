use alloc::collections::{btree_map::BTreeMap, btree_set::BTreeSet};
use alloc::fmt;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use log::{info, error, trace};
use miniarg::{ArgumentIterator, Key};
use serde::Deserialize;
use serde::de::{IntoDeserializer, value};

use super::{Config, Entry, Module, Quirk};

/// The default path to the configuration file.
pub const CONFIG_FILE: &str = "towboot.toml";

/// Where to load the configuration from
pub enum ConfigSource {
    File(String),
    Given(Config),
}

/// Available options.
#[derive(Debug, Key)]
pub enum LoadOptionKey {
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

/// Parse the command line options.
///
/// See [`LoadOptionKey`] for available options.
///
/// This function returns None if the user just asked for help or the version.
/// This function errors, if the command line options are not valid.
/// That is:
/// * general reasons
/// * keys without values
/// * values without keys
/// * invalid keys
///
/// [`LoadOptionKey`]: enum.LoadOptionKey.html
pub fn parse_load_options(
    load_options: &str,
    #[allow(unused_variables)]
    version_info: &str,
) -> Result<Option<ConfigSource>, ()> {
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
                            return Err(());
                        }
                    },
                    LoadOptionKey::Help => {
                        info!("Usage:\n{}", LoadOptionKey::help_text());
                        return Ok(None)
                    }
                    #[cfg(target_os = "uefi")]
                    LoadOptionKey::Version => {
                        info!("{}", version_info);
                        return Ok(None)
                    }
                }
            },
            Err(e) => {
                error!("failed parsing load options: {e:?}");
                return Err(())
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
