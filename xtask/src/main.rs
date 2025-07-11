#![feature(exit_status_error)]
use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process;

use argh::{FromArgs, from_env};
use log::info;

use towbootctl::{BootImageCommand, create_image};

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
    BootImage(BootImageCommand),
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
    fn r#do(self) -> Result<(), Box<dyn Error>> {
        let mut cargo_command = process::Command::new("cargo");
        cargo_command
            .arg("build")
            .arg("--package")
            .arg("towboot");
        if self.release {
            cargo_command.arg("--release");
        }
        if !self.no_i686 {
            info!("building for i686, pass --no-i686 to skip this");
            cargo_command
                .arg("--target")
                .arg("i686-unknown-uefi")
                .status()?.exit_ok()?;
        }
        if !self.no_x86_64 {
            info!("building for x86_64, pass --no-x86-64 to skip this");
            cargo_command
                .arg("--target")
                .arg("x86_64-unknown-uefi")
                .status()?.exit_ok()?;
        }
        let build = if self.release { "release" } else { "debug" };
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

/// This gets started from the command line.
fn main() -> Result<(), Box<dyn Error>> {
    if env::var("RUST_LOG").is_err() {
        unsafe { env::set_var("RUST_LOG", "info"); }
    }
    env_logger::init();
    let args: Cli = from_env();
    match args.command {
        Command::Build(build) => build.r#do(),
        Command::BootImage(boot_image) => boot_image.r#do(),
    }
}
