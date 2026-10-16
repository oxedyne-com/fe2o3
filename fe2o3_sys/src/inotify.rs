//! Watching a directory tree for change, as a wake signal and never as a record.
//!
//! The kernel's inotify queue is reduced to one fact, that something under the root
//! moved, together with whatever rename pairs the kernel's cookies offer. What changed
//! is for a re-reading of the tree to decide, so a dropped or late event costs a look
//! and never a lost change. Placement follows the caller's predicate, so a tree is
//! watched exactly where its owner would read it.
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids.
//! Nothing outside this file names it, so another system call crate would replace this
//! file alone. The kernel's own list of watches, `/proc/self/fdinfo/<fd>`, is the
//! oracle the tests check placement against.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::{
        HashMap,
        HashSet,
    },
    ffi::OsStr,
    fs,
    os::{
        fd::{
            AsFd,
            AsRawFd,
            RawFd,
        },
        unix::ffi::OsStrExt,
    },
    path::{
        Path,
        PathBuf,
    },
    time::{
        Duration,
        Instant,
    },
};

use nix::{
    errno::Errno,
    poll::{
        poll,
        PollFd,
        PollFlags,
        PollTimeout,
    },
    sys::inotify::{
        AddWatchFlags,
        InitFlags,
        Inotify,
        InotifyEvent,
        WatchDescriptor,
    },
};

pub const DEFAULT_BUDGET: usize = 8_192; // an eighth of the per-user 65,536

const DRAIN_READS:  usize   = 4_096;    // reads per wait, so a flood cannot hold a caller
const POLL_CHUNK:   u128    = 60_000;   // longest single poll, in milliseconds

// Why a tree could not be, or could no longer be, fully watched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Limit {
    Watches,    // the kernel's per-user watch count (ENOSPC)
    Instances,  // the instance or descriptor limit (EMFILE)
    Budget,     // this tree's own budget
}

/// A rename the kernel paired by cookie, as paths relative to the root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Renamed {
    pub from:   Vec<u8>,
    pub to:     Vec<u8>,
    pub dir:    bool,
}

/// What a [`Watch::wait`] woke for. `Moved` and `LookNow` are the only wakes that say
/// the tree changed; the rest are about the watch itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Wake {
    Moved(Vec<Renamed>),    // something moved; the pairs may be empty
    LookNow,                // the queue overflowed, so only a full reading is safe
    Exhausted(Limit),       // every watch has been released; fall back to polling
    RootGone,               // the root was deleted or moved; every watch is released
    Timeout,
}

/// The result of placing the watches.
#[derive(Debug)]
pub enum Opened {
    Watching(Watch),
    Exhausted(Limit),   // nothing is left watched, and no instance is kept
}

#[derive(Debug)]
struct Dir {
    path:   Vec<u8>,    // relative to the root, `/` separated, empty for the root
}

#[derive(Debug)]
struct Pending {
    from:   Vec<u8>,
    dir:    bool,
}

#[derive(Debug)]
enum Done {
    Gone,
    Short(Limit),
}

enum Walked {
    Held,
    Short(Limit),
    Gone,
}

#[derive(Default)]
struct Reduced {
    moved:      bool,
    overflow:   bool,
    dirty:      bool,   // directories moved, so the whole tree is reconciled
    gone:       bool,
    grafts:     Vec<Vec<u8>>,
    pairs:      Vec<Renamed>,
}

/// An inotify instance holding one watch on each admitted directory of a tree.
///
/// Whenever [`Watch::open`] or [`Watch::wait`] returns, every admitted directory under
/// the root holds exactly one watch and no other directory holds one, so far as the
/// events read can tell. A directory the process cannot read is not watched.
#[derive(Debug)]
pub struct Watch {
    ino:    Inotify,
    root:   PathBuf,
    rootwd: Option<WatchDescriptor>,
    budget: usize,
    dirs:   HashMap<WatchDescriptor, Dir>,
    paths:  HashMap<Vec<u8>, WatchDescriptor>,
    carry:  HashMap<u32, Pending>,  // renames seen leaving, awaiting one more read
    done:   Option<Done>,
}

