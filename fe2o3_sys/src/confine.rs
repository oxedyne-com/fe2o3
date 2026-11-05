//! Confining a process tree as root: namespaces, a mount plan, a pid-1 reaper and a veth pair.
//!
//! A root service that starts a child under another uid can leave that child's view of the
//! machine exactly the host's, so that whatever the child's uid can reach, the child can. This
//! module holds the parts a root starter needs to narrow that view before it drops privilege:
//!
//! - [`unshare`]: new mount, IPC, network and pid namespaces for the caller.
//! - [`Step`] and [`apply`]: a mount plan, written as data so it can be checked before it runs.
//! - [`reap_until`]: the loop a pid 1 runs, reaping every orphan until its own child ends.
//! - [`exit_on_hangup`]: a pid 1 that ends when the stage that started it does.
//! - [`Rtnl`]: a small rtnetlink writer that makes a veth pair across two network namespaces,
//!   addresses it, brings it up and routes through it, so a root starter never execs `ip` (a
//!   binary named by a config is a binary whoever owns the config may choose).
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids.
//!
//! 06 Nov 2026 (KIMB-USERMGR): written for Kimberlite's parent, whose network namespace gets one
//! veth pair to the host and whose mount namespace hides the owner's session.

use oxedyne_fe2o3_core::prelude::*;

use nix::errno::Errno;
use nix::mount::{
    mount,
    MsFlags,
};
use nix::poll::{
    poll,
    PollFd,
    PollFlags,
    PollTimeout,
};
use nix::sched::CloneFlags;
use nix::sys::socket::{
    bind,
    recv,
    send,
    socket,
    AddressFamily,
    MsgFlags,
    NetlinkAddr,
    SockFlag,
    SockProtocol,
    SockType,
};
use nix::sys::wait::{
    waitpid,
    WaitStatus,
};
use nix::unistd::Pid;

use std::{
    fs,
    net::Ipv4Addr,
    os::fd::{
        AsFd,
        AsRawFd,
        BorrowedFd,
        OwnedFd,
    },
    os::unix::fs::{
        symlink,
        PermissionsExt,
    },
    path::{
        Path,
        PathBuf,
    },
};

// ┌───────────────────────────────────────────────────────────────┐
// │ Namespaces                                                     │
// └───────────────────────────────────────────────────────────────┘

/// One kind of namespace the caller may leave.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ns {
    Mount,  // CLONE_NEWNS
    Ipc,    // CLONE_NEWIPC
    Net,    // CLONE_NEWNET
    Pid,    // CLONE_NEWPID: only the caller's next child is born in it
}

impl Ns {
    fn flag(&self) -> CloneFlags {
        match self {
            Self::Mount => CloneFlags::CLONE_NEWNS,
            Self::Ipc   => CloneFlags::CLONE_NEWIPC,
            Self::Net   => CloneFlags::CLONE_NEWNET,
            Self::Pid   => CloneFlags::CLONE_NEWPID,
        }
    }
}

/// Leaves the caller's namespaces of each kind named for new ones.  Needs `CAP_SYS_ADMIN`.
pub fn unshare(kinds: &[Ns]) -> Outcome<()> {
    let flags = kinds.iter().fold(CloneFlags::empty(), |f, k| f | k.flag());
    nix::sched::unshare(flags).map_err(|e| err!(e,
        "unshare({:?}) failed.", kinds; System, Init))
}

// ┌───────────────────────────────────────────────────────────────┐
// │ The mount plan                                                 │
// └───────────────────────────────────────────────────────────────┘

