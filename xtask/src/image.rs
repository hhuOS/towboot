use std::{fs::{File, OpenOptions}, path::Path, io::Error};

use fatfs::{FileSystem, format_volume, FormatVolumeOptions, FsOptions};

pub (super) struct Image {
    fs: FileSystem<File>,
}

impl Image {
    pub(super) fn new(path: &Path, size: u64) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)?;
        file.set_len(size)?;
        format_volume(&file, FormatVolumeOptions::new())?;
        Ok(Self { fs: FileSystem::new(file, FsOptions::new())? })
    }
}
