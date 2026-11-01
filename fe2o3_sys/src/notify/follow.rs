//! Following files across renames, by anyone, through held descriptors.
//!
//! Each followed file is held by an `O_PATH` descriptor, and its inotify watch is added through
//! `/proc/self/fd/<held>`, never a path string, so the watch is on the file the caller was given
//! even if it was renamed before the watch went on. The mask is `IN_MOVE_SELF`, `IN_DELETE_SELF`
//! and `IN_ATTRIB`: while the descriptor is held the file's inode cannot go, so `IN_DELETE_SELF`
//! does not fire on an unlink or an atomic save over it, and the drop of its link count to zero
//! shows as `IN_ATTRIB` instead (BR-4e S1, probe 4). On any event the held link is read again
//! and the difference reported. On `IN_Q_OVERFLOW` every held link is read again.
//!
//! A held descriptor keeps its mount busy: [`Follower::unfollow_under`] lets a mount go.
//! A limit met, `follow_max`, the inotify watches (`ENOSPC`) or the descriptors (`EMFILE`,
//! `ENFILE`), is answered as [`Followed::Full`], never swallowed.
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids.

use oxedyne_fe2o3_core::prelude::*;

use nix::{
    errno::Errno,
    fcntl::{
        open,
        OFlag,
    },
    sys::{
        inotify::{
            AddWatchFlags,
            InitFlags,
            Inotify,
            WatchDescriptor,
        },
        stat::{
            fstat,
            Mode,
        },
    },
};

use std::{
    collections::BTreeMap,
    fs,
    os::fd::{
        AsFd,
        AsRawFd,
        BorrowedFd,
        OwnedFd,
    },
    path::{
        Path,
        PathBuf,
    },
};

