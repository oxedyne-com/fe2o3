//! The QA SWT4 arms in d8b04911's API, which `red.sh` copies beside d8b04911's `overlay.rs` to show
//! each red there.  Each asserts what the fix delivers, as far as the old API can say it.

#![cfg(target_os = "linux")]

use oxedyne_fe2o3_file::overlay::{
    changes,
    Kind,
    Op,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

const MAX: usize = 200_000;

fn base(name: &str) -> Outcome<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("overlay_base").join(name);
    if dir.exists() {
        // overlayfs leaves its work directory at mode 0, and some arms leave entries so.
        res!(Command::new("chmod").arg("-R").arg("u+rwx").arg(&dir).status());
        res!(fs::remove_dir_all(&dir));
    }
    for sub in ["L", "U", "W"] {
        res!(fs::create_dir_all(dir.join(sub)));
    }
    Ok(dir)
}

fn staged(name: &str, lower: impl Fn(&Path) -> Outcome<()>, script: &str)
    -> Outcome<(PathBuf, Outcome<Vec<Op>>)>
{
    let dir = res!(base(name));
    res!(lower(&dir.join("L")));
    let out = res!(Command::new("bwrap")
        .args(["--unshare-user", "--dev-bind", "/", "/", "--overlay-src"])
        .arg(dir.join("L"))
        .arg("--overlay").arg(dir.join("U")).arg(dir.join("W")).arg("/mnt")
        .args(["--chdir", "/mnt", "--", "sh", "-ec"])
        .arg(format!("umask 022; {}", script))
        .output());
    if !out.status.success() {
        return Err(err!("bwrap overlay for {} failed: {}", name,
            String::from_utf8_lossy(&out.stderr); Test));
    }
    let r = changes(&dir.join("U"), &dir.join("L"), MAX);
    Ok((dir, r))
}

fn write(path: &Path, text: &str) -> Outcome<()> {
    res!(fs::write(path, text));
    res!(fs::set_permissions(path, fs::Permissions::from_mode(0o644)));
    Ok(())
}

#[test]
fn base_f1_deep() -> Outcome<()> {
    let dir = res!(base("deep"));
    let mut fd = res!(rustix::fs::open(dir.join("U"), rustix::fs::OFlags::DIRECTORY,
        rustix::fs::Mode::empty()));
    for _ in 0..2_000 {
        res!(rustix::fs::mkdirat(&fd, "d", rustix::fs::Mode::from_raw_mode(0o755)));
        fd = res!(rustix::fs::openat(&fd, "d", rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty()));
    }
    let (up, low) = (dir.join("U"), dir.join("L"));
    let walk = res!(std::thread::Builder::new().stack_size(2 << 20)
        .spawn(move || changes(&up, &low, MAX)));
    match walk.join() {
        Ok(r) => { res!(r); Ok(()) }
        Err(_) => Err(err!("The deep walk panicked."; Test)),
    }
}

#[test]
fn base_f2_odd_alone() -> Outcome<()> {
    let (_, r) = res!(staged("odd", |l| write(&l.join("f"), "a\n"), "echo w >> f; mkfifo pipe"));
    match r {
        Ok(_) => Ok(()),
        Err(e) => Err(err!("One odd entry refused the whole walk: {}", e; Test)),
    }
}

#[test]
fn base_f3_xattr_meta() -> Outcome<()> {
    let (_, r) = res!(staged("xattr", |l| write(&l.join("f"), "f\n"),
        "python3 -c \"import os; os.setxattr('f', 'user.tag', b'2')\""));
    let ops = res!(r);
    if ops != vec![Op::Meta(PathBuf::from("f"), Kind::File)] {
        return Err(err!("An attribute-only change gave {:?}.", ops; Test));
    }
    Ok(())
}

#[test]
fn base_f4_user_mark_is_data_under_trusted() -> Outcome<()> {
    // The old reader takes no namespace, so it cannot hold a user attribute as data: it lists
    // Opaque, which under a trusted mount would empty a lower directory the kernel kept.
    let dir = res!(base("ns"));
    for side in ["L/d", "U/d"] {
        res!(fs::create_dir(dir.join(side)));
        res!(fs::set_permissions(dir.join(side), fs::Permissions::from_mode(0o755)));
    }
    res!(write(&dir.join("L/d/x"), "x\n"));
    res!(rustix::fs::setxattr(dir.join("U/d"), "user.overlay.opaque", b"y",
        rustix::fs::XattrFlags::empty()));
    let ops = res!(changes(&dir.join("U"), &dir.join("L"), MAX));
    if ops.iter().any(|op| matches!(op, Op::Opaque(_))) {
        return Err(err!("A user attribute was read as a mark: {:?}.", ops; Test));
    }
    Ok(())
}
