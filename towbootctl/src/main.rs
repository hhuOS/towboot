//! A companion utility for towboot.
use std::fs;
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};

use argh::{FromArgs, from_env};
use anyhow::Result;
use log::info;
use tempfile::NamedTempFile;

use towbootctl::{add_config_to_image, config, runtime_args_to_load_options, Image, DEFAULT_IMAGE_SIZE, IA32_BOOT_PATH, X64_BOOT_PATH};

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
    Install(InstallCommand),
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
        let load_options = runtime_args_to_load_options(&self.runtime_args);
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
#[argh(subcommand, name = "install")]
/// Install towboot, the configuration file, its kernels and modules to a disk.
struct InstallCommand {
    /// whether to do a removable install (meaning writing to /EFI/BOOT/)
    #[argh(switch)]
    removable: bool,

    /// whether to register the install with the firmware (otherwise it can only be chain-loaded)
    #[argh(switch)]
    register: bool,

    /// the operating system's name
    /// This is being used as the folder name inside /EFI and as the name for
    /// the boot entry.
    #[argh(option)]
    name: Option<String>,

    #[argh(positional)]
    /// the root of the mounted ESP
    esp_path: PathBuf,

    /// runtime options to pass to towboot
    #[argh(positional, greedy)]
    runtime_args: Vec<String>,
}

impl InstallCommand {
    fn r#do(&self) -> Result<()> {
        assert!(self.esp_path.is_dir());
        let mut install_path = self.esp_path.clone();
        install_path.push("EFI");
        if !install_path.exists() {
            fs::create_dir(&install_path)?;
        }
        install_path.push(if self.removable {
            "BOOT"
        } else {
            self.name.as_ref().expect("non-removable installs must have a name")
        });
        if !install_path.exists() {
            fs::create_dir(&install_path)?;
        }
        info!("installing to {}", install_path.display());
        if !self.runtime_args.is_empty() {
            let load_options = runtime_args_to_load_options(&self.runtime_args);
            if let Some(mut config) = config::get(&load_options)? {
                // Write the given configuration to the ESP.
                let mut config_path = PathBuf::from(config.src.clone());
                config_path.pop();
                // go through all needed files; including them (but without the original path)
                for src_file in config.needed_files() {
                    let src_path = config_path.join(PathBuf::from(&src_file));
                    let dst_file = src_path.file_name().unwrap();
                    let mut dst_path = if self.removable {
                        self.esp_path.clone()
                    } else {
                        install_path.clone()
                    };
                    dst_path.push(&dst_file);
                    src_file.clear();
                    src_file.push_str(dst_file.to_str().unwrap());
                    fs::copy(&src_path, &dst_path)?;
                }
                // write the configuration itself
                let mut config_path = if self.removable {
                    self.esp_path.clone()
                } else {
                    install_path.clone()
                };
                config_path.push("towboot.toml");
                fs::write(&config_path, toml::to_vec(&config)?)?;
            } else {
                // Exit if the options were just -help.
                return Ok(())
            }
        }
        // add towboot itself
        // TODO: rename this maybe for non-removable installs?
        fs::write(Path::join(&install_path, "BOOTIA32.efi"), towboot_ia32::TOWBOOT)?;
        fs::write(Path::join(&install_path, "BOOTX64.efi"), towboot_x64::TOWBOOT)?;
        if self.register {
            assert!(!self.removable);
            todo!("registration with the firmware is not supported, yet");
        }
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
        Command::Install(install_command) => install_command.r#do(),
        Command::Version(version_command) => version_command.r#do(),
    }
}
