use crate::{
    prelude::*,
    file::{
        core::{
            FileAccess,
            FileType,
        },
        floc::FileNum,
        live::{
            LiveFile,
            LivePair,
        },
    },
    format_data_file,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
};
use oxedyne_fe2o3_text::string::Stringer;

use std::{
    fs::{
        File,
        OpenOptions,
    },
    path::{
        Path,
        PathBuf,
    },
};

/// Allows zone information held in `Config` to be read from a `Daticle` map.
#[derive(Clone, Debug, FromDatMap)]
pub struct ZoneDirStr {
    pub dir:        String,
    pub max_size:   u64,
}

impl Default for ZoneDirStr {
    fn default() -> Self {
        Self {
            dir:        fmt!(""),
            max_size:   constant::DEFAULT_MAX_ZONE_DIR_BYTES,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ZoneDir {
    pub dir:        PathBuf,
    pub max_size:   u64,
}

impl ZoneDir {

    pub fn file_seq_name(n: FileNum) -> String {
        Stringer::new(fmt!(format_data_file!(), n)).insert_every("_", 3).into_inner()
    }
    
    pub fn relative_file_path(typ: &FileType, n: FileNum) -> PathBuf {
        let mut result = PathBuf::new();
        match typ {
            FileType::Data => {
                result.push(Self::file_seq_name(n));
                result.set_extension(constant::DATA_FILE_EXT);
            },
            FileType::Index => {
                result.push(Self::file_seq_name(n));
                result.set_extension(constant::INDEX_FILE_EXT);
            },
        }
        result
    }
    
    pub fn open_ozone_file(
        &self,
        fnum:   FileNum,
        typ:    &FileType,
        how:    &FileAccess,
    )
        -> Outcome<(PathBuf, File)>
    {
        let mut path = self.dir.clone();
        path.push(Self::relative_file_path(typ, fnum));
        let file = res!(Self::open_file(&path, how));
        Ok((path, file))
    }
    
    pub fn open_file(p: &PathBuf, access: &FileAccess) -> Outcome<File> {
        match access {
            FileAccess::Reading => match OpenOptions::new()
                .read(true)
                .open(p)
            {
                Err(e) => Err(err!(e,
                    "While opening file {:?} for {:?}", p, access;
                    IO, File, Read)),
                Ok(file) => Ok(file),
            },
            FileAccess::Writing => match OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .append(true)
                .open(p)
            {
                Err(e) => Err(err!(e,
                    "While opening file {:?} for {:?}", p, access;
                    IO, File, Write, Create)),
                Ok(file) => Ok(file),
            },
        }
    }
    
    pub fn ozone_file_number_and_type(path: &Path) -> Outcome<(FileNum, FileType)> {
        let stem_str = match path.file_stem() {
            None => return Err(err!(
                "File {:?} has an invalid file name.  Ozone zone directories \
                should not contain files with names like this.", path;
                IO, File, Path, Invalid)),
            Some(os_str) => match os_str.to_str() {
                None => return Err(err!(
                    "File {:?} file name is not valid Unicode", path;
                    IO, File, Path, Decode, Invalid)),
                Some(s) => s.replace("_", ""),
            },
        };
        let fnum = res!(stem_str.parse::<FileNum>());
        if fnum == 0 {
            return Err(err!(
                "File {:?} has an invalid file number.  Ozone data and index \
                file numbers start from 1.", path;
                IO, File, Path, Invalid));
        }
        let ftyp = match path.extension() {
            None => return Err(err!(
                "File {:?} has an invalid file extension.  Ozone zone directories \
                should not contain files of this type.", path;
                IO, File, Path, Invalid)),
            Some(os_str) => match os_str.to_str() {
                None => return Err(err!(
                    "File {:?} file extension is not valid Unicode", path;
                    IO, File, Path, Decode, Invalid)),
                Some(s) => match s {
                    constant::DATA_FILE_EXT => FileType::Data,
                    constant::INDEX_FILE_EXT => FileType::Index,
                    _ => return Err(err!(
                        "File {:?} extension not valid for Ozone database", path;
                        IO, File, Name, Invalid)),
                },
            },
        };
        Ok((fnum, ftyp))
    }

    pub fn open_live(&self, fnum: FileNum) -> Outcome<LivePair> {
        let (path, file) = res!(self.open_ozone_file(
            fnum,
            &FileType::Data,
            &FileAccess::Writing,
        ));
        let mut dat = LiveFile {
            path,
            file: Some(file),
            size: 0,
        };
        dat.size = res!(dat.get_file_len());
        let (path, file) = res!(self.open_ozone_file(
            fnum,
            &FileType::Index,
            &FileAccess::Writing,
        ));
        let mut ind = LiveFile {
            path,
            file: Some(file),
            size: 0,
        };
        ind.size = res!(ind.get_file_len());
        Ok(LivePair {
            fnum,
            dat,
            ind,
        })
    }
}
