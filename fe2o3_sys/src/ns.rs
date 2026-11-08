//! A user namespace in which the caller keeps its own uid and gid, a mount namespace of its own,
//! and the mounts such a process may make there: an overlay, so that writes land in an upper
//! folder and the lower one is left as it was, a tmpfs, a `/proc` for a new pid namespace, a bind
//! and a read-only remount of one, and the network namespace's loopback brought up.
//!
//! The kernel lets an unprivileged program make these only where its security module allows. Under
//! AppArmor's `apparmor_restrict_unprivileged_userns`, an unconfined program's namespace is made but
//! its id maps are refused, so [`enter_own`] fails (`EPERM`, or `EACCES` at the
//! `setgroups` write on kernel 7.0); a program a profile grants
//! `userns`, `capability sys_admin` and `mount` succeeds.
//!
//! The system calls come from `nix`, which owns the `unsafe` this crate forbids.

use oxedyne_fe2o3_core::prelude::*;

use nix::mount::{
    mount,
    umount2,
    MntFlags,
    MsFlags,
};
use nix::sys::socket::{
    bind as bind_socket,
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
use nix::sys::statvfs::{
    statvfs,
    FsFlags,
};
use nix::sched::{
    unshare,
    CloneFlags,
};
use nix::unistd::{
    getgid,
    getuid,
};

use std::{
    fs,
    path::Path,
};

/// Moves the caller into a new user namespace and a new mount namespace, keeping its own uid and
/// gid, with every mount private so nothing it mounts reaches the namespace it left.  With `pid`,
/// its next child is the first process of a new pid namespace; with `net`, it has a network
/// namespace of its own, whose loopback it alone reaches.  The caller must have one thread:
/// the kernel refuses a user namespace to a process with more.
pub fn enter_own(pid: bool, net: bool) -> Outcome<()> {
    let uid = getuid();
    let gid = getgid();
    let mut flags = CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNS;
    if pid {
        flags |= CloneFlags::CLONE_NEWPID;
    }
    if net {
        flags |= CloneFlags::CLONE_NEWNET;
    }
    if let Err(e) = unshare(flags) {
        return Err(err!("A user, mount or network namespace could not be made: {}.", e;
            System, Security));
    }
    // fixed: kernel ABI paths and formats.
    for (file, line) in [
        ("/proc/self/setgroups",    fmt!("deny")),
        ("/proc/self/uid_map",      fmt!("{} {} 1", uid, uid)),
        ("/proc/self/gid_map",      fmt!("{} {} 1", gid, gid)),
    ] {
        res!(fs::write(file, line.as_bytes()).map_err(|e| err!(e,
            "The new user namespace's {} could not be written.", file; System, Security)));
    }
    if let Err(e) = mount(None::<&str>, "/", None::<&str>, MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>)
    {
        return Err(err!("The new mount namespace could not be made private: {}.", e;
            System, Security));
    }
    Ok(())
}

/// Mounts an overlay on `at` whose writes land in `upper`, with `work` its work folder on the same
/// file system, and `lower` left unchanged.  No set-user-id bit or device node in it takes effect.
pub fn mount_overlay(lower: &Path, upper: &Path, work: &Path, at: &Path) -> Outcome<()> {
    let mut parts: Vec<&str> = Vec::new();
    for p in [lower, upper, work] {
        let s = res!(p.to_str().ok_or_else(|| err!(
            "The overlay folder {} is not UTF-8.", p.display(); Invalid, Input, Path)));
        // The kernel splits the options on these, and `:` separates lower layers.
        if s.contains([',', ':', '\\']) {
            return Err(err!("The overlay folder {} holds a ',', ':' or '\\', which the kernel \
                would read as a separator.", s; Invalid, Input, Path));
        }
        parts.push(s);
    }
    let opts = fmt!("lowerdir={},upperdir={},workdir={},userxattr", parts[0], parts[1], parts[2]);
    match mount(Some("overlay"), at, Some("overlay"), MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some(opts.as_str()))
    {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!("The overlay on {} could not be mounted ({}): {}.", at.display(), opts, e;
            System, Security)),
    }
}

/// Detaches the mount on `at` from the caller's mount namespace.
pub fn unmount(at: &Path) -> Outcome<()> {
    match umount2(at, MntFlags::MNT_DETACH) {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!("The mount on {} could not be detached: {}.", at.display(), e;
            System, Security)),
    }
}

/// Mounts an empty tmpfs on `at`, in which no set-user-id bit or device node takes effect.
pub fn mount_tmpfs(at: &Path) -> Outcome<()> {
    mount_as("tmpfs", at, MsFlags::MS_NOSUID | MsFlags::MS_NODEV)
}

/// Mounts a `/proc` on `at` for the caller's pid namespace, which must be one the caller's user
/// namespace owns: the first child after [`enter_own`] with `pid`, or one it starts.
pub fn mount_proc(at: &Path) -> Outcome<()> {
    mount_as("proc", at, MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC)
}

fn mount_as(fstype: &str, at: &Path, flags: MsFlags) -> Outcome<()> {
    match mount(Some(fstype), at, Some(fstype), flags, None::<&str>) {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!("A {} could not be mounted on {}: {}.", fstype, at.display(), e;
            System, Security)),
    }
}

