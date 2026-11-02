//! Mount tables and the host-path resolver.
//!
//! A file written inside a container or another mount namespace reaches a fanotify reader with a
//! descriptor whose link, read from the reader's `/proc/self/fd`, is a path in the writer's view.
//! [`resolve`] turns it into a path in the reader's own view, the host path. Where the descriptor's
//! mount (by `statx` `STATX_MNT_ID`) is one of the reader's, the link already is that path. Where it
//! is not, the mount must appear in the writer's `/proc/<pid>/mountinfo` (anything else, a pid
//! reused in another namespace for one, is unresolved, never guessed); the link is taken relative
//! to that mount's point, joined to the mount's root within its filesystem, and placed under the
//! reader's mount of the same superblock whose root holds it, the shortest such root first.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::BTreeMap,
    fs,
    os::fd::AsFd,
    path::PathBuf,
};

/// One line of a mountinfo file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mount {
    pub id:     u64,
    pub dev:    String, // major:minor of the superblock.
    pub root:   String, // The mount's root within its filesystem.
    pub point:  String, // Where it is mounted, in the reading process's view.
    pub fstype: String,
    pub source: String,
    pub rw:     bool,   // Writable both as mounted and as a superblock.
}

/// What a mount's filesystem is, for a caller that must not hold a descriptor on a disk he may
/// pull, or a network or FUSE filesystem whose server may hang.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsClass {
    Local,      // A fixed local disk, tmpfs, overlay, btrfs and the like.
    Removable,  // A block device whose disk sysfs marks `removable`.
    Fuse,
    Network,
    Pseudo,     // proc, sysfs, cgroup and the other kernel views: no files of anyone's.
}

// fixed: kernel and mount(8) filesystem type names.
const PSEUDO: &[&str] = &["proc", "sysfs", "cgroup", "cgroup2", "devpts", "mqueue", "debugfs", "tracefs",
    "securityfs", "pstore", "bpf", "configfs", "fusectl", "efivarfs", "binfmt_misc", "autofs", "hugetlbfs",
    "nsfs", "rpc_pipefs", "selinuxfs", "ramfs"];
const NETWORK: &[&str] = &["nfs", "nfs4", "cifs", "smb3", "smbfs", "9p", "ceph", "afs", "glusterfs",
    "lustre", "ocfs2", "gfs2", "ncpfs", "coda", "virtiofs"];

impl Mount {
    pub fn class(&self) -> FsClass {
        let t = self.fstype.as_str();
        if t == "fuse" || t == "fuseblk" || t.starts_with("fuse.") {
            return FsClass::Fuse;
        }
        if NETWORK.contains(&t) {
            return FsClass::Network;
        }
        if PSEUDO.contains(&t) {
            return FsClass::Pseudo;
        }
        // A partition's flag is its disk's, one directory up.
        let dir = format!("/sys/dev/block/{}", self.dev);
        let flag = |d: &str| fs::read_to_string(format!("{}/removable", d)).ok().map(|s| s.trim() == "1");
        match flag(&dir).or_else(|| flag(&format!("{}/..", dir))) {
            Some(true)  => FsClass::Removable,
            _           => FsClass::Local,
        }
    }
}

/// The mounts one process sees, by mount id.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MountTable {
    pub mounts: BTreeMap<u64, Mount>,
}

impl MountTable {
    /// Parses mountinfo text (proc(5)); a line that does not fit the format is an error.
    pub fn parse(text: &str) -> Outcome<Self> {
        let mut mounts = BTreeMap::new();
        for (n, line) in text.lines().enumerate() {
            if line.is_empty() { continue; }
            let f: Vec<&str> = line.split(' ').collect();
            let dash = match f.iter().position(|x| *x == "-") {
                Some(d) if d >= 6 && f.len() >= d + 3 => d,
                _ => return Err(err!("Mountinfo line {} is not in the proc(5) format: '{}'.",
                    n + 1, line; Invalid, Input)),
            };
            let id = res!(f[0].parse::<u64>().map_err(|_|
                err!("Mountinfo line {} has mount id '{}'.", n + 1, f[0]; Invalid, Input)));
            mounts.insert(id, Mount {
                id,
                dev:    f[2].to_string(),
                root:   unescape(f[3]),
                point:  unescape(f[4]),
                fstype: f[dash + 1].to_string(),
                source: unescape(f[dash + 2]),
                rw:     has_rw(f[5]) && f.get(dash + 3).map_or(true, |o| has_rw(o)),
            });
        }
        Ok(Self { mounts })
    }

    pub fn read(path: &str) -> Outcome<Self> {
        let text = res!(fs::read_to_string(path).map_err(|e|
            err!("Reading '{}': {}.", path, e; IO, File, Read)));
        Self::parse(&text)
    }

    pub fn of_self() -> Outcome<Self> {
        Self::read("/proc/self/mountinfo")
    }