impl AsRawFd for Watch {
    fn as_raw_fd(&self) -> RawFd {
        self.ino.as_fd().as_raw_fd()
    }
}

impl Watch {

    /// Places a watch on `root` and on every directory beneath it that `descend` admits.
    ///
    /// `descend` is asked with each directory's path relative to the root, as bytes
    /// with `/` separators, and is never asked about the root, which is always watched.
    /// It must give the same answer for the same path every time. A symbolic link to a
    /// directory is never descended. The budget bounds the watches held when a call
    /// returns; a call that rearranges many directories at once can briefly hold more.
    pub fn open<F>(
        root:       &Path,
        budget:     usize,
        mut descend: F,
    )
        -> Outcome<Opened>
        where F: FnMut(&[u8]) -> bool,
    {
        let ino = match Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC) {
            Ok(ino) => ino,
            Err(e) => match init_limit(e) {
                Some(limit) => return Ok(Opened::Exhausted(limit)),
                None => return Err(err!(
                    "Cannot initialise inotify for {:?}: {}.", root, e;
                    IO, System, Init)),
            },
        };
        match fs::metadata(root) {
            Ok(m) if m.is_dir() => (),
            Ok(_) => return Err(err!(
                "Cannot watch {:?}: not a directory.", root;
                Input, Invalid, Path)),
            Err(e) => return Err(err!(
                "Cannot watch {:?}: {}.", root, e;
                IO, File, Missing)),
        }
        let mut watch = Self {
            ino,
            root:   root.to_path_buf(),
            rootwd: None,
            budget,
            dirs:   HashMap::new(),
            paths:  HashMap::new(),
            carry:  HashMap::new(),
            done:   None,
        };
        match res!(watch.walk(vec![Vec::new()], true, &mut descend)) {
            Walked::Held => Ok(Opened::Watching(watch)),
            Walked::Short(limit) => {
                watch.release();
                Ok(Opened::Exhausted(limit))
            },
            Walked::Gone => Err(err!(
                "Cannot watch {:?}: it vanished while the watches were placed.", root;
                IO, File, Missing)),
        }
    }

    /// Waits up to `timeout` for the tree to change, and reduces what the kernel queued.
    ///
    /// An overflow always gives `LookNow` and a full reconcile, and any other event
    /// gives `Moved`. A rename is paired only when the kernel's cookie pairs it, and one
    /// seen leaving is remembered for a single further read in case its arrival follows.
    /// `Exhausted` and `RootGone` release every watch and repeat at once on every later call.
    pub fn wait<F>(
        &mut self,
        timeout:    Duration,
        mut descend: F,
    )
        -> Outcome<Wake>
        where F: FnMut(&[u8]) -> bool,
    {
        match &self.done {
            Some(Done::Gone)        => return Ok(Wake::RootGone),
            Some(Done::Short(l))    => return Ok(Wake::Exhausted(*l)),
            None => (),
        }
        let deadline = Instant::now().checked_add(timeout);
        loop {
            if !res!(self.ready(deadline)) {
                return Ok(Wake::Timeout);
            }
            if let Some(wake) = res!(self.drain(&mut descend)) {
                return Ok(wake);
            }
            // Only bookkeeping was read, such as a release of ours; wait on.
        }
    }

    /// How many watches are held.
    pub fn watches(&self) -> usize {
        self.dirs.len()
    }

    // Polls until the descriptor is readable or the deadline passes.
    fn ready(&self, deadline: Option<Instant>) -> Outcome<bool> {
        loop {
            let ms = match deadline {
                None => POLL_CHUNK,
                Some(d) => {
                    // Rounded up, so the wait never ends before its time.
                    let rem = d.saturating_duration_since(Instant::now());
                    let ms = rem.as_millis();
                    if rem.subsec_nanos() % 1_000_000 != 0 { ms + 1 } else { ms }
                },
            };
            let ms = ms.min(POLL_CHUNK) as u16;
            let mut fds = [PollFd::new(self.ino.as_fd(), PollFlags::POLLIN)];
            match poll(&mut fds, PollTimeout::from(ms)) {
                Ok(0) => match deadline {
                    Some(d) if Instant::now() >= d => return Ok(false),
                    _ => continue,
                },
                Ok(_) => {
                    let rev = fds[0].revents().unwrap_or(PollFlags::empty());
                    if rev.intersects(PollFlags::POLLIN) {
                        return Ok(true);
                    }
                    return Err(err!(
                        "Polling the inotify descriptor of {:?} reported {:?}.",
                        self.root, rev;
                        IO, System));
                },
                Err(Errno::EINTR) => continue,
                Err(e) => return Err(err!(
                    "Polling the inotify descriptor of {:?} failed: {}.", self.root, e;
                    IO, System)),
            }
        }
    }

    // Reads what is queued and acts on it. None means nothing worth a wake was read.
    fn drain<F>(&mut self, descend: &mut F) -> Outcome<Option<Wake>>
        where F: FnMut(&[u8]) -> bool,
    {
        let mut evs = Vec::new();
        for _ in 0..drain_reads() {
            match self.ino.read_events() {
                Ok(mut batch)           => evs.append(&mut batch),
                Err(Errno::EAGAIN)      => break,
                Err(Errno::EINTR)       => continue,
                Err(e) => return Err(err!(
                    "Reading inotify events for {:?} failed: {}.", self.root, e;
                    IO, Read, System)),
            }
        }
        let red = self.reduce(evs);
        if red.gone {
            return Ok(Some(self.finish(Done::Gone)));
        }
        if !red.moved && !red.overflow {
            return Ok(None);
        }
        let walked = if red.overflow || red.dirty {
            res!(self.walk(vec![Vec::new()], true, descend))
        } else {
            let mut starts = Vec::new();
            for graft in red.grafts {
                if descend(&graft) {
                    starts.push(graft);
                }
            }
            if starts.is_empty() {
                Walked::Held
            } else {
                res!(self.walk(starts, false, descend))
            }
        };
        match walked {
            Walked::Held        => (),
            Walked::Short(l)    => return Ok(Some(self.finish(Done::Short(l)))),
            Walked::Gone        => return Ok(Some(self.finish(Done::Gone))),
        }
        if red.overflow {
            Ok(Some(Wake::LookNow))
        } else {
            Ok(Some(Wake::Moved(red.pairs)))
        }
    }

    // Boils a batch of events down to the facts the walk and the caller need, keeping
    // the maps of watched directories current as renames go by.
    fn reduce(&mut self, evs: Vec<InotifyEvent>) -> Reduced {
        let mut red = Reduced::default();
        let mut old = std::mem::take(&mut self.carry);
        let mut new: HashMap<u32, Pending> = HashMap::new();
        for ev in evs {
            let m = ev.mask;
            if m.contains(AddWatchFlags::IN_Q_OVERFLOW) {
                red.overflow = true;
                continue;
            }
            if !self.dirs.contains_key(&ev.wd) {
                continue; // from a watch already released
            }
            red.moved = true;
            if m.contains(AddWatchFlags::IN_IGNORED) {
                self.untrack(ev.wd);
                continue;
            }
            if m.contains(AddWatchFlags::IN_UNMOUNT) {
                red.dirty = true;
            }
            if m.intersects(AddWatchFlags::IN_DELETE_SELF | AddWatchFlags::IN_MOVE_SELF)
                && Some(ev.wd) == self.rootwd
            {
                red.gone = true;
            }
            let name = match &ev.name {
                Some(n) => n.as_os_str().as_bytes(),
                None    => continue,
            };
            let isdir = m.contains(AddWatchFlags::IN_ISDIR);
            if m.contains(AddWatchFlags::IN_CREATE) {
                if isdir {
                    red.grafts.push(self.child(ev.wd, name));
                }
            } else if m.contains(AddWatchFlags::IN_MOVED_FROM) {
                let from = self.child(ev.wd, name);
                new.insert(ev.cookie, Pending { from, dir: isdir });
            } else if m.contains(AddWatchFlags::IN_MOVED_TO) {
                let to = self.child(ev.wd, name);
                match new.remove(&ev.cookie).or_else(|| old.remove(&ev.cookie)) {
                    Some(p) => {
                        if isdir {
                            // Events that follow in this batch name the new paths.
                            self.relabel(&p.from, &to);
                            red.dirty = true;
                        }
                        red.pairs.push(Renamed { from: p.from, to, dir: isdir });
                    },
                    None => if isdir {
                        red.grafts.push(to);
                    },
                }
            }
        }
        if red.overflow {
            red.pairs.clear();
            new.clear();
        } else {
            // A directory seen leaving may have left the tree, with its watch.
            for p in new.values() {
                if p.dir {
                    red.dirty = true;
                }
            }
        }
        self.carry = new;
        red
    }

    // Walks from each start, placing a watch on every admitted directory found. A
    // full walk starts at the root and then releases every watch it did not reach, so
    // the maps end as the tree is. A graft only adds.
    fn walk<F>(
        &mut self,
        starts:     Vec<Vec<u8>>,
        full:       bool,
        descend:    &mut F,
    )
        -> Outcome<Walked>
        where F: FnMut(&[u8]) -> bool,
    {
        let mut visited: HashSet<WatchDescriptor> = HashSet::new();
        let mut stack = starts;
        while let Some(rel) = stack.pop() {
            let is_root = rel.is_empty();
            let abs = if is_root {
                self.root.clone()
            } else {
                self.root.join(OsStr::from_bytes(&rel))
            };
            // The watch goes on before the directory is listed, so an entry made
            // after the listing is in the queue, and one made before it is in the list.
            let wd = match self.add(&abs, is_root) {
                Ok(wd) => wd,
                Err(e) => {
                    if let Some(limit) = add_limit(e) {
                        return Ok(Walked::Short(limit));
                    }
                    match e {
                        Errno::ENOENT | Errno::ENOTDIR if is_root => return Ok(Walked::Gone),
                        // Gone, replaced by a link, or closed to us: not watched.
                        Errno::ENOENT | Errno::ENOTDIR | Errno::EACCES | Errno::ELOOP
                            if !is_root => continue,
                        _ => return Err(err!(
                            "Cannot watch {:?}: {}.", abs, e;
                            IO, System)),
                    }
                },
            };
            if is_root {
                match self.rootwd {
                    // A different directory now sits at the root's path.
                    Some(rw) if rw != wd => return Ok(Walked::Gone),
                    _ => self.rootwd = Some(wd),
                }
            }
            if !visited.insert(wd) {
                continue; // reached twice, as by a bind mount: walked once
            }
            let fresh = !self.dirs.contains_key(&wd);
            let count = if full { visited.len() } else { self.dirs.len() + fresh as usize };
            if count > self.budget {
                if fresh {
                    // Not yet in the map, so release would miss it.
                    let _ = self.ino.rm_watch(wd);
                }
                return Ok(Walked::Short(Limit::Budget));
            }
            self.track(wd, rel.clone());
            let rd = match fs::read_dir(&abs) {
                Ok(rd) => rd,
                Err(e) => match e.raw_os_error().map(Errno::from_raw) {
                    Some(Errno::EMFILE) | Some(Errno::ENFILE) =>
                        return Ok(Walked::Short(Limit::Instances)),
                    _ => continue,
                },
            };
            for ent in rd {
                let ent = match ent {
                    Ok(ent) => ent,
                    Err(_)  => continue,
                };
                let is_dir = match ent.file_type() {
                    Ok(t)   => t.is_dir(),  // false for a link, whatever it points at
                    Err(_)  => false,
                };
                if !is_dir {
                    continue;
                }
                let child = join(&rel, ent.file_name().as_bytes());
                if descend(&child) {
                    stack.push(child);
                }
            }
        }
        if full {
            let stale: Vec<WatchDescriptor> = self.dirs.keys()
                .filter(|wd| !visited.contains(*wd))
                .copied()
                .collect();
            for wd in stale {
                // An error means the kernel dropped it already.
                let _ = self.ino.rm_watch(wd);
                self.untrack(wd);
            }
        }
        Ok(Walked::Held)
    }

    fn add(&self, abs: &Path, is_root: bool) -> Result<WatchDescriptor, Errno> {
        if let Some(e) = injected() {
            return Err(e);
        }
        // The root is followed if it is a link, since its owner named it. Beneath it a
        // link is never followed, even if one replaces a directory after it was listed.
        let mut flags = mask() | AddWatchFlags::IN_ONLYDIR;
        if !is_root {
            flags |= AddWatchFlags::IN_DONT_FOLLOW;
        }
        self.ino.add_watch(abs, flags)
    }

    fn track(&mut self, wd: WatchDescriptor, path: Vec<u8>) {
        if let Some(old) = self.dirs.get(&wd) {
            if old.path != path && self.paths.get(&old.path) == Some(&wd) {
                self.paths.remove(&old.path);
            }
        }
        self.paths.insert(path.clone(), wd);
        self.dirs.insert(wd, Dir { path });
    }

    fn untrack(&mut self, wd: WatchDescriptor) {
        if let Some(dir) = self.dirs.remove(&wd) {
            if self.paths.get(&dir.path) == Some(&wd) {
                self.paths.remove(&dir.path);
            }
        }
    }

    // Rewrites the paths of a directory and of everything beneath it.
    fn relabel(&mut self, from: &[u8], to: &[u8]) {
        let mut moves = Vec::new();
        for (wd, dir) in &self.dirs {
            let rest = if dir.path == from {
                Some(&dir.path[from.len()..])
            } else if dir.path.len() > from.len()
                && dir.path.starts_with(from)
                && dir.path[from.len()] == b'/'
            {
                Some(&dir.path[from.len()..])
            } else {
                None
            };
            if let Some(rest) = rest {
                let mut path = to.to_vec();
                path.extend_from_slice(rest);
                moves.push((*wd, path));
            }
        }
        for (wd, _) in &moves {
            if let Some(dir) = self.dirs.get(wd) {
                if self.paths.get(&dir.path) == Some(wd) {
                    self.paths.remove(&dir.path);
                }
            }
        }
        for (wd, path) in moves {
            self.paths.insert(path.clone(), wd);
            self.dirs.insert(wd, Dir { path });
        }
    }

    fn child(&self, wd: WatchDescriptor, name: &[u8]) -> Vec<u8> {
        match self.dirs.get(&wd) {
            Some(dir)   => join(&dir.path, name),
            None        => name.to_vec(),
        }
    }

    // Releases every watch this tree holds. The kernel may have dropped some already.
    fn release(&mut self) {
        let wds: Vec<WatchDescriptor> = self.dirs.keys().copied().collect();
        for wd in wds {
            let _ = self.ino.rm_watch(wd);
        }
        self.dirs.clear();
        self.paths.clear();
        self.carry.clear();
    }

    fn finish(&mut self, done: Done) -> Wake {
        self.release();
        let wake = match &done {
            Done::Gone          => Wake::RootGone,
            Done::Short(limit)  => Wake::Exhausted(*limit),
        };
        self.done = Some(done);
        wake
    }
}