/// Binds the folder `from` on `at`, without the mounts below `from`.
pub fn bind(from: &Path, at: &Path) -> Outcome<()> {
    match mount(Some(from), at, None::<&str>, MsFlags::MS_BIND, None::<&str>) {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!("{} could not be bound on {}: {}.", from.display(), at.display(), e;
            System, Security)),
    }
}

/// Makes the mount on `at`, a bind, read-only.  Its other flags are kept, since a namespace may
/// not clear those of a mount it was handed.
pub fn remount_ro(at: &Path) -> Outcome<()> {
    let have = match statvfs(at) {
        Ok(s)   => s.flags(),
        Err(e)  => return Err(err!("The mount on {} could not be read: {}.", at.display(), e;
            System, IO)),
    };
    let mut flags = MsFlags::MS_REMOUNT | MsFlags::MS_BIND | MsFlags::MS_RDONLY;
    for (f, m) in [
        (FsFlags::ST_NOSUID,        MsFlags::MS_NOSUID),
        (FsFlags::ST_NODEV,         MsFlags::MS_NODEV),
        (FsFlags::ST_NOEXEC,        MsFlags::MS_NOEXEC),
        (FsFlags::ST_NOATIME,       MsFlags::MS_NOATIME),
        (FsFlags::ST_NODIRATIME,    MsFlags::MS_NODIRATIME),
        (FsFlags::ST_RELATIME,      MsFlags::MS_RELATIME),
    ] {
        if have.contains(f) {
            flags |= m;
        }
    }
    match mount(None::<&str>, at, None::<&str>, flags, None::<&str>) {
        Ok(())  => Ok(()),
        Err(e)  => Err(err!("The mount on {} could not be made read-only: {}.", at.display(), e;
            System, Security)),
    }
}

// fixed: rtnetlink ABI (linux/netlink.h, linux/rtnetlink.h, linux/if.h).
const RTM_NEWLINK:      u16 = 16;
const NLMSG_ERROR:      u16 = 2;
const NLM_F_REQUEST:    u16 = 1;
const NLM_F_ACK:        u16 = 4;
const IFF_UP:           u32 = 1;
const LOOPBACK_INDEX:   i32 = 1; // a new network namespace's first link
const NLMSG_HDR_LEN:    usize = 16;
const IFINFO_LEN:       usize = 16;

/// Brings up the loopback of the caller's network namespace, over rtnetlink, as `ip link set lo
/// up` does.  The caller needs `CAP_NET_ADMIN` in the user namespace that owns it.
pub fn loopback_up() -> Outcome<()> {
    let fd = match socket(AddressFamily::Netlink, SockType::Raw, SockFlag::SOCK_CLOEXEC,
        SockProtocol::NetlinkRoute)
    {
        Ok(f)   => f,
        Err(e)  => return Err(err!("A route netlink socket could not be opened: {}.", e;
            System, Network)),
    };
    if let Err(e) = bind_socket(std::os::fd::AsRawFd::as_raw_fd(&fd), &NetlinkAddr::new(0, 0)) {
        return Err(err!("The route netlink socket could not be bound: {}.", e; System, Network));
    }
    let len = NLMSG_HDR_LEN + IFINFO_LEN;
    let mut msg = Vec::with_capacity(len);
    msg.extend_from_slice(&(len as u32).to_ne_bytes());
    msg.extend_from_slice(&RTM_NEWLINK.to_ne_bytes());
    msg.extend_from_slice(&(NLM_F_REQUEST | NLM_F_ACK).to_ne_bytes());
    msg.extend_from_slice(&1u32.to_ne_bytes()); // sequence
    msg.extend_from_slice(&0u32.to_ne_bytes()); // port: the kernel's
    msg.extend_from_slice(&[0u8, 0u8]);         // AF_UNSPEC, padding
    msg.extend_from_slice(&0u16.to_ne_bytes()); // device type
    msg.extend_from_slice(&LOOPBACK_INDEX.to_ne_bytes());
    msg.extend_from_slice(&IFF_UP.to_ne_bytes());
    msg.extend_from_slice(&IFF_UP.to_ne_bytes()); // change mask
    let raw = std::os::fd::AsRawFd::as_raw_fd(&fd);
    if let Err(e) = send(raw, &msg, MsgFlags::empty()) {
        return Err(err!("The loopback's link-up request could not be sent: {}.", e; System, Network));
    }
    let mut buf = [0u8; 512]; // fixed: an error ack is a header, an errno and the request echoed
    let n = match recv(raw, &mut buf, MsgFlags::empty()) {
        Ok(n)   => n,
        Err(e)  => return Err(err!("The loopback's link-up ack could not be read: {}.", e;
            System, Network)),
    };
    if n < NLMSG_HDR_LEN + 4 || u16::from_ne_bytes([buf[4], buf[5]]) != NLMSG_ERROR {
        return Err(err!("The kernel's answer to the loopback's link-up is not an ack ({} bytes).", n;
            System, Network, Unexpected));
    }
    let code = i32::from_ne_bytes([buf[16], buf[17], buf[18], buf[19]]);
    if code != 0 {
        return Err(err!("The kernel refused to bring the loopback up: {}.",
            nix::errno::Errno::from_raw(-code); System, Network));
    }
    Ok(())
}
