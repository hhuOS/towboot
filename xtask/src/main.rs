use std::path::PathBuf;
use std::process;

use cli_xtask::clap;
use cli_xtask::config::Config;
use cli_xtask::tracing::info;
use cli_xtask::{Result, Run, Xtask};

mod config;
mod image;
use config::get_files_for_config;
use image::Image;

const DEFAULT_IMAGE_SIZE: u64 = 50*1024*1024;

fn main() -> Result<()> {
    Xtask::<Command>::main()
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum Arch {
    I686, X86_64,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Build a bootable image, containing towboot, a kernel and modules
    Build {
        #[arg( long )]
        release: bool,
        #[arg( long )]
        no_i686: bool,
        #[arg( long )]
        no_x86_64: bool,
        #[arg( long, default_value = "towboot.toml" )]
        config: PathBuf,
        #[arg( long, default_value = "image.img" )]
        target: PathBuf,
    },
    Run {
        #[arg( long, default_value = "image.img" )]
        image: PathBuf,
        #[arg( long, default_value = "i686" )]
        arch: Arch,
        #[arg( long )]
        kvm: bool,
        #[arg( long )]
        gdb: bool,
        #[arg( long )]
        /// use the specified firmware instead of OVMF
        firmware: Option<PathBuf>,
    },
}
impl Command {
    fn do_build(&self, release: &bool, no_i686: &bool, no_x86_64: &bool, config: &PathBuf, target: &PathBuf) -> Result<()> {
        let mut cargo_command = process::Command::new("cargo");
        let mut build_command = cargo_command.arg("build");
        if *release {
            build_command = cargo_command.arg("--release");
        }
        if !no_i686 {
            info!("building for i686, pass --no-i686 to skip this");
            build_command
                .arg("--target")
                .arg("i686-unknown-uefi")
                .spawn()?.wait()?;
        }
        if !no_x86_64 {
            info!("building for x86_64, pass --no-x86-64 to skip this");
            build_command
                .arg("--target")
                .arg("x86_64-unknown-uefi")
                .spawn()?.wait()?;
        }
        info!("creating image at {}", target.display());
        let mut image = Image::new(target, DEFAULT_IMAGE_SIZE)?;
        let build = match release {
            true => "release",
            false => "debug",
        };
        if !no_i686 {
            let source: PathBuf = ["target", "i686-unknown-uefi", build, "towboot.efi"].into_iter().collect();
            image.add_file(&source, &PathBuf::from("EFI/Boot/bootia32.efi"))?;
        }
        if !no_x86_64 {
            let source: PathBuf = ["target", "x86_64-unknown-uefi", build, "towboot.efi"].into_iter().collect();
            image.add_file(&source, &PathBuf::from("EFI/Boot/bootx64.efi"))?;
        }
        image.add_file(&config, &PathBuf::from("towboot.toml"))?;
        for file in get_files_for_config(&config)? {
            image.add_file(&file, &file)?;
        }
        Ok(())
    }

    fn do_run(&self, image:&PathBuf, arch: &Arch, kvm: &bool, gdb: &bool, firmware: &Option<PathBuf>) -> Result<()> {
        info!("spawning QEMU");
        let firmware_path: PathBuf = if let Some(path) = firmware {
            assert!(path.exists());
            path.clone()
        } else {
            // TODO: replace this script
            process::Command::new("bash").arg("download.sh")
                .current_dir("ovmf").spawn()?.wait()?;
            ["ovmf", match arch {
                Arch::I686 => "ia32",
                Arch::X86_64 => "x64",
            }, "OVMF.fd"].into_iter().collect()
        };
        let mut qemu_base = process::Command::new(match arch {
            Arch::I686 => "qemu-system-i386",
            Arch::X86_64 => "qemu-system-x86_64",
        });
        let mut qemu = qemu_base
            .arg("-m").arg("256")
            .arg("-hda").arg(image)
            .arg("-serial").arg("stdio")
            .arg("-bios").arg(firmware_path);
        if *kvm {
            qemu = qemu.arg("-machine").arg("pc,accel=kvm,kernel-irqchip=off");
        }
        if *gdb {
            info!("The machine starts paused, waiting for GDB to attach to localhost:1234.");
            qemu = qemu.arg("-s").arg("-S");
        }
        qemu.spawn()?.wait()?;
        Ok(())
    }
}

impl Run for Command {
    fn run(&self, _config: &Config) -> Result<()> {
        match self {
            Self::Build {
                release, no_i686, no_x86_64, config, target,
            } => self.do_build(release, no_i686, no_x86_64, config, target),
            Self::Run {
                image, arch, kvm, gdb, firmware,
            } => self.do_run(image, arch, kvm, gdb, firmware),
        }
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
