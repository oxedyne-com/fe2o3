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

#[test]
fn test_own_dir_makes_judges_and_refuses_by_name() {
    use oxedyne_fe2o3_sys::nolink::own_dir;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let d = root("nolink_own");
    let at = open(None, &d, Want::Dir).expect("at");
    let me = fs::metadata(&d).expect("meta");
    let (uid, gid) = (me.uid(), me.gid());
    // Made: exactly the mode asked, whatever the umask.
    own_dir(at.as_fd(), &d, "made", uid, gid, 0o700).expect("made");
    assert_eq!(0o700, fs::metadata(d.join("made")).expect("m").mode() & 0o7777);
    // Found as asked: judged, not changed.
    own_dir(at.as_fd(), &d, "made", uid, gid, 0o700).expect("again");
    // Found with another mode: refused by name, and left as it was.
    fs::create_dir(d.join("wide")).expect("wide");
    fs::set_permissions(d.join("wide"), fs::Permissions::from_mode(0o755)).expect("chmod");
    let e = format!("{}", own_dir(at.as_fd(), &d, "wide", uid, gid, 0o700).expect_err("wide"));
    assert!(e.contains("wide") && e.contains("mode 755") && e.contains(&format!("uid {}", uid)), "{}", e);
    assert_eq!(0o755, fs::metadata(d.join("wide")).expect("w").mode() & 0o7777);
    // Another uid expected: refused, naming the owner found and the uid expected.
    let e = format!("{}", own_dir(at.as_fd(), &d, "made", uid + 1, gid, 0o700).expect_err("uid"));
    assert!(e.contains(&format!("owned by uid {}", uid)) && e.contains(&format!("uid {}, mode 700", uid + 1)), "{}", e);
    // A link at the name: never followed, refused as a link; its target untouched.
    fs::set_permissions(d.join("real"), fs::Permissions::from_mode(0o755)).expect("chmod");
    let e = format!("{}", own_dir(at.as_fd(), &d, "link", uid, gid, 0o700).expect_err("link"));
    assert!(e.contains("is a link"), "{}", e);
    assert_eq!(0o755, fs::metadata(d.join("real")).expect("r").mode() & 0o7777);
    // A regular file at the name: refused as one.
    fs::write(d.join("file"), b"x").expect("file");
    let e = format!("{}", own_dir(at.as_fd(), &d, "file", uid, gid, 0o700).expect_err("file"));
    assert!(e.contains("is a regular file"), "{}", e);
    assert!(own_dir(at.as_fd(), &d, "a/b", uid, gid, 0o700).is_err());
}

#[test]
fn test_held_dir_judges_the_folder_and_every_folder_above() {
    use oxedyne_fe2o3_sys::nolink::held_dir;
    assert!(held_dir(std::path::Path::new("/"), 0, 0o755).is_ok());
    let e = format!("{}", held_dir(std::path::Path::new("/"), 0, 0o700).expect_err("mode"));
    assert!(e.contains("mode 755") && e.contains("uid 0, mode 700"), "{}", e);
    // A folder in a folder of a user's own: someone other than root could rename it away.
    let d = root("nolink_held");
    let e = format!("{}", held_dir(&d.join("real"), 0, 0o755).expect_err("above"));
    assert!(e.contains("sits in"), "{}", e);
}
