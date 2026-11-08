//! The changes an overlayfs upper directory holds over its lower directory, as a typed op list.
//!
//! [`changes`] walks the upper beside the lower and names what applying it would do: [`Op::Create`]
//! for a path the lower lacks, [`Op::Write`] for new contents, [`Op::Delete`] for a whiteout,
//! [`Op::Opaque`] for a directory whose lower contents are hidden, [`Op::Meta`] for a change of
//! mode, modification time or extended attributes alone, and [`Op::Refused`] for an entry this list
//! cannot carry, with the reason. With `redirect_dir` not creating redirects, overlayfs renames a
//! file by copying it up and leaves a whiteout behind, and refuses a directory rename with `EXDEV`,
//! so `mv` copies it; either way a rename reaches the list as a delete of the old path and a create
//! of the new, and there is no rename op.
//!
//! The upper may have been written by a hostile program, so the walk follows no symbolic link:
//! both roots are opened with `RESOLVE_NO_SYMLINKS`, every child by name from its parent's
//! descriptor with `O_NOFOLLOW`, and a directory is reopened for its turn only if it is still the
//! inode first seen. The walk keeps a work list rather than recursing, so a deep upper costs heap,
//! not stack. An odd entry is refused alone, as an op; the whole walk is refused only past the
//! caller's entry cap.
//!
//! Only the mount's own overlay attributes ([`Namespace`]) are marks. Every other extended
//! attribute is data, compared for [`Op::Meta`].
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::BTreeMap,
    ffi::{
        OsStr,
        OsString,
    },
    fs::File,
    io::Read,
    os::{
        fd::OwnedFd,
        unix::ffi::OsStrExt,
    },
    path::{
        Path,
        PathBuf,
    },
    rc::Rc,
};

use rustix::fs::{
    AtFlags,
    FileType,
    Mode,
    OFlags,
    ResolveFlags,
    Stat,
    CWD,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Link,
}

/// The extended attribute namespace the mount keeps its marks in: `user` for a mount with
/// `userxattr`, as in an unprivileged user namespace, `trusted` otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    User,
    Trusted,
}

impl Namespace {
    fn prefix(&self) -> &'static str {
        match self {
            Self::User      => "user.overlay.",     // fixed: overlayfs names
            Self::Trusted   => "trusted.overlay.",  // fixed: overlayfs names
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    Name,               // Not UTF-8, holds a control character, or longer than a path may be.
    Special,            // A pipe, socket or device other than a whiteout.
    Unreadable(String), // Could not be opened, listed or read; the error.
    Mark(String),       // Carries an overlay attribute no op expresses; its name.
    LinkParent,         // A directory over a lower symbolic link without an opaque mark.
    OverNonDir,         // A directory over a lower non-directory without an opaque mark.
    Changed,            // Replaced while it was read.
}

/// One change, at a path relative to the roots.  The applier reads what to write from the upper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Create(PathBuf, Kind),  // The lower has nothing here.
    Write(PathBuf, Kind),   // New file contents or a new link target.
    Delete(PathBuf),        // A whiteout over a lower entry.
    Opaque(PathBuf),        // Lower directory kept, its contents removed; creates beneath follow.
    Meta(PathBuf, Kind),    // Same contents, new mode, modification time or attributes.
    Refused(PathBuf, Why),  // The only op on its path, and none beneath it.
}

impl Op {
    pub fn path(&self) -> &Path {
        match self {
            Self::Create(p, _)
            | Self::Write(p, _)
            | Self::Delete(p)
            | Self::Opaque(p)
            | Self::Meta(p, _)
            | Self::Refused(p, _) => p,
        }
    }
}

const BUF_LEN: usize = 64 * 1024; // fixed: a read size, not a limit
const PATH_MAX: usize = 4096;      // fixed: Linux's limit on a path, its closing NUL included

