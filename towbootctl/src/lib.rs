//! This crate offers functionality to use towboot for your own operating system.
#![cfg_attr(feature = "args", feature(exit_status_error))]
use std::error::Error;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::anyhow;
#[cfg(feature = "args")]
use argh::FromArgs;
use log::info;
use tempfile::{NamedTempFile, TempPath};

use towboot_config::Config;

mod bochs;
pub mod config;
mod firmware;
mod image;
use bochs::bochsrc;
use image::Image;

/// Where to place the 32-bit EFI file
pub const IA32_BOOT_PATH: &str = "EFI/Boot/bootia32.efi";

/// Where to place the 64-bit EFI file
pub const X64_BOOT_PATH: &str = "EFI/Boot/bootx64.efi";

/// Where to place the AArch64 EFI file
pub const AA64_BOOT_PATH: &str = "EFI/Boot/bootaa64.efi";

/// The firmware architecture used to boot an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    I686,
    X86_64,
    Aarch64,
}

/// Get the source and destination paths of all files referenced in the config.
fn get_config_files(config: &mut Config) -> Vec<(PathBuf, PathBuf)> {
    let mut paths = Vec::<(PathBuf, PathBuf)>::new();
    let mut config_path = PathBuf::from(config.src.clone());
    config_path.pop();

    // go through all needed files; including them (but without the original path)
    for src_file in config.needed_files() {
        let src_path = config_path.join(PathBuf::from(&src_file));
        let dst_file = src_path.file_name().unwrap();
        let dst_path = PathBuf::from(&dst_file);
        src_file.clear();
        src_file.push_str(dst_file.to_str().unwrap());
        paths.push((src_path, dst_path));
    }

    paths
}

/// Joins a slice of strings.
#[must_use]
pub fn runtime_args_to_load_options(runtime_args: &[String]) -> String {
    let mut load_options = "towboot.efi".to_owned();
    for string in runtime_args {
        load_options.push(' ');
        if string.contains(' ') {
            load_options.push('"');
        }
        load_options.push_str(string);
        if string.contains(' ') {
            load_options.push('"');
        }
    }
    load_options
}

/// Create an image, containing a configuration file, kernels, modules and towboot.
pub fn create_image(
    target: &Path,
    runtime_args: &[String],
    i686: Option<&Path>,
    x86_64: Option<&Path>,
    aarch64: Option<&Path>,
) -> Result<Image, Box<dyn Error>> {
    info!("calculating image size");
    let mut paths = Vec::<(PathBuf, PathBuf)>::new();

    // generate a configuration file from the load options
    let load_options = runtime_args_to_load_options(runtime_args);
    let mut config_file = NamedTempFile::new()?;
    if let Some(mut config) = config::get(&load_options)? {
        // get paths to all files referenced by config
        // this also sets the correct config file paths inside the image
        let mut config_paths = get_config_files(&mut config);
        paths.append(&mut config_paths);

        // generate temp config file
        config_file.as_file_mut().write_all(
            toml::to_string(&config)?.as_bytes()
        )?;
        paths.push((PathBuf::from(config_file.path()), PathBuf::from("towboot.toml")));
    }

    // add towboot itself
    if let Some(src) = i686 {
        paths.push((PathBuf::from(src), PathBuf::from(IA32_BOOT_PATH)));
    }
    if let Some(src) = x86_64 {
        paths.push((PathBuf::from(src), PathBuf::from(X64_BOOT_PATH)));
    }
    if let Some(src) = aarch64 {
        paths.push((PathBuf::from(src), PathBuf::from(AA64_BOOT_PATH)));
    }

    let mut image_size = 0;
    for pair in &paths {
        info!("adding {:?} as {:?}", pair.0, pair.1);
        let file = OpenOptions::new()
            .read(true)
            .open(PathBuf::from(&pair.0))?;
        image_size += file.metadata()?.len();
    }

    info!(
        "creating image at {} (size: {} MiB)",
        target.display(),
        image_size.div_ceil(1024).div_ceil(1024),
    );
    let mut image = Image::new(target, image_size)?;
    for pair in paths {
        image.add_file(pair.0.as_path(), pair.1.as_path())?;
    }

    Ok(image)
}

