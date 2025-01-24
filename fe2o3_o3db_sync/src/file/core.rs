use crate::{
    prelude::*,
    regex_data_file,
};

use oxedize_fe2o3_core::prelude::*;

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
};

use regex::Regex;


#[derive(Debug)]
pub enum FileAccess {
    Reading,
    Writing,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FileType {
    Data    = 0,
    Index   = 1,
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub typ: String,
    pub size: u64,
    pub mods: u64,
    pub name: String,
}

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
                debug!("Looking in {:?}", path);
                res!(search_recursively(&path, re, matching_files));
            } else if let Some(file_name) = path.file_name() {
                debug!("Found {:?}", path);
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
