//! The rtnetlink writer's messages against `ip`'s own (iproute2, captured with
//! `strace -e sendmsg -xx` on 06 Nov 2026), and one pair made for real as root.
#![cfg(all(target_os = "linux", feature = "confine", target_endian = "little"))]

use oxedyne_fe2o3_sys::confine::{
    addr4_msg,
    default4_msg,
    forwarding4_msg,
    up_msg,
    veth_msg,
};

use std::net::Ipv4Addr;

// fixed: the sequence number `ip` sent in the capture.
const SEQ: u32 = 1793897316;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn head(len: u8, kind: u8, flags: &str) -> String {
    let seq = SEQ.to_le_bytes();
    format!("{:02x} 00 00 00 {:02x} 00 {} {:02x}{:02x}{:02x}{:02x} 00000000",
        len, kind, flags, seq[0], seq[1], seq[2], seq[3])
}

#[test]
fn test_confine_veth_msg_is_ips() {
    // ip link add kh0 type veth peer name kp0 netns /proc/self/ns/net  (the netns fd was 4)
    let want = hex(&format!("{} {}",
        head(96, 16, "0506"),
        "00000000 00000000 00000000 00000000
         0800 0300 6b683000
         3800 1200
           0900 0100 76657468 00000000
           2800 0200
             2400 0100 00000000 00000000 00000000 00000000
               0800 1c00 04000000
               0800 0300 6b703000"));
    assert_eq!(veth_msg(SEQ, "kh0", "kp0", 4), want);
}

#[test]
fn test_confine_addr4_msg_is_ips() {
    // ip addr add 10.77.0.1/30 dev lo
    let want = hex(&format!("{} {}", head(40, 20, "0506"),
        "021e0000 01000000 0800 0200 0a4d0001 0800 0100 0a4d0001"));
    assert_eq!(addr4_msg(SEQ, 1, Ipv4Addr::new(10, 77, 0, 1), 30), want);
}

#[test]
fn test_confine_default4_msg_is_ips() {
    // ip route add default via 10.77.0.1
    let want = hex(&format!("{} {}", head(36, 24, "0506"),
        "02000000 fe030001 00000000 0800 0500 0a4d0001"));
    assert_eq!(default4_msg(SEQ, Ipv4Addr::new(10, 77, 0, 1)), want);
}

#[test]
fn test_confine_up_msg_is_ips() {
    // ip link set lo up
    let want = hex(&format!("{} {}", head(32, 16, "0500"),
        "00000000 01000000 01000000 01000000"));
    assert_eq!(up_msg(SEQ, 1), want);
}

/// No `ip` verb sets it, so this is the layout of linux/if_link.h's `IFLA_AF_SPEC` nest by hand:
/// `AF_INET`, then `IFLA_INET_CONF`, then `IPV4_DEVCONF_FORWARDING` as a u32.
#[test]
fn test_confine_forwarding4_msg_layout() {
    let want = hex(&format!("{} {} {}", head(52, 16, "0500"),
        "00000000 07000000 00000000 00000000",
        "1400 1a00 1000 0200 0c00 0100 0800 0100 01000000"));
    assert_eq!(forwarding4_msg(SEQ, 7, true), want);
}

/// Root only (the VM arm): a pair across two namespaces, addressed, up and routed, then deleted.
#[test]
#[ignore]
fn test_confine_veth_pair_made_as_root() {
    use oxedyne_fe2o3_sys::confine::{unshare, Ns, Rtnl};
    use std::os::fd::AsFd;
    let mut host = match Rtnl::open() {
        Ok(h)   => h,
        Err(e)  => panic!("the host socket: {}", e),
    };
    let _ = host.del_link("fe2o3t0");
    let made = std::thread::spawn(move || -> Result<Rtnl, String> {
        unshare(&[Ns::Net]).map_err(|e| format!("{}", e))?;
        let ns = std::fs::File::open("/proc/thread-self/ns/net").map_err(|e| format!("{}", e))?;
        let mut inner = Rtnl::open().map_err(|e| format!("{}", e))?;
        host.add_veth("fe2o3t0", "fe2o3t1", ns.as_fd()).map_err(|e| format!("{}", e))?;
        host.add_addr4("fe2o3t0", Ipv4Addr::new(10, 251, 0, 1), 30).map_err(|e| format!("{}", e))?;
        host.up("fe2o3t0").map_err(|e| format!("{}", e))?;
        inner.up("lo").map_err(|e| format!("{}", e))?;
        inner.add_addr4("fe2o3t1", Ipv4Addr::new(10, 251, 0, 2), 30).map_err(|e| format!("{}", e))?;
        inner.up("fe2o3t1").map_err(|e| format!("{}", e))?;
        inner.default_route4(Ipv4Addr::new(10, 251, 0, 1)).map_err(|e| format!("{}", e))?;
        // From inside the new namespace: /proc/sys/net there is this namespace's, so only
        // rtnetlink on the host's socket reaches the host end's flag.
        host.forwarding4("fe2o3t0", true).map_err(|e| format!("{}", e))?;
        if host.index("fe2o3t1").is_ok() {
            return Err("the peer is visible in the host namespace".into());
        }
        Ok(host)
    }).join();
    let glob = std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward").unwrap_or_default();
    let mut host = match made {
        Ok(Ok(h))   => h,
        Ok(Err(e))  => panic!("{}", e),
        Err(_)      => panic!("the namespace thread panicked"),
    };
    let fwd = std::fs::read_to_string("/proc/sys/net/ipv4/conf/fe2o3t0/forwarding").unwrap_or_default();
    let gone = host.del_link("fe2o3t0");
    assert_eq!(fwd.trim(), "1", "the host end forwards");
    assert_eq!(std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward").unwrap_or_default(), glob,
        "the global ip_forward is untouched");
    assert!(matches!(gone, Ok(true)), "the host end was there to delete: {:?}", gone.map_err(|e| format!("{}", e)));
}

/// The floor steps (KIMB-USERMGR UM3b), in a mount namespace of a thread of its own: a folder
/// and a file hidden, a folder made read-only, and a link anywhere in a path left alone, so a
/// `~/.bashrc -> /etc/hostname` never puts root's mount on /etc/hostname.  Run with TMPDIR on
/// a real filesystem (his home in the VM).
#[test]
#[ignore]
fn test_confine_floor_steps_by_open_file_as_root() {
    use oxedyne_fe2o3_sys::confine::{apply_noting, unshare, Ns, Skip, Step};
    use std::{fs, os::unix::fs::symlink, path::PathBuf};
    let d = std::env::temp_dir().join(format!("fe2o3_floor_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("cfg")).expect("cfg");
    fs::write(d.join("cfg/unit"), b"u").expect("unit");
    fs::write(d.join("bashrc"), b"b").expect("bashrc");
    fs::create_dir_all(d.join("ro/sub")).expect("ro");
    fs::write(d.join("ro/sub/f"), b"r").expect("ro file");
    fs::write(d.join("blank"), b"").expect("blank");
    symlink("/etc/hostname", d.join("link")).expect("link");
    symlink(&d, d.join("via")).expect("via");
    let mounts = || fs::read_to_string("/proc/thread-self/mountinfo").unwrap_or_default();
    let on_hostname = |m: &str| m.lines().filter(|l| l.split(' ').nth(4) == Some("/etc/hostname")).count();
    let host_before = on_hostname(&mounts());
    let dd = d.clone();
    let r = std::thread::spawn(move || -> Result<(), String> {
        let d = dd;
        unshare(&[Ns::Mount]).map_err(|e| format!("{}", e))?;
        let blank = d.join("blank");
        let h = |p: &str| Step::Hide { at: d.join(p), blank: blank.clone() };
        let steps = vec![
            Step::Slave { at: PathBuf::from("/") },
            h("cfg"), h("bashrc"), Step::ReadOnly { at: d.join("ro") },
            h("link"), h("absent"), h("via/bashrc"), Step::ReadOnly { at: d.join("via/ro") },
        ];
        let skipped = apply_noting(&steps).map_err(|e| format!("{}", e))?;
        let want = vec![(d.join("link"), Skip::Linked), (d.join("absent"), Skip::Absent),
            (d.join("via/bashrc"), Skip::Linked), (d.join("via/ro"), Skip::Linked)];
        if skipped != want { return Err(format!("skipped {:?}", skipped)); }
        if fs::read_dir(d.join("cfg")).map(|r| r.count()).unwrap_or(9) != 0 { return Err("cfg not hidden".into()); }
        if fs::write(d.join("cfg/new"), b"x").is_ok() { return Err("cfg writable".into()); }
        if fs::read(d.join("bashrc")).map_err(|e| format!("{}", e))? != b"" { return Err("bashrc not hidden".into()); }
        if fs::write(d.join("bashrc"), b"x").is_ok() { return Err("bashrc writable".into()); }
        if fs::read(d.join("ro/sub/f")).map_err(|e| format!("{}", e))? != b"r" { return Err("ro unreadable".into()); }
        if fs::write(d.join("ro/sub/f"), b"x").is_ok() { return Err("ro writable".into()); }
        if fs::create_dir(d.join("ro/new")).is_ok() { return Err("ro takes a folder".into()); }
        if on_hostname(&mounts()) != host_before { return Err("a mount landed on /etc/hostname".into()); }
        Ok(())
    }).join();
    let host_ok = fs::read(d.join("bashrc")).ok() == Some(b"b".to_vec()) && fs::write(d.join("cfg/host"), b"h").is_ok();
    let _ = fs::remove_dir_all(&d);
    match r {
        Ok(Ok(()))  => (),
        Ok(Err(e))  => panic!("{}", e),
        Err(_)      => panic!("the namespace thread panicked"),
    }
    assert!(host_ok, "the host's view is untouched");
}