/// Boot a built image, returning the running process.
pub fn boot_image(
    firmware: Option<&Path>, image: &Path, architecture: Architecture, use_bochs: bool,
    use_kvm: bool, use_gdb: bool,
) -> Result<(Command, Vec<TempPath>), Box<dyn Error>> {
    info!("getting firmware");
    Ok(if use_bochs {
        if architecture == Architecture::Aarch64 {
            return Err(anyhow!("Bochs is not supported for AArch64").into());
        }
        let firmware_path = if let Some(path) = firmware {
            if !path.exists() {
                return Err(anyhow!("given firmware path does not exist").into());
            }
            path.to_path_buf()
        } else if architecture == Architecture::X86_64 {
            firmware::x64()?
        } else {
            firmware::ia32()?
        };
        info!("spawning Bochs");
        if use_kvm {
            return Err(anyhow!("can't do KVM in Bochs").into());
        }
        let config = bochsrc(&firmware_path, image, use_gdb)?.into_temp_path();
        let mut bochs = Command::new("bochs");
        bochs.arg("-qf").arg(config.as_os_str());
        (bochs, vec![config])
    } else {
        info!("spawning QEMU");
        let mut temp_files = vec![];
        let mut qemu = if architecture == Architecture::Aarch64 {
            let (firmware_code, firmware_vars_template) = if let Some(path) = firmware {
                if !path.exists() {
                    return Err(anyhow!("given firmware path does not exist").into());
                }
                let (_, vars) = firmware::aarch64()?;
                (path.to_path_buf(), vars)
            } else {
                firmware::aarch64()?
            };
            let vars = NamedTempFile::new()?;
            fs::copy(&firmware_vars_template, vars.path())?;
            temp_files.push(vars.into_temp_path());
            let mut qemu = Command::new("qemu-system-aarch64");
            qemu
                .arg("-machine").arg("virt")
                .arg("-cpu").arg("cortex-a57")
                .arg("-m").arg("256")
                .arg("-serial").arg("stdio")
                .arg("-drive").arg(format!("if=pflash,format=raw,readonly=on,file={}", firmware_code.display()))
                .arg("-drive").arg(format!("if=pflash,format=raw,file={}", temp_files.last().unwrap().display()))
                .arg("-drive").arg(format!("driver=raw,if=none,id=boot,file.filename={}", image.display()))
                .arg("-device").arg("virtio-blk-device,drive=boot");
            qemu
        } else {
            let firmware_path = if let Some(path) = firmware {
                if !path.exists() {
                    return Err(anyhow!("given firmware path does not exist").into());
                }
                path.to_path_buf()
            } else if architecture == Architecture::X86_64 {
                firmware::x64()?
            } else {
                firmware::ia32()?
            };
            let mut qemu = Command::new(if architecture == Architecture::X86_64 {
                "qemu-system-x86_64"
            } else {
                "qemu-system-i386"
            });
            qemu
                .arg("-m").arg("256")
                .arg("-hda").arg(image)
                .arg("-serial").arg("stdio")
                .arg("-bios").arg(firmware_path);
            if use_kvm {
                qemu.arg("-machine").arg("pc,accel=kvm");
            }
            qemu
        };
        if use_gdb {
            info!("The machine starts paused, waiting for GDB to attach to localhost:1234.");
            qemu.arg("-s").arg("-S");
        }
        (qemu, temp_files)
    })
}

#[cfg(feature = "args")]
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "boot-image")]
/// Boot an image.
pub struct BootImageCommand {
    /// what image to boot
    #[argh(option, default = "PathBuf::from(\"image.img\")")]
    image: PathBuf,

    /// use `x86_64` instead of `i686`
    #[argh(switch)]
    x86_64: bool,

    /// use `aarch64` instead of `i686`
    #[argh(switch)]
    aarch64: bool,

    /// enable KVM
    #[argh(switch)]
    kvm: bool,

    /// use Bochs instead of QEMU
    #[argh(switch)]
    bochs: bool,

    /// wait for GDB to attach
    #[argh(switch)]
    gdb: bool,

    /// use the specified firmware instead of OVMF
    #[argh(option)]
    firmware: Option<PathBuf>,

    /// additional arguments to pass to the hypervisor
    #[argh(positional, greedy)]
    args: Vec<String>,
}

#[cfg(feature = "args")]
impl BootImageCommand {
    pub fn r#do(&self) -> Result<(), Box<dyn Error>> {
        let architecture = match (self.x86_64, self.aarch64) {
            (false, false) => Architecture::I686,
            (true, false) => Architecture::X86_64,
            (false, true) => Architecture::Aarch64,
            (true, true) => return Err(anyhow!("choose at most one architecture").into()),
        };
        let (mut process, _temp_files) = boot_image(
            self.firmware.as_deref(), &self.image, architecture, self.bochs,
            self.kvm, self.gdb,
        )?;
        process
            .args(&self.args)
            .status()?
            .exit_ok()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aarch64_bochs_is_rejected() {
        let result = boot_image(
            None,
            Path::new("image.img"),
            Architecture::Aarch64,
            true,
            false,
            false,
        );
        assert!(result.is_err());
    }
}
