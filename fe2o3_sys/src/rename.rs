//! Two paths exchanged in one step.
//!
//! A program that replaces one directory with another it has prepared, a store's log with its
//! rewritten copy, has only two renames to do it with in the standard library: the old one aside,
//! then the new one into its place. Between them there is no directory at the name at all, and a
//! process killed there leaves a reader that finds nothing and a person who must choose by hand
//! which of the two to put back. [`exchange`] is Linux's `renameat2` with `RENAME_EXCHANGE`: the
//! two names swap what they refer to in one step of the filesystem, so at every instant each name
//! holds one of the two, complete, and a kill leaves either the old state or the swapped one.
//!
//! A filesystem that cannot exchange, and a C library without the call, answer an error tagged
//! [`ErrTag::Unimplemented`], which [`unsupported`] recognises, so a caller can take its own
//! slower road on that filesystem and refuse everything else. Nothing is changed when it fails.
//!
//! The system call comes from `nix`, which owns the `unsafe` this crate forbids.

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;


/// Swaps what `a` and `b` name, both of which must exist, in one step.
///
/// Either may be a file or a directory, and they need not be of one kind. Both must be on one
/// filesystem. The swap is atomic against a crash in the sense the filesystem gives a rename: a
/// caller that needs it to survive power loss syncs the parent directory afterwards.
#[cfg(target_env = "gnu")]
pub fn exchange(a: &Path, b: &Path) -> Outcome<()> {
    use nix::errno::Errno;
    use nix::fcntl::{
        renameat2,
        RenameFlags,
        AT_FDCWD,
    };
    match renameat2(AT_FDCWD, a, AT_FDCWD, b, RenameFlags::RENAME_EXCHANGE) {
        Ok(())  => Ok(()),
        // EINVAL is what a filesystem that has no exchange answers, ENOSYS a kernel older than
        // 3.15, and EOPNOTSUPP what some network filesystems answer instead.
        Err(e @ (Errno::EINVAL | Errno::ENOSYS | Errno::EOPNOTSUPP)) => Err(err!(
            "{:?} and {:?} cannot be exchanged in one step here: {}.", a, b, e;
            IO, File, Unimplemented)),
        Err(e)  => Err(err!(
            "{:?} and {:?} could not be exchanged: {}.", a, b, e;
            IO, File, Write)),
    }
}

/// Swaps what `a` and `b` name in one step: a C library without `renameat2` cannot, and says so.
#[cfg(not(target_env = "gnu"))]
pub fn exchange(a: &Path, b: &Path) -> Outcome<()> {
    Err(err!(
        "{:?} and {:?} cannot be exchanged in one step: this build's C library has no renameat2.",
        a, b;
        IO, File, Unimplemented))
}

/// Says whether an error from [`exchange`] means only that the filesystem or the system cannot
/// exchange, so that nothing was attempted and another road may be taken.
pub fn unsupported(e: &Error<ErrTag>) -> bool {
    e.tags().contains(&ErrTag::Unimplemented)
}
