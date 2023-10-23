use std::{fs::{File, OpenOptions}, path::Path, io::{Error, Write, Read}};

use cli_xtask::tracing::debug;
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

    pub(super) fn add_file(&mut self, source: &Path, dest: &Path) -> Result<(), Error> {
        debug!("adding {} as {}", source.display(), dest.display());
        let mut source_file = File::open(source)?;
        let mut dir = self.fs.root_dir();
        let components: Vec<_> = dest.components().collect();
        let (file_name, dir_names) = components.split_last().unwrap();
        for dir_name in dir_names {
            dir = dir.create_dir(dir_name.as_os_str().to_str().unwrap())?;
        }
        let mut dest_file = dir.create_file(
            file_name.as_os_str().to_str().unwrap()
        )?;
        let mut buf = Vec::new();
        source_file.read_to_end(&mut buf)?;
        dest_file.write_all(&buf)?;
        Ok(())
    }
}
