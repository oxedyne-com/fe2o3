//! The changes an overlayfs upper directory holds over its lower directory, as a typed op list.
//!
//! [`changes`] walks the upper beside the lower and names what applying it would do: [`Op::Create`]
//! for a path the lower lacks, [`Op::Write`] for new contents, [`Op::Delete`] for a whiteout,
//! [`Op::Opaque`] for a directory whose lower contents are hidden, and [`Op::Meta`] for a mode or
//! time change alone. With `redirect_dir` off, overlayfs renames a file by copying it up and leaves
//! a whiteout behind, and refuses a directory rename with `EXDEV`, so `mv` copies it; either way a
//! rename reaches the list as a delete of the old path and a create of the new, and there is no
//! rename op.
//!
//! The upper may have been written by a hostile program, so the walk follows no symbolic link:
//! both roots are opened with `RESOLVE_NO_SYMLINKS`, every child by name from its parent's
//! descriptor with `O_NOFOLLOW`. It refuses, by `err!`, an upper directory standing over a lower
//! symbolic link without the opaque mark overlayfs gives it (applying it would write through the
//! link), more entries than the caller allows, a name that is not UTF-8 or holds a control
//! character, a device, pipe or socket other than a whiteout, and the overlay attributes this list
//! cannot express (`redirect`, `metacopy`, an `x` opaque mark, an attribute whiteout).
//!
//! Both `trusted.overlay.` and `user.overlay.` (a mount with `userxattr`, as in an unprivileged
//! user namespace) attributes are read.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::{
    ffi::CStr,
    fs::File,
    io::Read,
    os::fd::OwnedFd,
    path::{
        Path,
        PathBuf,
    },
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

/// One change, at a path relative to the roots.  The applier reads what to write from the upper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Create(PathBuf, Kind),  // The lower has nothing here.
    Write(PathBuf, Kind),   // New file contents or a new link target.
    Delete(PathBuf),        // A whiteout over a lower entry.
    Opaque(PathBuf),        // Lower directory kept, its contents removed; creates beneath follow.
    Meta(PathBuf, Kind),    // Same contents, new mode or modification time.
}

const XATTR_PREFIXES: [&str; 2] = ["trusted.overlay.", "user.overlay."]; // fixed: overlayfs names
const BUF_LEN: usize = 64 * 1024; // fixed: a read size, not a limit

/// Lists what `upper` changes over `lower`, refusing an upper with more than `max_entries` entries.
pub fn changes(upper: &Path, lower: &Path, max_entries: usize) -> Outcome<Vec<Op>> {
    let up = res!(open_root(upper));
    let low = res!(open_root(lower));
    let mut walk = Walker { ops: Vec::new(), seen: 0, max: max_entries };
    res!(walk.dir(&up, Some(&low), PathBuf::new()));
    Ok(walk.ops)
}

struct Walker {
    ops:    Vec<Op>,
    seen:   usize,
    max:    usize,
}

impl Walker {

