#![cfg(unix)]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_core::file::{
    HeldDir,
    copy_link,
    make_link,
    read_link_bytes,
};

use std::{
    ffi::OsStr,
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::symlink,
    },
    path::PathBuf,
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

/// A directory that removes itself however the test ends.
struct Scratch {
    path: PathBuf, // where it is
}

impl Scratch {
    fn new(what: &str) -> Outcome<Self> {
        let stamp = res!(SystemTime::now().duration_since(UNIX_EPOCH));
        let path = std::env::temp_dir().join(fmt!(
            "fe2o3_link_{}_{}_{}", what, std::process::id(), stamp.as_nanos()));
        res!(fs::create_dir_all(&path));
        Ok(Self { path: res!(fs::canonicalize(&path)) })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

const ODD: &[u8] = b"caf\xe9/target"; // a target that is not UTF-8

/// The folder is renamed away and another put in its place after it was held: the link lands in
/// the held folder, never the one now at its path.
#[cfg(target_os = "linux")]
#[test]
fn test_make_link_lands_in_held_folder() -> Outcome<()> {
    let s = res!(Scratch::new("held"));
    let a = s.path.join("a");
    let moved = s.path.join("a2");
    res!(fs::create_dir(&a));
    let held = res!(HeldDir::open(&a));
    assert_eq!(held.real(), a.as_path());
    res!(fs::rename(&a, &moved));
    res!(fs::create_dir(&a));
    res!(make_link(&held, OsStr::new("l"), ODD));
    assert_eq!(res!(read_link_bytes(&moved.join("l"))), Some(ODD.to_vec()));
    assert!(fs::symlink_metadata(a.join("l")).is_err(), "the link landed in the swapped-in folder");
    Ok(())
}

#[test]
fn test_make_link_refuses_where_something_stands_and_a_path_leaf() -> Outcome<()> {
    let s = res!(Scratch::new("refuse"));
    res!(fs::write(s.path.join("f"), b"x"));
    let held = res!(HeldDir::open(&s.path));
    assert!(make_link(&held, OsStr::new("f"), b"t").is_err(), "made over a file");
    assert_eq!(res!(fs::read(s.path.join("f"))), b"x");
    for leaf in ["x/y", "..", ""] {
        assert!(make_link(&held, OsStr::new(leaf), b"t").is_err(), "made at {:?}", leaf);
    }
    Ok(())
}

#[test]
fn test_read_link_bytes_answers_a_link_only() -> Outcome<()> {
    let s = res!(Scratch::new("read"));
    let f = s.path.join("f");
    res!(fs::write(&f, b"x"));
    let l = s.path.join("l");
    res!(symlink(OsStr::from_bytes(ODD), &l));
    assert_eq!(res!(read_link_bytes(&l)), Some(ODD.to_vec()));
    assert_eq!(res!(read_link_bytes(&f)), None);
    assert_eq!(res!(read_link_bytes(&s.path)), None);
    assert_eq!(res!(read_link_bytes(&s.path.join("none"))), None);
    Ok(())
}

#[test]
fn test_copy_link_copies_the_link_not_its_target() -> Outcome<()> {
    let s = res!(Scratch::new("copy"));
    let f = s.path.join("f");
    res!(fs::write(&f, b"x"));
    let l = s.path.join("l");
    res!(symlink(&f, &l));
    let c = s.path.join("c");
    res!(copy_link(&l, &c));
    assert!(res!(fs::symlink_metadata(&c)).file_type().is_symlink());
    assert_eq!(res!(read_link_bytes(&c)), Some(f.as_os_str().as_bytes().to_vec()));
    assert!(copy_link(&f, &s.path.join("c2")).is_err(), "a file was copied as a link");
    assert!(copy_link(&l, &c).is_err(), "copied over a standing name");
    Ok(())
}
