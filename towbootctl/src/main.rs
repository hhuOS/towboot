use std::env;

use argh::{FromArgs, from_env};

use anyhow::Result;

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
}

#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "image")]
/// Build a bootable image containing towboot, kernels and their modules.
struct ImageCommand {
    
}

fn main() -> Result<()> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }
    env_logger::init();
    let args: Cli = from_env();
    match args.command {
        Command::Image(image_command) => todo!(),
    }
}
