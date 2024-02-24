//! A companion utility for towboot.
use std::env;
use std::io::Write;
use std::path::PathBuf;

use argh::{FromArgs, from_env};
use anyhow::Result;
use log::info;
use tempfile::NamedTempFile;

use towbootctl::{add_config_to_image, config, Image, DEFAULT_IMAGE_SIZE, IA32_BOOT_PATH, X64_BOOT_PATH};

#[allow(dead_code)]
mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

#[derive(Debug, FromArgs)]
/// Top-level command.
struct Cli {
    #[argh(subcommand)]
    command: Command,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
enum Command {
    Image(ImageCommand),
    Version(VersionCommand),
}

#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "image")]
/// Build a bootable image containing towboot, kernels and their modules.
struct ImageCommand {
    /// where to place the image
    #[argh(option, default = "PathBuf::from(\"image.img\")")]
    target: PathBuf,

    /// runtime options to pass to towboot
    #[argh(positional, greedy)]
    runtime_args: Vec<String>,
}

impl ImageCommand {
    fn r#do(&self) -> Result<()> {
        info!("creating image at {}", self.target.display());
        let mut image = Image::new(&self.target, DEFAULT_IMAGE_SIZE)?;

        // generate a configuration file from the load options
        let mut load_options = "towboot.efi".to_owned();
        for string in self.runtime_args.iter() {
            load_options.push(' ');
            if string.contains(' ') {
                load_options.push('"');
            }
            load_options.push_str(string);
            if string.contains(' ') {
                load_options.push('"');
            }
        }
        if let Some(mut config) = config::get(&load_options)? {
            add_config_to_image(&mut image, &mut config)?;
        }

        // add towboot itself
        let mut towboot_temp_ia32 = NamedTempFile::new()?;
        towboot_temp_ia32.as_file_mut().write_all(towboot_ia32::TOWBOOT)?;
        image.add_file(
            &towboot_temp_ia32.into_temp_path(), &PathBuf::from(IA32_BOOT_PATH)
        )?;
        let mut towboot_temp_x64 = NamedTempFile::new()?;
        towboot_temp_x64.as_file_mut().write_all(towboot_x64::TOWBOOT)?;
        image.add_file(
            &towboot_temp_x64.into_temp_path(), &PathBuf::from(X64_BOOT_PATH)
        )?;

        Ok(())
    }
}

#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "version")]
/// Display information about this application.
struct VersionCommand {}

impl VersionCommand {
    fn r#do(&self) -> Result<()> {
        println!(
            "This is {} {}{}, built as {} for {} on {}.",
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
        );
        Ok(())
    }
}

fn main() -> Result<()> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }
    env_logger::init();
    let args: Cli = from_env();
    match args.command {
        Command::Image(image_command) => image_command.r#do(),
        Command::Version(version_command) => version_command.r#do(),
    }
}
