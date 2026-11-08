//! A user namespace in which the caller keeps its own uid and gid, a mount namespace of its own,
//! and the mounts such a process may make there: an overlay, so that writes land in an upper
//! folder and the lower one is left as it was.
//!
//! The kernel lets an unprivileged program make these only where its security module allows. Under
//! AppArmor's `apparmor_restrict_unprivileged_userns`, an unconfined program's namespace is made but
//! its id maps are refused, so [`enter_own`] fails with `EPERM`; a program a profile grants
//! `userns`, `capability sys_admin` and `mount` succeeds.
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids.

use oxedyne_fe2o3_core::prelude::*;

use nix::mount::{
    mount,
    umount2,
    MntFlags,
    MsFlags,
};
use nix::sched::{
    unshare,
    CloneFlags,
};
use nix::unistd::{
    getgid,
    getuid,
};

use std::{
    fs,
    path::Path,
};

/// Moves the caller into a new user namespace and a new mount namespace, keeping its own uid and
/// gid, with every mount private so nothing it mounts reaches the namespace it left.  With `pid`,
/// its next child is the first process of a new pid namespace.  The caller must have one thread:
/// the kernel refuses a user namespace to a process with more.
pub fn enter_own(pid: bool) -> Outcome<()> {
    let uid = getuid();
    let gid = getgid();
    let mut flags = CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS;
    if pid {
        flags |= CloneFlags::CLONE_NEWPID;
    }
    if let Err(e) = unshare(flags) {
        return Err(err!("A user and mount namespace could not be made: {}.", e;
            System, Security));
    }
    // fixed: kernel ABI paths and formats.
    for (file, line) in [
        ("/proc/self/setgroups",    fmt!("deny")),
        ("/proc/self/uid_map",      fmt!("{} {} 1", uid, uid)),
        ("/proc/self/gid_map",      fmt!("{} {} 1", gid, gid)),
    ] {
        res!(fs::write(file, line.as_bytes()).map_err(|e| err!(e,
            "The new user namespace's {} could not be written.", file; System, Security)));
    }
    if let Err(e) = mount(None::<&str>, "/", None::<&str>, MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>)
    {
        return Err(err!("The new mount namespace could not be made private: {}.", e;
            System, Security));
    }
    Ok(())
}

/// Mounts an overlay on `at` whose writes land in `upper`, with `work` its work folder on the same
/// file system, and `lower` left unchanged.  No set-user-id bit or device node in it takes effect.
pub fn mount_overlay(lower: &Path, upper: &Path, work: &Path, at: &Path) -> Outcome<()> {
    let mut parts: Vec<&str> = Vec::new();
    for p in [lower, upper, work] {
        let s = res!(p.to_str().ok_or_else(|| err!(
            "The overlay folder {} is not UTF-8.", p.display(); Invalid, Input, Path)));
        // The kernel splits the options on these, and `:` separates lower layers.
        if s.contains([',', ':', '\\']) {
            return Err(err!("The overlay folder {} holds a ',', ':' or '\\', which the kernel \
                would read as a separator.", s; Invalid, Input, Path));
        }
        parts.push(s);
    }
    let opts = fmt!("lowerdir={},upperdir={},workdir={},userxattr", parts[0], parts[1], parts[2]);
    match mount(Some("overlay"), at, Some("overlay"), MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some(opts.as_str()))
    {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!("The overlay on {} could not be mounted ({}): {}.", at.display(), opts, e;
            System, Security)),
    }
}

/// Detaches the mount on `at` from the caller's mount namespace.
pub fn unmount(at: &Path) -> Outcome<()> {
    match umount2(at, MntFlags::MNT_DETACH) {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!("The mount on {} could not be detached: {}.", at.display(), e;
            System, Security)),
    }
}
