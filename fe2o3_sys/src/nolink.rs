//! Opening a path through no symbolic link at all, for a root process acting on a path a less
//! trusted user can shape.
//!
//! `O_NOFOLLOW` covers only the last component: a root open of `/home/u/a/b` still follows a
//! link at `a`, so a user who owns `a` can point root anywhere.  [`open`] resolves with
//! `openat2` and `RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, so a link at any component,
//! `/proc/self/fd` style magic links included, fails the open (`ELOOP`) instead of being
//! followed.  The kernel needs `openat2` (Linux 5.6); without it the open fails rather than
//! falling back to a following open.  The system call comes from `nix`, which owns the `unsafe`
//! this crate forbids.
//!
//! [`held_dir`] and [`own_dir`] find or make a folder by descriptor for a root process making
//! a folder for another account: the base is opened through no link and judged by `fstat`, and
//! each folder in it is made with `mkdirat` on the base's descriptor and handed over with
//! `fchown` and `fchmod` on its own open descriptor, so no step resolves a name twice and none
//! changes a file by path.
use oxedyne_fe2o3_core::prelude::*;

use std::{
    os::fd::{
        BorrowedFd,
        OwnedFd,
    },
    path::Path,
};

use nix::{
    errno::Errno,
    fcntl::{
        openat2,
        OFlag,
        OpenHow,
        ResolveFlag,
        AT_FDCWD,
    },
    sys::stat::{
        fchmod,
        fstat,
        fstatat,
        mkdirat,
        FileStat,
        Mode,
        SFlag,
    },
    unistd::{
        fchown,
        geteuid,
        Gid,
        Uid,
    },
};

/// What the open is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Want {
    Dir,    // A folder, read-only (`O_DIRECTORY`).
    File,   // Anything but a folder is refused by the caller's own check; read-only.
    Path,   // A handle only (`O_PATH`), to mount on or to stat; fdget-based calls refuse it.
}

/// What [`try_open`] found.
#[derive(Debug)]
pub enum Got {
    Fd(OwnedFd),
    Absent, // no entry at some component (ENOENT)
    Linked, // a symbolic link at some component (ELOOP)
}

/// Opens `path`, relative to `at` where given, through no symbolic link in any component.
/// Every open is close-on-exec and never blocks on a FIFO.
pub fn open(at: Option<BorrowedFd>, path: &Path, want: Want) -> Outcome<OwnedFd> {
    raw(at, path, want).map_err(|e| err!(e, "Opening {:?} through no symbolic link ({:?}).", path, want; IO, File))
}

/// As [`open`], telling an absent path and a linked one from any other failure.
pub fn try_open(at: Option<BorrowedFd>, path: &Path, want: Want) -> Outcome<Got> {
    match raw(at, path, want) {
        Ok(fd)              => Ok(Got::Fd(fd)),
        Err(Errno::ENOENT)  => Ok(Got::Absent),
        Err(Errno::ELOOP)   => Ok(Got::Linked),
        Err(e) => Err(err!(e, "Opening {:?} through no symbolic link ({:?}).", path, want; IO, File)),
    }
}

fn raw(at: Option<BorrowedFd>, path: &Path, want: Want) -> nix::Result<OwnedFd> {
    // No O_NOFOLLOW: with O_PATH it would open a trailing link itself, where the resolve
    // flags alone refuse it.
    let fl = OFlag::O_CLOEXEC | match want {
        Want::Dir   => OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NONBLOCK,
        Want::File  => OFlag::O_RDONLY | OFlag::O_NONBLOCK,
        Want::Path  => OFlag::O_PATH,
    };
    let how = OpenHow::new()
        .flags(fl)
        .mode(Mode::empty())
        .resolve(ResolveFlag::RESOLVE_NO_SYMLINKS | ResolveFlag::RESOLVE_NO_MAGICLINKS);
    match at {
        Some(d) => openat2(d, path, how),
        None    => openat2(AT_FDCWD, path, how),
    }
}

