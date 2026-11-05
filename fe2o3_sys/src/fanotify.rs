//! A root watcher of a whole filesystem's directory entries, and a gate that holds a file's
//! first access until its holder has copied it.
//!
//! [`Entries`] is a notification group marked on a filesystem (`FAN_MARK_FILESYSTEM`) that
//! reports each event by file handle and name (`FAN_REPORT_DFID_NAME_TARGET`): the directory
//! an entry is in, the entry's name and the entry itself, never a path. A path is the
//! caller's to keep, keyed by the whole [`Handle`]: its layout is the filesystem's own (tmpfs
//! puts the generation first under the same type ext4 uses for inode then generation, QA 06 Nov
//! 2026), so no inode number is read out of it. [`Entries::handle_of`] gives an open file's handle
//! with no `name_to_handle_at`: an ignore-only inode mark makes the group's
//! `/proc/self/fdinfo` list it, the same bytes the entry events carry (proven on ext4 and tmpfs,
//! VM 06 Nov 2026). A queue that overflowed comes back as [`Change::Lost`], which means that
//! anything may have changed.
//!
//! [`Gate`] is a pre-content group (`FAN_CLASS_PRE_CONTENT`, Linux 6.14+) on single inodes,
//! marked for `FAN_OPEN_PERM` and `FAN_PRE_ACCESS`. The kernel raises one pre-access event for a
//! read, a write, an `mmap` or a truncate alike, and the event's range does not say which, so a
//! holder that must see a file before any change copies it on the first open or access of any
//! kind. The open event is needed because `open(O_TRUNC)` truncates without a pre-access event
//! (the VM round of 06 Nov 2026 saw a held file emptied by `printf x > f` before its write was
//! held). The kernel decides at open whether a file's accesses are asked about at all: only
//! when the inode, or the folder it is in, has a pre-content mark then. So a descriptor opened,
//! or a mapping made, before [`Gate::hold`] of its inode writes it with no event (QA, 06 Nov
//! 2026). [`Gate::guard`] marks a folder for its files (`FAN_EVENT_ON_CHILD`): a file of it opened
//! after the mark is gated (proven in the VM, 06 Nov 2026), so a folder guarded before anything
//! can open its files leaves no such descriptor. The accessing process waits until
//! [`Held::answer`] allows it.
//!
//! A gate fails closed. Its queue is unlimited (`FAN_UNLIMITED_QUEUE`): a bounded one drops a
//! permission event it cannot fit and the kernel lets that access through. A [`Held`] dropped
//! unanswered denies its access. Closing a group allows every access it still holds, so
//! [`Gate::close`] (and Drop) removes every mark and denies what is queued first, and the group
//! closes only when the last `Held` has gone.
//!
//! Both need `CAP_SYS_ADMIN`. The system calls come from `nix`, which owns the `unsafe` this
//! crate forbids; the info records nix does not parse are read here from the event bytes.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    ffi::OsString,
    os::{
        fd::{
            AsFd,
            AsRawFd,
            BorrowedFd,
        },
        unix::{
            ffi::OsStringExt,
            fs::MetadataExt,
        },
    },
    path::Path,
    sync::Arc,
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
const META_VERSION:         u8 = 3;
const INFO_FID:             u8 = 1;
const INFO_DFID_NAME:       u8 = 2;
const INFO_OLD_DFID_NAME:   u8 = 10;
const INFO_NEW_DFID_NAME:   u8 = 12;

const READ_BUF: usize = 64 * 1024; // whole events only, the kernel never splits one

/// A file handle within one filesystem: its exportfs type and bytes, opaque.  It names one inode
/// for that inode's life (the bytes carry its generation), so it is never another's after reuse.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Handle {
    pub kind:   i32,
    pub bytes:  Vec<u8>,
}

/// A file handle as fanotify reports it: the filesystem's id and the handle.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Fid {
    pub fsid:   [i32; 2],
    pub handle: Handle,
}