fn mask() -> AddWatchFlags {
    AddWatchFlags::IN_CREATE
        | AddWatchFlags::IN_DELETE
        | AddWatchFlags::IN_MODIFY
        | AddWatchFlags::IN_CLOSE_WRITE
        | AddWatchFlags::IN_ATTRIB
        | AddWatchFlags::IN_MOVED_FROM
        | AddWatchFlags::IN_MOVED_TO
        | AddWatchFlags::IN_DELETE_SELF
        | AddWatchFlags::IN_MOVE_SELF
}

fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut path = Vec::with_capacity(dir.len() + 1 + name.len());
    path.extend_from_slice(dir);
    if !dir.is_empty() {
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}

// What a failed `inotify_add_watch` says about the machine, if anything. ENOMEM is the
// same call failing for want of kernel memory, which a fall back to polling also cures.
fn add_limit(e: Errno) -> Option<Limit> {
    match e {
        Errno::ENOSPC | Errno::ENOMEM => Some(Limit::Watches),
        _ => None,
    }
}

fn init_limit(e: Errno) -> Option<Limit> {
    match e {
        Errno::EMFILE | Errno::ENFILE | Errno::ENOMEM => Some(Limit::Instances),
        _ => None,
    }
}

#[cfg(test)]
fn injected() -> Option<Errno> {
    tests::inject_next()
}

