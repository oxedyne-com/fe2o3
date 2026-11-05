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
    sys::stat::Mode,
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