/// One step of a mount plan, applied in order by [`apply`] inside a mount namespace of its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Step {
    /// Makes every mount under `at` a slave, so nothing done here propagates back to the host.
    Slave       { at: PathBuf },
    /// A fresh tmpfs, read-only once made when `ro`.
    Tmpfs       { at: PathBuf, mode: u32, ro: bool },
    /// A new devpts instance whose `ptmx` serves it alone.
    Devpts      { at: PathBuf },
    Mqueue      { at: PathBuf },
    Proc        { at: PathBuf },
    /// A bind of `from` over `at`, recursive when `rec`, then remounted read-only when `ro`.
    /// `at` is made (an empty file or directory, as `from` is) when it is missing.
    Bind        { from: PathBuf, at: PathBuf, ro: bool, rec: bool },
    Dir         { at: PathBuf, mode: u32 },
    Link        { target: PathBuf, at: PathBuf },
    /// A file written with `text`, mode 0644.
    File        { at: PathBuf, text: String },
}

/// Applies a mount plan step by step.  The first refusal ends it, naming the step.
pub fn apply(steps: &[Step]) -> Outcome<()> {
    for s in steps {
        res!(apply_one(s));
    }
    Ok(())
}

fn mnt(src: Option<&str>, at: &Path, fs: Option<&str>, flags: MsFlags, data: Option<&str>, s: &Step)
    -> Outcome<()>
{
    mount(src, at, fs, flags, data).map_err(|e| err!(e, "The mount step {:?} failed.", s; System, Init))
}

