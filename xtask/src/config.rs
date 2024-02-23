//! This module contains functions to load the configuration.
//!
//! The configuration can come from a file or from the command line.
//! The command line options take precedence if they are specified.
//!
//! Most of the actual structs can be found in the `towboot_config` crate.
//! The towboot package has its own config.rs.
use std::fs::read_to_string;

use anyhow::{Result, anyhow};

use towboot_config::{Config, ConfigSource, parse_load_options};

/// Get the config.
/// If there are command line options, try them first.
/// Otherwise, read and parse a configuration file.
///
/// Returns None if just a help text has been displayed.
pub fn get(load_options: &str) -> Result<Option<Config>> {
    match parse_load_options(load_options, &"") {
        Ok(Some(ConfigSource::File(s))) => Ok(Some(read_file(&s)?)),
        Ok(Some(ConfigSource::Given(c))) => Ok(Some(c)),
        Ok(None) => Ok(None),
        Err(()) => Err(anyhow!("invalid parameters")),
    }
}

/// Try to read and parse the configuration from the given file.
fn read_file(file_name: &str) -> Result<Config> {
    let text = read_to_string(file_name)?;
    let mut config: Config = toml::from_str(&text).expect("failed to parse config file");
    config.src = file_name.to_string();
    Ok(config)
}
