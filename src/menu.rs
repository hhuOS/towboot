//! Menu goes here.
//!
//! This file is currently mostly empty.
//! If it's ever written, this will contain a menu to select an entry to boot.
//! (And maybe edit it. Who knows?)

use core::fmt::Write;

use uefi::prelude::*;

use crate::config::{Config, Entry};

/// Choose an entry to boot.
///
/// Pass in a parsed config, get out the entry portion that was selected.
/// Currently, this will just return the default entry (as there's no menu yet).
///
/// If the default entry is missing, it will try to use the first one instead.
/// If there are no entries, it will panic.
// TODO: perhaps this should return a Result?
pub fn choose<'a>(config: &'a Config, systab: &SystemTable<Boot>) -> &'a Entry {
    let default_entry = config.entries.get(&config.default).unwrap_or_else(|| {
        writeln!(systab.stdout(), "default entry is missing, trying the first one").unwrap();
        config.entries.values().next().expect("no entries")
    });
    // TODO: display a menu or something like that
    default_entry
}
