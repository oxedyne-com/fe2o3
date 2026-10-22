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
        refusal::Refusal,
    },
    format_data_file,
    test::hooks,
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
    io::ErrorKind,
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
    
    pub fn relative_gc_temp_path(typ: &FileType, n: FileNum) -> PathBuf {
        let mut result = PathBuf::from(constant::GC_TEMP_FILE_PREFIX);
        result.set_extension(Self::relative_file_path(typ, n));
        result
    }

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
        let path = self.ozone_file_path(fnum, typ);
        let file = res!(Self::open_file(&path, how));
        Ok((path, file))
    }

    pub fn ozone_file_path(&self, fnum: FileNum, typ: &FileType) -> PathBuf {
        self.dir.join(Self::relative_file_path(typ, fnum))
    }
    
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

    #[test]
    fn a_kernel_release_is_read_by_major_and_minor() {
        assert!(release_at_least("7.0.0-38-generic", (5, 8)));
        assert!(release_at_least("5.8.0", (5, 8)));
        assert!(release_at_least("5.10", (5, 8)));
        assert!(!release_at_least("5.7.19-arch1", (5, 8)));
        assert!(!release_at_least("4.19.0", (5, 8)));
        assert!(!release_at_least("", (5, 8)));
        assert!(!release_at_least("garbage", (5, 8)));
    }

    fn scratch(what: &str)
        -> Outcome<ZoneDir>
    {
        let dir = std::env::temp_dir().join(fmt!(
            "o3db_claim_{}_{}", std::process::id(), what));
        let _ = std::fs::remove_dir_all(&dir);
        res!(std::fs::create_dir_all(&dir));
        Ok(ZoneDir { dir, max_size: 0 })
    }

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

/// Forces a directory's entries to stable storage.  A `rename` or an unlink is a directory
/// metadata operation, so fsyncing a file's contents does not persist it; this is called after
/// them so a power loss cannot undo what the caller then relies on.
pub fn sync_dir(dir: &Path) -> Outcome<()> {
    match File::open(dir) {
        Err(e) => Err(err!(e,
            "While opening directory {:?} to fsync it.", dir;
            IO, File, Read)),
        Ok(d) => match d.sync_all() {
            Err(e) => Err(err!(e,
                "While fsyncing directory {:?}.", dir;
                IO, File, Write)),
            Ok(()) => Ok(()),
        },
    }
}

/// Writes `text` to `dir/name` through a temporary file that is synced, renamed into place, and
/// its directory synced, so that a crash leaves the file whole or absent.
pub fn write_whole(dir: &Path, name: &str, text: &str, halt: Option<hooks::Halt>) -> Outcome<PathBuf> {
    use std::io::Write;
    let path = dir.join(name);
    let tmp = dir.join(fmt!("{}.tmp", name));
    {
        let mut file = res!(File::create(&tmp));
        res!(file.write_all(text.as_bytes()));
        res!(file.sync_all());
    }
    if let Some(at) = halt {
        hooks::halt(at);
    }
    res!(std::fs::rename(&tmp, &path));
    res!(sync_dir(dir));
    Ok(path)
}

/// Makes every data and index file in the zone directories `dirs` durable before replay calls
/// the records in them durable (R2): after a process crash a tail may sit only in the page cache,
/// and a write-back error nobody has seen yet must refuse the open.  Where the kernel reports
/// write-back errors through `syncfs` (Linux 5.8 on) this is one `syncfs` per filesystem the
/// zones sit on, else a sync of each file, which on a store of thousands of files costs seconds.
///
/// The kernel reports a write-back error to the first caller that asks, and to no later one: a
/// second open, or another program's `syncfs` on the same filesystem, would be told nothing.  So
/// a failed flush leaves the store's refusal marker (`Refusal`), and every later open is refused
/// until an operator clears it.  A marker that cannot be written either is said so in the error.
/// `syncfs` also reports another program's failed write on the same filesystem, which refuses
/// the open as well.  Each zone directory is synced last.
pub fn flush_for_replay(root: &Path, dirs: &[PathBuf]) -> Outcome<()> {
    let by_fs = match hooks::replay_flush() {
        hooks::ReplayFlush::Kernel  => syncfs_reports_errors(),
        hooks::ReplayFlush::Syncfs  => cfg!(target_os = "linux"),
        hooks::ReplayFlush::PerFile => false,
    };
    if by_fs {
        res!(syncfs_each(root, dirs));
    } else {
        for dir in dirs {
            res!(sync_files_in(root, dir));
        }
    }
    for dir in dirs {
        res!(sync_dir(dir));
    }
    Ok(())
}