/// What kind of file `mode` names, for a refusal.
fn kind(mode: u32) -> &'static str {
    let t = SFlag::from_bits_truncate(mode) & SFlag::S_IFMT;
    if t == SFlag::S_IFDIR {
        "a folder"
    } else if t == SFlag::S_IFLNK {
        "a link"
    } else if t == SFlag::S_IFREG {
        "a regular file"
    } else {
        "a special file"
    }
}

/// Refuses `st`, found at `shown`, unless it is a real folder owned by `uid` with exactly `mode`.
/// The refusal names the folder, what it is, its owner and mode, and what was expected.
fn judge(st: &FileStat, shown: &Path, uid: u32, mode: u32) -> Outcome<()> {
    let m = st.st_mode as u32;
    let dir = SFlag::from_bits_truncate(st.st_mode) & SFlag::S_IFMT == SFlag::S_IFDIR;
    if !dir || st.st_uid != uid || m & 0o7777 != mode {
        return Err(err!("'{}' is {} owned by uid {} gid {}, mode {:o}; it must be a folder of \
            uid {}, mode {:o}.", shown.display(), kind(m), st.st_uid, st.st_gid, m & 0o7777, uid, mode;
            Invalid, Security, Path));
    }
    Ok(())
}

/// Opens the folder `path` through no symbolic link and refuses it unless it is a real folder
/// owned by `uid` with exactly `mode`, and every folder above it is root's and writable by no
/// other account unless sticky, so no other account can rename it, or a folder above it, away.
///
/// # Arguments
/// * `path` - The folder, absolute.
/// * `uid` - Its owner.
/// * `mode` - Its permission bits, exactly (`0o755`).
pub fn held_dir(path: &Path, uid: u32, mode: u32) -> Outcome<OwnedFd> {
    if !path.is_absolute() {
        return Err(err!("The folder '{}' is not absolute.", path.display(); Invalid, Input, Path));
    }
    for a in path.ancestors().skip(1) {
        let fd = res!(open(None, a, Want::Path));
        let st = res!(fstat(&fd).map_err(|e| err!(e, "Could not stat '{}'.", a.display(); IO, File)));
        let m = st.st_mode as u32;
        if st.st_uid != 0 || (m & 0o022 != 0 && m & 0o1000 == 0) {
            return Err(err!("'{}' sits in '{}', owned by uid {} gid {}, mode {:o}: it must be root's \
                and writable by no other account unless sticky, so no one can rename it away.",
                path.display(), a.display(), st.st_uid, st.st_gid, m & 0o7777; Invalid, Security, Path));
        }
    }
    let fd = match res!(try_open(None, path, Want::Path)) {
        Got::Fd(fd) => fd,
        Got::Absent => return Err(err!("The folder '{}' is missing; it must be a folder of uid {}, \
            mode {:o}.", path.display(), uid, mode; Missing, Path)),
        Got::Linked => return Err(err!("'{}' is a link or lies under one; it must be a folder of \
            uid {}, mode {:o}.", path.display(), uid, mode; Invalid, Security, Path)),
    };
    let st = res!(fstat(&fd).map_err(|e| err!(e, "Could not stat '{}'.", path.display(); IO, File)));
    res!(judge(&st, path, uid, mode));
    open(None, path, Want::Dir).and_then(|d| {
        let again = res!(fstat(&d).map_err(|e| err!(e, "Could not stat '{}'.", path.display(); IO, File)));
        if (again.st_dev, again.st_ino) != (st.st_dev, st.st_ino) {
            return Err(err!("The folder '{}' changed while it was opened.", path.display();
                Security, Mismatch, Path));
        }
        Ok(d)
    })
}

