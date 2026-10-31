//! Tests for `fe2o3_sys::rename` against a real filesystem, the build directory's own.
//!
//! The scratch directories are made under the target directory rather than `/tmp`, which may be a
//! tmpfs; both are filesystems that exchange, so a refusal there is a failure, not a skip.

#![cfg(all(target_os = "linux", feature = "rename"))]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::rename::{
    exchange,
    unsupported,
};

use std::fs;
use std::path::PathBuf;


fn scratch(name: &str) -> Outcome<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    if dir.exists() {
        res!(fs::remove_dir_all(&dir));
    }
    res!(fs::create_dir_all(&dir));
    Ok(dir)
}

/// **Two directories swap in one step, contents and all,** and a swap back restores them.
#[test]
fn two_directories_swap_whole() -> Outcome<()> {
    let dir = res!(scratch("rename_dirs"));
    let (a, b) = (dir.join("log"), dir.join("log.new"));
    res!(fs::create_dir(&a));
    res!(fs::create_dir(&b));
    res!(fs::write(a.join("000000.seg"), b"old"));
    res!(fs::write(b.join("000000.seg"), b"new"));
    res!(fs::write(b.join("000001.seg"), b"more"));
    res!(exchange(&a, &b));
    assert_eq!(res!(fs::read(a.join("000000.seg"))), b"new");
    assert!(a.join("000001.seg").is_file(), "the new directory's second file did not come with it");
    assert_eq!(res!(fs::read(b.join("000000.seg"))), b"old");
    assert!(!b.join("000001.seg").exists(), "the old directory gained a file");
    res!(exchange(&a, &b));
    assert_eq!(res!(fs::read(a.join("000000.seg"))), b"old");
    Ok(())
}

/// **A file and a directory swap as well,** since the call does not ask what either is.
#[test]
fn a_file_and_a_directory_swap() -> Outcome<()> {
    let dir = res!(scratch("rename_kinds"));
    let (a, b) = (dir.join("a"), dir.join("b"));
    res!(fs::write(&a, b"file"));
    res!(fs::create_dir(&b));
    res!(exchange(&a, &b));
    assert!(a.is_dir() && b.is_file(), "the two did not swap");
    Ok(())
}

/// **A missing path is refused, changes nothing, and is not mistaken for a filesystem that cannot
/// exchange,** so a caller does not take its fallback for a mistake of its own.
#[test]
fn a_missing_path_is_refused_and_is_not_unsupported() -> Outcome<()> {
    let dir = res!(scratch("rename_missing"));
    let a = dir.join("a");
    res!(fs::create_dir(&a));
    let got = exchange(&a, &dir.join("absent"));
    match got {
        Ok(())  => return Err(err!("a missing path was exchanged"; Test, Unexpected)),
        Err(e)  => assert!(!unsupported(&e), "a missing path read as unsupported: {}", e),
    }
    assert!(a.is_dir(), "the refused exchange moved the path that existed");
    Ok(())
}
