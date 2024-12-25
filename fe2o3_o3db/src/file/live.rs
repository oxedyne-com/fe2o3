use crate::{
    prelude::*,
    file::floc::FileNum,
};


use std::{
    fs::File,
    path::PathBuf,
};

#[derive(Debug, Default)]
pub struct LiveFile {
    pub path:   PathBuf,
    pub file:   Option<File>,
    pub size:   u64,
}

impl LiveFile {

    pub fn get_file_len(&self) -> Outcome<u64> {
        match &self.file {
            Some(file) => match file.metadata() {
                Err(e) => Err(err!(e, errmsg!(
                    "Could not read metadata for file {:?}.", self.path,
                ), File, Read)),
                Ok(metadata) => Ok(metadata.len()),
            },
            None => Err(err!(errmsg!(
                "Attempt to get length of a live file {:?} but the file is None.", self.path,
            ), Invalid, Input)),
        }
    }

    pub fn close(&mut self) {
        if let Some(file) = std::mem::take(&mut self.file) {
            drop(file);
        }
    }
    
}

/// Acts as a cache to reduce repeated reformulation of the live file path and reacquisition of
/// the files and the data file length.
#[derive(Debug, Default)]
pub struct LivePair {
    pub fnum:   FileNum,
    pub dat:    LiveFile,
    pub ind:    LiveFile,
}

impl LivePair {
    pub fn close(&mut self) {
        self.dat.close();
        self.ind.close();
    }
}
