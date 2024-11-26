//! This module contains functionality to work with images.
use std::error::Error;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Write, Read};
use std::path::Path;

use fscommon::StreamSlice;
use gpt::{GptConfig, disk::LogicalBlockSize, mbr::ProtectiveMBR, partition_types};
use log::debug;
use fatfs::{FileSystem, format_volume, FormatVolumeOptions, FsOptions};

/// An image that is currently being constructed.
pub struct Image {
    fs: FileSystem<StreamSlice<Box<File>>>,
}

impl Image {
    /// Create a new image at the given location with the given size.
    /// If the file exists already, it will be overwritten.
    pub fn new(path: &Path, size: u64) -> Result<Self, Box<dyn Error>> {
        debug!("creating disk image");
        let mut file = Box::new(OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?);
        file.set_len(size)?;
        // protective MBR
        let mbr = ProtectiveMBR::with_lb_size(
            u32::try_from((size / 512) - 1).unwrap_or(0xFF_FF_FF_FF)
        );
        mbr.overwrite_lba0(&mut file)?;
        let mut disk = GptConfig::new()
            .writable(true)
            .logical_block_size(LogicalBlockSize::Lb512)
            .create_from_device(file, None)?;
        disk.update_partitions(BTreeMap::new())?;
        debug!("creating partition");
        disk.add_partition("towboot", size - 1024 * 1024, partition_types::EFI, 0, None)?;
        let partitions = disk.partitions().clone();
        let (_, partition) = partitions.iter().next().unwrap();
        let file = disk.write()?;
        let mut part = StreamSlice::new(
            file, partition.first_lba * 512, partition.last_lba * 512,
        )?;
        debug!("formatting {}", partition);
        format_volume(&mut part, FormatVolumeOptions::new())?;
        Ok(Self { fs: FileSystem::new(part, FsOptions::new())? })
    }

    /// Copy a file from the local filesystem to the image.
    pub fn add_file(&mut self, source: &Path, dest: &Path) -> Result<(), Box<dyn Error>> {
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
