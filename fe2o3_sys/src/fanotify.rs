//! A root watcher of a whole filesystem's directory entries, and a gate that holds a file's
//! first access until its holder has copied it.
//!
//! [`Entries`] is a notification group marked on a filesystem (`FAN_MARK_FILESYSTEM`) that
//! reports each event by file handle and name (`FAN_REPORT_DFID_NAME_TARGET`): the directory
//! an entry is in, the entry's name and the entry itself, never a path. A path is the
//! caller's to keep, from the inode numbers it already holds: [`Fid::ino`] reads them out of
//! the handle where the filesystem's handle format is known. A queue that overflowed comes
//! back as [`Change::Lost`], which means that anything may have changed.
//!
//! [`Gate`] is a pre-content group (`FAN_CLASS_PRE_CONTENT`, Linux 6.14+) on single inodes,
//! marked for `FAN_OPEN_PERM` and `FAN_PRE_ACCESS`. The kernel raises one pre-access event for a
//! read, a write, an `mmap` or a truncate alike, and the event's range does not say which, so a
//! holder that must see a file before any change copies it on the first open or access of any
//! kind. The open event is needed because `open(O_TRUNC)` truncates without a pre-access event
//! (the VM round of 06 Nov 2026 saw a held file emptied by `printf x > f` before its write was
//! held). A descriptor opened before the filesystem had any pre-content mark is never asked
//! about its accesses: the kernel decides that at open. The accessing process waits until
//! [`Gate::answer`] allows it.
//!
//! Both need `CAP_SYS_ADMIN`. The system calls come from `nix`, which owns the `unsafe` this
//! crate forbids; the info records nix does not parse are read here from the event bytes.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    ffi::OsString,
    os::{
        fd::{
            AsFd,
            BorrowedFd,
        },
        unix::ffi::OsStringExt,
    },
    path::Path,
    time::Duration,
};

use nix::{
    errno::Errno,
    poll::{
        poll,
        PollFd,
        PollFlags,
        PollTimeout,
    },
    sys::fanotify::{
        EventFFlags,
        Fanotify,
        FanotifyEvent,
        FanotifyResponse,
        InitFlags,
        MarkFlags,
        MaskFlags,
        Response,
    },
};

// fixed: the kernel's uapi <linux/fanotify.h>, which libc does not yet carry.
const FAN_PRE_ACCESS:       u64 = 0x0010_0000;
const FAN_REPORT_DFID_NAME_TARGET: u32 = 0x0000_1e00; // DIR_FID | NAME | FID | TARGET_FID
const FAN_Q_OVERFLOW:       u64 = 0x0000_4000;
const FAN_CREATE:           u64 = 0x0000_0100;
const FAN_DELETE:           u64 = 0x0000_0200;
const FAN_OPEN_PERM:        u64 = 0x0001_0000;
const FAN_RENAME:           u64 = 0x1000_0000;
const FAN_ONDIR:            u64 = 0x4000_0000;
// fixed: fanotify_event_metadata and the info record header, kernel ABI.
const META_LEN:             usize = 24;
const INFO_FID:             u8 = 1;
const INFO_DFID_NAME:       u8 = 2;
const INFO_OLD_DFID_NAME:   u8 = 10;
const INFO_NEW_DFID_NAME:   u8 = 12;
// fixed: <linux/exportfs.h> handle types whose first word is the inode number.
const FILEID_INO32_GEN:         i32 = 1;
const FILEID_INO32_GEN_PARENT:  i32 = 2;
const FILEID_INO64_GEN:         i32 = 0x81;
const FILEID_INO64_GEN_PARENT:  i32 = 0x82;
const FILEID_BTRFS_WITHOUT_PARENT:  i32 = 0x4d;
const FILEID_BTRFS_WITH_PARENT:     i32 = 0x4e;
const FILEID_BTRFS_WITH_PARENT_ROOT: i32 = 0x4f;

const READ_BUF: usize = 64 * 1024; // whole events only, the kernel never splits one

/// A file handle as fanotify reports it: the filesystem's id, the handle's type and bytes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Fid {
    pub fsid:   [i32; 2],
    pub kind:   i32,        // the exportfs handle type
    pub handle: Vec<u8>,
}

