//! Tests for `fe2o3_sys::notify`: the host-path resolver on given mount tables and the live one,
//! the Follower against real renames, saves, unlinks and a real queue overflow, and the fanotify
//! group.
//!
//! The fanotify arms need `CAP_SYS_ADMIN`. They are ignored here and run as root in the VM (BR-4e
//! S6) with `cargo test --features notify -- --ignored`.

#![cfg(all(target_os = "linux", feature = "notify"))]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::notify::{
    fan::{
        FanEvent,
        FanGroup,
        Writer,
    },
    follow::{
        Change,
        Followed,
        FollowId,
        Follower,
        Limit,
    },
    mounts::{
        mount_id,
        resolve,
        unescape,
        FsClass,
        HostPath,
        MountTable,
        Unresolved,
    },
};

use nix::poll::{
    poll,
    PollFd,
    PollFlags,
    PollTimeout,
};

use std::{
    env,
    fs::{
        self,
        File,
    },
    os::fd::AsFd,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{
        Duration,
        Instant,
    },
};

// ── Resolver ──

const OWN: &str = "\
22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw
30 22 8:1 /home/jason/x /mnt/b rw,relatime shared:1 - ext4 /dev/sda1 rw
31 22 8:2 /data /srv rw,relatime - ext4 /dev/sdb1 rw
40 22 0:5 / /dev rw - devtmpfs udev rw
";

const WRITER: &str = "\
500 400 8:1 /home/jason/x /x rw,relatime - ext4 /dev/sda1 rw
501 400 8:2 /data/proj /p rw - ext4 /dev/sdb1 rw
502 400 8:9 / /other rw - ext4 /dev/sdz rw
";

fn tables() -> Outcome<(MountTable, MountTable)> {
    Ok((res!(MountTable::parse(OWN)), res!(MountTable::parse(WRITER))))
}

fn expect(got: HostPath, want: HostPath) -> Outcome<()> {
    if got != want {
        return Err(err!("Resolved to {:?}, wanted {:?}.", got, want; Test));
    }
    Ok(())
}

#[test]
fn a_file_on_an_own_mount_is_its_link() -> Outcome<()> {
    let (own, w) = res!(tables());
    expect(resolve("/mnt/b/w3", 30, &own, Some(&w)), HostPath::Own(PathBuf::from("/mnt/b/w3")))
}

#[test]
fn a_container_bind_maps_through_the_shortest_host_root() -> Outcome<()> {
    // Both host mounts 22 (root "/") and 30 (root "/home/jason/x") hold it; "/" wins.
    let (own, w) = res!(tables());
    expect(resolve("/x/w5", 500, &own, Some(&w)), HostPath::Mapped(PathBuf::from("/home/jason/x/w5")))
}

#[test]
fn a_bind_maps_under_a_host_bind_when_no_whole_mount_exists() -> Outcome<()> {
    let (own, w) = res!(tables());
    expect(resolve("/p/src/f", 501, &own, Some(&w)), HostPath::Mapped(PathBuf::from("/srv/proj/src/f")))
}

#[test]
fn a_mount_absent_from_the_writer_table_is_never_mapped() -> Outcome<()> {
    // N1: a pid reused in another namespace must not give a host path.
    let (own, w) = res!(tables());
    expect(resolve("/x/w5", 777, &own, Some(&w)), HostPath::Unresolved(Unresolved::MountNotInWriter { mnt: 777 }))
}

#[test]
fn a_writer_gone_is_unresolved() -> Outcome<()> {
    let (own, _) = res!(tables());
    expect(resolve("/x/w5", 500, &own, None), HostPath::Unresolved(Unresolved::WriterGone))
}

#[test]
fn a_superblock_the_host_lacks_is_unresolved() -> Outcome<()> {
    let (own, w) = res!(tables());
    expect(resolve("/other/f", 502, &own, Some(&w)), HostPath::Unresolved(Unresolved::NoHostMount {
        dev: "8:9".to_string(), fs_path: "/f".to_string() }))
}

#[test]
fn a_link_outside_its_mount_point_is_unresolved() -> Outcome<()> {
    let (own, w) = res!(tables());
    expect(resolve("/y/f", 500, &own, Some(&w)), HostPath::Unresolved(Unresolved::NotUnderMount {
        link: "/y/f".to_string(), point: "/x".to_string() }))
}

