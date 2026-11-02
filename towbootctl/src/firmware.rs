//! This module downloads and provides firmware images.
//!
//! See [`WantedFirmware`] for the possible sources.

use std::{fs, ops::Deref, path::{Path, PathBuf}};

use anyhow::{Result, anyhow};
use cached_path::Cache;
use directories::ProjectDirs;
use log::warn;
use tempfile::{NamedTempFile, TempPath};

use crate::Architecture;

const OVMF_X64_URL: &str = "https://retrage.github.io/edk2-nightly/bin/RELEASEX64_OVMF.fd";
const OVMF_IA32_URL: &str = "https://retrage.github.io/edk2-nightly/bin/RELEASEIa32_OVMF.fd";
const OVMF_AARCH64_CODE_URL: &str = "https://retrage.github.io/edk2-nightly/bin/RELEASEAARCH64_QEMU_EFI.fd";
const OVMF_AARCH64_VARS_URL: &str = "https://retrage.github.io/edk2-nightly/bin/RELEASEAARCH64_QEMU_VARS.fd";

const OVMF_X64_PATHS: &[&str] = &[
    "/usr/share/ovmf/OVMF.fd", // Ubuntu
    // TODO: Homebrew and Fedora only have this split in code and vars
];
const OVMF_IA32_PATHS: &[&str] = &[
    // Ubuntu doesn't seem to ship this
    // TODO: Homebrew and Fedora only have this split in code and vars
];

const OVMF_AARCH64_CODE_PATHS: &[&str] = &[
    "/opt/homebrew/share/qemu/edk2-aarch64-code.fd", // Homebrew on macOS aarch64
    "/usr/local/share/qemu/edk2-aarch64-code.fd", // Homebrew on macOS x86_64
    "/usr/share/AAVMF/AAVMF_CODE.fd", // Ubuntu
    "/usr/share/qemu-efi-aarch64/QEMU_EFI.fd",
];

const OVMF_AARCH64_VARS_PATHS: &[&str] = &[
    "/opt/homebrew/share/qemu/edk2-arm-vars.fd", // Homebrew on macOS aarch64
    "/usr/local/share/qemu/edk2-aarch64-vars.fd", // Homebrew on macOS x86_64
    "/usr/share/AAVMF/AAVMF_VARS.fd", // Ubuntu
    "/usr/share/qemu-efi-aarch64/QEMU_VARS.fd",
];
const UEFI_FLASH_SIZE: u64 = 64 * 1024 * 1024;

/// The firmware we want to use for the VM.
/// 
/// The common case is probably [`WantedFirmware::SystemOvmf`] which first
/// tries common local paths (eg. the packages from apt or brew) and then falls
/// back to [retrage/edk2-nightly](https://retrage.github.io/edk2-nightly/).
pub enum WantedFirmware {
    /// Use the firmware that is already present on the system.
    /// This usually comes from apt or brew.
    /// See the constants above for where we look for the firmware.
    /// This falls back to [`WantedFirmware::NightlyOvmf`] if no local image
    /// is found.
    SystemOvmf,
    /// Download a nightly build and use that.
    NightlyOvmf,
    /// Use a specified file.
    LocalFile(PathBuf),
}