impl Fid {
    /// The inode number the handle carries, where its type is one whose layout is known.
    pub fn ino(&self) -> Option<u64> {
        let h = &self.handle;
        match self.kind {
            FILEID_INO32_GEN | FILEID_INO32_GEN_PARENT if h.len() >= 4 =>
                Some(u32::from_ne_bytes([h[0], h[1], h[2], h[3]]) as u64),
            FILEID_INO64_GEN | FILEID_INO64_GEN_PARENT
            | FILEID_BTRFS_WITHOUT_PARENT | FILEID_BTRFS_WITH_PARENT
            | FILEID_BTRFS_WITH_PARENT_ROOT if h.len() >= 8 =>
                Some(u64::from_ne_bytes([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]])),
            _ => None,
        }
    }
}

/// What happened to one directory entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op {
    Create,
    Delete,
    Rename,
}

/// One event of an [`Entries`] group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Change {
    /// `name` in directory `dir` was made or removed (`to` is `None`), or renamed from `dir`/`name`
    /// to `to`.  `target` is the entry itself, `dir_entry` whether it is a directory.
    Entry {
        op:         Op,
        dir:        Fid,
        name:       OsString,
        to:         Option<(Fid, OsString)>,
        target:     Option<Fid>,
        dir_entry:  bool,
    },
    /// The kernel's queue overflowed: any entry may have changed since the last event.
    Lost,
}

/// A filesystem-wide watch of directory entries made, removed and renamed.
#[derive(Debug)]
pub struct Entries {
    fan: Fanotify,
}

impl Entries {
    /// Marks the whole filesystem `on` lies on.  The queue's bound is the kernel's
    /// `fs.fanotify.max_queued_events` at this call.
    pub fn filesystem(on: &Path) -> Outcome<Self> {
        let en = res!(Self::group());
        res!(en.fan.mark(MarkFlags::FAN_MARK_ADD | MarkFlags::FAN_MARK_FILESYSTEM, Self::mask(),
            nix::fcntl::AT_FDCWD, Some(on)).map_err(|e| err!(e,
            "fanotify_mark of the filesystem under {} failed.", on.display(); IO, System)));
        Ok(en)
    }

    /// Marks the whole filesystem the open file `on` lies on, by descriptor, so no path is
    /// resolved again (open it with [`crate::nolink`] where a user can shape the path).  The
    /// kernel refuses an `O_PATH` descriptor here (EBADF): open a folder for reading.
    pub fn filesystem_of(on: BorrowedFd) -> Outcome<Self> {
        let en = res!(Self::group());
        res!(en.fan.mark(MarkFlags::FAN_MARK_ADD | MarkFlags::FAN_MARK_FILESYSTEM, Self::mask(),
            on, None::<&Path>).map_err(|e| err!(e,
            "fanotify_mark of the filesystem of an open file failed."; IO, System)));
        Ok(en)
    }

    fn group() -> Outcome<Self> {
        let flags = InitFlags::FAN_CLASS_NOTIF | InitFlags::FAN_CLOEXEC | InitFlags::FAN_NONBLOCK
            | InitFlags::from_bits_retain(FAN_REPORT_DFID_NAME_TARGET);
        let fan = res!(Fanotify::init(flags, EventFFlags::O_RDONLY).map_err(|e| err!(e,
            "fanotify_init for a filesystem watch failed (it needs CAP_SYS_ADMIN)."; IO, System)));
        Ok(Self { fan })
    }

    fn mask() -> MaskFlags {
        MaskFlags::from_bits_retain(FAN_CREATE | FAN_DELETE | FAN_RENAME | FAN_ONDIR)
    }

    /// Waits up to `wait` for events (`None` waits for ever) and returns all that are queued.
    pub fn next(&self, wait: Option<Duration>) -> Outcome<Vec<Change>> {
        if !res!(readable(self.fan.as_fd(), wait)) {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; READ_BUF];
        let mut out = Vec::new();
        loop {
            let n = match nix::unistd::read(self.fan.as_fd(), &mut buf) {
                Ok(0)               => break,
                Ok(n)               => n,
                Err(Errno::EAGAIN)  => break,
                Err(Errno::EINTR)   => continue,
                Err(e) => return Err(err!(e, "Reading the filesystem watch failed."; IO, System)),
            };
            res!(parse_events(&buf[..n], &mut out));
        }
        Ok(out)
    }
}

