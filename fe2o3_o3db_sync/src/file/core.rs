use crate::{
    prelude::*,
    regex_data_file,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
};

use regex::Regex;


/// The mode a file is opened in.
#[derive(Debug)]
pub enum FileAccess {
    /// Opened for reading.
    Reading,
    /// Opened for writing.
    Writing,
}

/// The two kinds of file the database writes: value data files and their
/// companion index files. The discriminant is the on-disk type code.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FileType {
    /// A data file holding appended key-value records.
    Data    = 0,
    /// An index file holding the file locations of records.
    Index   = 1,
}

/// A single entry in a directory listing, describing one file.
#[derive(Clone, Debug)]
pub struct FileEntry {
    /// Type indicator (file, directory or symlink).
    pub typ: String,
    /// Size in bytes.
    pub size: u64,
    /// Seconds since last modification.
    pub mods: u64,
    /// Object name.
    pub name: String,
}

/// Recursively finds all data and index files beneath `dir`, matching the
/// database's numbered file-name pattern and extensions.
pub fn find_files(dir: &Path) -> Outcome<Vec<PathBuf>> {
    let pattern = fmt!(
        "^{}\\.({}|{})$",
        regex_data_file!(),
        constant::DATA_FILE_EXT,
        constant::INDEX_FILE_EXT,
    );
    let re = res!(regex::Regex::new(&pattern));

    let mut matching_files = Vec::new();

    res!(search_recursively(dir, &re, &mut matching_files));

    Ok(matching_files)
}

fn search_recursively(
    dir: &Path,
    re: &Regex,
    matching_files: &mut Vec<PathBuf>,
)
    -> Outcome<()>
{
    if dir.is_dir() {
        for entry in res!(fs::read_dir(dir)) {
            let entry = res!(entry);
            let path = entry.path();
            if path.is_dir() {
                debug!(sync_log::stream(), "Looking in {:?}", path);
                res!(search_recursively(&path, re, matching_files));
            } else if let Some(file_name) = path.file_name() {
                debug!(sync_log::stream(), "Found {:?}", path);
                if let Some(file_name_str) = file_name.to_str() {
                    if re.is_match(file_name_str) {
                        matching_files.push(path);
                    }
                }
            }
        }
    }
    Ok(())
}
