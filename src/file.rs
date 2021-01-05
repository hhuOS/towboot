//! File handling

use core::convert::TryInto;
use core::convert::TryFrom;

use alloc::format;
use alloc::vec::Vec;

use log::{info, error};

use uefi::prelude::*;
use uefi::proto::media::file::{
    Directory, File as UefiFile, FileAttribute, FileInfo, FileMode, FileType, RegularFile
};

use super::mem::Allocation;

/// An opened file.
pub(crate) struct File<'a> {
    name: &'a str,
    file: RegularFile,
    size: usize,
}

impl<'a> File<'a> {
    /// Opens a file.
    ///
    /// The path is relative to the volume we're loaded from.
    ///
    /// Possible errors:
    /// * `Status::NOT_FOUND`: the file does not exist
    /// * `Status::UNSUPPORTED`: the given path does exist, but it's a directory
    pub(crate) fn open(name: &'a str, volume: &mut Directory) -> Result<Self, Status> {
        info!("loading file '{}'...", name);
        let file_handle = match volume.open(name, FileMode::Read, FileAttribute::READ_ONLY) {
            Ok(file_handle) => file_handle.unwrap(),
            Err(e) => return {
                error!("Failed to find file '{}': {:?}", name, e);
                Err(Status::NOT_FOUND)
            }
        };
        let mut file = match file_handle.into_type()
        .expect_success(&format!("Failed to open file '{}'", name)) {
            FileType::Regular(file) => file,
            FileType::Dir(_) => return {
                error!("File '{}' is a directory", name);
                Err(Status::UNSUPPORTED)
            }
        };
        let mut info_vec = Vec::<u8>::new();
        
        // we try to get the metadata with a zero-sized buffer
        // this should throw BUFFER_TOO_SMALL and give us the needed size
        let info_result = file.get_info::<FileInfo>(info_vec.as_mut_slice());
        assert_eq!(info_result.status(), Status::BUFFER_TOO_SMALL);
        let info_size: usize = info_result.expect_err("metadata is 0 bytes").data()
        .expect("failed to get size of file metadata");
        info_vec.resize(info_size, 0);
        
        let size: usize = file.get_info::<FileInfo>(info_vec.as_mut_slice())
        .expect(&format!("Failed to get metadata of file '{}'", name))
        .unwrap().file_size().try_into().unwrap();
        Ok(Self { file, name, size })
    }
}

impl<'a> TryFrom<File<'a>> for Vec<u8> {
    type Error = Status;
    
    /// Read a whole file into memory and return the resulting byte vector.
    fn try_from(mut file: File) -> Result<Self, Self::Error> {
        // Vec::with_size would allocate enough space, but won't fill it with zeros.
        // file.read seems to need this.
        let mut content_vec = Vec::<u8>::new();
        content_vec.resize(file.size, 0);
        let read_size = file.file.read(content_vec.as_mut_slice())
        .log_warning().map_err(|e| {
            error!("Failed to read from file '{}': {:?}", file.name, e);
            e.status()
        })?;
        if read_size == file.size {
            Ok(content_vec)
        } else {
            error!("Failed to fully read from file '{}", file.name);
            Err(Status::END_OF_FILE)
        }
    }
}

impl<'a> TryFrom<File<'a>> for Allocation {
    type Error = Status;
    
    /// Read a whole file into memory and return the resulting allocation.
    ///
    /// (The difference to `Into<Vec<u8>>` is that the allocated memory
    /// is page-aligned and under 4GB.)
    fn try_from(mut file: File) -> Result<Self, Self::Error> {
        let mut allocation = Allocation::new_under_4gb(file.size)?;
        let read_size = file.file.read(allocation.as_mut_slice())
        .log_warning().map_err(|e| {
            error!("Failed to read from file '{}': {:?}", file.name, e);
            e.status()
        })?;
        if read_size == file.size {
            Ok(allocation)
        } else {
            error!("Failed to fully read from file '{}", file.name);
            Err(Status::END_OF_FILE)
        }
    }
}