impl WantedFirmware {
    /// Get the firmware for the specified architecture.
    pub(super) fn get(self, architecture: Architecture) -> Result<ExistingFirmware> {
        let mut fw = match self {
            WantedFirmware::LocalFile(path) => {
                // for a local file, we need to check if it exists
                if !path.exists() {
                    return Err(anyhow!("given firmware path does not exist").into());
                }
                // if it does, we have our firmware
                // TODO: though we might want to check if it's for the correct architecture etc.
                ExistingFirmware::Combined(path)
            },
            WantedFirmware::SystemOvmf => {
                // try to find the system files for the specified architecture
                match architecture {
                    Architecture::I686 => try_local(OVMF_IA32_PATHS).map(
                        ExistingFirmware::Combined
                    ),
                    Architecture::X86_64 => try_local(OVMF_X64_PATHS).map(
                        ExistingFirmware::Combined
                    ),
                    Architecture::Aarch64 => {
                        try_local(OVMF_AARCH64_CODE_PATHS)
                            .and_then(|code|
                                try_local(OVMF_AARCH64_VARS_PATHS).map(|vars|
                                    ExistingFirmware::CodeAndVars(code.into(), vars.into())
                                )
                            )
                    },
                }.ok_or(anyhow!("couldn't find a local OVMF image"))
                .or_else(|err| {
                    warn!("{err}, downloading a nightly one");
                    Self::NightlyOvmf.get(architecture)
                })?
            },
            WantedFirmware::NightlyOvmf => {
                // if we need to download a nightly build,
                // we need to know for what architecture
                // 
                // For I686 and X86_64, there are both combined and split images.
                // For now, just use the combined one because it's easier.
                match architecture {
                    Architecture::I686 => ExistingFirmware::Combined(download(OVMF_IA32_URL)?.into()),
                    Architecture::X86_64 => ExistingFirmware::Combined(download(OVMF_X64_URL)?.into()),
                    Architecture::Aarch64 => ExistingFirmware::CodeAndVars(
                        download(OVMF_AARCH64_CODE_URL)?.into(),
                        download(OVMF_AARCH64_VARS_URL)?.into(),
                    ),
                }
            },
        };
        // if we have separate code and vars,
        // we might need to copy them to make sure that they have the correct size
        if let ExistingFirmware::CodeAndVars(code, vars) = &mut fw {
            let mut code_copy = NamedTempFile::new()?;
            fs::copy(&*code, code_copy.path())?;
            code_copy.as_file_mut().set_len(UEFI_FLASH_SIZE).expect("failed to enlarge UEFI code");
            *code = code_copy.into_temp_path().into();
            let mut vars_copy = NamedTempFile::new()?;
            fs::copy(&*vars, vars_copy.path())?;
            vars_copy.as_file_mut().set_len(UEFI_FLASH_SIZE).expect("failed to enlarge UEFI vars");
            *vars = vars_copy.into_temp_path().into();
        }
        Ok(fw)
    }
}

/// The firmware we are going to use for the VM.
#[derive(Debug)]
pub(super) enum ExistingFirmware {
    Combined(PathBuf),
    CodeAndVars(MaybeTempPath, MaybeTempPath),
}

/// A path that maybe either [`PathBuf`] (persistent) or [`TempPath`] (temporary).
#[derive(Debug)]
pub(super) enum MaybeTempPath {
    Persistent(PathBuf),
    Temporary(TempPath),
}

impl MaybeTempPath {
    /// If this is a temporary path, add it to this [`Vec`].
    pub(super) fn add_to(self, v: &mut Vec<TempPath>) {
        if let MaybeTempPath::Temporary(temp_path) = self {
            v.push(temp_path);
        }
    }
}

impl From<PathBuf> for MaybeTempPath {
    fn from(value: PathBuf) -> Self {
        Self::Persistent(value)
    }
}

impl From<TempPath> for MaybeTempPath {
    fn from(value: TempPath) -> Self {
        Self::Temporary(value)
    }
}

impl Deref for MaybeTempPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        match self {
            MaybeTempPath::Persistent(path_buf) => path_buf,
            MaybeTempPath::Temporary(temp_path) => temp_path,
        }
    }
}

impl AsRef<Path> for MaybeTempPath {
    fn as_ref(&self) -> &Path {
        self.deref()
    }
}

/// Download the firmware and provide a path to it.
/// It is cached to prevent unneccessary downloads.
fn download(url: &str) -> Result<PathBuf> {
    let mut cache = Cache::new()?;
    if let Some(dirs) = ProjectDirs::from_path("towbootctl".into()) {
        cache.dir = dirs.cache_dir().to_path_buf();
    }
    Ok(cache.cached_path(url)?)
}

/// Try different local files, return the first one that exists.
fn try_local(paths: &[&str]) -> Option<PathBuf> {
    paths
        .iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
}
