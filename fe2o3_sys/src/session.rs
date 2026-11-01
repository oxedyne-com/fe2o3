//! Dropping a root process to a user, with a check that the drop held, and the ownership test a
//! root process applies to anything it trusts.
//!
//! [`drop_to`] sets the supplementary groups, then the real, effective and saved group ids, then
//! the three user ids, in that order: the groups and the gid need the privilege the uid change
//! gives up. It then reads all three back with `getresuid`, `getresgid` and `getgroups`, and where
//! the target is not root, tries to take uid 0 back and requires that to fail. A drop that did not
//! hold is an error, never a warning, so the caller cannot `exec` on a half-dropped process.
//!
//! [`strict_owner`] is OpenSSH's StrictModes for a root daemon: the path and every directory above
//! it, along the path as named and along the path it resolves to, must be owned by root and not
//! writable by group or others. Otherwise a user who can write any one of them can replace what
//! root reads.
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids.

use oxedyne_fe2o3_core::prelude::*;

use nix::unistd::{
    getgrouplist,
    getgroups,
    getresgid,
    getresuid,
    setgroups,
    setresgid,
    setresuid,
    setsid,
    Gid,
    Uid,
    User,
};

use std::{
    collections::BTreeSet,
    ffi::CString,
    fs,
    os::unix::fs::MetadataExt,
    path::{
        Component,
        Path,
        PathBuf,
    },
};

/// The ids a process runs as once dropped.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    pub uid:    u32,
    pub gid:    u32,
    pub groups: Vec<u32>, // Supplementary groups, the primary gid among them.
}

impl Identity {
    /// The user's ids from the password and group databases, as `initgroups` would set them.
    pub fn of_user(name: &str) -> Outcome<Self> {
        let user = match User::from_name(name) {
            Ok(Some(u)) => u,
            Ok(None)    => return Err(err!("No user named '{}' in the password database.", name;
                Missing, Input)),
            Err(e)      => return Err(err!("Looking up user '{}': {}.", name, e; IO, System)),
        };
        let cname = res!(CString::new(name.as_bytes()).map_err(|_|
            err!("User name '{}' holds a NUL byte.", name; Invalid, Input)));
        let groups = match getgrouplist(&cname, user.gid) {
            Ok(g)   => g,
            Err(e)  => return Err(err!("Reading the groups of user '{}': {}.", name, e; IO, System)),
        };
        Ok(Self::new(user.uid.as_raw(), user.gid.as_raw(), groups.iter().map(|g| g.as_raw()).collect()))
    }

    /// An identity with `groups` sorted and free of repeats, and `gid` among them.
    pub fn new(uid: u32, gid: u32, groups: Vec<u32>) -> Self {
        let mut set: BTreeSet<u32> = groups.into_iter().collect();
        set.insert(gid);
        Self { uid, gid, groups: set.into_iter().collect() }
    }

    /// The ids the calling process runs as now, taking the real ids.
    pub fn current() -> Outcome<Self> {
        let u = res!(getresuid().map_err(|e| err!("getresuid: {}.", e; IO, System)));
        let g = res!(getresgid().map_err(|e| err!("getresgid: {}.", e; IO, System)));
        let groups = res!(getgroups().map_err(|e| err!("getgroups: {}.", e; IO, System)));
        Ok(Self::new(u.real.as_raw(), g.real.as_raw(), groups.iter().map(|g| g.as_raw()).collect()))
    }
}

/// A user's entry in the password database, found by uid through NSS (`getpwuid_r`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Account {
    pub name:   String,
    pub uid:    u32,
    pub gid:    u32,
    pub home:   PathBuf,
    pub shell:  PathBuf,    // the login shell, empty when the entry names none
}

impl Account {
    pub fn of_uid(uid: u32) -> Outcome<Self> {
        match User::from_uid(Uid::from_raw(uid)) {
            Ok(Some(u)) => Ok(Self {
                name:   u.name,
                uid:    u.uid.as_raw(),
                gid:    u.gid.as_raw(),
                home:   u.dir,
                shell:  u.shell,
            }),
            Ok(None)    => Err(err!("No user with uid {} in the password database.", uid; Missing, Input)),
            Err(e)      => Err(err!("Looking up uid {}: {}.", uid, e; IO, System)),
        }
    }
}

/// Drops the calling process to `id` and checks that the drop held. Needs root, or `CAP_SETUID`
/// and `CAP_SETGID`. Every thread must be dropped, so call it before any thread is started.
pub fn drop_to(id: &Identity) -> Outcome<()> {
    let groups: Vec<Gid> = id.groups.iter().map(|g| Gid::from_raw(*g)).collect();
    let gid = Gid::from_raw(id.gid);
    let uid = Uid::from_raw(id.uid);
    res!(setgroups(&groups).map_err(|e|
        err!("Setting supplementary groups {:?}: {}.", id.groups, e; IO, System)));
    res!(setresgid(gid, gid, gid).map_err(|e| err!("setresgid({}): {}.", id.gid, e; IO, System)));
    res!(setresuid(uid, uid, uid).map_err(|e| err!("setresuid({}): {}.", id.uid, e; IO, System)));
    check_dropped(id)
}