/// In the folder `at`, opens the folder `name`, making it first when it is absent, and refuses
/// it unless it is a real folder owned by `uid` with exactly `mode`.  A folder this call makes is
/// one `mkdirat` on `at`, then `fchown` and `fchmod` on its own descriptor, opened with `openat2`
/// through no link and no mount crossing (`RESOLVE_NO_XDEV`); one found is judged, never changed.
/// A mount on `name` is refused as one, since a mount stands in for the folder whatever lies
/// beneath it; a link is refused as a link even when it is swapped for a folder between the open
/// and the look that names it.  `at` must be writable by no account but the caller and `uid`, as
/// [`held_dir`] proves of a base.
///
/// # Arguments
/// * `at` - The folder it lies in, open.
/// * `at_path` - Where `at` is, for the messages.
/// * `name` - One path component.
/// * `uid` - Its owner.
/// * `gid` - Its group, when made.
/// * `mode` - Its permission bits, exactly (`0o700`).
pub fn own_dir(
    at:         BorrowedFd,
    at_path:    &Path,
    name:       &str,
    uid:        u32,
    gid:        u32,
    mode:       u32,
)
    -> Outcome<OwnedFd>
{
    let shown = at_path.join(name);
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(err!("'{}' is not one folder name.", name; Invalid, Input, Path));
    }
    // No O_NOFOLLOW: the resolve flags refuse a trailing link (ELOOP) and a mount on the name
    // (EXDEV), where O_NOFOLLOW with O_DIRECTORY gives a bare ENOTDIR for a link.
    let how = OpenHow::new()
        .flags(OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK)
        .mode(Mode::empty())
        .resolve(ResolveFlag::RESOLVE_NO_SYMLINKS | ResolveFlag::RESOLVE_NO_MAGICLINKS
            | ResolveFlag::RESOLVE_NO_XDEV);
    let refuse = |e: Errno| -> Error<ErrTag> {
        if e == Errno::EXDEV {
            return err!(e, "'{}' is a mount point; it must be a folder of uid {}, mode {:o}, on the \
                filesystem of '{}', since a mount stands in for whatever lies beneath it.",
                shown.display(), uid, mode, at_path.display(); Invalid, Security, Path);
        }
        // The second look judges what is there now; a refusal the open met and the look did not
        // see was swapped in between, and is named as what the open met.
        let met = match e {
            Errno::ELOOP    => Some("a link"),
            Errno::ENOTDIR  => Some("not a folder"),
            _               => None,
        };
        let swapped = |found: &str| match met {
            Some(m) => err!(e, "'{}' was {} when it was opened and {} when looked at again: it was \
                swapped in between; it must be a folder of uid {}, mode {:o}.", shown.display(), m,
                found, uid, mode; Invalid, Security, Path),
            None    => err!(e, "Could not open the folder '{}'.", shown.display(); IO, File),
        };
        match fstatat(at, name, nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(st) => match judge(&st, &shown, uid, mode) {
                Err(j) => j,
                Ok(()) => swapped("a folder"),
            },
            Err(Errno::ENOENT) => swapped("absent"),
            Err(_) => swapped("not to be looked at"),
        }
    };
    let mut made = false;
    let fd = match openat2(at, name, how) {
        Ok(fd) => fd,
        Err(Errno::ENOENT) => {
            match mkdirat(at, name, Mode::from_bits_truncate(mode)) {
                Ok(()) => made = true,
                Err(Errno::EEXIST) => (),
                Err(e) => return Err(err!(e, "Could not make the folder '{}' for uid {} (as uid {}).",
                    shown.display(), uid, geteuid().as_raw(); IO, File, Path)),
            }
            res!(openat2(at, name, how).map_err(refuse))
        },
        Err(e) => return Err(refuse(e)),
    };
    if made {
        let st = res!(fstat(&fd).map_err(|e| err!(e, "Could not stat '{}'.", shown.display(); IO, File)));
        // What was just made is the caller's: anything else was put there in between.
        let me = geteuid().as_raw();
        if SFlag::from_bits_truncate(st.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR || st.st_uid != me {
            return Err(err!("The folder '{}' made by uid {} was replaced before it was handed to uid {}.",
                shown.display(), me, uid; Security, Mismatch, Path));
        }
        if me != uid {
            res!(fchown(&fd, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid))).map_err(|e| err!(e,
                "Could not hand the folder '{}' to uid {}.", shown.display(), uid; IO, File, Path)));
        }
        res!(fchmod(&fd, Mode::from_bits_truncate(mode)).map_err(|e| err!(e,
            "Could not set the mode of the folder '{}' to {:o}.", shown.display(), mode; IO, File, Path)));
    }
    let st = res!(fstat(&fd).map_err(|e| err!(e, "Could not stat '{}'.", shown.display(); IO, File)));
    res!(judge(&st, &shown, uid, mode));
    Ok(fd)
}