#[cfg(not(test))]
fn injected() -> Option<Errno> {
    None
}

#[cfg(test)]
fn drain_reads() -> usize {
    tests::drain_cap().unwrap_or(DRAIN_READS)
}

#[cfg(not(test))]
fn drain_reads() -> usize {
    DRAIN_READS
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        cell::Cell,
        fs::File,
        process::Command,
    };

    thread_local! {
        // Fails the nth `inotify_add_watch` from now with the given errno.
        static INJECT:  Cell<Option<(usize, Errno)>>    = const { Cell::new(None) };
        static CAP:     Cell<Option<usize>>             = const { Cell::new(None) };
    }

    pub(super) fn inject_next() -> Option<Errno> {
        match INJECT.get() {
            Some((1, e))    => { INJECT.set(None); Some(e) },
            Some((n, e))    => { INJECT.set(Some((n - 1, e))); None },
            None            => None,
        }
    }

    pub(super) fn drain_cap() -> Option<usize> {
        CAP.get()
    }

    // The kernel's own count of the watches on an inotify descriptor.
    fn held(fd: RawFd) -> Outcome<usize> {
        let body = res!(fs::read_to_string(format!("/proc/self/fdinfo/{}", fd))
            .map_err(|e| err!("Cannot read the fdinfo of {}: {}.", fd, e; IO, File, Read)));
        Ok(body.lines().filter(|l| l.starts_with("inotify wd:")).count())
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Outcome<Self> {
            let dir = std::env::temp_dir().join(format!("fe2o3-sys-ino-{}-{}", tag, std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            res!(fs::create_dir_all(&dir)
                .map_err(|e| err!("Cannot make {:?}: {}.", dir, e; IO, File, Create)));
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn build(root: &Path, dirs: &[&str]) -> Outcome<()> {
        for d in dirs {
            res!(fs::create_dir_all(root.join(d))
                .map_err(|e| err!("Cannot make {}: {}.", d, e; IO, File, Create)));
        }
        Ok(())
    }

    #[test]
    fn errno_mapping_names_the_limit() {
        assert_eq!(add_limit(Errno::ENOSPC),    Some(Limit::Watches));
        assert_eq!(add_limit(Errno::ENOMEM),    Some(Limit::Watches));
        assert_eq!(add_limit(Errno::ENOENT),    None);
        assert_eq!(add_limit(Errno::EACCES),    None);
        assert_eq!(init_limit(Errno::EMFILE),   Some(Limit::Instances));
        assert_eq!(init_limit(Errno::ENFILE),   Some(Limit::Instances));
        assert_eq!(init_limit(Errno::ENOSPC),   None);
    }

    // ENOSPC cannot be produced for real without taking watches from Syncthing and the
    // desktop, so the call is made to fail at the point the kernel would fail it.
    #[test]
    fn enospc_during_open_releases_the_tree() -> Outcome<()> {
        let s = res!(Scratch::new("enospc-open"));
        res!(build(&s.0, &["a/b", "c", "d/e", "f"]));
        INJECT.set(Some((4, Errno::ENOSPC)));
        let opened = res!(Watch::open(&s.0, 100, |_| true));
        INJECT.set(None);
        match opened {
            Opened::Exhausted(Limit::Watches) => (),
            other => return Err(err!("Expected Exhausted(Watches), got {:?}.", other; Test)),
        }
        Ok(())
    }

    #[test]
    fn enospc_during_wait_releases_every_watch() -> Outcome<()> {
        let s = res!(Scratch::new("enospc-wait"));
        res!(build(&s.0, &["a/b", "c"]));
        let mut w = match res!(Watch::open(&s.0, 100, |_| true)) {
            Opened::Watching(w) => w,
            other => return Err(err!("Expected a watch, got {:?}.", other; Test)),
        };
        assert_eq!(res!(held(w.as_raw_fd())), 4);
        res!(build(&s.0, &["x/y", "z"]));
        INJECT.set(Some((2, Errno::ENOSPC)));
        let wake = res!(w.wait(Duration::from_secs(2), |_| true));
        INJECT.set(None);
        assert_eq!(wake, Wake::Exhausted(Limit::Watches));
        assert_eq!(res!(held(w.as_raw_fd())), 0);
        assert_eq!(w.watches(), 0);
        // The answer repeats, and nothing is placed again behind the caller's back.
        let again = res!(w.wait(Duration::from_millis(50), |_| true));
        assert_eq!(again, Wake::Exhausted(Limit::Watches));
        assert_eq!(res!(held(w.as_raw_fd())), 0);
        Ok(())
    }

    // A rename arrives as two events. With one read per wait, 127 events ahead of it
    // put the first of the pair last in the first 4,096-byte read and the second first
    // in the next, so the pair straddles two waits as the carry exists to handle.
    #[test]
    fn a_rename_split_across_reads_is_paired_by_the_carry() -> Outcome<()> {
        let s = res!(Scratch::new("carry"));
        let mut w = match res!(Watch::open(&s.0, 100, |_| true)) {
            Opened::Watching(w) => w,
            other => return Err(err!("Expected a watch, got {:?}.", other; Test)),
        };
        res!(File::create(s.0.join("fx")).map_err(|e| err!("{}", e; IO, File, Create)));
        for i in 0..62 {
            res!(File::create(s.0.join(format!("f{:02}", i)))
                .map_err(|e| err!("{}", e; IO, File, Create)));
        }
        // Each file made is two events, CREATE and CLOSE_WRITE: 126, plus one ATTRIB.
        let chmod = Command::new("chmod").arg("600").arg(s.0.join("fx")).status();
        assert!(chmod.map(|st| st.success()).unwrap_or(false));
        let mv = Command::new("mv").arg(s.0.join("fx")).arg(s.0.join("fy")).status();
        assert!(mv.map(|st| st.success()).unwrap_or(false));
        CAP.set(Some(1));
        let first = res!(w.wait(Duration::from_secs(2), |_| true));
        let second = res!(w.wait(Duration::from_secs(2), |_| true));
        CAP.set(None);
        assert_eq!(first, Wake::Moved(Vec::new()));
        assert_eq!(second, Wake::Moved(vec![Renamed {
            from:   b"fx".to_vec(),
            to:     b"fy".to_vec(),
            dir:    false,
        }]));
        Ok(())
    }
}
