//! This module contains structs and functions to parse the main configuration file.

use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use uefi::prelude::*;
use uefi::proto::media::file::Directory;

use serde::Deserialize;

const CONFIG_FILE: &str = "\\bootloader.toml";

/// Read and parse the config.
pub fn get_config(volume: &mut Directory) -> Result<Config, Status> {
    let text = crate::read_file(CONFIG_FILE, volume)?;
    Ok(toml::from_slice(text.as_slice()).expect("failed to parse config file"))
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
    pub modules: Option<Vec<Module>>,
}

#[derive(Deserialize, Debug)]
pub struct Module {
    pub argv: Option<String>,
    pub image: String,
}
