//! This crate offers functionality to use towboot for your own operating system.
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Result};
use log::info;
use tempfile::{NamedTempFile, TempPath};

use towboot_config::Config;

mod bochs;
pub mod config;
mod firmware;
mod image;
use bochs::bochsrc;
use image::Image;

/// How big the image should be
pub const DEFAULT_IMAGE_SIZE: u64 = 50*1024*1024;

/// Where to place the 32-bit EFI file
pub const IA32_BOOT_PATH: &str = "EFI/Boot/bootia32.efi";

/// Where to place the 64-bit EFI file
pub const X64_BOOT_PATH: &str = "EFI/Boot/bootx64.efi";

/// Get the source and destination paths of all files referenced in the config.
fn get_config_files(config: &mut Config) -> Result<Vec<(PathBuf, PathBuf)>> {
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

    Ok(paths)
}

/// Joins a slice of strings.
pub fn runtime_args_to_load_options(runtime_args: &[String]) -> String {
    let mut load_options = "towboot.efi".to_owned();
    for string in runtime_args.iter() {
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
    target: &Path, runtime_args: &[String], i686: Option<&Path>, x86_64: Option<&Path>,
) -> Result<Image> {
    info!("calculating image size");
    let mut paths = Vec::<(PathBuf, PathBuf)>::new();

    // generate a configuration file from the load options
    let load_options = runtime_args_to_load_options(runtime_args);
    let mut config_file = NamedTempFile::new()?;
    if let Some(mut config) = config::get(&load_options)? {
        // get paths to all files referenced by config
        // this also sets the correct config file paths inside the image
        let mut config_paths = get_config_files(&mut config)?;
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

    let mut image_size = 0x00_20_00_00;
    for pair in paths.iter() {
        let file = OpenOptions::new()
            .read(true)
            .open(PathBuf::from(&pair.0))?;
        image_size += file.metadata()?.len();
    }

    info!("creating image at {} (size: {} MiB)", target.display(), image_size / 1024 / 1024);
    let mut image = Image::new(target, image_size)?;
    for pair in paths {
        image.add_file(pair.0.as_path(), pair.1.as_path())?
    }

    Ok(image)
}

/// Boot a built image, returning the running process.
pub fn boot_image(
    firmware: Option<&Path>, image: &Path, is_x86_64: bool, use_bochs: bool,
    use_kvm: bool, use_gdb: bool,
) -> Result<(Command, Vec<TempPath>)> {
    info!("getting firmware");
    let firmware_path = if let Some(path) = firmware {
        assert!(path.exists());
        path.to_path_buf()
    } else {
        match is_x86_64 {
            false => firmware::ia32()?,
            true => firmware::x64()?,
        }
    };
    Ok(if use_bochs {
        info!("spawning Bochs");
        if use_kvm {
            return Err(anyhow!("can't do KVM in Bochs"));
        }
        let config = bochsrc(&firmware_path, image, use_gdb)?.into_temp_path();
        let mut bochs = Command::new("bochs");
        bochs.arg("-qf").arg(config.as_os_str());
        (bochs, vec![config])
    } else {
        info!("spawning QEMU");
        let mut qemu = Command::new(match is_x86_64 {
            false => "qemu-system-i386",
            true => "qemu-system-x86_64",
        });
        qemu
            .arg("-m").arg("256")
            .arg("-hda").arg(image)
            .arg("-serial").arg("stdio")
            .arg("-bios").arg(firmware_path);
        if use_kvm {
            qemu.arg("-machine").arg("pc,accel=kvm");
        }
        if use_gdb {
            info!("The machine starts paused, waiting for GDB to attach to localhost:1234.");
            qemu.arg("-s").arg("-S");
        }
        (qemu, vec![])
    })
}

