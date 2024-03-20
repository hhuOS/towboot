#![feature(exit_status_error)]
use std::env;
use std::path::PathBuf;
use std::process;

use anyhow::Result;
use argh::{FromArgs, from_env};
use log::info;

use towbootctl::{boot_image, create_image};

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
        let build = match self.release {
            true => "release",
            false => "debug",
        };
        let i686: Option<PathBuf> = (!self.no_i686).then_some(
            ["target", "i686-unknown-uefi", build, "towboot.efi"].into_iter().collect()
        );
        let x86_64: Option<PathBuf> = (!self.no_x86_64).then_some(
            ["target", "x86_64-unknown-uefi", build, "towboot.efi"].into_iter().collect()
        );
        create_image(&self.target, &self.runtime_args, i686.as_deref(), x86_64.as_deref())?;
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
        let (mut process, _temp_files) = boot_image(
            self.firmware.as_deref(), &self.image, self.x86_64, self.bochs,
            self.kvm, self.gdb,
        )?;
        process.status()?.exit_ok()?;
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