/// One followed file.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FollowId(WatchDescriptor);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Limit {
    FollowMax(usize),   // The follower's own bound.
    Watches,            // inotify_add_watch gave ENOSPC: fs.inotify.max_user_watches.
    Descriptors,        // EMFILE or ENFILE on the O_PATH open.
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Followed {
    Held(FollowId),
    Already(FollowId),  // The same file is followed already.
    Full(Limit),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Change {
    Moved   { id: FollowId, old: PathBuf, new: PathBuf },
    Gone    { id: FollowId, last: PathBuf },    // Unlinked or saved over; no longer followed.
    Dropped { id: FollowId, last: PathBuf },    // The kernel removed the watch; no longer followed.
    Overflow,                                   // The queue overflowed; the changes after it are from a full re-read.
}

struct Held {
    fd:     OwnedFd,
    path:   PathBuf,
}

pub struct Follower {
    ino:    Inotify,
    held:   BTreeMap<WatchDescriptor, Held>,
    max:    usize,
}

// fixed: the suffix the kernel's d_path gives an unlinked file.
const DELETED: &str = " (deleted)";

impl Follower {
    /// A follower that holds at most `max` files, which must sit below the process's
    /// `RLIMIT_NOFILE` with room for its other descriptors.
    pub fn new(max: usize) -> Outcome<Self> {
        let ino = res!(Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).map_err(|e|
            err!("inotify_init1: {}.", e; IO, System)));
        Ok(Self { ino, held: BTreeMap::new(), max })
    }

    /// Readable (`POLLIN`) when events wait; then call [`Self::read`].
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.ino.as_fd()
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// The followed file's path as last read.
    pub fn path(&self, id: FollowId) -> Option<&Path> {
        self.held.get(&id.0).map(|h| h.path.as_path())
    }

    /// The held `O_PATH` descriptor, for `statx` and the host-path resolver.
    pub fn held_fd(&self, id: FollowId) -> Option<BorrowedFd<'_>> {
        self.held.get(&id.0).map(|h| h.fd.as_fd())
    }

    pub fn ids(&self) -> Vec<FollowId> {
        self.held.keys().map(|w| FollowId(*w)).collect()
    }

    /// Follows the file `path` names now.
    pub fn follow_path(&mut self, path: &Path) -> Outcome<Followed> {
        self.hold(path)
    }

    /// Follows the file `fd` is open on, wherever it now is, such as a fanotify event's.
    pub fn follow_fd(&mut self, fd: BorrowedFd<'_>) -> Outcome<Followed> {
        self.hold(Path::new(&format!("/proc/self/fd/{}", fd.as_raw_fd())))
    }

    fn hold(&mut self, at: &Path) -> Outcome<Followed> {
        if self.held.len() >= self.max {
            return Ok(Followed::Full(Limit::FollowMax(self.max)));
        }
        let fd = match open(at, OFlag::O_PATH | OFlag::O_CLOEXEC, Mode::empty()) {
            Ok(fd)                              => fd,
            Err(Errno::EMFILE | Errno::ENFILE)  => return Ok(Followed::Full(Limit::Descriptors)),
            Err(e)                              => return Err(err!("Opening '{}' O_PATH: {}.",
                at.display(), e; IO, File, Read)),
        };
        let proc_path = format!("/proc/self/fd/{}", fd.as_raw_fd());
        let mask = AddWatchFlags::IN_MOVE_SELF | AddWatchFlags::IN_DELETE_SELF | AddWatchFlags::IN_ATTRIB;
        let wd = match self.ino.add_watch(proc_path.as_str(), mask) {
            Ok(wd)              => wd,
            Err(Errno::ENOSPC)  => return Ok(Followed::Full(Limit::Watches)),
            Err(e)              => return Err(err!("inotify_add_watch on '{}': {}.",
                at.display(), e; IO, System)),
        };
        if self.held.contains_key(&wd) {
            return Ok(Followed::Already(FollowId(wd)));
        }
        let path = res!(read_held(&fd));
        self.held.insert(wd, Held { fd, path });
        Ok(Followed::Held(FollowId(wd)))
    }

    /// Stops following `id`, answering its last path.
    pub fn unfollow(&mut self, id: FollowId) -> Option<PathBuf> {
        match self.held.remove(&id.0) {
            Some(h) => {
                // The watch may have gone with its file already; its removal is then moot.
                let _ = self.ino.rm_watch(id.0);
                Some(h.path)
            }
            None    => None,
        }
    }

    /// Stops following every file whose last path is under `prefix`, so its mount can go.
    pub fn unfollow_under(&mut self, prefix: &Path) -> Vec<(FollowId, PathBuf)> {
        let ids: Vec<FollowId> = self.held.iter()
            .filter(|(_, h)| h.path.starts_with(prefix))
            .map(|(w, _)| FollowId(*w))
            .collect();
        ids.into_iter().filter_map(|id| self.unfollow(id).map(|p| (id, p))).collect()
    }

    /// Reads the events waiting and answers what changed, none if nothing did.
    pub fn read(&mut self) -> Outcome<Vec<Change>> {
        let mut out = Vec::new();
        loop {
            let evs = match self.ino.read_events() {
                Ok(v)               => v,
                Err(Errno::EAGAIN)  => break,
                Err(e)              => return Err(err!("Reading inotify events: {}.", e; IO, Read)),
            };
            if evs.is_empty() { break; }
            for ev in evs {
                if ev.mask.contains(AddWatchFlags::IN_Q_OVERFLOW) {
                    out.push(Change::Overflow);
                    let mut all = res!(self.resync());
                    out.append(&mut all);
                    continue;
                }
                if !self.held.contains_key(&ev.wd) {
                    continue; // Unfollowed already, its IN_IGNORED arriving after.
                }
                let id = FollowId(ev.wd);
                if ev.mask.contains(AddWatchFlags::IN_IGNORED) {
                    if let Some(h) = self.held.remove(&ev.wd) {
                        out.push(Change::Dropped { id, last: h.path });
                    }
                    continue;
                }
                if ev.mask.contains(AddWatchFlags::IN_DELETE_SELF) {
                    if let Some(last) = self.unfollow(id) {
                        out.push(Change::Gone { id, last });
                    }
                    continue;
                }
                if let Some(c) = res!(self.check(id)) {
                    out.push(c);
                }
            }
        }
        Ok(out)
    }

    /// Reads every held link again and answers each difference.
    pub fn resync(&mut self) -> Outcome<Vec<Change>> {
        let mut out = Vec::new();
        for id in self.ids() {
            if let Some(c) = res!(self.check(id)) {
                out.push(c);
            }
        }
        Ok(out)
    }

    fn check(&mut self, id: FollowId) -> Outcome<Option<Change>> {
        let (now, nlink) = match self.held.get(&id.0) {
            Some(h) => {
                let now = res!(read_held(&h.fd));
                let st = res!(fstat(&h.fd).map_err(|e| err!("fstat of a held file: {}.", e; IO, System)));
                (now, st.st_nlink)
            }
            None    => return Ok(None),
        };
        if nlink == 0 || now.to_string_lossy().ends_with(DELETED) {
            return Ok(self.unfollow(id).map(|last| Change::Gone { id, last }));
        }
        match self.held.get_mut(&id.0) {
            Some(h) if h.path != now => {
                let old = std::mem::replace(&mut h.path, now.clone());
                Ok(Some(Change::Moved { id, old, new: now }))
            }
            _ => Ok(None),
        }
    }
}

fn read_held(fd: &OwnedFd) -> Outcome<PathBuf> {
    let p = format!("/proc/self/fd/{}", fd.as_raw_fd());
    fs::read_link(&p).map_err(|e| err!("Reading the held link '{}': {}.", p, e; IO, Read))
}
