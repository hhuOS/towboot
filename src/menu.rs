//! Select an entry to boot by displaying a menu.
use core::fmt::Write;
use alloc::string::String;

use uefi::prelude::*;
use uefi::Completion;
use uefi::proto::console::text::{Key, ScanCode};
use uefi::table::boot::{EventType, TimerTrigger, Tpl};

use log::{error, warn};

use hashbrown::hash_map::HashMap;

use crate::config::{Config, Entry};

/// Choose an entry to boot.
///
/// Pass in a parsed config, get out the entry portion that was selected.
/// This will print a message and then wait for the timeout or for the escape key to be pressed.
/// On timeout, it will boot the default entry.
/// On escape, it will list the available entries and ask which one to boot.
///
/// If the default entry is missing, it will try to use the first one instead.
/// If there are no entries, it will panic.
// TODO: perhaps this should return a Result?
pub fn choose<'a>(config: &'a Config, systab: &SystemTable<Boot>) -> &'a Entry {
    let default_entry = config.entries.get(&config.default).unwrap_or_else(|| {
        warn!("default entry is missing, trying the first one");
        config.entries.values().next().expect("no entries")
    });
    if let Some(0) = config.timeout {
        return default_entry
    }
    match display_menu(config, default_entry, systab) {
        Ok(entry) => entry.log(),
        Err(err) => {
            error!("failed to display menu: {:?}", err);
            warn!("booting default entry");
            default_entry
        }
    }
}

/// Display the menu. This can fail.
fn display_menu<'a>(
    config: &'a Config, default_entry: &'a Entry, systab: &SystemTable<Boot>
) -> uefi::Result<&'a Entry> {
    if let Some(timeout) = config.timeout {
        writeln!(
            systab.stdout(),
            "towboot: booting {} ({}) in {} seconds... (press ESC to change)",
            config.default, default_entry.name.as_ref().unwrap_or(&config.default), timeout,
        ).unwrap();
        // This is safe because there is no callback.
        let timer = unsafe { systab.boot_services().create_event(
            EventType::TIMER, Tpl::APPLICATION, None
        ) }?.log();
        systab.boot_services().set_timer(
            timer, TimerTrigger::Relative(u64::from(timeout) * 10_000_000)
        )?.log();
        let key_event = systab.stdin().wait_for_key_event();
        loop {
            match systab.boot_services().wait_for_event(
                &mut [key_event, timer]
            ).discard_errdata()?.log() {
                // key
                0 => match systab.stdin().read_key()?.log() {
                    Some(Key::Special(ScanCode::ESCAPE)) => break,
                    _ => (),
                },
                // timer
                1 => return Ok(Completion::new(Status::SUCCESS, default_entry)),
                e => warn!("firmware returned invalid event {}", e),
            }
        }
        systab.boot_services().set_timer(timer, TimerTrigger::Cancel)?.log();
    }
    writeln!(systab.stdout(), "available entries:").unwrap();
    for (index, (key, entry)) in config.entries.iter().enumerate() {
        writeln!(
            systab.stdout(), "{}. [{}] {}", index, key, entry.name.as_ref().unwrap_or(key)
        ).unwrap();
    }
    loop {
        match select_entry(&config.entries, &systab) {
            Ok(entry) => return Ok(entry),
            Err(err) => {
                writeln!(systab.stdout(), "invalid choice: {:?}", err).unwrap();
            }
        }
    }
}

/// Try to select an entry.
fn select_entry<'a>(
    entries: &'a HashMap<String, Entry>, systab: &SystemTable<Boot>
) -> uefi::Result<&'a Entry> {
    let mut value = String::new();
    let key_event = systab.stdin().wait_for_key_event();
    loop {
        write!(systab.stdout(), "\rplease select an entry to boot: {} ", value).unwrap();
        systab.boot_services().wait_for_event(&mut [key_event]).discard_errdata()?.log();
        if let Some(Key::Printable(c)) = systab.stdin().read_key()?.log() {
            match c.into() {
                '\r' => break, // enter
                '\u{8}' => {value.pop();}, // backspace
                chr => value.push(chr),
            }
        }
    }
    writeln!(systab.stdout(), ).unwrap();
    // support lookup by both index and key
    match value.parse::<usize>() {
        Ok(index) => entries.values().nth(index),
        Err(_) => entries.get(&value),
    }.ok_or(Status::INVALID_PARAMETER.into()).map(|v| Completion::new(Status::SUCCESS, v))
}
