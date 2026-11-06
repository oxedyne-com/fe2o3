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

/// A folder of his and his link swapped (`RENAME_EXCHANGE`) again and again while `own_dir` opens
/// the name: every refusal names a link, including those where the open met the link and the
/// second look met the folder (QA umrig fcA N2: these were a bare ENOTDIR).
#[test]
fn test_own_dir_refusals_under_a_swap_race_name_the_link() {
    use nix::fcntl::{renameat2, RenameFlags, AT_FDCWD};
    use oxedyne_fe2o3_sys::nolink::own_dir;
    use std::{
        os::unix::fs::{MetadataExt, PermissionsExt},
        sync::{atomic::{AtomicBool, Ordering}, Arc},
        time::{Duration, Instant},
    };
    let d = root("nolink_race");
    fs::create_dir(d.join("sw")).expect("sw");
    fs::set_permissions(d.join("sw"), fs::Permissions::from_mode(0o700)).expect("chmod");
    symlink(d.join("real"), d.join("lk")).expect("lk");
    let at = open(None, &d, Want::Dir).expect("at");
    let me = fs::metadata(&d).expect("meta");
    let (uid, gid) = (me.uid(), me.gid());
    let stop = Arc::new(AtomicBool::new(false));
    let (s2, d2) = (stop.clone(), d.clone());
    let swap = std::thread::spawn(move || {
        let mut n = 0u64;
        while !s2.load(Ordering::Relaxed) {
            renameat2(AT_FDCWD, &d2.join("sw"), AT_FDCWD, &d2.join("lk"), RenameFlags::RENAME_EXCHANGE)
                .expect("exchange");
            n += 1;
        }
        n
    });
    // Until the open has met the link with the folder back in place a few times, or a bound.
    let (mut opened, mut links, mut raced, mut bare) = (0, 0, 0, Vec::new());
    let t0 = Instant::now();
    while raced < 5 && t0.elapsed() < Duration::from_secs(20) {
        match own_dir(at.as_fd(), &d, "sw", uid, gid, 0o700) {
            Ok(_)   => opened += 1,
            Err(e)  => {
                let e = format!("{}", e);
                if e.contains("is a link") {
                    links += 1;
                } else if e.contains("was a link when it was opened") {
                    raced += 1;
                } else {
                    raced += 1;
                    if bare.len() < 3 {
                        bare.push(e);
                    }
                }
            },
        }
    }
    stop.store(true, Ordering::Relaxed);
    let swaps = swap.join().expect("swap thread");
    eprintln!("race: {} swaps, {} opened, {} refused as a link, {} raced", swaps, opened, links, raced);
    assert!(raced > 0, "the race was never met: {} swaps, {} opened, {} links", swaps, opened, links);
    assert!(bare.is_empty(), "a refusal under the swap race names no link: {:?}", bare);
    let _ = fs::remove_dir_all(&d);
}

/// A mount on the name stands in for the folder: refused as a mount point, though the mount's
/// root is a folder of the right owner and mode.  Root (`mount`), so run in the rig's VM.
#[test]
#[ignore]
fn test_own_dir_refuses_a_mount_crossing_as_root() {
    use oxedyne_fe2o3_sys::nolink::own_dir;
    use std::process::Command;
    let d = std::env::temp_dir().join(format!("fe2o3_nolink_xdev_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    let at = open(None, &d, Want::Dir).expect("at");
    // The folder itself: made and found as asked.
    own_dir(at.as_fd(), &d, "leaf", 0, 0, 0o700).expect("made");
    own_dir(at.as_fd(), &d, "leaf", 0, 0, 0o700).expect("found");
    let st = Command::new("mount").args(["-t", "tmpfs", "-o", "mode=0700,uid=0,gid=0", "none"])
        .arg(d.join("leaf")).status().expect("mount");
    assert!(st.success(), "mount tmpfs: {}", st);
    let r = own_dir(at.as_fd(), &d, "leaf", 0, 0, 0o700);
    let _ = Command::new("umount").arg(d.join("leaf")).status();
    let e = format!("{}", r.expect_err("a mount on the name was opened"));
    assert!(e.contains("is a mount point") && e.contains(&d.join("leaf").display().to_string()), "{}", e);
    let _ = fs::remove_dir_all(&d);
}