fn apply_one(s: &Step) -> Outcome<()> {
    let ro_remount = MsFlags::MS_REMOUNT | MsFlags::MS_BIND | MsFlags::MS_RDONLY
        | MsFlags::MS_NOSUID | MsFlags::MS_NODEV;
    match s {
        Step::Slave { at } => mnt(None, at, None, MsFlags::MS_REC | MsFlags::MS_SLAVE, None, s),
        Step::Tmpfs { at, mode, ro } => {
            let data = fmt!("mode={:o}", mode);
            res!(mnt(Some("tmpfs"), at, Some("tmpfs"), MsFlags::MS_NOSUID, Some(&data), s));
            if *ro {
                res!(mnt(None, at, None, MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY | MsFlags::MS_NOSUID,
                    Some(&data), s));
            }
            Ok(())
        },
        Step::Devpts { at } => {
            res!(make_dir(at, 0o755, s));
            // fixed: the kernel's devpts options for a private instance.
            mnt(Some("devpts"), at, Some("devpts"), MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
                Some("newinstance,ptmxmode=0666,mode=0620"), s)
        },
        Step::Mqueue { at } => {
            res!(make_dir(at, 0o755, s));
            mnt(Some("mqueue"), at, Some("mqueue"),
                MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC, None, s)
        },
        Step::Proc { at } => mnt(Some("proc"), at, Some("proc"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC, None, s),
        Step::Bind { from, at, ro, rec } => {
            if fs::symlink_metadata(at).is_err() {
                let dir = res!(fs::metadata(from).map_err(|e| err!(e,
                    "The mount step {:?} found no {}.", s, from.display(); IO, File))).is_dir();
                if dir {
                    res!(make_dir(at, 0o755, s));
                } else {
                    res!(fs::write(at, b"").map_err(|e| err!(e,
                        "The mount step {:?} could not make {}.", s, at.display(); IO, File)));
                }
            }
            let mut f = MsFlags::MS_BIND;
            if *rec {
                f |= MsFlags::MS_REC;
            }
            res!(mnt(Some(&from.to_string_lossy()), at, None, f, None, s));
            if *ro {
                res!(mnt(None, at, None, ro_remount, None, s));
            }
            Ok(())
        },
        Step::Dir { at, mode } => make_dir(at, *mode, s),
        Step::Link { target, at } => symlink(target, at).map_err(|e| err!(e,
            "The mount step {:?} could not link.", s; IO, File)),
        Step::File { at, text } => {
            res!(fs::write(at, text).map_err(|e| err!(e,
                "The mount step {:?} could not write.", s; IO, File)));
            fs::set_permissions(at, fs::Permissions::from_mode(0o644)).map_err(|e| err!(e,
                "The mount step {:?} could not set its mode.", s; IO, File))
        },
    }
}

fn make_dir(at: &Path, mode: u32, s: &Step) -> Outcome<()> {
    if let Err(e) = fs::create_dir_all(at) {
        return Err(err!(e, "The mount step {:?} could not make {}.", s, at.display(); IO, File));
    }
    fs::set_permissions(at, fs::Permissions::from_mode(mode)).map_err(|e| err!(e,
        "The mount step {:?} could not set the mode of {}.", s, at.display(); IO, File))
}

// ┌───────────────────────────────────────────────────────────────┐
// │ Pid 1                                                          │
// └───────────────────────────────────────────────────────────────┘

/// Reaps every process that ends under the caller, as a pid 1 must, until `child` ends, and
/// returns its exit code (128 plus the signal's number when a signal ended it).
pub fn reap_until(child: u32) -> Outcome<i32> {
    let want = Pid::from_raw(child as i32);
    loop {
        match waitpid(None, None) {
            Ok(WaitStatus::Exited(p, code)) if p == want        => return Ok(code),
            Ok(WaitStatus::Signaled(p, sig, _)) if p == want    => return Ok(128 + sig as i32),
            Ok(_)                                               => continue,
            Err(Errno::EINTR)                                   => continue,
            Err(e) => return Err(err!(e,
                "Reaping stopped before pid {} ended.", child; System, Missing)),
        }
    }
}

/// Starts a thread that ends the whole process with `code` once the far side of the pipe `fd`
/// has gone: every writer, for a read end, or every reader, for a write end.  A pid 1 holding one
/// end of a pipe whose other end its starter keeps ends with its starter, which a parent-death
/// signal alone does not promise (it follows the thread that started the child, not the process).
pub fn exit_on_hangup(fd: OwnedFd, code: i32) -> Outcome<()> {
    let spawned = std::thread::Builder::new().name("hangup".to_string()).spawn(move || {
        loop {
            // No events asked for: the kernel reports POLLHUP and POLLERR whatever is asked.
            let mut fds = [PollFd::new(fd.as_fd(), PollFlags::empty())];
            match poll(&mut fds, PollTimeout::NONE) {
                Ok(_)               => {
                    let gone = PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL;
                    if fds[0].revents().is_some_and(|r| r.intersects(gone)) {
                        std::process::exit(code);
                    }
                },
                Err(Errno::EINTR)   => continue,
                Err(_)              => std::process::exit(code),
            }
        }
    });
    match spawned {
        Ok(_)   => Ok(()),
        Err(e)  => Err(err!(e, "The hang-up watch could not start."; System, Init)),
    }
}

// ┌───────────────────────────────────────────────────────────────┐
// │ rtnetlink                                                      │
// └───────────────────────────────────────────────────────────────┘

// fixed: rtnetlink's ABI (linux/netlink.h, linux/rtnetlink.h, linux/if_link.h, linux/veth.h).
const NLMSG_ERROR:      u16 = 2;
const RTM_NEWLINK:      u16 = 16;
const RTM_DELLINK:      u16 = 17;
const RTM_GETLINK:      u16 = 18;
const RTM_NEWADDR:      u16 = 20;
const RTM_NEWROUTE:     u16 = 24;
const NLM_F_REQUEST:    u16 = 0x1;
const NLM_F_ACK:        u16 = 0x4;
const NLM_F_EXCL:       u16 = 0x200;
const NLM_F_CREATE:     u16 = 0x400;
const IFLA_IFNAME:      u16 = 3;
const IFLA_LINKINFO:    u16 = 18;
const IFLA_NET_NS_FD:   u16 = 28;
const IFLA_INFO_KIND:   u16 = 1;
const IFLA_INFO_DATA:   u16 = 2;
const VETH_INFO_PEER:   u16 = 1;
const IFA_ADDRESS:      u16 = 1;
const IFA_LOCAL:        u16 = 2;
const RTA_GATEWAY:      u16 = 5;
const AF_INET:          u8  = 2;
const IFF_UP:           u32 = 1;
const RT_TABLE_MAIN:    u8  = 254;
const RTPROT_BOOT:      u8  = 3;
const RTN_UNICAST:      u8  = 1;
const IFNAMSIZ:         usize = 16;

/// A request message under construction: the netlink header, a fixed body, then attributes.
struct Msg {
    buf:    Vec<u8>,
    nests:  Vec<usize>,
}

impl Msg {
    fn new(kind: u16, flags: u16, seq: u32) -> Self {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&0u32.to_ne_bytes());
        buf.extend_from_slice(&kind.to_ne_bytes());
        buf.extend_from_slice(&(flags | NLM_F_REQUEST).to_ne_bytes());
        buf.extend_from_slice(&seq.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes());
        Self { buf, nests: Vec::new() }
    }

    /// `struct ifinfomsg`.
    fn ifinfo(mut self, index: i32, flags: u32, change: u32) -> Self {
        self.buf.extend_from_slice(&[0, 0, 0, 0]); // family, pad, type
        self.buf.extend_from_slice(&index.to_ne_bytes());
        self.buf.extend_from_slice(&flags.to_ne_bytes());
        self.buf.extend_from_slice(&change.to_ne_bytes());
        self
    }

    fn pad(&mut self) {
        while self.buf.len() % 4 != 0 {
            self.buf.push(0);
        }
    }

    fn attr(&mut self, kind: u16, val: &[u8]) {
        self.buf.extend_from_slice(&((4 + val.len()) as u16).to_ne_bytes());
        self.buf.extend_from_slice(&kind.to_ne_bytes());
        self.buf.extend_from_slice(val);
        self.pad();
    }

    fn name(&mut self, kind: u16, name: &str) {
        let mut v = name.as_bytes().to_vec();
        v.push(0);
        self.attr(kind, &v);
    }

    fn open(&mut self, kind: u16) {
        self.nests.push(self.buf.len());
        self.buf.extend_from_slice(&0u16.to_ne_bytes());
        self.buf.extend_from_slice(&kind.to_ne_bytes());
    }

    fn close(&mut self) {
        if let Some(at) = self.nests.pop() {
            let len = (self.buf.len() - at) as u16;
            self.buf[at..at + 2].copy_from_slice(&len.to_ne_bytes());
        }
    }

    fn done(mut self) -> Vec<u8> {
        let len = self.buf.len() as u32;
        self.buf[0..4].copy_from_slice(&len.to_ne_bytes());
        self.buf
    }
}

