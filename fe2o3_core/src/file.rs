use crate::{
    prelude::*,
    path::NormalPath,
};

use std::{
    fmt,
    fs::{
        self,
        File,
        OpenOptions,
    },
    path::{
        Path,
        PathBuf,
    },
};


#[derive(Clone, Debug)]
pub enum OsPath {
    Dir(PathBuf),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug)]
pub enum PathState {
    DirMustExist,
    FileMustExist,
    Create,
}

impl PathState {

    pub fn validate(
        &self,
        root:       &PathBuf,
        rel_path:   &str,
    )
        -> Outcome<()>
    {
        let rel_path = Path::new(rel_path).normalise();
        if rel_path.escapes() {
            return Err(err!(errmsg!(
                "The relative path '{:?}' escapes the root directory.", rel_path,
            ), Invalid, Input, Path));
        }
        let abs_path = root.clone().join(rel_path).normalise().absolute();
        if abs_path.exists() {
            if let Self::DirMustExist = self {
                if !abs_path.is_dir() {
                    return Err(err!(errmsg!(
                        "Path '{:?}' exists but is not a directory.", root,
                    ), Input, Invalid, File, Path));
                }
            }
        } else {
            match self {
                Self::DirMustExist |
                Self::FileMustExist => return Err(err!(errmsg!(
                    "The path '{:?}' must exist but was not found.", abs_path,
                ), Path, File, Missing)),
                Self::Create => res!(fs::create_dir_all(&abs_path)),
            }
        }
        Ok(())
    }
}


pub trait Loadable {
    fn load<P: AsRef<Path>>(path: P) -> Outcome<Self> where Self: Sized;
}

pub fn touch(path: &Path) -> Outcome<File> {
    Ok(res!(
        OpenOptions::new().create(true).write(true).open(path),
        File, Write,
    ))
}

#[derive(Debug, Default)]
pub struct TextFileState {
    pub path:       String,
    pub line_num:   usize,
}

impl fmt::Display for TextFileState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.path, self.line_num)
    }
}