/// What happened to one directory entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op {
    Create,
    Delete,
    Rename,
    Recheck,    // made and removed in an order the kernel does not keep: stat the name
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

    /// The handle of the open file `fd` (an `O_PATH` one will do), the one this group's events
    /// carry for it.  An ignore-only inode mark, made and removed here, raises no event.
    pub fn handle_of(&self, fd: BorrowedFd) -> Outcome<Handle> {
        // The descriptor's own magic link: the lookup lands on its inode, by no name.
        let at = fmt!("/proc/self/fd/{}", fd.as_raw_fd());
        let m = res!(std::fs::metadata(&at).map_err(|e| err!(e,
            "stat of an open file to find its handle failed."; IO, File)));
        let how = MarkFlags::FAN_MARK_INODE | MarkFlags::FAN_MARK_IGNORED_MASK
            | MarkFlags::FAN_MARK_IGNORED_SURV_MODIFY;
        res!(self.fan.mark(MarkFlags::FAN_MARK_ADD | how, MaskFlags::FAN_OPEN, nix::fcntl::AT_FDCWD,
            Some(Path::new(&at))).map_err(|e| err!(e,
            "The ignore-only mark that lists an open file's handle failed."; IO, System)));
        let info = std::fs::read_to_string(fmt!("/proc/self/fdinfo/{}", self.fan.as_fd().as_raw_fd()));
        let _ = self.fan.mark(MarkFlags::FAN_MARK_REMOVE | how, MaskFlags::FAN_OPEN,
            nix::fcntl::AT_FDCWD, Some(Path::new(&at)));
        let info = res!(info.map_err(|e| err!(e, "Reading the watch group's fdinfo failed."; IO, System)));
        fdinfo_handle(&info, m.ino(), kernel_dev(m.dev()))
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
            // A framing error has already pushed `Lost`; the next read starts at a whole event.
            let _ = parse_events(&buf[..n], &mut out);
        }
        Ok(out)
    }
}

/// Parses whole fanotify events with FID info records into `out`.  An event that cannot be read
/// is pushed as [`Change::Lost`] and parsing goes on.  A buffer whose framing cannot be trusted
/// (a length that does not fit, a short tail, another metadata version) keeps what was parsed,
/// pushes `Lost` and returns the error.
pub fn parse_events(mut b: &[u8], out: &mut Vec<Change>) -> Outcome<()> {
    while !b.is_empty() {
        if b.len() < META_LEN {
            out.push(Change::Lost);
            return Err(err!("{} bytes after the last whole fanotify event are not an event.", b.len();
                Invalid, Input));
        }
        let len = u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let vers = b[4];
        let meta = u16::from_ne_bytes([b[6], b[7]]) as usize;
        if vers != META_VERSION {
            out.push(Change::Lost);
            return Err(err!("A fanotify event's metadata version is {}, not {}.", vers, META_VERSION;
                Invalid, Input));
        }
        if len < META_LEN || len > b.len() || meta < META_LEN || meta > len {
            out.push(Change::Lost);
            return Err(err!("A fanotify event's length {} (metadata {}) does not fit the {} bytes read.",
                len, meta, b.len(); Invalid, Input));
        }
        let mask = u64::from_ne_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]);
        if mask & FAN_Q_OVERFLOW != 0 || parse_one(mask, &b[meta..len], out).is_err() {
            // An overflow, or an event the kernel could not fully encode (no handle record).
            out.push(Change::Lost);
        }
        b = &b[len..];
    }
    Ok(())
}

/// One entry event, pushed only when the whole of it was read.  The kernel merges a queued event
/// with the next of the same folder, name and entry, and a merged `FAN_CREATE | FAN_DELETE` keeps no
/// order: `unlink a; link f a` merges into one event while `a` exists (QA, 06 Nov 2026).  It is
/// reported as [`Op::Recheck`], and the caller looks the name up to learn what is there now.
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
    let op = match (mask & FAN_CREATE != 0, mask & FAN_DELETE != 0) {
        (true, true)    => Op::Recheck,
        (true, false)   => Op::Create,
        _               => Op::Delete,
    };
    out.push(Change::Entry { op, dir, name, to: None, target, dir_entry });
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
    let end = match 16usize.checked_add(n) {
        Some(e) if e <= r.len() => e,
        _ => return Err(err!("A fanotify file handle of {} bytes runs past its record.", n;
            Invalid, Input)),
    };
    let bytes = r[16..end].to_vec();
    let tail = &r[end..];
    let name = match tail.iter().position(|c| *c == 0) {
        Some(end) => tail[..end].to_vec(),
        None => tail.to_vec(),
    };
    Ok((Fid { fsid, handle: Handle { kind, bytes } }, OsString::from_vec(name)))
}