// Before 5.8 `syncfs` returned success whatever write-back had failed.  A release that cannot
// be read or parsed is taken as too old.
fn syncfs_reports_errors() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    let rel = match hooks::kernel_release() {
        Some(rel)   => Ok(rel),
        None        => std::fs::read_to_string("/proc/sys/kernel/osrelease"),
    };
    match rel {
        Ok(rel) => release_at_least(&rel, (5, 8)),
        Err(_)  => false,
    }
}

// Is the kernel release, as `uname -r` gives it, at least `want` (major, minor)?
fn release_at_least(rel: &str, want: (u32, u32)) -> bool {
    let mut parts = rel.trim().split(|c: char| !c.is_ascii_digit());
    let major = parts.next().and_then(|p| p.parse::<u32>().ok());
    let minor = parts.next().and_then(|p| p.parse::<u32>().ok());
    match (major, minor) {
        (Some(a), Some(b))  => (a, b) >= want,
        _                   => false,
    }
}

#[cfg(target_os = "linux")]
fn syncfs_each(root: &Path, dirs: &[PathBuf]) -> Outcome<()> {
    use std::collections::BTreeSet;
    use std::os::unix::fs::MetadataExt;
    let mut devs = BTreeSet::new();
    for dir in dirs {
        let d = match File::open(dir) {
            Ok(d) => d,
            Err(e) => return Err(err!(e,
                "Cannot open zone directory {:?} of the store at {:?} to flush its filesystem \
                before replay, so the store is not opened.", dir, root;
                Init, IO, File, Read)),
        };
        let dev = res!(d.metadata()).dev();
        if !devs.insert(dev) {
            continue; // flushed already
        }
        hooks::count_replay_flush();
        let synced = hooks::replay_sync(nix::unistd::syncfs(&d), nix::errno::Errno::EIO);
        if let Err(e) = synced {
            return Err(Refusal::refuse(root, err!(e,
                "The filesystem holding {:?} reported a write-back error, so the open of the \
                store at {:?} is refused: a record replayed from it might not survive a power \
                cut.  The error may be another program's on the same filesystem.", dir, root;
                Init, IO, File, Write)));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn syncfs_each(root: &Path, dirs: &[PathBuf]) -> Outcome<()> {
    for dir in dirs {
        res!(sync_files_in(root, dir));
    }
    Ok(())
}

fn sync_files_in(root: &Path, dir: &Path) -> Outcome<()> {
    let list = match std::fs::read_dir(dir) {
        Ok(list) => list,
        Err(e) => return Err(err!(e,
            "Cannot list zone directory {:?} of the store at {:?} to sync its files before \
            replay, so the store is not opened.", dir, root;
            Init, IO, File, Read)),
    };
    for item in list {
        let path = res!(item).path();
        if ZoneDir::ozone_file_number_and_type(&path).is_err() {
            continue; // neither data nor index
        }
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(err!(e,
                "Cannot open {:?} to sync it before replay, so the store at {:?} is not opened.",
                path, root;
                Init, IO, File, Read)),
        };
        hooks::count_replay_flush();
        let synced = hooks::replay_sync(file.sync_data(), std::io::Error::from_raw_os_error(5));
        if let Err(e) = synced {
            return Err(Refusal::refuse(root, err!(e,
                "Cannot make {:?} durable before replaying it, so the open of the store at {:?} \
                is refused: a record replayed from it might not survive a power cut.", path, root;
                Init, IO, File, Write)));
        }
    }
    Ok(())
}
