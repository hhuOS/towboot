//! This emulates towboot/src/file.rs and a bit of uefi-rs.

use std::{path::Path, io::Read};

use thiserror::Error;

#[allow(non_camel_case_types)]
#[derive(Debug, Error)]
pub(crate) enum Status {
    #[error("the file wasn't found")]
    NOT_FOUND,
    #[error("the parameter was invalid")]
    INVALID_PARAMETER,
    #[error("the file could not be read")]
    LOAD_ERROR,
}

pub(crate) struct File {
    file: std::fs::File,
}

impl File {
    pub(crate) fn open(file_name: &str, _volume: &Path) -> Result<Self, Status> {
        match std::fs::File::open(file_name) {
            Ok(file) => Ok(Self { file }),
            Err(_) => Err(Status::NOT_FOUND),
        }
    }
}

impl TryFrom<File> for Vec<u8> {
    type Error = Status;

    fn try_from(mut file: File) -> Result<Self, Self::Error> {
        let mut buf = Vec::new();
        file.file.read_to_end(&mut buf).map_err(|_| Status::LOAD_ERROR)?;
        Ok(buf)
    }
}