fn check_name(name: &str) -> Outcome<()> {
    if name.is_empty() || name.len() >= IFNAMSIZ || name.contains('/') || name.contains(char::is_whitespace) {
        return Err(err!("'{}' is not an interface name (1 to {} bytes, no '/' or space).",
            name, IFNAMSIZ - 1; Invalid, Input));
    }
    Ok(())
}

/// `RTM_NEWLINK` for a veth pair: `host` stays in the socket's namespace, `peer` is made in the
/// namespace `ns_fd` names.
pub fn veth_msg(seq: u32, host: &str, peer: &str, ns_fd: i32) -> Vec<u8> {
    let mut m = Msg::new(RTM_NEWLINK, NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE, seq).ifinfo(0, 0, 0);
    m.name(IFLA_IFNAME, host);
    m.open(IFLA_LINKINFO);
    m.name(IFLA_INFO_KIND, "veth");
    m.open(IFLA_INFO_DATA);
    m.open(VETH_INFO_PEER);
    m.buf.extend_from_slice(&[0u8; 16]); // the peer's ifinfomsg
    m.attr(IFLA_NET_NS_FD, &(ns_fd as u32).to_ne_bytes());
    m.name(IFLA_IFNAME, peer);
    m.close();
    m.close();
    m.close();
    m.done()
}