/// Is the calling process exactly `id`, in all three user ids, all three group ids and its
/// supplementary groups, and, where `id` is not root, unable to take uid 0 back?
pub fn check_dropped(id: &Identity) -> Outcome<()> {
    let u = res!(getresuid().map_err(|e| err!("getresuid: {}.", e; IO, System)));
    let ids = [u.real.as_raw(), u.effective.as_raw(), u.saved.as_raw()];
    if ids.iter().any(|x| *x != id.uid) {
        return Err(err!("The drop to uid {} did not hold: real, effective and saved uids are {:?}.",
            id.uid, ids; Invalid, System));
    }
    let g = res!(getresgid().map_err(|e| err!("getresgid: {}.", e; IO, System)));
    let ids = [g.real.as_raw(), g.effective.as_raw(), g.saved.as_raw()];
    if ids.iter().any(|x| *x != id.gid) {
        return Err(err!("The drop to gid {} did not hold: real, effective and saved gids are {:?}.",
            id.gid, ids; Invalid, System));
    }
    let now = res!(getgroups().map_err(|e| err!("getgroups: {}.", e; IO, System)));
    let mut now: BTreeSet<u32> = now.iter().map(|g| g.as_raw()).collect();
    // The kernel may or may not list the primary gid among the supplementary groups.
    now.insert(id.gid);
    let want: BTreeSet<u32> = id.groups.iter().copied().collect();
    if now != want {
        return Err(err!("The drop to groups {:?} did not hold: the process has {:?}.",
            want, now; Invalid, System));
    }
    if id.uid != 0 {
        let root = Uid::from_raw(0);
        if setresuid(root, root, root).is_ok() {
            return Err(err!("The drop to uid {} did not hold: uid 0 could be taken back.",
                id.uid; Invalid, System));
        }
    }
    Ok(())
}

/// Starts a new session with the caller as its leader, so it has no controlling terminal, and
/// answers the session id.
pub fn new_session() -> Outcome<i32> {
    match setsid() {
        Ok(pid) => Ok(pid.as_raw()),
        Err(e)  => Err(err!("setsid: {}.", e; IO, System)),
    }
}

/// Is `path`, and every directory above it, owned by root and not writable by group or others,
/// both along the path as named and along the path it resolves to? A symbolic link on the way
/// must itself be owned by root; its own mode bits mean nothing. `path` must be absolute.
pub fn strict_owner(path: &Path) -> Outcome<()> {
    if !path.is_absolute() {
        return Err(err!("strict_owner needs an absolute path, not '{}'.", path.display();
            Invalid, Input));
    }
    res!(strict_chain(path));
    let real = res!(fs::canonicalize(path).map_err(|e|
        err!("Resolving '{}': {}.", path.display(), e; IO, File, Read)));
    if real != path {
        res!(strict_chain(&real));
    }
    Ok(())
}

/// Checks each prefix of `path`, from the root down, without following the last component.
fn strict_chain(path: &Path) -> Outcome<()> {
    let mut at = PathBuf::new();
    for c in path.components() {
        match c {
            Component::RootDir      => at.push("/"),
            Component::Normal(n)    => at.push(n),
            Component::CurDir       => continue,
            Component::ParentDir    => return Err(err!("strict_owner refuses '..' in '{}'.",
                path.display(); Invalid, Input)),
            Component::Prefix(_)    => return Err(err!("strict_owner met a path prefix in '{}'.",
                path.display(); Invalid, Input)),
        }
        let m = res!(fs::symlink_metadata(&at).map_err(|e|
            err!("Reading the owner of '{}': {}.", at.display(), e; IO, File, Read)));
        res!(strict_entry(&at, m.uid(), m.mode(), m.file_type().is_symlink()));
    }
    Ok(())
}

/// The test [`strict_owner`] applies to one entry, from its owner and mode.
pub fn strict_entry(at: &Path, uid: u32, mode: u32, is_link: bool) -> Outcome<()> {
    if uid != 0 {
        return Err(err!("'{}' is owned by uid {}, not root.", at.display(), uid; Invalid, Input));
    }
    // fixed: S_IWGRP | S_IWOTH, the POSIX mode bits.
    if !is_link && mode & 0o022 != 0 {
        return Err(err!("'{}' is writable by group or others (mode {:o}).", at.display(),
            mode & 0o7777; Invalid, Input));
    }
    Ok(())
}