    fn dir(&mut self, up: &OwnedFd, low: Option<&OwnedFd>, rel: PathBuf) -> Outcome<()> {
        for name in res!(self.names(up, &rel)) {
            let path = rel.join(&name);
            let ust = res!(sys(rustix::fs::statat(up, name.as_str(), AtFlags::SYMLINK_NOFOLLOW),
                "stat upper", &path));
            let lst = match low {
                Some(fd) => res!(lower_stat(fd, &name, &path)),
                None => None,
            };
            let lkind = lst.as_ref().map(kind_of);
            match FileType::from_raw_mode(ust.st_mode) {
                FileType::CharacterDevice if is_whiteout(&ust) => {
                    // A whiteout over nothing hides nothing: the lower entry is already gone.
                    if lst.is_some() {
                        self.ops.push(Op::Delete(path));
                    }
                }
                FileType::Directory => {
                    let fd = res!(open_child(up, &name, OFlags::DIRECTORY, &path));
                    let opaque = res!(overlay_marks(&fd, &path));
                    match lkind {
                        None => {
                            self.ops.push(Op::Create(path.clone(), Kind::Dir));
                            res!(self.dir(&fd, None, path));
                        }
                        Some(Some(Kind::Dir)) => {
                            let lfd = match low {
                                Some(l) => res!(open_child(l, &name, OFlags::DIRECTORY, &path)),
                                None => return Err(err!("Overlay walk lost the lower directory \
                                    above {:?}.", path; Bug)),
                            };
                            if mode_of(&ust) != lst.as_ref().map(mode_of).unwrap_or(0) {
                                self.ops.push(Op::Meta(path.clone(), Kind::Dir));
                            }
                            if opaque {
                                self.ops.push(Op::Opaque(path.clone()));
                                res!(self.dir(&fd, None, path));
                            } else {
                                res!(self.dir(&fd, Some(&lfd), path));
                            }
                        }
                        Some(Some(Kind::Link)) if !opaque => return Err(err!(
                            "Overlay upper directory {:?} stands over a lower symbolic link \
                            without an opaque mark; applying it would write through the link \
                            (symlinked parent).", path; Security, Path)),
                        Some(_) if !opaque => return Err(err!(
                            "Overlay upper directory {:?} stands over a lower non-directory \
                            without an opaque mark, which overlayfs does not write.", path;
                            Security, Path)),
                        Some(_) => {
                            self.ops.push(Op::Delete(path.clone()));
                            self.ops.push(Op::Create(path.clone(), Kind::Dir));
                            res!(self.dir(&fd, None, path));
                        }
                    }
                }
                FileType::RegularFile => {
                    let fd = res!(open_child(up, &name, OFlags::empty(), &path));
                    let fst = res!(sys(rustix::fs::fstat(&fd), "fstat upper", &path));
                    if FileType::from_raw_mode(fst.st_mode) != FileType::RegularFile
                        || fst.st_ino != ust.st_ino
                    {
                        return Err(err!("Overlay upper entry {:?} changed while it was read.",
                            path; Security, Path));
                    }
                    if res!(overlay_marks(&fd, &path)) {
                        return Err(err!("Overlay upper file {:?} carries an opaque mark, which \
                            only a directory may.", path; Security, Invalid));
                    }
                    match (lkind, low) {
                        (None, _) | (_, None) => self.ops.push(Op::Create(path, Kind::File)),
                        (Some(Some(Kind::File)), Some(l)) => {
                            let lfd = res!(open_child(l, &name, OFlags::empty(), &path));
                            if !res!(same_bytes(fd, lfd, &path)) {
                                self.ops.push(Op::Write(path, Kind::File));
                            } else if lst.as_ref().map(|l| meta_differs(&ust, l)).unwrap_or(false) {
                                self.ops.push(Op::Meta(path, Kind::File));
                            }
                        }
                        (Some(_), Some(_)) => {
                            self.ops.push(Op::Delete(path.clone()));
                            self.ops.push(Op::Create(path, Kind::File));
                        }
                    }
                }
                FileType::Symlink => {
                    match (lkind, low) {
                        (None, _) | (_, None) => self.ops.push(Op::Create(path, Kind::Link)),
                        (Some(Some(Kind::Link)), Some(l)) => {
                            let ut = res!(sys(rustix::fs::readlinkat(up, name.as_str(), Vec::new()),
                                "read upper link", &path));
                            let lt = res!(sys(rustix::fs::readlinkat(l, name.as_str(), Vec::new()),
                                "read lower link", &path));
                            if ut != lt {
                                self.ops.push(Op::Write(path, Kind::Link));
                            }
                        }
                        (Some(_), Some(_)) => {
                            self.ops.push(Op::Delete(path.clone()));
                            self.ops.push(Op::Create(path, Kind::Link));
                        }
                    }
                }
                other => return Err(err!("Overlay upper entry {:?} is a {:?}, which is neither a \
                    whiteout nor a file, directory or symbolic link.", path, other;
                    Security, Invalid)),
            }
        }
        Ok(())
    }

    /// The entries of one upper directory, sorted, each counted against the limit.
    fn names(&mut self, up: &OwnedFd, rel: &Path) -> Outcome<Vec<String>> {
        let mut dir = res!(sys(rustix::fs::Dir::read_from(up), "list upper", rel));
        let mut names = Vec::new();
        while let Some(entry) = dir.read() {
            let entry = res!(sys(entry, "list upper", rel));
            let raw = entry.file_name();
            if raw == c"." || raw == c".." {
                continue;
            }
            self.seen += 1;
            if self.seen > self.max {
                return Err(err!("Overlay upper holds more than {} entries; refused at {:?}.",
                    self.max, rel; TooBig, Security));
            }
            names.push(res!(name_of(raw, rel)));
        }
        names.sort();
        Ok(names)
    }
}

