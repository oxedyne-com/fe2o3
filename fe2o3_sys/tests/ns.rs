//! The `ns` calls outside a namespace of the caller's own: the kernel's refusal comes back as an
//! error naming it, so the request reached the kernel's permission check whole.
#![cfg(all(target_os = "linux", feature = "ns"))]

use oxedyne_fe2o3_sys::ns;

use std::path::Path;

fn as_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map(|m| m.uid() == 0).unwrap_or(true)
}

#[test]
fn test_loopback_up_without_net_admin_is_refused_by_the_kernel() {
    if as_root() {
        return;
    }
    match ns::loopback_up() {
        Ok(())  => panic!("the host's loopback was changed without CAP_NET_ADMIN"),
        Err(e)  => {
            let m = e.to_string();
            assert!(m.contains("EPERM"), "not the kernel's EPERM ack: {}", m);
        },
    }
}

#[test]
fn test_mounts_outside_a_namespace_are_refused() {
    if as_root() {
        return;
    }
    let at = Path::new(env!("CARGO_TARGET_TMPDIR"));
    for (what, r) in [
        ("tmpfs",       ns::mount_tmpfs(at)),
        ("bind",        ns::bind(at, at)),
        ("remount_ro",  ns::remount_ro(at)),
    ] {
        assert!(r.is_err(), "{} was made outside a namespace", what);
    }
}

#[test]
fn test_hand_down_without_the_capabilities_is_refused() {
    if as_root() {
        return;
    }
    assert!(ns::hand_down().is_err(), "capabilities the caller does not hold were handed down");
}

#[test]
fn test_drop_capabilities_leaves_none() {
    if let Err(e) = ns::drop_capabilities() {
        panic!("{}", e);
    }
    // fixed: kernel ABI path and fields; capabilities are per thread.
    let status = std::fs::read_to_string("/proc/thread-self/status").unwrap_or_default();
    for key in ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"] {
        let line = status.lines().find(|l| l.starts_with(key)).unwrap_or("");
        assert!(line.ends_with("0000000000000000"), "{} not empty: {}", key, line);
    }
}
