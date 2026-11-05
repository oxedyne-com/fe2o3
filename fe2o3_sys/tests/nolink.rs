#![cfg(all(target_os = "linux", feature = "nolink"))]

use oxedyne_fe2o3_sys::nolink::{
    open,
    Want,
};

use std::{
    fs,
    os::{
        fd::AsFd,
        unix::fs::symlink,
    },
    path::PathBuf,
};

fn root(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("real/inner")).expect("tree");
    fs::write(d.join("real/inner/f"), b"x").expect("file");
    symlink(d.join("real"), d.join("link")).expect("link");
    symlink(d.join("real/inner/f"), d.join("real/inner/flink")).expect("flink");
    d
}

#[test]
fn test_nolink_refuses_a_link_at_any_component() {
    let d = root("nolink_any");
    assert!(open(None, &d.join("real/inner"), Want::Dir).is_ok());
    assert!(open(None, &d.join("real/inner/f"), Want::File).is_ok());
    assert!(open(None, &d.join("real/inner/f"), Want::Path).is_ok());
    // A symlinked parent: O_NOFOLLOW alone would follow it.
    assert!(fs::File::open(d.join("link/inner/f")).is_ok());
    assert!(open(None, &d.join("link/inner"), Want::Dir).is_err());
    assert!(open(None, &d.join("link/inner/f"), Want::File).is_err());
    assert!(open(None, &d.join("link/inner/f"), Want::Path).is_err());
    // A symlink as the last component.
    assert!(open(None, &d.join("link"), Want::Path).is_err());
    assert!(open(None, &d.join("real/inner/flink"), Want::File).is_err());
    // Relative to a folder already open.
    let at = open(None, &d, Want::Dir).expect("root");
    assert!(open(Some(at.as_fd()), std::path::Path::new("real/inner/f"), Want::File).is_ok());
    assert!(open(Some(at.as_fd()), std::path::Path::new("link/inner/f"), Want::File).is_err());
    // A magic link.
    assert!(open(None, std::path::Path::new("/proc/self/exe"), Want::Path).is_err());
    let _ = fs::remove_dir_all(&d);
}
