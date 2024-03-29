#![feature(exit_status_error)]
use std::env;
use std::path::PathBuf;
use std::process;

use anyhow::{Result, anyhow};
use argh::{FromArgs, from_env};
use log::info;

use towbootctl::{add_config_to_image, bochsrc, config, runtime_args_to_load_options, Image, DEFAULT_IMAGE_SIZE, IA32_BOOT_PATH, X64_BOOT_PATH};

#[derive(Debug, FromArgs)]
/// Top-level command.
struct Cli {
    #[argh(subcommand)]
    command: Command,
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
        let mut build_command = cargo_command
            .arg("build")
            .arg("--package")
            .arg("towboot");
        if self.release {
            build_command = cargo_command.arg("--release");
        }
        if !self.no_i686 {
            info!("building for i686, pass --no-i686 to skip this");
            build_command
                .arg("--target")
                .arg("i686-unknown-uefi")
                .status()?.exit_ok()?;
        }
        if !self.no_x86_64 {
            info!("building for x86_64, pass --no-x86-64 to skip this");
            build_command
                .arg("--target")
                .arg("x86_64-unknown-uefi")
                .status()?.exit_ok()?;
        }
        info!("creating image at {}", self.target.display());
        let mut image = Image::new(&self.target, DEFAULT_IMAGE_SIZE)?;
        let build = match self.release {
            true => "release",
            false => "debug",
        };
        if !self.no_i686 {
            let source: PathBuf = ["target", "i686-unknown-uefi", build, "towboot.efi"].into_iter().collect();
            image.add_file(&source, &PathBuf::from(IA32_BOOT_PATH))?;
        }
        if !self.no_x86_64 {
            let source: PathBuf = ["target", "x86_64-unknown-uefi", build, "towboot.efi"].into_iter().collect();
            image.add_file(&source, &PathBuf::from(X64_BOOT_PATH))?;
        }

        // generate a configuration file from the load options
        let load_options = runtime_args_to_load_options(&self.runtime_args);
        if let Some(mut config) = config::get(&load_options)? {
            add_config_to_image(&mut image, &mut config)?;
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

    /// use x86_64 instead of i686
    #[argh(switch)]
    x86_64: bool,

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
}


impl Run {
    fn r#do(self) -> Result<()> {
        info!("getting firmware");
        let firmware_path: PathBuf = if let Some(path) = self.firmware {
            assert!(path.exists());
            path.clone()
        } else {
            // TODO: replace this script
            process::Command::new("bash").arg("download.sh")
                .current_dir("ovmf").status()?.exit_ok()?;
            ["ovmf", match self.x86_64 {
                false => "ia32",
                true => "x64",
            }, "OVMF.fd"].into_iter().collect()
        };
        if self.bochs {
            info!("spawning Bochs");
            if self.kvm {
                return Err(anyhow!("can't do KVM in Bochs"));
            }
            let config = bochsrc(&firmware_path, &self.image, self.gdb)?;
            process::Command::new("bochs")
                .arg("-qf").arg(config.into_temp_path().as_os_str())
                .status()?.exit_ok()?;
        } else {
            info!("spawning QEMU");
            let mut qemu_base = process::Command::new(match self.x86_64 {
                false => "qemu-system-i386",
                true => "qemu-system-x86_64",
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
            qemu.status()?.exit_ok()?;
        }
        Ok(())
    }
}

/// This gets started from the command line.
fn main() -> Result<()> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }
    env_logger::init();
    let args: Cli = from_env();
    match args.command {
        Command::Build(build) => build.r#do(),
        Command::Run(run) => run.r#do(),
    }
}
