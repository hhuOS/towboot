use core::fmt;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use alloc::string::String;

use serde::{Deserialize, Serialize};

/// The main configuration struct
#[derive(Deserialize, Debug, Serialize)]
pub struct Config {
    pub default: String,
    pub timeout: Option<u8>,
    pub log_level: Option<String>,
    pub entries: BTreeMap<String, Entry>,
    #[serde(skip)]
    /// the path of the configuration file itself
    pub src: String,
}

impl Config {
    /// Determine which files are referenced in the configuration.
    pub fn needed_files(&mut self) -> Vec<&mut String> {
        let mut files = Vec::new();
        for entry in self.entries.values_mut() {
            files.push(&mut entry.image);
            for module in &mut entry.modules {
                files.push(&mut module.image);
            }
        }
        files
    }
}

/// A menu entry -- an operating system to be booted.
#[derive(Deserialize, Debug, Serialize)]
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

/// Information about a module
#[derive(Deserialize, Debug, Serialize)]
pub struct Module {
    pub argv: Option<String>,
    pub image: String,
}

/// Runtime options to override information in kernel images.
#[derive(Deserialize, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Quirk {
    /// Do not exit Boot Services.
    /// This starts the kernel with more privileges and less available memory.
    /// In some cases this might also display more helpful error messages.
    DontExitBootServices,
    /// Treat the kernel always as an ELF file.
    /// This ignores bit 16 of the kernel's Multiboot header.
    ForceElf,
    /// Ignore the memory map when loading the kernel.
    /// This might damage your hardware!
    ForceOverwrite,
    /// Ignore the kernel's preferred resolution and just keep the current one.
    KeepResolution,
    /// Place modules below 200 MB.
    ModulesBelow200Mb,
}