/// Lists what `upper` changes over `lower`, in path order, a directory's op before those beneath
/// it.  Refuses the whole walk only for an unopenable root or more than `max_entries` entries.
pub fn changes(upper: &Path, lower: &Path, ns: Namespace, max_entries: usize) -> Outcome<Vec<Op>> {
    let up = Rc::new(res!(open_root(upper)));
    let low = Rc::new(res!(open_root(lower)));
    let mut walk = Walker { ops: Vec::new(), todo: Vec::new(), seen: 0, max: max_entries, ns };
    res!(walk.dir(up, Some(low), PathBuf::new()));
    while let Some(next) = walk.todo.pop() {
        let up = match reopen(&next.up, &next.name, &next.up_st) {
            Ok(fd) => fd,
            Err(why) => {
                walk.ops.push(Op::Refused(next.rel, why));
                continue;
            }
        };
        let low = match next.low {
            Some((fd, st)) => match reopen(&fd, &next.name, &st) {
                Ok(l) => Some(Rc::new(l)),
                Err(why) => {
                    walk.ops.push(Op::Refused(next.rel, why));
                    continue;
                }
            },
            None => None,
        };
        res!(walk.dir(Rc::new(up), low, next.rel));
    }
    // Stable, so two ops on one path keep their order: a delete before the create replacing it.
    walk.ops.sort_by(|a, b| a.path().cmp(b.path()));
    // A directory refused when reopened or listed already carries its parent's op for it, such as
    // the delete of the lower file it replaces, which must not be applied without its contents.
    // Nothing beneath it was listed, and its refusal sorts last on its path.
    let mut ops: Vec<Op> = Vec::with_capacity(walk.ops.len());
    for op in walk.ops {
        if let Op::Refused(p, _) = &op {
            while ops.last().map_or(false, |last| last.path() == p) {
                ops.pop();
            }
        }
        ops.push(op);
    }
    Ok(ops)
}

/// A directory owed a listing, held by its parents' descriptors and name so that a wide upper does
/// not hold a descriptor per directory.
struct Todo {
    up:     Rc<OwnedFd>,
    up_st:  Stat,
    low:    Option<(Rc<OwnedFd>, Stat)>,  // Present when the lower directory shows through.
    name:   OsString,
    rel:    PathBuf,
}

struct Walker {
    ops:    Vec<Op>,
    todo:   Vec<Todo>,
    seen:   usize,
    max:    usize,
    ns:     Namespace,
}

impl Walker {

