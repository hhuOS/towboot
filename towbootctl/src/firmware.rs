//! This module downloads and provides firmware images.
//!
//! For x64 and ia32, it uses [retrage/edk2-nightly](https://retrage.github.io/edk2-nightly/).
//! AArch64 firmware is supplied by host distributions, so common local paths
//! are checked instead.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use cached_path::Cache;
use directories::ProjectDirs;

const OVMF_X64_URL: &str = "https://retrage.github.io/edk2-nightly/bin/RELEASEX64_OVMF.fd";
const OVMF_IA32_URL: &str = "https://retrage.github.io/edk2-nightly/bin/RELEASEIa32_OVMF.fd";

const AARCH64_CODE_PATHS: &[&str] = &[
    "/opt/homebrew/share/qemu/edk2-aarch64-code.fd",
    "/usr/share/AAVMF/AAVMF_CODE.fd",
    "/usr/share/qemu-efi-aarch64/QEMU_EFI.fd",
];

const AARCH64_VARS_PATHS: &[&str] = &[
    "/opt/homebrew/share/qemu/edk2-arm-vars.fd",
    "/usr/share/AAVMF/AAVMF_VARS.fd",
    "/usr/share/qemu-efi-aarch64/QEMU_VARS.fd",
];

/// Download the firmware and provide a path to it.
/// It is cached to prevent unneccessary downloads.
fn get_firmware(url: &str) -> Result<PathBuf> {
    let mut cache = Cache::new()?;
    if let Some(dirs) = ProjectDirs::from_path("towbootctl".into()) {
        cache.dir = dirs.cache_dir().to_path_buf();
    }
    Ok(cache.cached_path(url)?)
}

/// Get OVMF for x64.
pub fn x64() -> Result<PathBuf> {
    get_firmware(OVMF_X64_URL)
}

/// Get OVMF for ia32.
pub fn ia32() -> Result<PathBuf> {
    get_firmware(OVMF_IA32_URL)
}

fn first_existing(paths: &[&str]) -> Result<PathBuf> {
    paths
        .iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
        .ok_or_else(|| anyhow!("failed to find local AArch64 firmware in common locations"))
}

/// Get local edk2 firmware paths for AArch64.
pub fn aarch64() -> Result<(PathBuf, PathBuf)> {
    Ok((
        first_existing(AARCH64_CODE_PATHS)?,
        first_existing(AARCH64_VARS_PATHS)?,
    ))
}
