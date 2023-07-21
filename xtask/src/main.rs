use cli_xtask::clap;
use cli_xtask::config::Config;
use cli_xtask::{Result, Run, Xtask};

fn main() -> Result<()> {
    Xtask::<Command>::main()
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    Build,
    Run,
}

impl Run for Command {
    fn run(&self, _config: &Config) -> Result<()> {
        match self {
            Self::Build => println!("build!"),
            Self::Run => println!("run!"),
        }
        Ok(())
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