#[test]
fn a_deleted_file_is_unresolved() -> Outcome<()> {
    let (own, w) = res!(tables());
    expect(resolve("/mnt/b/w3 (deleted)", 30, &own, Some(&w)), HostPath::Unresolved(Unresolved::Deleted {
        link: "/mnt/b/w3 (deleted)".to_string() }))
}

#[test]
fn mountinfo_escapes_are_undone_and_parsed() -> Outcome<()> {
    if unescape("/a\\040b\\011c\\134d\\012") != "/a b\tc\\d\n" {
        return Err(err!("unescape gave {:?}.", unescape("/a\\040b\\011c\\134d\\012"); Test));
    }
    if unescape("/no\\escape\\99") != "/no\\escape\\99" {
        return Err(err!("unescape changed a non-escape."; Test));
    }
    let t = res!(MountTable::parse("7 1 8:1 /my\\040dir /mnt/my\\040point rw - ext4 /dev/sda1 rw\n"));
    let m = res!(t.get(7).ok_or_else(|| err!("Mount 7 missing."; Test)));
    if m.root != "/my dir" || m.point != "/mnt/my point" || m.fstype != "ext4" || m.source != "/dev/sda1" {
        return Err(err!("Parsed {:?}.", m; Test));
    }
    Ok(())
}