/// `RTM_NEWADDR` of an IPv4 address with its prefix on the interface `index`.
pub fn addr4_msg(seq: u32, index: i32, addr: Ipv4Addr, prefix: u8) -> Vec<u8> {
    let mut m = Msg::new(RTM_NEWADDR, NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE, seq);
    m.buf.extend_from_slice(&[AF_INET, prefix, 0, 0]); // family, prefix, flags, scope universe
    m.buf.extend_from_slice(&index.to_ne_bytes());
    m.attr(IFA_LOCAL, &addr.octets());
    m.attr(IFA_ADDRESS, &addr.octets());
    m.done()
}

/// `RTM_NEWROUTE` of the IPv4 default route through `gw`, in the main table.
pub fn default4_msg(seq: u32, gw: Ipv4Addr) -> Vec<u8> {
    let mut m = Msg::new(RTM_NEWROUTE, NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE, seq);
    // family, dst_len, src_len, tos, table, protocol, scope universe, type; then flags
    m.buf.extend_from_slice(&[AF_INET, 0, 0, 0, RT_TABLE_MAIN, RTPROT_BOOT, 0, RTN_UNICAST]);
    m.buf.extend_from_slice(&0u32.to_ne_bytes());
    m.attr(RTA_GATEWAY, &gw.octets());
    m.done()
}

/// `RTM_NEWLINK` setting `IFF_UP` on the interface `index`.
pub fn up_msg(seq: u32, index: i32) -> Vec<u8> {
    Msg::new(RTM_NEWLINK, NLM_F_ACK, seq).ifinfo(index, IFF_UP, IFF_UP).done()
}

/// `RTM_DELLINK` of the interface called `name`.
pub fn del_msg(seq: u32, name: &str) -> Vec<u8> {
    let mut m = Msg::new(RTM_DELLINK, NLM_F_ACK, seq).ifinfo(0, 0, 0);
    m.name(IFLA_IFNAME, name);
    m.done()
}

/// `RTM_GETLINK` of the interface called `name`.
pub fn get_msg(seq: u32, name: &str) -> Vec<u8> {
    let mut m = Msg::new(RTM_GETLINK, 0, seq).ifinfo(0, 0, 0);
    m.name(IFLA_IFNAME, name);
    m.done()
}

/// An rtnetlink socket.  It speaks for the network namespace the caller was in when it was opened,
/// whatever the caller joins or leaves afterwards, which is how one process configures both ends
/// of a pair it made across two namespaces.
pub struct Rtnl {
    fd:     OwnedFd,
    seq:    u32,
}

/// What the kernel answered one request.
enum Answer {
    Ack,
    Link(i32),      // RTM_NEWLINK's ifi_index, the reply to RTM_GETLINK
    Refused(i32),   // errno
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_ne_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads the first message of a reply.
fn answer(b: &[u8]) -> Outcome<Answer> {
    if b.len() < 16 {
        return Err(err!("The rtnetlink reply was {} bytes, short of a header.", b.len(); Invalid, Input));
    }
    match u16_at(b, 4) {
        NLMSG_ERROR if b.len() >= 20 => {
            let e = u32_at(b, 16) as i32;
            Ok(if e == 0 { Answer::Ack } else { Answer::Refused(-e) })
        },
        RTM_NEWLINK if b.len() >= 32 => Ok(Answer::Link(u32_at(b, 20) as i32)),
        other => Err(err!("The rtnetlink reply was of type {} ({} bytes), not an answer.",
            other, b.len(); Invalid, Input)),
    }
}

impl Rtnl {
    pub fn open() -> Outcome<Self> {
        let fd = res!(socket(AddressFamily::Netlink, SockType::Raw, SockFlag::SOCK_CLOEXEC,
            SockProtocol::NetlinkRoute).map_err(|e| err!(e,
            "The rtnetlink socket could not be opened."; System, Init)));
        res!(bind(fd.as_raw_fd(), &NetlinkAddr::new(0, 0)).map_err(|e| err!(e,
            "The rtnetlink socket could not be bound."; System, Init)));
        Ok(Self { fd, seq: 1 })
    }

