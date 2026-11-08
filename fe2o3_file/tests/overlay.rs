//! Real uppers, written by an overlay mounted by `bwrap` in an unprivileged user namespace, give
//! exact op lists; hand-made hostile uppers are refused.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

#![cfg(target_os = "linux")]

use oxedyne_fe2o3_file::overlay::{
    changes,
    Kind,
    Op,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{
            symlink,
            PermissionsExt,
        },
    },
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

const MAX: usize = 200_000; // The cap Kimberlite configures.

fn base(name: &str) -> Outcome<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("overlay").join(name);
    if dir.exists() {
        // overlayfs leaves its work directory at mode 0, which an earlier run's removal trips on.
        let work = dir.join("W/work");
        if work.exists() {
            res!(fs::set_permissions(&work, fs::Permissions::from_mode(0o700)));
        }
        res!(fs::remove_dir_all(&dir));
    }
    for sub in ["L", "U", "W"] {
        res!(fs::create_dir_all(dir.join(sub)));
    }
    Ok(dir)
}

/// Runs `script` in an overlay of `L` with upper `U`, so `U` is what overlayfs itself wrote.
fn staged(name: &str, lower: impl Fn(&Path) -> Outcome<()>, script: &str) -> Outcome<Vec<Op>> {
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
    changes(&dir.join("U"), &dir.join("L"), MAX)
}