/// The handle of inode `ino` on device `sdev` (the kernel's encoding) in a group's fdinfo, from
/// its mark line: `fanotify ino:<hex> sdev:<hex> ... fhandle-bytes:<hex> fhandle-type:<hex>
/// f_handle:<hex>`.
pub fn fdinfo_handle(info: &str, ino: u64, sdev: u64) -> Outcome<Handle> {
    let head = fmt!("fanotify ino:{:x} sdev:{:x} ", ino, sdev);
    let line = res!(info.lines().find(|l| l.starts_with(&head)).ok_or_else(|| err!(
        "The watch group's fdinfo lists no mark for inode {} (device {:#x}).", ino, sdev; Missing, System)));
    let field = |k: &str| line.split(' ').find_map(|w| w.strip_prefix(k)).ok_or_else(|| err!(
        "The fdinfo mark line for inode {} has no {} field.", ino, k; Missing, System));
    let hexnum = |k: &str| -> Outcome<u32> {
        let v = res!(field(k));
        u32::from_str_radix(v, 16).map_err(|e| err!(e, "The fdinfo field {}{} is not hex.", k, v; Invalid, Input))
    };
    let n = res!(hexnum("fhandle-bytes:")) as usize;
    let kind = res!(hexnum("fhandle-type:")) as i32;
    let hex = res!(field("f_handle:")).as_bytes();
    if n == 0 || hex.len() != 2 * n {
        return Err(err!("The fdinfo handle of inode {} is {} hex digits for {} bytes (a filesystem \
            without handles?).", ino, hex.len(), n; Invalid, Input));
    }
    let mut bytes = Vec::with_capacity(n);
    for p in hex.chunks(2) {
        let d = |c: u8| (c as char).to_digit(16);
        match (d(p[0]), d(p[1])) {
            (Some(h), Some(l))  => bytes.push((h * 16 + l) as u8),
            _                   => return Err(err!("The fdinfo handle of inode {} is not hex.", ino;
                Invalid, Input)),
        }
    }
    Ok(Handle { kind, bytes })
}

/// A device number as user space has it (glibc's `makedev`) in the kernel's own encoding, which
/// fdinfo prints.
fn kernel_dev(dev: u64) -> u64 {
    // fixed: <sys/sysmacros.h> and the kernel's MKDEV (major << 20 | minor).
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    (major << 20) | minor
}

/// A pre-content group: each marked file's next access waits for [`Held::answer`].
#[derive(Debug)]
pub struct Gate {
    fan:    Arc<Fanotify>,
    closed: bool,
}

/// What [`Gate::next`] read.
#[derive(Debug)]
pub enum Access {
    Held(Held),
    /// An event with no descriptor (or an overflow): an access the gate cannot answer or name,
    /// so any hold may have been passed.
    Lost,
}

/// One access held at the gate.  The descriptor reads the file without raising an event.  Dropped
/// unanswered, it denies the access.
#[derive(Debug)]
pub struct Held {
    ev:     Option<FanotifyEvent>,
    fan:    Arc<Fanotify>,
}

impl Held {
    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.ev.as_ref().and_then(|e| e.fd())
    }

    /// The accessing process.
    pub fn pid(&self) -> i32 {
        self.ev.as_ref().map(|e| e.pid()).unwrap_or(0)
    }

    /// Lets the held access go on (`allow`) or fails it with `EPERM`.
    pub fn answer(mut self, allow: bool) -> Outcome<()> {
        let ev = res!(self.ev.take().ok_or_else(|| err!(
            "A held access was already answered."; Missing, Input)));
        respond(&self.fan, &ev, allow)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Some(ev) = self.ev.take() {
            let _ = respond(&self.fan, &ev, false);
        }
    }
}

fn respond(fan: &Fanotify, ev: &FanotifyEvent, allow: bool) -> Outcome<()> {
    let fd = res!(ev.fd().ok_or_else(|| err!(
        "A held access has no descriptor to answer."; Missing, Input)));
    let r = if allow { Response::FAN_ALLOW } else { Response::FAN_DENY };
    res!(fan.write_response(FanotifyResponse::new(fd, r)).map_err(|e| err!(e,
        "Answering a held access failed."; IO, System)));
    Ok(())
}

