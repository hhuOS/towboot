//! This module downloads and provides current builds of OVMF.
//! 
//! It uses [retrage/edk2-nightly](https://retrage.github.io/edk2-nightly/),
//! as this provides builds for both x64 and ia32 as single files.
//! When <https://github.com/epwalsh/rust-cached-path/pull/74> is merged,
//! we might want to switch back to the Arch Linux builds.

use std::path::PathBuf;

use anyhow::Result;
use cached_path::Cache;
use directories::ProjectDirs;

const OVMF_X64_URL: &str = "https://retrage.github.io/edk2-nightly/bin/RELEASEX64_OVMF.fd";
const OVMF_IA32_URL: &str = "https://retrage.github.io/edk2-nightly/bin/RELEASEIa32_OVMF.fd";

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
