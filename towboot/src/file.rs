//! File handling

use core::cell::RefCell;

use alloc::borrow::ToOwned;
use alloc::collections::btree_set::BTreeSet;
use alloc::format;
use alloc::rc::Rc;
use alloc::{vec::Vec, vec};
use alloc::string::ToString;

use log::{info, error};

use uefi::prelude::*;
use uefi::boot::{find_handles, open_protocol_exclusive};
use uefi::fs::{Path, PathBuf};
use uefi::data_types::CString16;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::proto::media::file::{
    File as UefiFile, FileAttribute, FileInfo, FileMode, FileType, RegularFile
};

use super::mem::{Allocation, Allocator};
use towboot_config::Quirk;

/// An opened file.
pub(crate) struct File<'a> {
    name: &'a str,
    file: RegularFile,
    size: usize,
}

impl<'a> File<'a> {
    /// Opens a file.
    ///
    /// The path can be:
    /// * relative to the volume we're loaded from
    /// * on a different volume (if it starts with `fs?:`)
    ///
    /// Possible errors:
    /// * `Status::INVALID_PARAMETER`: the volume identifier is invalid
    /// * `Status::NOT_FOUND`: the file does not exist
    /// * `Status::PROTOCOL_ERROR`: the file name is not a valid string
    /// * `Status::UNSUPPORTED`: the given path does exist, but it's a directory
    pub(crate) fn open(name: &'a str, image_fs_handle: Handle) -> Result<Self, Status> {
        info!("loading file '{name}'...");
        let file_name = CString16::try_from(name)
            .map_err(|e| {
                error!("filename is invalid because of {e:?}");
                Status::PROTOCOL_ERROR
            })?;
        let file_path = Path::new(&file_name);
        let mut file_path_components = file_path.components();
        let (
            fs_handle, file_name,
        ) = if let Some(root) = file_path_components.next() && root.to_string().ends_with(':') {
            if let Some(idx) = root
                .to_string()
                .to_lowercase()
                .strip_suffix(':')
                .unwrap()
                .strip_prefix("fs") {
                let filesystems = find_handles::<SimpleFileSystem>()
                    .map_err(|e| e.status())?;
                let fs = filesystems.into_iter().nth(
                    idx.parse::<usize>().map_err(|_| {
                        error!("{idx} is not a number");
                        Status::INVALID_PARAMETER
                    })?
                ).ok_or(Status::NOT_FOUND)?;
                let mut file_path = PathBuf::new();
                for c in file_path_components {
                    file_path.push(c.as_ref());
                }
                Ok((fs, file_path.to_cstr16().to_owned()))
            } else {
                error!("don't know how to open {root}");
                Err(Status::INVALID_PARAMETER)
            }?
        } else {
            (image_fs_handle, file_name)
        };
        let mut fs = open_protocol_exclusive::<SimpleFileSystem>(fs_handle)
            .map_err(|e| e.status())?;
        let file_handle = match fs.open_volume().map_err(|e| e.status())?.open(
            &file_name,
            FileMode::Read,
            FileAttribute::READ_ONLY,
        ) {
            Ok(file_handle) => file_handle,
            Err(e) => return {
                error!("Failed to find file '{name}': {e:?}");
                Err(Status::NOT_FOUND)
            }
        };
        let mut file = match file_handle.into_type()
        .expect(&format!("Failed to open file '{name}'")) {
            FileType::Regular(file) => file,
            FileType::Dir(_) => return {
                error!("File '{name}' is a directory");
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
        .expect(&format!("Failed to get metadata of file '{name}'"))
        .file_size().try_into().unwrap();
        Ok(Self { name, file, size })
    }
    
    /// Read a whole file into memory and return the resulting allocation.
    ///
    /// (The difference to `TryInto<Vec<u8>>` is that the allocated memory
    /// is page-aligned and under 4GB.)
    pub(crate) fn try_into_allocation(
        mut self, allocator: &Rc<RefCell<Allocator>>, quirks: &BTreeSet<Quirk>,
    ) -> Result<Allocation, Status> {
        let mut allocation = Allocation::new_under_4gb(allocator, self.size, quirks)?;
        let read_size = self.file.read(allocation.as_mut_slice())
        .map_err(|e| {
            error!("Failed to read from file '{}': {:?}", self.name, e);
            e.status()
        })?;
        if read_size == self.size {
            Ok(allocation)
        } else {
            error!("Failed to fully read from file '{}", self.name);
            Err(Status::END_OF_FILE)
        }
    }
}

impl TryFrom<File<'_>> for Vec<u8> {
    type Error = Status;
    
    /// Read a whole file into memory and return the resulting byte vector.
    fn try_from(mut file: File) -> Result<Self, Self::Error> {
        // Vec::with_size would allocate enough space, but won't fill it with zeros.
        // file.read seems to need this.
        let mut content_vec = vec![0; file.size];
        let read_size = file.file.read(content_vec.as_mut_slice())
        .map_err(|e| {
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