/// Parses whole fanotify events with FID info records into `out`.
pub fn parse_events(mut b: &[u8], out: &mut Vec<Change>) -> Outcome<()> {
    while b.len() >= META_LEN {
        let len = u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let meta = u16::from_ne_bytes([b[6], b[7]]) as usize;
        if len < META_LEN || len > b.len() || meta < META_LEN || meta > len {
            return Err(err!("A fanotify event's length {} (metadata {}) does not fit the {} bytes read.",
                len, meta, b.len(); Invalid, Input));
        }
        let mask = u64::from_ne_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]);
        if mask & FAN_Q_OVERFLOW != 0 {
            out.push(Change::Lost);
        } else {
            res!(parse_one(mask, &b[meta..len], out));
        }
        b = &b[len..];
    }
    Ok(())
}

/// One entry event.  The kernel merges a queued event with the next of the same folder, name and
/// entry, so `FAN_CREATE | FAN_DELETE` is one entry made and then removed (a new entry at the
/// name has a new handle and is never merged with it), and is reported as both, in that order.
/// A rename is asked for only as `FAN_RENAME`, which carries both names and is never merged with
/// a create or a delete; `FAN_MOVED_FROM` and `FAN_MOVED_TO` are not asked for, since each merges
/// with a create or delete of the same name and would hide it.
fn parse_one(mask: u64, mut r: &[u8], out: &mut Vec<Change>) -> Outcome<()> {
    let mut from: Option<(Fid, OsString)> = None;
    let mut to: Option<(Fid, OsString)> = None;
    let mut target = None;
    while r.len() >= 4 {
        let kind = r[0];
        let len = u16::from_ne_bytes([r[2], r[3]]) as usize;
        if len < 4 || len > r.len() {
            return Err(err!("A fanotify info record's length {} does not fit the {} bytes left.",
                len, r.len(); Invalid, Input));
        }
        let rec = &r[4..len];
        match kind {
            INFO_FID => target = Some(res!(fid_of(rec)).0),
            INFO_DFID_NAME | INFO_OLD_DFID_NAME => from = Some(res!(fid_of(rec))),
            INFO_NEW_DFID_NAME => to = Some(res!(fid_of(rec))),
            _ => (), // a record kind this watch does not ask for
        }
        r = &r[len..];
    }
    let (dir, name) = res!(from.ok_or_else(|| err!(
        "A fanotify entry event (mask {:#x}) carries no directory and name.", mask; Missing, Input)));
    let dir_entry = mask & FAN_ONDIR != 0;
    if mask & FAN_RENAME != 0 {
        out.push(Change::Entry { op: Op::Rename, dir, name, to, target, dir_entry });
        return Ok(());
    }
    if mask & (FAN_CREATE | FAN_DELETE) == 0 {
        return Err(err!("A fanotify event's mask {:#x} is not one this watch asked for.", mask;
            Unexpected, Input));
    }
    if mask & FAN_CREATE != 0 {
        out.push(Change::Entry { op: Op::Create, dir: dir.clone(), name: name.clone(), to: None,
            target: target.clone(), dir_entry });
    }
    if mask & FAN_DELETE != 0 {
        out.push(Change::Entry { op: Op::Delete, dir, name, to: None, target, dir_entry });
    }
    Ok(())
}

/// One `fanotify_event_info_fid` body after its header: fsid, handle, then a NUL-ended name
/// (empty when the record has none).
fn fid_of(r: &[u8]) -> Outcome<(Fid, OsString)> {
    if r.len() < 16 {
        return Err(err!("A fanotify FID record of {} bytes is shorter than its fixed 16.", r.len();
            Invalid, Input));
    }
    let w = |i: usize| [r[i], r[i + 1], r[i + 2], r[i + 3]];
    let fsid = [i32::from_ne_bytes(w(0)), i32::from_ne_bytes(w(4))];
    let n = u32::from_ne_bytes(w(8)) as usize;
    let kind = i32::from_ne_bytes(w(12));
    if 16 + n > r.len() {
        return Err(err!("A fanotify file handle of {} bytes runs past its record.", n; Invalid, Input));
    }
    let handle = r[16..16 + n].to_vec();
    let tail = &r[16 + n..];
    let name = match tail.iter().position(|c| *c == 0) {
        Some(end) => tail[..end].to_vec(),
        None => tail.to_vec(),
    };
    Ok((Fid { fsid, kind, handle }, OsString::from_vec(name)))
}