#[test]
fn writability_and_class_are_read_from_the_line() -> Outcome<()> {
    let t = res!(MountTable::parse("\
1 0 0:1 / /a rw - fuse.sshfs h:/ rw
2 0 0:2 / /b ro,relatime - ext4 /dev/sda1 rw
3 0 0:3 / /c rw - ext4 /dev/sda1 ro
4 0 0:4 / /d rw - nfs4 h:/ rw
5 0 0:5 / /proc rw - proc proc rw
6 0 0:6 / /tmp rw,nosuid - tmpfs tmpfs rw,size=1k
"));
    let got: Vec<(bool, FsClass)> = t.mounts.values().map(|m| (m.rw, m.class())).collect();
    let want = vec![(true, FsClass::Fuse), (false, FsClass::Local), (false, FsClass::Local),
        (true, FsClass::Network), (true, FsClass::Pseudo), (true, FsClass::Local)];
    if got != want {
        return Err(err!("Mounts read as {:?}, not {:?}.", got, want; Test));
    }
    Ok(())
}

#[test]
fn a_malformed_mountinfo_line_is_refused() -> Outcome<()> {
    if MountTable::parse("22 1 8:1 / / rw\n").is_ok() {
        return Err(err!("A line without the separator parsed."; Test));
    }
    if MountTable::parse("x 1 8:1 / / rw - ext4 /dev/sda1 rw\n").is_ok() {
        return Err(err!("A line with a bad id parsed."; Test));
    }
    Ok(())
}

#[test]
fn new_mounts_are_found_against_an_older_table() -> Outcome<()> {
    let (own, _) = res!(tables());
    let more = res!(MountTable::parse(&format!("{}55 22 8:1 / /mnt/loop rw - ext4 /dev/loop0 rw\n", OWN)));
    let added = more.added_since(&own);
    if added.len() != 1 || added[0].point != "/mnt/loop" {
        return Err(err!("Added {:?}.", added; Test));
    }
    Ok(())
}

#[test]
fn the_live_mount_id_of_a_file_is_in_the_live_table() -> Outcome<()> {
    let exe = res!(env::current_exe());
    let f = res!(File::open(&exe));
    let (mnt, ino) = res!(mount_id(f.as_fd()));
    let own = res!(MountTable::of_self());
    if own.get(mnt).is_none() || ino == 0 {
        return Err(err!("Mount id {} (ino {}) is not in the live table.", mnt, ino; Test));
    }
    let link = res!(fs::read_link(format!("/proc/self/fd/{}", std::os::fd::AsRawFd::as_raw_fd(&f))));
    expect(resolve(&link.to_string_lossy(), mnt, &own, None), HostPath::Own(link))
}

// ── Follower ──

fn scratch(tag: &str) -> Outcome<PathBuf> {
    let dir = env::temp_dir().join(format!("fe2o3-sys-notify-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    res!(fs::create_dir_all(dir.join("sub")));
    Ok(dir)
}

fn held(f: Followed) -> Outcome<FollowId> {
    match f {
        Followed::Held(id)  => Ok(id),
        other               => Err(err!("Expected a held file, got {:?}.", other; Test)),
    }
}

/// Reads changes until one arrives or a second passes.
fn changes(fo: &mut Follower) -> Outcome<Vec<Change>> {
    let end = Instant::now() + Duration::from_secs(1);
    loop {
        let mut p = [PollFd::new(fo.fd(), PollFlags::POLLIN)];
        res!(poll(&mut p, PollTimeout::from(100u16)));
        let c = res!(fo.read());
        if !c.is_empty() || Instant::now() > end { return Ok(c); }
    }
}

#[test]
fn a_rename_across_directories_is_reported_old_and_new() -> Outcome<()> {
    let d = res!(scratch("mv"));
    let a = d.join("a");
    res!(fs::write(&a, b"1"));
    let mut fo = res!(Follower::new(8));
    let id = res!(held(res!(fo.follow_path(&a))));
    let b = d.join("sub").join("b");
    res!(fs::rename(&a, &b));
    let c = res!(changes(&mut fo));
    let _ = fs::remove_dir_all(&d);
    if c != vec![Change::Moved { id, old: a.clone(), new: b.clone() }] || fo.path(id) != Some(b.as_path()) {
        return Err(err!("Changes {:?}.", c; Test));
    }
    Ok(())
}

#[test]
fn a_file_followed_by_its_descriptor_is_followed_wherever_it_went() -> Outcome<()> {
    // As with a fanotify event fd: the rename comes before the follow.
    let d = res!(scratch("fd"));
    let part = d.join("f.part");
    res!(fs::write(&part, b"1"));
    let open = res!(File::open(&part));
    let f = d.join("f");
    res!(fs::rename(&part, &f));
    let mut fo = res!(Follower::new(8));
    let id = res!(held(res!(fo.follow_fd(open.as_fd()))));
    drop(open);
    if fo.path(id) != Some(f.as_path()) {
        return Err(err!("Followed at {:?}, not {:?}.", fo.path(id), f; Test));
    }
    let g = d.join("g");
    res!(fs::rename(&f, &g));
    let c = res!(changes(&mut fo));
    let _ = fs::remove_dir_all(&d);
    if c != vec![Change::Moved { id, old: f, new: g }] {
        return Err(err!("Changes {:?}.", c; Test));
    }
    Ok(())
}

#[test]
fn an_atomic_save_over_the_file_is_gone() -> Outcome<()> {
    let d = res!(scratch("save"));
    let f = d.join("doc");
    res!(fs::write(&f, b"1"));
    let mut fo = res!(Follower::new(8));
    let id = res!(held(res!(fo.follow_path(&f))));
    let tmp = d.join(".doc.swp");
    res!(fs::write(&tmp, b"2"));
    res!(fs::rename(&tmp, &f));
    let c = res!(changes(&mut fo));
    let _ = fs::remove_dir_all(&d);
    if c != vec![Change::Gone { id, last: f }] || !fo.is_empty() {
        return Err(err!("Changes {:?}, {} still held.", c, fo.len(); Test));
    }
    Ok(())
}

#[test]
fn an_unlink_is_gone() -> Outcome<()> {
    let d = res!(scratch("rm"));
    let f = d.join("x");
    res!(fs::write(&f, b"1"));
    let mut fo = res!(Follower::new(8));
    let id = res!(held(res!(fo.follow_path(&f))));
    res!(fs::remove_file(&f));
    let c = res!(changes(&mut fo));
    let _ = fs::remove_dir_all(&d);
    if c != vec![Change::Gone { id, last: f }] {
        return Err(err!("Changes {:?}.", c; Test));
    }
    Ok(())
}

#[test]
fn a_chmod_alone_changes_nothing() -> Outcome<()> {
    let d = res!(scratch("chmod"));
    let f = d.join("x");
    res!(fs::write(&f, b"1"));
    let mut fo = res!(Follower::new(8));
    res!(held(res!(fo.follow_path(&f))));
    res!(fs::set_permissions(&f, fs::Permissions::from_mode(0o600)));
    let c = res!(changes(&mut fo));
    let _ = fs::remove_dir_all(&d);
    if !c.is_empty() || fo.len() != 1 {
        return Err(err!("Changes {:?}.", c; Test));
    }
    Ok(())
}

#[test]
fn the_same_file_twice_is_held_once() -> Outcome<()> {
    let d = res!(scratch("twice"));
    let f = d.join("x");
    res!(fs::write(&f, b"1"));
    let mut fo = res!(Follower::new(8));
    let id = res!(held(res!(fo.follow_path(&f))));
    let again = res!(fo.follow_path(&f));
    let _ = fs::remove_dir_all(&d);
    if again != Followed::Already(id) || fo.len() != 1 {
        return Err(err!("Second follow gave {:?}, {} held.", again, fo.len(); Test));
    }
    Ok(())
}

#[test]
fn follow_max_is_reported_not_swallowed() -> Outcome<()> {
    let d = res!(scratch("max"));
    let mut fo = res!(Follower::new(2));
    let mut last = Followed::Full(Limit::Watches);
    for i in 0..3 {
        let f = d.join(format!("f{}", i));
        res!(fs::write(&f, b"1"));
        last = res!(fo.follow_path(&f));
    }
    let _ = fs::remove_dir_all(&d);
    if last != Followed::Full(Limit::FollowMax(2)) || fo.len() != 2 {
        return Err(err!("Third follow gave {:?}, {} held.", last, fo.len(); Test));
    }
    Ok(())
}

#[test]
fn unfollow_under_releases_every_file_below_a_prefix() -> Outcome<()> {
    let d = res!(scratch("under"));
    let a = d.join("sub").join("a");
    let b = d.join("b");
    res!(fs::write(&a, b"1"));
    res!(fs::write(&b, b"1"));
    let mut fo = res!(Follower::new(8));
    let ia = res!(held(res!(fo.follow_path(&a))));
    res!(held(res!(fo.follow_path(&b))));
    let gone = fo.unfollow_under(&d.join("sub"));
    // Renaming the released file afterwards reports nothing.
    res!(fs::rename(&a, d.join("sub").join("a2")));
    let c = res!(changes(&mut fo));
    let _ = fs::remove_dir_all(&d);
    if gone != vec![(ia, a)] || fo.len() != 1 || !c.is_empty() {
        return Err(err!("Released {:?}, {} held, changes {:?}.", gone, fo.len(), c; Test));
    }
    Ok(())
}

#[test]
fn resync_reports_the_final_path_after_many_renames() -> Outcome<()> {
    let d = res!(scratch("resync"));
    let f = d.join("v0");
    res!(fs::write(&f, b"1"));
    let mut fo = res!(Follower::new(8));
    let id = res!(held(res!(fo.follow_path(&f))));
    let mut at = f.clone();
    for i in 1..20 {
        let next = d.join(format!("v{}", i));
        res!(fs::rename(&at, &next));
        at = next;
    }
    let c = res!(fo.resync());
    let _ = fs::remove_dir_all(&d);
    if c != vec![Change::Moved { id, old: f, new: at }] {
        return Err(err!("Resync gave {:?}.", c; Test));
    }
    Ok(())
}

#[test]
fn a_queue_overflow_is_reported_and_resynced() -> Outcome<()> {
    let max: usize = res!(res!(fs::read_to_string("/proc/sys/fs/inotify/max_queued_events")).trim().parse());
    if max > 1 << 20 {
        return Err(err!("fs.inotify.max_queued_events is {}; this arm needs it at most {}.", max, 1 << 20; Test));
    }
    let d = res!(scratch("ovf"));
    let f = d.join("o");
    res!(fs::write(&f, b"1"));
    let mut fo = res!(Follower::new(8));
    let id = res!(held(res!(fo.follow_path(&f))));
    // A chmod between renames keeps consecutive events distinct, so the kernel cannot merge them.
    let (a, b) = (d.join("p"), d.join("q"));
    res!(fs::rename(&f, &a));
    for i in 0..(max / 2 + 64) {
        res!(fs::set_permissions(&a, fs::Permissions::from_mode(if i % 2 == 0 { 0o600 } else { 0o644 })));
        res!(fs::rename(&a, &b));
        res!(fs::rename(&b, &a));
    }
    let end = d.join("final");
    res!(fs::rename(&a, &end));
    let mut all = Vec::new();
    loop {
        let c = res!(fo.read());
        if c.is_empty() { break; }
        all.extend(c);
    }
    let _ = fs::remove_dir_all(&d);
    if !all.contains(&Change::Overflow) {
        return Err(err!("No overflow among {} changes.", all.len(); Test));
    }
    if fo.path(id) != Some(end.as_path()) {
        return Err(err!("Followed at {:?} after the overflow, not {:?}.", fo.path(id), end; Test));
    }
    Ok(())
}

// ── Fanotify (root) ──

#[test]
fn an_unprivileged_group_fails_loudly() -> Outcome<()> {
    if nix::unistd::geteuid().is_root() { return Ok(()); }
    match FanGroup::new() {
        Ok(_)   => Err(err!("fanotify_init succeeded without CAP_SYS_ADMIN."; Test)),
        Err(e)  => {
            let msg = format!("{}", e);
            if msg.contains("fanotify_init") { Ok(()) }
            else { Err(err!("Unexpected refusal: {}", msg; Test)) }
        }
    }
}

fn fan_until<F: Fn(&FanEvent) -> bool>(g: &FanGroup, want: F) -> Outcome<Option<FanEvent>> {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        let mut p = [PollFd::new(g.fd(), PollFlags::POLLIN)];
        res!(poll(&mut p, PollTimeout::from(200u16)));
        for ev in res!(g.read()) {
            if want(&ev) { return Ok(Some(ev)); }
        }
    }
    Ok(None)
}

fn root_scratch(tag: &str) -> Outcome<PathBuf> {
    // Cargo's scratch directory for integration tests, inside the target directory.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fe2o3-sys-notify").join(tag);
    let _ = fs::remove_dir_all(&dir);
    res!(fs::create_dir_all(&dir));
    Ok(dir)
}

#[test]
#[ignore = "needs CAP_SYS_ADMIN for fanotify; BR-4e S6 runs it as root in the VM"]
fn root_hears_a_write_on_its_own_mount_with_its_writer() -> Outcome<()> {
    let d = res!(root_scratch("own"));
    let mut g = res!(FanGroup::new());
    res!(g.mark_filesystem(&d));
    let _ = res!(g.refresh_mounts());
    let f = d.join("w1");
    res!(fs::write(&f, b"x"));
    let ev = res!(fan_until(&g, |e| matches!(e, FanEvent::Written(w) if w.link.ends_with("/w1"))));
    res!(g.unmark_filesystem(&d));
    let _ = fs::remove_dir_all(&d);
    match ev {
        Some(FanEvent::Written(w)) => {
            if w.host != HostPath::Own(f.clone()) || w.fd().is_none() {
                return Err(err!("Heard {:?}.", w; Test));
            }
            match w.writer {
                Writer::Live { pid, uid: 0, .. } if pid == std::process::id() as i32 => Ok(()),
                other => Err(err!("Writer {:?}.", other; Test)),
            }
        }
        other => Err(err!("No write heard: {:?}.", other; Test)),
    }
}

#[test]
#[ignore = "needs root for a mount namespace and fanotify; BR-4e S6 runs it in the VM"]
fn root_maps_a_write_through_another_namespace_bind_to_the_host_path() -> Outcome<()> {
    let d = res!(root_scratch("ns"));
    let src = d.join("src");
    let dst = d.join("dst");
    res!(fs::create_dir_all(&src));
    res!(fs::create_dir_all(&dst));
    let g = res!(FanGroup::new());
    res!(g.mark_filesystem(&d));
    // The bind exists only in the child's namespace; the writer stays alive to be read.
    let script = format!("mount --bind '{}' '{}' && echo x > '{}/w3' && sleep 3", src.display(), dst.display(), dst.display());
    let mut child = res!(Command::new("unshare").args(["-m", "--propagation", "private", "sh", "-c", &script]).spawn());
    let ev = res!(fan_until(&g, |e| matches!(e, FanEvent::Written(w) if w.link.ends_with("/w3"))));
    let _ = child.wait();
    let _ = fs::remove_dir_all(&d);
    let want = HostPath::Mapped(src.join("w3"));
    match ev {
        Some(FanEvent::Written(w)) if w.host == want => Ok(()),
        Some(FanEvent::Written(w)) => Err(err!("Heard {:?}, wanted {:?}.", w, want; Test)),
        other => Err(err!("No write heard: {:?}.", other; Test)),
    }
}

#[test]
#[ignore = "needs CAP_SYS_ADMIN for fanotify; BR-4e S6 runs it as root in the VM"]
fn root_sees_overflow_on_the_bounded_queue() -> Outcome<()> {
    let max = res!(FanGroup::max_queued_events());
    let d = res!(root_scratch("ovf"));
    let g = res!(FanGroup::new());
    res!(g.mark_filesystem(&d));
    // Distinct files, so the kernel cannot merge the events.
    for i in 0..(max + 256) {
        res!(fs::write(d.join(format!("f{}", i)), b"x"));
    }
    let ev = res!(fan_until(&g, |e| matches!(e, FanEvent::Overflow)));
    let _ = fs::remove_dir_all(&d);
    match ev {
        Some(FanEvent::Overflow) => Ok(()),
        other => Err(err!("No overflow after {} writes: {:?}.", max + 256, other; Test)),
    }
}