    fn next(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn ask(&mut self, msg: &[u8], what: &str) -> Outcome<Answer> {
        res!(send(self.fd.as_raw_fd(), msg, MsgFlags::empty()).map_err(|e| err!(e,
            "rtnetlink: {} could not be sent.", what; System, Write)));
        let mut buf = vec![0u8; 8192]; // fixed: one page-sized reply holds an ack or one link
        loop {
            match recv(self.fd.as_raw_fd(), &mut buf, MsgFlags::empty()) {
                Ok(n)               => return answer(&buf[..n]),
                Err(Errno::EINTR)   => continue,
                Err(e)              => return Err(err!(e,
                    "rtnetlink: no answer to {}.", what; System, Read)),
            }
        }
    }

    fn must(&mut self, msg: &[u8], what: &str) -> Outcome<()> {
        match res!(self.ask(msg, what)) {
            Answer::Ack | Answer::Link(_) => Ok(()),
            Answer::Refused(e) => Err(err!(Errno::from_raw(e),
                "rtnetlink refused {}.", what; System, Unauthorised)),
        }
    }

    /// The index of the interface called `name` in this socket's namespace.
    pub fn index(&mut self, name: &str) -> Outcome<i32> {
        res!(check_name(name));
        let seq = self.next();
        match res!(self.ask(&get_msg(seq, name), &fmt!("finding '{}'", name))) {
            Answer::Link(i)     => Ok(i),
            Answer::Ack         => Err(err!("rtnetlink gave no link for '{}'.", name; Missing, Input)),
            Answer::Refused(e)  => Err(err!(Errno::from_raw(e),
                "rtnetlink found no interface '{}'.", name; Missing, Input)),
        }
    }

    /// Makes the pair `host` (here) and `peer` (in the namespace `ns`).
    pub fn add_veth(&mut self, host: &str, peer: &str, ns: BorrowedFd<'_>) -> Outcome<()> {
        res!(check_name(host));
        res!(check_name(peer));
        let seq = self.next();
        self.must(&veth_msg(seq, host, peer, ns.as_raw_fd()), &fmt!("the veth pair {}/{}", host, peer))
    }

    /// Deletes the interface `name`.  False when there was none.
    pub fn del_link(&mut self, name: &str) -> Outcome<bool> {
        res!(check_name(name));
        let seq = self.next();
        match res!(self.ask(&del_msg(seq, name), &fmt!("deleting '{}'", name))) {
            Answer::Ack | Answer::Link(_)                           => Ok(true),
            Answer::Refused(e) if e == Errno::ENODEV as i32         => Ok(false),
            Answer::Refused(e) => Err(err!(Errno::from_raw(e),
                "rtnetlink refused deleting '{}'.", name; System, Unauthorised)),
        }
    }

    pub fn add_addr4(&mut self, name: &str, addr: Ipv4Addr, prefix: u8) -> Outcome<()> {
        if prefix > 32 {
            return Err(err!("An IPv4 prefix of {} is over 32.", prefix; Invalid, Input));
        }
        let i = res!(self.index(name));
        let seq = self.next();
        self.must(&addr4_msg(seq, i, addr, prefix), &fmt!("{}/{} on '{}'", addr, prefix, name))
    }

    pub fn up(&mut self, name: &str) -> Outcome<()> {
        let i = res!(self.index(name));
        let seq = self.next();
        self.must(&up_msg(seq, i), &fmt!("bringing '{}' up", name))
    }

    pub fn default_route4(&mut self, gw: Ipv4Addr) -> Outcome<()> {
        let seq = self.next();
        self.must(&default4_msg(seq, gw), &fmt!("the default route via {}", gw))
    }
}