    fn dir(&mut self, up: Rc<OwnedFd>, low: Option<Rc<OwnedFd>>, rel: PathBuf) -> Outcome<()> {
        let names = match self.names(&up, &rel) {
            Ok(Ok(n)) => n,
            Ok(Err(why)) => {
                self.ops.push(Op::Refused(rel, why));
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        for name in names {
            let path = rel.join(&name);
            if let Err(why) = self.entry(&up, low.as_ref(), &name, &path) {
                self.ops.push(Op::Refused(path, why));
            }
        }
        Ok(())
    }

    /// The entries of one upper directory, each counted against the cap, which alone ends the walk.
    fn names(&mut self, up: &OwnedFd, rel: &Path) -> Outcome<Result<Vec<OsString>, Why>> {
        let mut dir = match rustix::fs::Dir::read_from(up) {
            Ok(d) => d,
            Err(e) => return Ok(Err(Why::Unreadable(e.to_string()))),
        };
        let mut names = Vec::new();
        while let Some(entry) = dir.read() {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => return Ok(Err(Why::Unreadable(e.to_string()))),
            };
            let raw = entry.file_name().to_bytes();
            if raw == b"." || raw == b".." {
                continue;
            }
            self.seen += 1;
            if self.seen > self.max {
                return Err(err!("Overlay upper holds more than {} entries; refused at {:?}.",
                    self.max, rel; TooBig, Security));
            }
            names.push(OsStr::from_bytes(raw).to_os_string());
        }
        Ok(Ok(names))
    }

    fn entry(&mut self, up: &Rc<OwnedFd>, low: Option<&Rc<OwnedFd>>, name: &OsStr, path: &Path)
        -> Result<(), Why>
    {
        match name.to_str() {
            Some(s) if !s.chars().any(char::is_control) => {}
            _ => return Err(Why::Name),
        }
        // No call can name a longer path, and refusing here bounds a deep chain's paths.
        if path.as_os_str().len() >= PATH_MAX {
            return Err(Why::Name);
        }
        let ust = ok!(sys(rustix::fs::statat(up, name, AtFlags::SYMLINK_NOFOLLOW)));
        let lower = match low {
            Some(fd) => match rustix::fs::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) => Some((fd, st)),
                Err(rustix::io::Errno::NOENT) => None,
                Err(e) => return Err(Why::Unreadable(e.to_string())),
            },
            None => None,
        };
        let path = path.to_path_buf();
        match FileType::from_raw_mode(ust.st_mode) {
            FileType::CharacterDevice if is_whiteout(&ust) => {
                // A whiteout over nothing hides nothing: the lower entry is already gone.
                if lower.is_some() {
                    self.ops.push(Op::Delete(path));
                }
            }
            FileType::Directory => {
                let fd = ok!(open_child(up, name, OFlags::DIRECTORY, &ust));
                let (opaque, data) = ok!(self.attrs(&fd));
                match lower {
                    None => {
                        self.ops.push(Op::Create(path.clone(), Kind::Dir));
                        self.owe(up, ust, None, name, path);
                    }
                    Some((lfd, lst)) => match FileType::from_raw_mode(lst.st_mode) {
                        FileType::Directory => {
                            let ldir = ok!(open_child(lfd, name, OFlags::DIRECTORY, &lst));
                            let (_, ldata) = ok!(self.attrs(&ldir));
                            if mode_of(&ust) != mode_of(&lst) || data != ldata {
                                self.ops.push(Op::Meta(path.clone(), Kind::Dir));
                            }
                            if opaque {
                                self.ops.push(Op::Opaque(path.clone()));
                                self.owe(up, ust, None, name, path);
                            } else {
                                self.owe(up, ust, Some((Rc::clone(lfd), lst)), name, path);
                            }
                        }
                        FileType::Symlink if !opaque => return Err(Why::LinkParent),
                        _ if !opaque => return Err(Why::OverNonDir),
                        _ => {
                            self.ops.push(Op::Delete(path.clone()));
                            self.ops.push(Op::Create(path.clone(), Kind::Dir));
                            self.owe(up, ust, None, name, path);
                        }
                    },
                }
            }
            FileType::RegularFile => {
                let fd = ok!(open_child(up, name, OFlags::empty(), &ust));
                let (opaque, data) = ok!(self.attrs(&fd));
                if opaque {
                    return Err(Why::Mark(format!("{}opaque", self.ns.prefix())));
                }
                match lower {
                    None => self.ops.push(Op::Create(path, Kind::File)),
                    Some((lfd, lst)) if FileType::from_raw_mode(lst.st_mode)
                        == FileType::RegularFile =>
                    {
                        let lfile = ok!(open_child(lfd, name, OFlags::empty(), &lst));
                        let (_, ldata) = ok!(self.attrs(&lfile));
                        if !ok!(same_bytes(fd, lfile, &ust, &lst)) {
                            self.ops.push(Op::Write(path, Kind::File));
                        } else if meta_differs(&ust, &lst) || data != ldata {
                            self.ops.push(Op::Meta(path, Kind::File));
                        }
                    }
                    Some(_) => {
                        self.ops.push(Op::Delete(path.clone()));
                        self.ops.push(Op::Create(path, Kind::File));
                    }
                }
            }
            FileType::Symlink => match lower {
                None => self.ops.push(Op::Create(path, Kind::Link)),
                Some((lfd, lst)) if FileType::from_raw_mode(lst.st_mode) == FileType::Symlink => {
                    let ut = ok!(sys(rustix::fs::readlinkat(up, name, Vec::new())));
                    let lt = ok!(sys(rustix::fs::readlinkat(lfd, name, Vec::new())));
                    if ut != lt {
                        self.ops.push(Op::Write(path, Kind::Link));
                    }
                }
                Some(_) => {
                    self.ops.push(Op::Delete(path.clone()));
                    self.ops.push(Op::Create(path, Kind::Link));
                }
            },
            _ => return Err(Why::Special),
        }
        Ok(())
    }

    fn owe(&mut self, up: &Rc<OwnedFd>, up_st: Stat, low: Option<(Rc<OwnedFd>, Stat)>,
        name: &OsStr, rel: PathBuf)
    {
        self.todo.push(Todo { up: Rc::clone(up), up_st, low, name: name.to_os_string(), rel });
    }

    /// Is the entry marked opaque, and what are its data attributes?  Refuses a mark no op
    /// expresses, including an attribute a program set through the mount, which overlayfs escapes
    /// into the mark namespace.
    fn attrs(&self, fd: &OwnedFd) -> Result<(bool, BTreeMap<Vec<u8>, Vec<u8>>), Why> {
        let prefix = self.ns.prefix().as_bytes();
        let mut opaque = false;
        let mut data = BTreeMap::new();
        for name in ok!(xattr_names(fd)) {
            let val = ok!(xattr_value(fd, &name));
            match name.strip_prefix(prefix) {
                None => {
                    data.insert(name, val);
                }
                Some(b"origin") | Some(b"impure") | Some(b"uuid") => {}
                Some(b"opaque") if val == b"y" => opaque = true,
                Some(_) => return Err(Why::Mark(String::from_utf8_lossy(&name).into_owned())),
            }
        }
        Ok((opaque, data))
    }
}