impl Gate {
    pub fn new() -> Outcome<Self> {
        let flags = InitFlags::FAN_CLASS_PRE_CONTENT | InitFlags::FAN_CLOEXEC | InitFlags::FAN_NONBLOCK
            | InitFlags::FAN_UNLIMITED_QUEUE;
        let fan = res!(Fanotify::init(flags, EventFFlags::O_RDONLY | EventFFlags::O_LARGEFILE
            | EventFFlags::O_CLOEXEC).map_err(|e| err!(e,
            "fanotify_init for a pre-content gate failed (it needs CAP_SYS_ADMIN)."; IO, System)));
        Ok(Self { fan: Arc::new(fan), closed: false })
    }

    /// Holds the next access of the open file `file` (its inode, whatever names it).  A
    /// descriptor already open on it is not asked about: see the module note.
    pub fn hold(&self, file: impl AsFd) -> Outcome<()> {
        self.mark(MarkFlags::FAN_MARK_ADD, 0, file)
    }

    /// Stops holding `file`'s accesses.
    pub fn release(&self, file: impl AsFd) -> Outcome<()> {
        self.mark(MarkFlags::FAN_MARK_REMOVE, 0, file)
    }

    /// Holds every access of a file in the open folder `dir` opened after this call.
    pub fn guard(&self, dir: impl AsFd) -> Outcome<()> {
        self.mark(MarkFlags::FAN_MARK_ADD, MaskFlags::FAN_EVENT_ON_CHILD.bits(), dir)
    }

    /// Stops guarding the folder `dir`.
    pub fn unguard(&self, dir: impl AsFd) -> Outcome<()> {
        self.mark(MarkFlags::FAN_MARK_REMOVE, MaskFlags::FAN_EVENT_ON_CHILD.bits(), dir)
    }

    fn mark(&self, how: MarkFlags, more: u64, file: impl AsFd) -> Outcome<()> {
        let mask = MaskFlags::from_bits_retain(FAN_PRE_ACCESS | FAN_OPEN_PERM | more);
        res!(self.fan.mark(how | MarkFlags::FAN_MARK_INODE, mask, file, None::<&Path>).map_err(|e| err!(e,
            "fanotify_mark ({:?}) of a held inode failed (pre-content needs Linux 6.14 and a \
            filesystem that allows it).", how; IO, System)));
        Ok(())
    }

    /// Waits up to `wait` (`None`: for ever) and returns the accesses now held.
    pub fn next(&self, wait: Option<Duration>) -> Outcome<Vec<Access>> {
        if !res!(readable(self.fan.as_fd(), wait)) {
            return Ok(Vec::new());
        }
        match self.fan.read_events() {
            Ok(v) => Ok(v.into_iter().map(|ev| {
                // nix's mask() keeps FAN_Q_OVERFLOW (a bit it knows); never read FAN_PRE_ACCESS through it.
                if ev.fd().is_none() || ev.mask().contains(MaskFlags::FAN_Q_OVERFLOW) {
                    Access::Lost
                } else {
                    Access::Held(Held { ev: Some(ev), fan: self.fan.clone() })
                }
            }).collect()),
            Err(Errno::EAGAIN) | Err(Errno::EINTR) => Ok(Vec::new()),
            Err(e) => Err(err!(e, "Reading the pre-content gate failed."; IO, System)),
        }
    }

    /// Removes every mark, then denies each access still queued, so nothing the gate held goes
    /// through when the group closes.  The group closes once the last [`Held`] has gone.
    pub fn close(&mut self) -> Outcome<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        res!(self.fan.mark(MarkFlags::FAN_MARK_FLUSH, MaskFlags::empty(), nix::fcntl::AT_FDCWD,
            None::<&Path>).map_err(|e| err!(e, "Flushing the gate's marks failed."; IO, System)));
        loop {
            match self.fan.read_events() {
                Ok(v) if v.is_empty()   => break,
                Ok(v)                   => for ev in v {
                    let _ = respond(&self.fan, &ev, false); // no fd: nothing to answer
                },
                Err(Errno::EINTR)       => continue,
                Err(Errno::EAGAIN)      => break,
                Err(e) => return Err(err!(e, "Draining the gate at its close failed."; IO, System)),
            }
        }
        Ok(())
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        let _ = self.close();
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

