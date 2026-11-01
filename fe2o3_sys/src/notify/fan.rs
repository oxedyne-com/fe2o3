//! A fanotify group that hears every close after a write on whole filesystems.
//!
//! The group is a notification group (`FAN_CLASS_NOTIF`) on the kernel's bounded queue, never
//! `FAN_UNLIMITED_QUEUE`, so a reader that lags behind a big build cannot grow its own memory: when
//! the queue (`fs.fanotify.max_queued_events`) fills, the kernel drops events and the reader gets
//! one [`FanEvent::Overflow`] in their place. Marks are `FAN_MARK_FILESYSTEM`, so a write through any
//! bind mount, container or namespace of a marked filesystem is heard.
//!
//! Each written file is resolved to a host path (see [`super::mounts`]), and the writer is read
//! from `/proc/<pid>` when the event is read. `FAN_REPORT_PIDFD` is not used: nix 0.31 leaves the
//! pidfd it carries open (BR-4e S1, probe 3: open fds grew 6 to 8 over five events). A writer
//! that has exited by then is reported as [`Writer::Exited`], and its file may be
//! [`Unresolved::WriterGone`]; both are losses the caller makes good by a rescan. A pid reused in
//! the meantime can still give a wrong uid on one of the reader's own mounts; a mapped path is
//! guarded, since the mount must appear in that process's table.
//! The root fix is the pidfd: once nix's `FAN_REPORT_PIDFD` leak is fixed upstream, this code
//! takes the pidfd and the residual goes.
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids, and `statx` from
//! `rustix`.
//!
//! [`Unresolved::WriterGone`]: super::mounts::Unresolved::WriterGone

use crate::notify::mounts::{
    mount_id,
    resolve,
    HostPath,
    Mount,
    MountTable,
};

use oxedyne_fe2o3_core::prelude::*;

use nix::{
    errno::Errno,
    sys::fanotify::{
        EventFFlags,
        Fanotify,
        FanotifyEvent,
        InitFlags,
        MarkFlags,
        MaskFlags,
    },
};

use std::{
    fs::{
        self,
        File,
    },
    io::{
        Read,
        Seek,
        SeekFrom,
    },
    os::fd::{
        AsFd,
        AsRawFd,
        BorrowedFd,
    },
    path::Path,
};

/// Who wrote a file, as `/proc/<pid>` showed when the event was read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Writer {
    Live {
        pid:    i32,
        uid:    u32,    // Real uid.
        fsuid:  u32,    // The uid the write was checked against.
        cgroup: String, // The cgroup v2 path, from `0::` in /proc/<pid>/cgroup.
    },
    Exited { pid: i32 },
}

/// One write the group heard. The event's descriptor stays open while this is held, so a caller
/// can follow the file through it ([`super::follow::Follower::follow_fd`]); drop it to close it.
#[derive(Debug)]
pub struct Written {
    pub mnt:    u64,
    pub ino:    u64,
    pub link:   String,     // The descriptor's path in the writer's view.
    pub host:   HostPath,
    pub writer: Writer,
    event:      FanotifyEvent,
}

impl Written {
    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.event.fd()
    }
}

#[derive(Debug)]
pub enum FanEvent {
    Written(Written),
    Overflow,               // The kernel's queue filled and events were dropped.
    NoFd { pid: i32 },      // An event with no descriptor that is not an overflow: also a loss.
}

/// A fanotify notification group with its own view of the mounts.
pub struct FanGroup {
    fan:        Fanotify,
    mountinfo:  File,       // Held for `POLLPRI` on a mount or unmount.
    own:        MountTable,
}

impl FanGroup {
    /// Needs `CAP_SYS_ADMIN`.
    pub fn new() -> Outcome<Self> {
        let flags = InitFlags::FAN_CLASS_NOTIF | InitFlags::FAN_CLOEXEC | InitFlags::FAN_NONBLOCK;
        let ev = EventFFlags::O_RDONLY | EventFFlags::O_LARGEFILE | EventFFlags::O_CLOEXEC;
        let fan = res!(Fanotify::init(flags, ev).map_err(|e|
            err!("fanotify_init (needs CAP_SYS_ADMIN): {}.", e; IO, System)));
        let mut mountinfo = res!(File::open("/proc/self/mountinfo").map_err(|e|
            err!("Opening /proc/self/mountinfo: {}.", e; IO, File, Read)));
        let own = res!(read_table(&mut mountinfo));
        Ok(Self { fan, mountinfo, own })
    }

    /// The kernel's queue length for a group, `fs.fanotify.max_queued_events`.
    pub fn max_queued_events() -> Outcome<u64> {
        let p = "/proc/sys/fs/fanotify/max_queued_events";
        let s = res!(fs::read_to_string(p).map_err(|e| err!("Reading '{}': {}.", p, e; IO, File, Read)));
        Ok(res!(s.trim().parse::<u64>().map_err(|_|
            err!("'{}' holds '{}', not a count.", p, s.trim(); Invalid, Input))))
    }

