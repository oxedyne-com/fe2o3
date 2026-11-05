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
    let made = std::thread::spawn(move || -> Result<(), String> {
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
