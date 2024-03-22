//! This crate contains integration tests.
#![cfg(test)]
#![feature(exit_status_error)]
use std::{path::{Path, PathBuf}, process::{Command, Stdio}, thread::sleep, time::Duration};

use anyhow::Result;
use tempfile::NamedTempFile;
use towbootctl::{boot_image, create_image};

#[derive(PartialEq, Clone, Copy)]
enum Arch {
    I686,
    X86_64,
}

#[cfg(test)]
#[ctor::ctor]
fn init() {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "warn");
    }
    env_logger::init();
}

/// Builds the given folder as an image and boots it.
fn build_and_boot(
    folder: &Path, towboot_arch: Arch, machine_arch: Arch, firmware_arch: Arch, release: bool,
) -> Result<String> {
    // ensure we have a current build of towboot
    let mut cargo_command = Command::new("cargo");
    cargo_command
        .arg("build")
        .arg("--package")
        .arg("towboot");
    if release {
        cargo_command.arg("--release");
    }
    cargo_command.arg("--target").arg(match towboot_arch {
        Arch::I686 => "i686-unknown-uefi",
        Arch::X86_64 => "x86_64-unknown-uefi",
    });
    cargo_command.status()?.exit_ok()?;
    let build = match release {
        true => "release",
        false => "debug",
    };
    let i686: Option<PathBuf> = matches!(towboot_arch, Arch::I686).then_some(
        ["..", "target", "i686-unknown-uefi", build, "towboot.efi"].into_iter().collect()
    );
    let x86_64: Option<PathBuf> = matches!(towboot_arch, Arch::X86_64).then_some(
        ["..", "target", "x86_64-unknown-uefi", build, "towboot.efi"].into_iter().collect()
    );

    // make sure that the kernel is built
    Command::new("make")
        .current_dir(folder)
        .status()?.exit_ok()?;

    // build the image
    let image_path = NamedTempFile::new()?.into_temp_path();
    let mut config_path = folder.to_path_buf();
    config_path.push("towboot.toml");
    create_image(
        &image_path, &[
            "-config".to_string(),
            config_path.to_str().unwrap().to_string(),
        ], i686.as_deref(), x86_64.as_deref(),
    )?;

    // boot it
    assert!(firmware_arch == machine_arch); // TODO
    let (mut qemu_command, _temp_files) = boot_image(
        None,
        &image_path,
        matches!(machine_arch, Arch::X86_64),
        false,
        true, // the firmware seems to boot only on KVM
        false,
    )?;
    let mut qemu_process = qemu_command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .arg("-display").arg("none")
        .spawn()?;
    sleep(Duration::from_secs(5)); // TODO: kernels should probably terminate the VM
    qemu_process.kill()?; // there's no terminate here
    let qemu_output = qemu_process.wait_with_output()?;
    Ok(String::from_utf8(qemu_output.stdout)?)
}

#[test]
fn multiboot1() {
    for arch in [Arch::I686, Arch::X86_64] {
        for release in [false, true] {
            let stdout = build_and_boot(
                &PathBuf::from("multiboot1"),
                arch, arch, arch,
                release,
            ).expect("failed to run");
            assert!(stdout.ends_with("Halted."));
        }
    }
}

#[test]
fn multiboot2() {
    for arch in [Arch::I686, Arch::X86_64] {
        for release in [false, true] {
            let stdout = build_and_boot(
                &PathBuf::from("multiboot2"),
                arch, arch, arch,
                release,
            ).expect("failed to run");
            assert!(stdout.ends_with("Halted."));
        }
    }
}