/// A pre-content group: each marked file's next access waits for [`Gate::answer`].
#[derive(Debug)]
pub struct Gate {
    fan: Fanotify,
}

/// One access held at the gate.  The descriptor reads the file without raising an event.
#[derive(Debug)]
pub struct Held {
    ev: FanotifyEvent,
}

impl Held {
    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.ev.fd()
    }

    /// The accessing process.
    pub fn pid(&self) -> i32 {
        self.ev.pid()
    }
}

impl Gate {
    pub fn new() -> Outcome<Self> {
        let flags = InitFlags::FAN_CLASS_PRE_CONTENT | InitFlags::FAN_CLOEXEC | InitFlags::FAN_NONBLOCK;
        let fan = res!(Fanotify::init(flags, EventFFlags::O_RDONLY | EventFFlags::O_LARGEFILE
            | EventFFlags::O_CLOEXEC).map_err(|e| err!(e,
            "fanotify_init for a pre-content gate failed (it needs CAP_SYS_ADMIN)."; IO, System)));
        Ok(Self { fan })
    }

    /// Holds the next access of the open file `file` (its inode, whatever names it).
    pub fn hold(&self, file: impl AsFd) -> Outcome<()> {
        self.mark(MarkFlags::FAN_MARK_ADD, file)
    }

    /// Stops holding `file`'s accesses.
    pub fn release(&self, file: impl AsFd) -> Outcome<()> {
        self.mark(MarkFlags::FAN_MARK_REMOVE, file)
    }

    fn mark(&self, how: MarkFlags, file: impl AsFd) -> Outcome<()> {
        let mask = MaskFlags::from_bits_retain(FAN_PRE_ACCESS | FAN_OPEN_PERM);
        res!(self.fan.mark(how | MarkFlags::FAN_MARK_INODE, mask, file, None::<&Path>).map_err(|e| err!(e,
            "fanotify_mark ({:?}) of a held file's inode failed (pre-content needs Linux 6.14 and a \
            filesystem that allows it).", how; IO, System)));
        Ok(())
    }

    /// Waits up to `wait` (`None`: for ever) and returns the accesses now held.
    pub fn next(&self, wait: Option<Duration>) -> Outcome<Vec<Held>> {
        if !res!(readable(self.fan.as_fd(), wait)) {
            return Ok(Vec::new());
        }
        match self.fan.read_events() {
            Ok(v) => Ok(v.into_iter().map(|ev| Held { ev }).collect()),
            Err(Errno::EAGAIN) | Err(Errno::EINTR) => Ok(Vec::new()),
            Err(e) => Err(err!(e, "Reading the pre-content gate failed."; IO, System)),
        }
    }

    /// Lets the held access go on (`allow`) or fails it with `EPERM`.
    pub fn answer(&self, held: Held, allow: bool) -> Outcome<()> {
        let fd = res!(held.ev.fd().ok_or_else(|| err!(
            "A held access has no descriptor to answer."; Missing, Input)));
        let r = if allow { Response::FAN_ALLOW } else { Response::FAN_DENY };
        res!(self.fan.write_response(FanotifyResponse::new(fd, r)).map_err(|e| err!(e,
            "Answering a held access failed."; IO, System)));
        Ok(())
    }
}

/// The group's descriptor, for a caller's own poll.
impl AsFd for Entries {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fan.as_fd()
    }
}

impl AsFd for Gate {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fan.as_fd()
    }
}

/// Whether `fd` became readable within `wait`.
fn readable(fd: BorrowedFd<'_>, wait: Option<Duration>) -> Outcome<bool> {
    let t = match wait {
        None    => PollTimeout::NONE,
        Some(d) => PollTimeout::try_from(d.as_millis().min(i32::MAX as u128) as i32)
            .unwrap_or(PollTimeout::MAX),
    };
    let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
    match poll(&mut fds, t) {
        Ok(n)               => Ok(n > 0),
        Err(Errno::EINTR)   => Ok(false),
        Err(e) => Err(err!(e, "Polling a fanotify group failed."; IO, System)),
    }
}

