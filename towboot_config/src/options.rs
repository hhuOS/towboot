use alloc::fmt;
use alloc::string::String;
use miniarg::{ArgumentIterator, Key};

use super::Config;

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
