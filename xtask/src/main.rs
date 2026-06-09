use std::{env, io::Write};
use std::error::Error;
use std::path::PathBuf;

use argh::{FromArgs, from_env};

use tempfile::NamedTempFile;
use towbootctl::{IA32_IMAGE, X64_IMAGE, BootImageCommand, create_image};

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
    /// where to place the image
    #[argh(option, default = "PathBuf::from(\"image.img\")")]
    target: PathBuf,

    /// runtime options to pass to towboot
    #[argh(positional, greedy)]
    runtime_args: Vec<String>,
}

impl Build {
    fn r#do(self) -> Result<(), Box<dyn Error>> {
        let mut temp_ia32 = NamedTempFile::new()?;
        temp_ia32.as_file_mut().write_all(IA32_IMAGE)?;
        let mut temp_x64 = NamedTempFile::new()?;
        temp_x64.as_file_mut().write_all(X64_IMAGE)?;
        let temp_ia32_path = temp_ia32.into_temp_path();
        let temp_x64_path = temp_x64.into_temp_path();
        create_image(&self.target, &self.runtime_args, Some(&temp_ia32_path), Some(&temp_x64_path))?;
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
