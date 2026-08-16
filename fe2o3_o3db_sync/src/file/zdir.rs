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
    
    /// Returns the path, relative to the zone directory, of the temporary file written
    /// while the given file's garbage is collected.
    pub fn relative_gc_temp_path(typ: &FileType, n: FileNum) -> PathBuf {
        let mut result = PathBuf::from(constant::GC_TEMP_FILE_PREFIX);
        result.set_extension(Self::relative_file_path(typ, n));
        result
    }

    /// Returns whether the path names a garbage collection temporary.  Such a file is an
    /// abandoned transcription left by a collection that did not finish; the data file it
    /// was copied from is untouched, so the temporary can simply be removed.
    pub fn is_gc_temp_file(p: &Path) -> bool {
        match p.file_name().and_then(|s| s.to_str()) {
            Some(name) => name.starts_with(constant::GC_TEMP_FILE_PREFIX),
            None => false,
        }
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
    
    /// Claims a file number for this writer, by creating its files rather than by
    /// asking whether they exist.
    ///
    /// Returns whether the claim was won. Creation is attempted with `create_new`,
    /// which is one atomic operation, so of two writers racing for a number
    /// exactly one succeeds and the loser is told at once rather than discovering
    /// it by writing into somebody else's file.
    ///
    /// This is what makes a writer the sole appender to its own live files, which
    /// is what the cached length in [`LiveFile`] depends on: a length read when a
    /// file is opened stays true only while nobody else is adding to it. Two
    /// writers sharing a file would each place records at offsets predicted from
    /// their own cache, and the bytes would land correctly -- the files are opened
    /// for append, so the kernel puts every record whole at the end -- while the
    /// index entries pointed into the middle of each other's records.
    ///
    /// The data file is claimed first and the index second. A number whose data
    /// file was won and whose index was not is abandoned rather than used, and the
    /// data file is left behind: an empty file is inert, and removing one this
    /// process may not own is worse than leaving it.
    pub fn claim(&self, fnum: FileNum)
        -> Outcome<bool>
    {
        let mut won = Vec::new();
        for typ in [FileType::Data, FileType::Index] {
            let mut path = self.dir.clone();
            path.push(Self::relative_file_path(&typ, fnum));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => won.push(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
                Err(e) => return Err(err!(e,
                    "While claiming {:?} for zone file {}.", path, fnum;
                IO, File, Create)),
            }
        }
        Ok(!won.is_empty())
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


#[cfg(test)]
mod tests {
    use super::*;

    /// Returns a zone directory of its own for one test.
    fn scratch(what: &str)
        -> Outcome<ZoneDir>
    {
        let dir = std::env::temp_dir().join(fmt!(
            "o3db_claim_{}_{}", std::process::id(), what));
        let _ = std::fs::remove_dir_all(&dir);
        res!(std::fs::create_dir_all(&dir));
        Ok(ZoneDir { dir, max_size: 0 })
    }

    /// A file number goes to exactly one claimant.
    ///
    /// This is the whole of what makes a writer the sole appender to its own
    /// live file, and therefore the whole of what makes the length it caches
    /// when it opens that file stay true. Without it two writers place records
    /// at offsets each predicted from its own cache, and the index entries name
    /// positions inside each other's records.
    #[test]
    fn a_file_number_is_claimed_by_one_writer_only() -> Outcome<()> {
        let zdir = res!(scratch("one"));
        assert!(res!(zdir.claim(7)), "the first claim on a free number was refused");
        assert!(!res!(zdir.claim(7)), "a number already claimed was handed out twice");
        // And a different number is unaffected by the first one being taken.
        assert!(res!(zdir.claim(8)), "an untouched number could not be claimed");
        let _ = std::fs::remove_dir_all(&zdir.dir);
        Ok(())
    }

    /// A claim leaves both of the pair behind, so that neither half of a number
    /// can be taken by somebody else afterwards.
    #[test]
    fn a_claim_takes_the_data_file_and_the_index_together() -> Outcome<()> {
        let zdir = res!(scratch("pair"));
        assert!(res!(zdir.claim(3)), "the claim was refused");
        for typ in [FileType::Data, FileType::Index] {
            let mut path = zdir.dir.clone();
            path.push(ZoneDir::relative_file_path(&typ, 3));
            assert!(path.is_file(), "the claim left no {:?} file at {:?}", typ, path);
        }
        let _ = std::fs::remove_dir_all(&zdir.dir);
        Ok(())
    }

    /// A number whose data file exists is refused even where its index does not,
    /// which is the state a claim abandoned partway through leaves.
    #[test]
    fn half_a_pair_is_enough_to_refuse_a_number() -> Outcome<()> {
        let zdir = res!(scratch("half"));
        let mut path = zdir.dir.clone();
        path.push(ZoneDir::relative_file_path(&FileType::Data, 5));
        res!(std::fs::write(&path, b""));
        assert!(!res!(zdir.claim(5)),
            "a number whose data file was already there was claimed anyway");
        let _ = std::fs::remove_dir_all(&zdir.dir);
        Ok(())
    }
}
