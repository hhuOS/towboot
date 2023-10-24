extern crate alloc;

use std::io::Write;
use std::env;
use std::path::PathBuf;
use std::process;

use anyhow::{Result, anyhow};
use argh::{FromArgs, from_env};
use log::info;
use tempfile::NamedTempFile;

mod config;
mod file;
mod image;
use image::Image;

const DEFAULT_IMAGE_SIZE: u64 = 50*1024*1024;

#[derive(Debug, FromArgs)]
/// Top-level command.
struct Cli {
    #[argh(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, strum::EnumString, strum::Display)]
enum Arch {
    I686, X86_64,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
enum Command {
    Build(Build),
    Run(Run),
}

#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "build")]
/// Build a bootable image containing, towboot, kernels and their modules.
struct Build {
    /// do release builds
    #[argh(switch)]
    release: bool,

    /// do not include i686 build
    #[argh(switch)]
    no_i686: bool,

    /// do not include x86_64 build
    #[argh(switch)]
    no_x86_64: bool,

    /// where to place the image
    #[argh(option, default = "PathBuf::from(\"image.img\")")]
    target: PathBuf,

    /// runtime options to pass to towboot
    #[argh(positional, greedy)]
    runtime_args: Vec<String>,
}

impl Build {
    fn r#do(self) -> Result<()> {
        let mut cargo_command = process::Command::new("cargo");
        let mut build_command = cargo_command.arg("build");
        if self.release {
            build_command = cargo_command.arg("--release");
        }
        if !self.no_i686 {
            info!("building for i686, pass --no-i686 to skip this");
            build_command
                .arg("--target")
                .arg("i686-unknown-uefi")
                .spawn()?.wait()?;
        }
        if !self.no_x86_64 {
            info!("building for x86_64, pass --no-x86-64 to skip this");
            build_command
                .arg("--target")
                .arg("x86_64-unknown-uefi")
                .spawn()?.wait()?;
        }
        info!("creating image at {}", self.target.display());
        let mut image = Image::new(&self.target, DEFAULT_IMAGE_SIZE)?;
        let build = match self.release {
            true => "release",
            false => "debug",
        };
        if !self.no_i686 {
            let source: PathBuf = ["target", "i686-unknown-uefi", build, "towboot.efi"].into_iter().collect();
            image.add_file(&source, &PathBuf::from("EFI/Boot/bootia32.efi"))?;
        }
        if !self.no_x86_64 {
            let source: PathBuf = ["target", "x86_64-unknown-uefi", build, "towboot.efi"].into_iter().collect();
            image.add_file(&source, &PathBuf::from("EFI/Boot/bootx64.efi"))?;
        }

        // generate a configuration file from the load options
        let mut load_options = "towboot.efi".to_owned();
        if self.runtime_args.is_empty() {
            load_options.push_str(" -config towboot.toml");
        }
        for string in self.runtime_args.iter() {
            load_options.push(' ');
            if string.contains(" ") {
                load_options.push('"');
            }
            load_options.push_str(string);
            if string.contains(" ") {
                load_options.push('"');
            }
        }
        if let Some(config) = config::get(&mut PathBuf::from(""), Some(&load_options))? {
            // write it (and all files referenced inside) to the image
            let mut config_file = NamedTempFile::new()?;
            config_file.as_file_mut().write_all(
                toml::to_string(&config)?.as_bytes()
            )?;
            image.add_file(&config_file.into_temp_path().to_path_buf(), &PathBuf::from("towboot.toml"))?;
            for file in config.needed_files()
                .map_err(|msg| anyhow!("{}", msg))? {
                let path = PathBuf::from(file);
                image.add_file(&path, &path)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "run")]
/// Run an image in a VM
struct Run {
    /// what image to boot
    #[argh(option, default = "PathBuf::from(\"image.img\")")]
    image: PathBuf,

    /// what architecture to use for the VM
    #[argh(option, default = "Arch::I686")]
    arch: Arch,

    /// enable KVM
    #[argh(switch)]
    kvm: bool,

    /// wait for GDB to attach
    #[argh(switch)]
    gdb: bool,

    /// use the specified firmware instead of OVMF
    #[argh(option)]
    firmware: Option<PathBuf>,
}


impl Run {
    fn r#do(self) -> Result<()> {
        info!("spawning QEMU");
        let firmware_path: PathBuf = if let Some(path) = self.firmware {
            assert!(path.exists());
            path.clone()
        } else {
            // TODO: replace this script
            process::Command::new("bash").arg("download.sh")
                .current_dir("ovmf").spawn()?.wait()?;
            ["ovmf", match self.arch {
                Arch::I686 => "ia32",
                Arch::X86_64 => "x64",
            }, "OVMF.fd"].into_iter().collect()
        };
        let mut qemu_base = process::Command::new(match self.arch {
            Arch::I686 => "qemu-system-i386",
            Arch::X86_64 => "qemu-system-x86_64",
        });
        let mut qemu = qemu_base
            .arg("-m").arg("256")
            .arg("-hda").arg(self.image)
            .arg("-serial").arg("stdio")
            .arg("-bios").arg(firmware_path);
        if self.kvm {
            qemu = qemu.arg("-machine").arg("pc,accel=kvm,kernel-irqchip=off");
        }
        if self.gdb {
            info!("The machine starts paused, waiting for GDB to attach to localhost:1234.");
            qemu = qemu.arg("-s").arg("-S");
        }
        qemu.spawn()?.wait()?;
        Ok(())
    }
}

fn main() -> Result<()> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();
    let args: Cli = from_env();
    match args.command {
        Command::Build(build) => build.r#do(),
        Command::Run(run) => run.r#do(),
    }
}