    /// Marks the whole filesystem that holds `path` for `FAN_CLOSE_WRITE`.
    pub fn mark_filesystem(&self, path: &Path) -> Outcome<()> {
        self.mark(path, MarkFlags::FAN_MARK_ADD)
    }

    pub fn unmark_filesystem(&self, path: &Path) -> Outcome<()> {
        self.mark(path, MarkFlags::FAN_MARK_REMOVE)
    }

    fn mark(&self, path: &Path, how: MarkFlags) -> Outcome<()> {
        let at = res!(File::open(path).map_err(|e|
            err!("Opening '{}' to mark its filesystem: {}.", path.display(), e; IO, File, Read)));
        res!(self.fan.mark(how | MarkFlags::FAN_MARK_FILESYSTEM, MaskFlags::FAN_CLOSE_WRITE,
            at.as_fd(), None::<&Path>).map_err(|e|
            err!("fanotify_mark {:?} on the filesystem of '{}': {}.", how, path.display(), e; IO, System)));
        Ok(())
    }

    /// Readable (`POLLIN`) when events wait.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fan.as_fd()
    }

    /// Raises `POLLPRI` when a mount or unmount changes the reader's table; then call
    /// [`Self::refresh_mounts`].
    pub fn mounts_fd(&self) -> BorrowedFd<'_> {
        self.mountinfo.as_fd()
    }

    pub fn mounts(&self) -> &MountTable {
        &self.own
    }

    /// Re-reads the reader's mount table and answers the mounts that are new, which the caller
    /// marks if it wants them heard.
    pub fn refresh_mounts(&mut self) -> Outcome<Vec<Mount>> {
        let now = res!(read_table(&mut self.mountinfo));
        let added = now.added_since(&self.own);
        self.own = now;
        Ok(added)
    }

    /// Reads the events waiting, none if there are none.
    pub fn read(&self) -> Outcome<Vec<FanEvent>> {
        let evs = match self.fan.read_events() {
            Ok(v)               => v,
            Err(Errno::EAGAIN)  => return Ok(Vec::new()),
            Err(e)              => return Err(err!("Reading fanotify events: {}.", e; IO, Read)),
        };
        let mut out = Vec::with_capacity(evs.len());
        for ev in evs {
            if !ev.check_version() {
                return Err(err!("fanotify metadata version {} is not the one built against.",
                    ev.version(); Invalid, System));
            }
            if ev.mask().contains(MaskFlags::FAN_Q_OVERFLOW) {
                out.push(FanEvent::Overflow);
                continue;
            }
            let pid = ev.pid();
            let raw = match ev.fd() {
                Some(fd)    => fd.as_raw_fd(),
                None        => { out.push(FanEvent::NoFd { pid }); continue; }
            };
            let link = match fs::read_link(format!("/proc/self/fd/{}", raw)) {
                Ok(p)   => p.to_string_lossy().into_owned(),
                Err(e)  => return Err(err!("Reading the path of event fd {}: {}.", raw, e; IO, Read)),
            };
            let (mnt, ino) = match ev.fd() {
                Some(fd)    => res!(mount_id(fd)),
                None        => (0, 0),
            };
            let host = if self.own.get(mnt).is_some() {
                resolve(&link, mnt, &self.own, None)
            } else {
                resolve(&link, mnt, &self.own, MountTable::of_pid(pid).as_ref())
            };
            let writer = writer_of(pid);
            out.push(FanEvent::Written(Written { mnt, ino, link, host, writer, event: ev }));
        }
        Ok(out)
    }
}

fn read_table(f: &mut File) -> Outcome<MountTable> {
    let mut s = String::new();
    res!(f.seek(SeekFrom::Start(0)).map_err(|e| err!("Rewinding /proc/self/mountinfo: {}.", e; IO, Read)));
    res!(f.read_to_string(&mut s).map_err(|e| err!("Reading /proc/self/mountinfo: {}.", e; IO, Read)));
    MountTable::parse(&s)
}

/// The writer as `/proc/<pid>` shows it now, or exited if it cannot be read.
pub fn writer_of(pid: i32) -> Writer {
    let status = match fs::read_to_string(format!("/proc/{}/status", pid)) {
        Ok(s)   => s,
        Err(_)  => return Writer::Exited { pid },
    };
    let uids: Vec<u32> = status.lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .map(|r| r.split_whitespace().filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_default();
    if uids.len() != 4 { return Writer::Exited { pid }; }
    let cgroup = fs::read_to_string(format!("/proc/{}/cgroup", pid)).ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("0::").map(|c| c.to_string())))
        .unwrap_or_default();
    Writer::Live { pid, uid: uids[0], fsuid: uids[3], cgroup }
}
