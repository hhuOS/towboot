//! This crate contains integration tests.
#![cfg(test)]
#![feature(exit_status_error)]
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use tempfile::NamedTempFile;
use towbootctl::{Architecture, boot_image, create_image};

#[derive(PartialEq, Clone, Copy)]
enum Arch {
    I686,
    X86_64,
}

#[cfg(test)]
#[ctor::ctor]
fn init() {
    if std::env::var("RUST_LOG").is_err() {
        unsafe { std::env::set_var("RUST_LOG", "warn"); }
    }
    env_logger::init();
}

/// Builds the given folder as an image and boots it.
fn build_and_boot(
    folder: &Path, machine_arch: Arch, firmware_arch: Arch,
) -> Result<String, Box<dyn Error>> {
    // make sure that the kernel is built
    Command::new("make")
        .current_dir(folder)
        .status()?.exit_ok()?;

    // build the image
    let image_path = NamedTempFile::new()?.into_temp_path();
    let mut config_path = folder.to_path_buf();
    config_path.push("towboot.toml");
    create_image(&image_path, &[
        "-config".to_string(),
        config_path.to_str().unwrap().to_string(),
    ])?;

    // boot it
    assert!(firmware_arch == machine_arch); // TODO
    let (mut qemu_command, _temp_files) = boot_image(
        None,
        &image_path,
        if matches!(machine_arch, Arch::X86_64) {
            Architecture::X86_64
        } else {
            Architecture::I686
        },
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
    sleep(Duration::from_secs(10)); // TODO: kernels should probably terminate the VM
    qemu_process.kill()?; // there's no terminate here
    let qemu_output = qemu_process.wait_with_output()?;
    Ok(String::from_utf8(qemu_output.stdout)?)
}

#[test]
fn multiboot1() {
    for arch in [Arch::I686, Arch::X86_64] {
        let stdout = build_and_boot(
            &PathBuf::from("multiboot1"),
            arch, arch, arch,
        ).expect("failed to run");
        println!("{}", stdout);
        assert!(stdout.contains("cmdline = test of a cmdline"));
        assert!(stdout.contains("boot_loader_name = towboot"));
        assert!(!stdout.contains("mods_count"));
        assert!(stdout.contains("mem_lower = 640KB"));
        assert!(stdout.contains(", type = 0x1")); // available memory
        assert!(stdout.contains(", type = 0x2")); // reserved memory
        assert!(stdout.contains(", type = 0x3")); // ACPI memory
        assert!(stdout.contains(", type = 0x4")); // ACPI NVS memory
        assert!(stdout.ends_with("Halted."));
    }
}

#[test]
fn multiboot2() {
    for arch in [Arch::I686, Arch::X86_64] {
        let stdout = build_and_boot(
            &PathBuf::from("multiboot2"),
            arch, arch, arch
        ).expect("failed to run");
        println!("{}", stdout);
        assert!(stdout.contains("Command line = test of a cmdline"));
        assert!(stdout.contains("Boot loader name = towboot"));
        assert!(!stdout.contains("Module at"));
        assert!(stdout.contains("mem_lower = 640KB"));
        assert!(stdout.contains(", type = 0x1")); // available memory
        assert!(stdout.contains(", type = 0x2")); // reserved memory
        assert!(stdout.contains(", type = 0x3")); // ACPI memory
        assert!(stdout.contains(", type = 0x4")); // ACPI NVS memory
        assert!(stdout.ends_with("Halted."));
    }
}

#[test]
fn multiboot2_x64() {
    // it should boot on x86_64
    let stdout = build_and_boot(
        &PathBuf::from("multiboot2_x64"),
        Arch::X86_64, Arch::X86_64, Arch::X86_64,
    ).expect("failed to run");
    println!("{}", stdout);
    assert!(stdout.contains("Command line = test of a cmdline"));
    assert!(stdout.contains("Boot loader name = towboot"));
    assert!(!stdout.contains("Module at"));
    assert!(stdout.contains("mem_lower = 640KB"));
    assert!(stdout.contains(", type = 0x1")); // available memory
    assert!(stdout.contains(", type = 0x2")); // reserved memory
    assert!(stdout.contains(", type = 0x3")); // ACPI memory
    assert!(stdout.contains(", type = 0x4")); // ACPI NVS memory
    assert!(stdout.ends_with("Halted."));
    // it should not boot on i686
    let stdout = build_and_boot(
        &PathBuf::from("multiboot2_x64"),
        Arch::I686, Arch::I686, Arch::I686,
    ).expect("failed to run");
    println!("{}", stdout);
    assert!(stdout.contains("The kernel supports 64-bit UEFI systems, but we're running on 32-bit"));
    assert!(!stdout.contains("Halted."));
}