fn name_of(raw: &CStr, rel: &Path) -> Outcome<String> {
    let name = match raw.to_str() {
        Ok(s) => s,
        Err(_) => return Err(err!("Overlay upper name {:?} in {:?} is not UTF-8.",
            raw, rel; Security, Name)),
    };
    if name.chars().any(char::is_control) {
        return Err(err!("Overlay upper name {:?} in {:?} holds a control character.",
            name, rel; Security, Name));
    }
    Ok(name.to_string())
}

fn sys<T>(r: rustix::io::Result<T>, what: &str, path: &Path) -> Outcome<T> {
    match r {
        Ok(v) => Ok(v),
        Err(e) => Err(err!("Overlay walk could not {} {:?}: {}.", what, path, e; IO)),
    }
}

fn open_root(path: &Path) -> Outcome<OwnedFd> {
    sys(rustix::fs::openat2(CWD, path, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(), ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS),
        "open, through no symbolic link, the root", path)
}

fn open_child(dir: &OwnedFd, name: &str, extra: OFlags, path: &Path) -> Outcome<OwnedFd> {
    sys(rustix::fs::openat(dir, name, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC
        | OFlags::NONBLOCK | extra, Mode::empty()), "open, without following a link,", path)
}

fn lower_stat(low: &OwnedFd, name: &str, path: &Path) -> Outcome<Option<Stat>> {
    match rustix::fs::statat(low, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(st) => Ok(Some(st)),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(e) => Err(err!("Overlay walk could not stat lower {:?}: {}.", path, e; IO)),
    }
}

/// The lower's kind, or `None` for a kind no op names (a pipe, socket or device).
fn kind_of(st: &Stat) -> Option<Kind> {
    match FileType::from_raw_mode(st.st_mode) {
        FileType::RegularFile   => Some(Kind::File),
        FileType::Directory     => Some(Kind::Dir),
        FileType::Symlink       => Some(Kind::Link),
        _                       => None,
    }
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

/// Is the entry marked opaque?  Refuses every overlay attribute an op cannot express.
fn overlay_marks(fd: &OwnedFd, path: &Path) -> Outcome<bool> {
    let mut list = [0u8; 4096]; // fixed: XATTR_LIST_MAX is 64 KiB, an upper entry needs a few
    let n = res!(sys(rustix::fs::flistxattr(fd, &mut list[..]), "list attributes of", path));
    let mut opaque = false;
    for raw in list[..n].split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let attr = String::from_utf8_lossy(raw);
        let key = match XATTR_PREFIXES.iter().find_map(|p| attr.strip_prefix(p)) {
            Some(k) => k,
            None => continue,
        };
        match key {
            "origin" | "impure" | "uuid" => {}
            "opaque" => {
                let mut val = [0u8; 8];
                let n = res!(sys(rustix::fs::fgetxattr(fd, attr.as_ref(), &mut val[..]),
                    "read the opaque mark of", path));
                match &val[..n] {
                    b"y"    => opaque = true,
                    other   => return Err(err!("Overlay upper {:?} has opaque mark {:?}, not \
                        \"y\".", path, String::from_utf8_lossy(other); Security, Invalid)),
                }
            }
            _ => return Err(err!("Overlay upper {:?} carries {}, which this op list cannot \
                express.", path, attr; Security, Invalid)),
        }
    }
    Ok(opaque)
}

fn same_bytes(a: OwnedFd, b: OwnedFd, path: &Path) -> Outcome<bool> {
    let ast = res!(sys(rustix::fs::fstat(&a), "fstat upper", path));
    let bst = res!(sys(rustix::fs::fstat(&b), "fstat lower", path));
    if FileType::from_raw_mode(bst.st_mode) != FileType::RegularFile {
        return Err(err!("Overlay lower entry {:?} changed while it was read.", path; Path));
    }
    if ast.st_size != bst.st_size {
        return Ok(false);
    }
    let mut fa = File::from(a);
    let mut fb = File::from(b);
    let mut ba = vec![0u8; BUF_LEN];
    let mut bb = vec![0u8; BUF_LEN];
    loop {
        let n = res!(fill(&mut fa, &mut ba));
        let m = res!(fill(&mut fb, &mut bb));
        if n != m || ba[..n] != bb[..m] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
}

/// Reads until the buffer is full or the file ends, so two files chunk alike.
fn fill(f: &mut File, buf: &mut [u8]) -> Outcome<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = res!(f.read(&mut buf[n..]));
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}
