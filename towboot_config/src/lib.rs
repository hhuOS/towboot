//! This library contains configuration structs and functions to load them.
//!
//! The configuration can come from a file or from the command line.
//! The command line options take precedence if they are specified.
#![no_std]
extern crate alloc;

mod config;
pub use config::{Config, Entry, Module, Quirk};

#[cfg(feature = "options")]
mod options;
#[cfg(feature = "options")]
pub use options::{CONFIG_FILE, ConfigSource, LoadOptionKey};