fn sys<T>(r: rustix::io::Result<T>) -> Result<T, Why> {
    r.map_err(|e| Why::Unreadable(e.to_string()))
}

fn open_root(path: &Path) -> Outcome<OwnedFd> {
    match rustix::fs::openat2(CWD, path, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(), ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS)
    {
        Ok(fd) => Ok(fd),
        Err(e) => Err(err!("Overlay walk could not open, through no symbolic link, the root \
            {:?}: {}.", path, e; IO, Path)),
    }
}

/// Opens `name` without following a link, and only if it is still the inode `st` describes.
fn open_child(dir: &OwnedFd, name: &OsStr, extra: OFlags, st: &Stat) -> Result<OwnedFd, Why> {
    let fd = ok!(sys(rustix::fs::openat(dir, name, OFlags::RDONLY | OFlags::NOFOLLOW
        | OFlags::CLOEXEC | OFlags::NONBLOCK | extra, Mode::empty())));
    let now = ok!(sys(rustix::fs::fstat(&fd)));
    if now.st_ino != st.st_ino || now.st_dev != st.st_dev {
        return Err(Why::Changed);
    }
    Ok(fd)
}

fn reopen(dir: &OwnedFd, name: &OsStr, st: &Stat) -> Result<OwnedFd, Why> {
    open_child(dir, name, OFlags::DIRECTORY, st)
}

fn xattr_names(fd: &OwnedFd) -> Result<Vec<Vec<u8>>, Why> {
    let len = ok!(sys(rustix::fs::flistxattr(fd, &mut [0u8; 0][..])));
    let mut buf = vec![0u8; len];
    let n = ok!(sys(rustix::fs::flistxattr(fd, &mut buf[..])));
    Ok(buf[..n].split(|b| *b == 0).filter(|s| !s.is_empty()).map(<[u8]>::to_vec).collect())
}

fn xattr_value(fd: &OwnedFd, name: &[u8]) -> Result<Vec<u8>, Why> {
    let name = OsStr::from_bytes(name);
    let len = ok!(sys(rustix::fs::fgetxattr(fd, name, &mut [0u8; 0][..])));
    let mut buf = vec![0u8; len];
    let n = ok!(sys(rustix::fs::fgetxattr(fd, name, &mut buf[..])));
    buf.truncate(n);
    Ok(buf)
}

fn is_whiteout(st: &Stat) -> bool {
    rustix::fs::major(st.st_rdev) == 0 && rustix::fs::minor(st.st_rdev) == 0
}

fn mode_of(st: &Stat) -> u32 {
    st.st_mode as u32 & 0o7777
}

fn meta_differs(a: &Stat, b: &Stat) -> bool {
    mode_of(a) != mode_of(b) || a.st_mtime != b.st_mtime || a.st_mtime_nsec != b.st_mtime_nsec
}

fn same_bytes(a: OwnedFd, b: OwnedFd, ast: &Stat, bst: &Stat) -> Result<bool, Why> {
    if ast.st_size != bst.st_size {
        return Ok(false);
    }
    let mut fa = File::from(a);
    let mut fb = File::from(b);
    let mut ba = vec![0u8; BUF_LEN];
    let mut bb = vec![0u8; BUF_LEN];
    loop {
        let n = ok!(fill(&mut fa, &mut ba));
        let m = ok!(fill(&mut fb, &mut bb));
        if n != m || ba[..n] != bb[..m] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
}

/// Reads until the buffer is full or the file ends, so two files chunk alike.
fn fill(f: &mut File, buf: &mut [u8]) -> Result<usize, Why> {
    let mut n = 0;
    while n < buf.len() {
        let k = ok!(f.read(&mut buf[n..]).map_err(|e| Why::Unreadable(e.to_string())));
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}
