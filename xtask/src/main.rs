use std::env;
use std::error::Error;

use argh::{FromArgs, from_env};

use towbootctl::{BootImageCommand, ImageCommand};

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
    BootImage(BootImageCommand),
}

/// This gets started from the command line.
fn main() -> Result<(), Box<dyn Error>> {
    if env::var("RUST_LOG").is_err() {
        unsafe { env::set_var("RUST_LOG", "info"); }
    }
    env_logger::init();
    let args: Cli = from_env();
    match args.command {
        Command::Image(image) => image.r#do(),
        Command::BootImage(boot_image) => boot_image.r#do(),
    }
}