    /// The mounts process `pid` sees, or `None` when it has gone or its table cannot be read.
    pub fn of_pid(pid: i32) -> Option<Self> {
        Self::read(&format!("/proc/{}/mountinfo", pid)).ok()
    }

    pub fn get(&self, id: u64) -> Option<&Mount> {
        self.mounts.get(&id)
    }

    /// The mounts in `self` that `before` lacks.
    pub fn added_since(&self, before: &MountTable) -> Vec<Mount> {
        self.mounts.values().filter(|m| !before.mounts.contains_key(&m.id)).cloned().collect()
    }
}

fn has_rw(opts: &str) -> bool {
    opts.split(',').any(|o| o == "rw")
}

/// Undoes the octal escapes mountinfo writes for space, tab, newline and backslash.
pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let v = (b[i + 1] - b'0') as u32 * 64 + (b[i + 2] - b'0') as u32 * 8 + (b[i + 3] - b'0') as u32;
            if v <= 0xff {
                out.push(v as u8);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The mount id of the file `fd` refers to, from `statx` `STATX_MNT_ID` (Linux 5.8).
pub fn mount_id<Fd: AsFd>(fd: Fd) -> Outcome<(u64, u64)> {
    use rustix::fs::{AtFlags, StatxFlags};
    let x = res!(rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID | StatxFlags::INO)
        .map_err(|e| err!("statx for the mount id: {}.", e; IO, System)));
    if x.stx_mask & StatxFlags::MNT_ID.bits() == 0 {
        return Err(err!("The kernel did not report STATX_MNT_ID."; Missing, System));
    }
    Ok((x.stx_mnt_id, x.stx_ino))
}

/// Where a written file is, in the reader's own view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostPath {
    Own(PathBuf),           // On one of the reader's mounts: the link is the path.
    Mapped(PathBuf),        // Through the writer's mountinfo onto a reader mount.
    Unresolved(Unresolved),
}

impl HostPath {
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::Own(p) | Self::Mapped(p)  => Some(p),
            Self::Unresolved(_)             => None,
        }
    }
}

/// Why a written file has no host path. Every case is a loss the caller must make good, by a
/// rescan for one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unresolved {
    WriterGone,                                     // Its mountinfo could not be read.
    MountNotInWriter { mnt: u64 },                  // N1: never map through a stranger's table.
    NotUnderMount { link: String, point: String },
    NoHostMount { dev: String, fs_path: String },
    Deleted { link: String },                       // Unlinked before it was read.
}

// fixed: the suffix the kernel's d_path gives an unlinked file.
const DELETED: &str = " (deleted)";

/// Resolves `link`, a path read from `/proc/self/fd` for a descriptor on mount `mnt`, to a host
/// path, through `writer`'s mount table where the mount is not one of `own`.
pub fn resolve(link: &str, mnt: u64, own: &MountTable, writer: Option<&MountTable>) -> HostPath {
    if link.ends_with(DELETED) {
        return HostPath::Unresolved(Unresolved::Deleted { link: link.to_string() });
    }
    if own.mounts.contains_key(&mnt) {
        return HostPath::Own(PathBuf::from(link));
    }
    let theirs = match writer {
        Some(t) => t,
        None    => return HostPath::Unresolved(Unresolved::WriterGone),
    };
    let m = match theirs.get(mnt) {
        Some(m) => m,
        None    => return HostPath::Unresolved(Unresolved::MountNotInWriter { mnt }),
    };
    let rel = match under(link, &m.point) {
        Some(r) => r,
        None    => return HostPath::Unresolved(Unresolved::NotUnderMount {
            link: link.to_string(), point: m.point.clone() }),
    };
    let fs_path = join(&m.root, &rel);
    let mut best: Option<(usize, String)> = None;
    for h in own.mounts.values() {
        if h.dev != m.dev { continue; }
        if let Some(tail) = under(&fs_path, &h.root) {
            if best.as_ref().map_or(true, |(l, _)| h.root.len() < *l) {
                best = Some((h.root.len(), join(&h.point, &tail)));
            }
        }
    }
    match best {
        Some((_, p))    => HostPath::Mapped(PathBuf::from(p)),
        None            => HostPath::Unresolved(Unresolved::NoHostMount { dev: m.dev.clone(), fs_path }),
    }
}

/// `p` relative to `pre` as an absolute path ("/" for `pre` itself), or `None` when outside it.
fn under(p: &str, pre: &str) -> Option<String> {
    if pre == "/" { return Some(p.to_string()); }
    match p.strip_prefix(pre) {
        Some("")                        => Some("/".to_string()),
        Some(r) if r.starts_with('/')   => Some(r.to_string()),
        _                               => None,
    }
}

fn join(a: &str, b: &str) -> String {
    if b == "/" { a.to_string() } else if a == "/" { b.to_string() } else { format!("{}{}", a, b) }
}