fn write(path: &Path, text: &str) -> Outcome<()> {
    res!(fs::write(path, text));
    res!(fs::set_permissions(path, fs::Permissions::from_mode(0o644)));
    Ok(())
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn refused(r: Outcome<Vec<Op>>, says: &str) -> Outcome<()> {
    match r {
        Ok(ops) => Err(err!("Expected a refusal saying {:?}, got ops {:?}.", says, ops; Test)),
        Err(e) if format!("{}", e).contains(says) => Ok(()),
        Err(e) => Err(err!("Refused, but not saying {:?}: {}", says, e; Test)),
    }
}

fn expect(got: Vec<Op>, want: Vec<Op>) -> Outcome<()> {
    if got != want {
        return Err(err!("Op list\n got: {:?}\nwant: {:?}", got, want; Test));
    }
    Ok(())
}

// Real uppers.

#[test]
fn overlay_create() -> Outcome<()> {
    let ops = res!(staged("create", |_| Ok(()),
        "echo n > n; mkdir d; echo x > d/x; ln -s d l"));
    expect(ops, vec![
        Op::Create(p("d"),      Kind::Dir),
        Op::Create(p("d/x"),    Kind::File),
        Op::Create(p("l"),      Kind::Link),
        Op::Create(p("n"),      Kind::File),
    ])
}

#[test]
fn overlay_write() -> Outcome<()> {
    let ops = res!(staged("write", |l| {
        res!(write(&l.join("f"), "a\n"));
        res!(write(&l.join("same"), "s\n"));
        res!(symlink("f", l.join("lnk")));
        Ok(())
    }, "echo w >> f; cat same > /dev/null; ln -sfn same lnk"));
    expect(ops, vec![
        Op::Write(p("f"),       Kind::File),
        Op::Write(p("lnk"),     Kind::Link),
    ])
}

#[test]
fn overlay_delete() -> Outcome<()> {
    let ops = res!(staged("delete", |l| {
        res!(write(&l.join("g"), "g\n"));
        res!(fs::create_dir(l.join("h")));
        res!(write(&l.join("h/i"), "i\n"));
        res!(write(&l.join("k"), "k\n"));
        Ok(())
    }, "rm g; rm -r h; echo t > t; rm t"));
    expect(ops, vec![
        Op::Delete(p("g")),
        Op::Delete(p("h")),
    ])
}

#[test]
fn overlay_opaque() -> Outcome<()> {
    let ops = res!(staged("opaque", |l| {
        res!(fs::create_dir(l.join("e")));
        res!(fs::set_permissions(l.join("e"), fs::Permissions::from_mode(0o755)));
        res!(write(&l.join("e/y"), "y\n"));
        res!(write(&l.join("p"), "p\n"));
        Ok(())
    }, "rm -r e; mkdir e; echo q > e/q; rm p; mkdir p"));
    expect(ops, vec![
        Op::Opaque(p("e")),
        Op::Create(p("e/q"),    Kind::File),
        Op::Delete(p("p")),
        Op::Create(p("p"),      Kind::Dir),
    ])
}

#[test]
fn overlay_rename_as_copy() -> Outcome<()> {
    let ops = res!(staged("rename", |l| {
        res!(fs::create_dir(l.join("d")));
        res!(write(&l.join("d/x"), "x\n"));
        res!(write(&l.join("f2"), "f\n"));
        Ok(())
    }, "grep -q redirect_dir=nofollow /proc/self/mountinfo; mv f2 f3; mv d d2"));
    expect(ops, vec![
        Op::Delete(p("d")),
        Op::Create(p("d2"),     Kind::Dir),
        Op::Create(p("d2/x"),   Kind::File),
        Op::Delete(p("f2")),
        Op::Create(p("f3"),     Kind::File),
    ])
}

#[test]
fn overlay_meta() -> Outcome<()> {
    let ops = res!(staged("meta", |l| {
        res!(write(&l.join("m"), "m\n"));
        res!(write(&l.join("t"), "t\n"));
        res!(fs::create_dir(l.join("dm")));
        res!(fs::set_permissions(l.join("dm"), fs::Permissions::from_mode(0o755)));
        Ok(())
    }, "chmod 600 m; touch -d @0 t; chmod 700 dm"));
    expect(ops, vec![
        Op::Meta(p("dm"),       Kind::Dir),
        Op::Meta(p("m"),        Kind::File),
        Op::Meta(p("t"),        Kind::File),
    ])
}

// Hostile uppers, made by hand.

#[test]
fn overlay_refuses_symlinked_parent() -> Outcome<()> {
    let dir = res!(base("symparent"));
    let outside = dir.join("outside");
    res!(fs::create_dir(&outside));
    res!(write(&outside.join("secret"), "s\n"));
    res!(symlink(&outside, dir.join("L/link")));
    res!(fs::create_dir(dir.join("U/link")));
    res!(write(&dir.join("U/link/evil"), "e\n"));
    refused(changes(&dir.join("U"), &dir.join("L"), MAX), "symlinked parent")
}

#[test]
fn overlay_refuses_symlinked_root() -> Outcome<()> {
    let dir = res!(base("symroot"));
    res!(symlink(dir.join("U"), dir.join("UL")));
    res!(refused(changes(&dir.join("UL"), &dir.join("L"), MAX), "through no symbolic link"));
    res!(symlink(dir.join("L"), dir.join("LL")));
    refused(changes(&dir.join("U"), &dir.join("LL"), MAX), "through no symbolic link")
}

#[test]
fn overlay_upper_link_not_followed() -> Outcome<()> {
    let dir = res!(base("uplink"));
    let outside = dir.join("outside");
    res!(fs::create_dir(&outside));
    res!(write(&outside.join("secret"), "s\n"));
    res!(symlink(&outside, dir.join("U/out")));
    expect(res!(changes(&dir.join("U"), &dir.join("L"), MAX)), vec![
        Op::Create(p("out"),    Kind::Link),
    ])
}

#[test]
fn overlay_refuses_too_many_entries() -> Outcome<()> {
    let dir = res!(base("many"));
    let up = dir.join("U");
    for i in 0..MAX {
        res!(fs::File::create(up.join(format!("{:06}", i))));
    }
    let at_cap = changes(&up, &dir.join("L"), MAX);
    res!(fs::File::create(up.join("one-more")));
    let over = changes(&up, &dir.join("L"), MAX);
    res!(fs::remove_dir_all(&dir));
    match at_cap {
        Ok(ops) if ops.len() == MAX => {}
        other => return Err(err!("{} entries should pass at the cap: {:?}",
            MAX, other.map(|o| o.len()); Test)),
    }
    refused(over, "more than 200000 entries")
}

#[test]
fn overlay_refuses_odd_names() -> Outcome<()> {
    let dir = res!(base("oddnl"));
    res!(write(&dir.join("U/a\nb"), "x"));
    res!(refused(changes(&dir.join("U"), &dir.join("L"), MAX), "control character"));
    let dir = res!(base("oddutf"));
    let name = std::ffi::OsStr::from_bytes(b"bad\xff");
    res!(write(&dir.join("U").join(name), "x"));
    refused(changes(&dir.join("U"), &dir.join("L"), MAX), "not UTF-8")
}

#[test]
fn overlay_refuses_odd_kinds_and_marks() -> Outcome<()> {
    let dir = res!(base("fifo"));
    res!(rustix::fs::mknodat(rustix::fs::CWD, dir.join("U/pipe"), rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644), 0));
    res!(refused(changes(&dir.join("U"), &dir.join("L"), MAX), "Fifo"));

    let dir = res!(base("redirect"));
    res!(fs::create_dir(dir.join("U/d")));
    res!(rustix::fs::setxattr(dir.join("U/d"), "user.overlay.redirect", b"/x",
        rustix::fs::XattrFlags::empty()));
    res!(refused(changes(&dir.join("U"), &dir.join("L"), MAX), "user.overlay.redirect"));

    let dir = res!(base("metacopy"));
    res!(write(&dir.join("L/f"), "f"));
    res!(write(&dir.join("U/f"), ""));
    res!(rustix::fs::setxattr(dir.join("U/f"), "user.overlay.metacopy", b"",
        rustix::fs::XattrFlags::empty()));
    res!(refused(changes(&dir.join("U"), &dir.join("L"), MAX), "user.overlay.metacopy"));

    let dir = res!(base("opaquex"));
    res!(fs::create_dir(dir.join("U/d")));
    res!(rustix::fs::setxattr(dir.join("U/d"), "user.overlay.opaque", b"x",
        rustix::fs::XattrFlags::empty()));
    refused(changes(&dir.join("U"), &dir.join("L"), MAX), "opaque mark \"x\"")
}
