// QA agent B probes (06 Nov 2026) of fe2o3_sys fanotify.rs @ 54bb4109. Root arms are #[ignore]d: run in the VM.
// Each arm asserts the SAFE behaviour, so a RED is a fault shown.
#![cfg(all(target_os = "linux", feature = "fanotify"))]

use oxedyne_fe2o3_sys::fanotify::{
    parse_events,
    Change,
    Entries,
    Gate,
    Op,
};

use std::{
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn rec(kind: u8, ino: u32, name: &str) -> Vec<u8> {
    let mut b = vec![kind, 0, 0, 0];
    b.extend_from_slice(&7i32.to_ne_bytes());
    b.extend_from_slice(&9i32.to_ne_bytes());
    b.extend_from_slice(&8u32.to_ne_bytes());
    b.extend_from_slice(&1i32.to_ne_bytes());
    b.extend_from_slice(&ino.to_ne_bytes());
    b.extend_from_slice(&3u32.to_ne_bytes());
    b.extend_from_slice(name.as_bytes());
    b.push(0);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    let n = b.len() as u16;
    b[2..4].copy_from_slice(&n.to_ne_bytes());
    b
}

fn event(vers: u8, mask: u64, recs: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = recs.concat();
    let mut b = Vec::new();
    b.extend_from_slice(&((24 + body.len()) as u32).to_ne_bytes());
    b.extend_from_slice(&[vers, 0]);
    b.extend_from_slice(&24u16.to_ne_bytes());
    b.extend_from_slice(&mask.to_ne_bytes());
    b.extend_from_slice(&(-1i32).to_ne_bytes());
    b.extend_from_slice(&42i32.to_ne_bytes());
    b.extend_from_slice(&body);
    b
}

// ---- Surface 1, host (no root) ----

#[test]
fn qa_b_trailing_partial_event_is_refused() {
    let mut b = event(3, 0x100, &[rec(2, 1, "x")]);
    b.extend_from_slice(&[1, 2, 3, 4, 5]); // 5 stray bytes: not a whole event
    let r = parse_events(&b, &mut Vec::new());
    assert!(r.is_err(), "stray trailing bytes were accepted silently");
}

#[test]
fn qa_b_wrong_metadata_version_is_refused() {
    let b = event(2, 0x100, &[rec(2, 1, "x")]);
    let r = parse_events(&b, &mut Vec::new());
    assert!(r.is_err(), "an event of metadata version 2 was parsed as version 3");
}

#[test]
fn qa_b_handle_length_wrap() {
    // handle_bytes = u32::MAX: must be refused, never panic.
    let mut r = rec(2, 1, "x");
    r[12..16].copy_from_slice(&u32::MAX.to_ne_bytes());
    let b = event(3, 0x100, &[r]);
    // Refused as an unreadable event (B6: it becomes Lost and parsing goes on).
    let res = std::panic::catch_unwind(|| {
        let mut out = Vec::new();
        let r = parse_events(&b, &mut out);
        r.is_ok() && out == vec![Change::Lost]
    });
    assert_eq!(res.ok(), Some(true));
}

#[test]
fn qa_b_one_bad_event_loses_the_batch_without_lost() {
    // Two good events then one with no directory record (e.g. a fid the kernel failed to encode).
    let mut b = event(3, 0x100, &[rec(2, 1, "a")]);
    b.extend(event(3, 0x200, &[rec(2, 1, "b")]));
    b.extend(event(3, 0x100, &[rec(1, 5, "")]));
    let mut out = Vec::new();
    let r = parse_events(&b, &mut out);
    // Safe behaviour: the batch is kept and the unreadable event reported as Lost.
    assert!(r.is_ok() && out.last() == Some(&Change::Lost), "r={:?} out={:?}", r.is_ok(), out);
}

#[test]
fn qa_b_misaligned_buffer_parses() {
    let e = event(3, 0x100, &[rec(2, 11, "n")]);
    let mut v = vec![0u8];
    v.extend_from_slice(&e);
    let mut out = Vec::new();
    parse_events(&v[1..], &mut out).expect("misaligned");
    assert_eq!(out.len(), 1);
}

// ---- helpers for root arms ----

fn scratch(tag: &str) -> PathBuf {
    let base = std::env::var("QA_DIR").unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    let d = Path::new(&base).join(format!("qab_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn drain(e: &Entries, rounds: usize) -> Vec<Change> {
    let mut v = Vec::new();
    for _ in 0..rounds {
        v.extend(e.next(Some(Duration::from_millis(50))).expect("next"));
    }
    v
}

fn sh(c: &str) -> bool {
    std::process::Command::new("sh").arg("-c").arg(c).status().map(|s| s.success()).unwrap_or(false)
}

// ---- Surface 1, root: Fid::ino against stat, per filesystem ----

fn ino_matches_stat(d: &Path) -> (Vec<String>, bool) {
    let e = Entries::filesystem(d).expect("watch");
    let dino = std::fs::metadata(d).expect("meta").ino();
    std::fs::create_dir(d.join("sub")).expect("mkdir");
    let sino = std::fs::metadata(d.join("sub")).expect("meta").ino();
    let v = drain(&e, 10);
    let mut lines = Vec::new();
    let mut good = false;
    for c in &v {
        if let Change::Entry { op: Op::Create, dir, name, target, .. } = c {
            if name == "sub" {
                let t = target.as_ref().expect("target");
                lines.push(format!("dir kind={:#x} handle={:02x?} ino()={:?} stat={}", dir.kind, dir.handle, dir.ino(), dino));
                lines.push(format!("sub kind={:#x} handle={:02x?} ino()={:?} stat={}", t.kind, t.handle, t.ino(), sino));
                good = dir.ino() == Some(dino) && t.ino() == Some(sino);
            }
        }
    }
    (lines, good)
}

#[test]
#[ignore]
fn qa_b_fid_ino_on_tmpfs_as_root() {
    let d = scratch("tmpfs");
    assert!(sh(&format!("mount -t tmpfs qab {}", d.display())), "mount tmpfs");
    let (lines, good) = ino_matches_stat(&d);
    for l in &lines { println!("{}", l); }
    let _ = sh(&format!("umount {}", d.display()));
    let _ = std::fs::remove_dir_all(&d);
    assert!(!lines.is_empty(), "no create event seen");
    assert!(good, "Fid::ino is not the inode number on tmpfs");
}

#[test]
#[ignore]
fn qa_b_fid_ino_on_home_fs_as_root() {
    let d = scratch("home");
    let (lines, good) = ino_matches_stat(&d);
    for l in &lines { println!("{}", l); }
    let _ = std::fs::remove_dir_all(&d);
    assert!(good, "Fid::ino is not the inode number on the home filesystem");
}

// ---- Surface 1, root: a merged DELETE|CREATE of one inode (hard link) is reported in the wrong order ----

#[test]
#[ignore]
fn qa_b_merge_order_hardlink_as_root() {
    let d = scratch("merge");
    std::fs::write(d.join("f"), b"x").expect("write");
    std::fs::hard_link(d.join("f"), d.join("a")).expect("link");
    let e = Entries::filesystem(&d).expect("watch");
    // Removed, then the same inode linked back at the same name, both before the watch reads.
    std::fs::remove_file(d.join("a")).expect("unlink");
    std::fs::hard_link(d.join("f"), d.join("a")).expect("relink");
    let v = drain(&e, 10);
    let ops: Vec<Op> = v.iter().filter_map(|c| match c {
        Change::Entry { op, name, .. } if name == "a" => Some(*op),
        _ => None,
    }).collect();
    println!("events for a: {:?}; a exists: {}", ops, d.join("a").exists());
    let _ = std::fs::remove_dir_all(&d);
    // Safe: the last thing reported for "a" agrees with the disk (it exists).
    // (A merged event reports Recheck: the caller stats the name and finds it.)
    assert!(matches!(ops.last(), Some(&Op::Create) | Some(&Op::Recheck)),
        "the watch reports a as removed while it exists: {:?}", ops);
}

// ---- Surface 1, root: next() under a hostile flood of entries ----

#[test]
#[ignore]
fn qa_b_next_bounded_under_flood_as_root() {
    let d = scratch("flood");
    let e = Entries::filesystem(&d).expect("watch");
    let stop = Arc::new(AtomicBool::new(false));
    let mut hs = Vec::new();
    for t in 0..8 {
        let s = stop.clone();
        let dd = d.clone();
        hs.push(std::thread::spawn(move || {
            let mut i = 0u64;
            while !s.load(Ordering::Relaxed) {
                let p = dd.join(format!("t{}_{}", t, i % 64));
                let _ = std::fs::File::create(&p);
                let _ = std::fs::remove_file(&p);
                i += 1;
            }
        }));
    }
    std::thread::sleep(Duration::from_millis(200));
    let flood = Duration::from_secs(8);
    let t0 = Instant::now();
    let mut longest = Duration::ZERO;
    let mut most = 0usize;
    let mut calls = 0;
    while t0.elapsed() < flood {
        let c0 = Instant::now();
        let n = e.next(Some(Duration::from_millis(50))).expect("next").len();
        longest = longest.max(c0.elapsed());
        most = most.max(n);
        calls += 1;
        if c0.elapsed() > flood {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    for h in hs { let _ = h.join(); }
    println!("calls {} longest next() {:?} most changes in one call {}", calls, longest, most);
    let _ = std::fs::remove_dir_all(&d);
    assert!(longest < Duration::from_secs(1), "one next() ran {:?} with {} changes: unbounded", longest, most);
}

// ---- Surface 2, root: what the gate mask misses ----

#[test]
#[ignore]
fn qa_b_fd_opened_before_hold_bypasses_as_root() {
    let d = scratch("prefd");
    let other = d.join("other.txt");
    let p = d.join("held.txt");
    std::fs::write(&other, b"o").expect("write");
    std::fs::write(&p, b"before").expect("write");
    let g = Gate::new().expect("gate");
    // The filesystem already has a pre-content mark (another file) when the hostile fd opens.
    let of = std::fs::File::open(&other).expect("open other");
    g.hold(&of).expect("hold other");
    let mut early = std::fs::OpenOptions::new().write(true).open(&p).expect("early open");
    let f = std::fs::File::open(&p).expect("open");
    g.hold(&f).expect("hold");
    let w = std::thread::spawn(move || { early.write_all(b"HOSTILE").map(|_| ()) });
    let mut seen = 0;
    for _ in 0..10 {
        let v = g.next(Some(Duration::from_millis(100))).expect("next");
        seen += v.len();
        for h in v { let _ = g.answer(h, true); }
    }
    let wr = w.join().expect("join");
    g.release(&f).expect("release");
    g.release(&of).expect("release other");
    let now = std::fs::read(&p).expect("read");
    println!("write {:?} held events {} content now {:?}", wr.is_ok(), seen, String::from_utf8_lossy(&now));
    let _ = std::fs::remove_dir_all(&d);
    assert!(seen > 0 || now == b"before", "an fd opened before hold() wrote the held file with no event");
}

#[test]
#[ignore]
fn qa_b_path_truncate_is_held_as_root() {
    let d = scratch("trunc");
    let p = d.join("held.txt");
    std::fs::write(&p, b"before").expect("write");
    let g = Gate::new().expect("gate");
    let f = std::fs::File::open(&p).expect("open");
    g.hold(&f).expect("hold");
    let pc = p.clone();
    let w = std::thread::spawn(move || sh(&format!("python3 -c 'import os; os.truncate(\"{}\", 0)'", pc.display())));
    let mut seen = 0;
    let mut len_at_hold = None;
    for _ in 0..20 {
        let v = g.next(Some(Duration::from_millis(100))).expect("next");
        for h in v {
            seen += 1;
            if len_at_hold.is_none() { len_at_hold = std::fs::metadata(&p).ok().map(|m| m.len()); }
            let _ = g.answer(h, false);
        }
    }
    let ok = w.join().expect("join");
    let len = std::fs::metadata(&p).expect("meta").len();
    println!("truncate(2) succeeded {} held {} len at hold {:?} len now {}", ok, seen, len_at_hold, len);
    let _ = std::fs::remove_dir_all(&d);
    assert!(len == 6, "path truncate(2) emptied a held file (held events {})", seen);
}

// ---- Surface 2/3, root: the gate's queue overflow lets accesses through unanswered ----

#[test]
#[ignore]
fn qa_b_gate_overflow_fails_open_as_root() {
    let q = "/proc/sys/fs/fanotify/max_queued_events";
    let was = std::fs::read_to_string(q).expect("read sysctl");
    std::fs::write(q, "16").expect("set sysctl");
    let g = Gate::new();
    std::fs::write(q, was.trim()).expect("restore sysctl");
    let g = g.expect("gate");
    let d = scratch("govf");
    let p = d.join("held.txt");
    std::fs::write(&p, b"before").expect("write");
    let f = std::fs::File::open(&p).expect("open");
    g.hold(&f).expect("hold");
    let done = Arc::new(AtomicUsize::new(0));
    let mut hs = Vec::new();
    for _ in 0..40 {
        let pp = p.clone();
        let dn = done.clone();
        hs.push(std::thread::spawn(move || {
            if std::fs::read(&pp).is_ok() { dn.fetch_add(1, Ordering::SeqCst); }
        }));
    }
    // The holder does not read for a second (a slow copy of a big file).
    std::thread::sleep(Duration::from_secs(1));
    let through = done.load(Ordering::SeqCst);
    let mut nofd = 0;
    for _ in 0..50 {
        let v = g.next(Some(Duration::from_millis(50))).expect("next");
        for h in v {
            if h.fd().is_none() { nofd += 1; }
            let _ = g.answer(h, false);
        }
        if hs.iter().all(|h| h.is_finished()) { break; }
    }
    for h in hs { let _ = h.join(); }
    println!("accesses through unanswered: {} of 40; held without fd (overflow): {}", through, nofd);
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(through, 0, "{} accesses of a held file went through with no answer", through);
}

// ---- Surface 3, root: a Held dropped unanswered ----

#[test]
#[ignore]
fn qa_b_dropped_held_as_root() {
    let d = scratch("drop");
    let p = d.join("held.txt");
    std::fs::write(&p, b"before").expect("write");
    let g = Gate::new().expect("gate");
    let f = std::fs::File::open(&p).expect("open");
    g.hold(&f).expect("hold");
    let pc = p.clone();
    let t0 = Instant::now();
    let w = std::thread::spawn(move || { let r = std::fs::write(&pc, b"after!"); (r.is_ok(), Instant::now()) });
    let held = loop {
        let mut v = g.next(Some(Duration::from_secs(5))).expect("next");
        if let Some(h) = v.pop() { break h; }
    };
    drop(held); // closes its fd, writes no response
    std::thread::sleep(Duration::from_secs(2));
    let blocked = !w.is_finished();
    println!("after drop: accessor still blocked after 2 s: {}", blocked);
    drop(g); // the group closes, as when the service ends or crashes
    let (ok, at) = w.join().expect("join");
    let now = std::fs::read(&p).expect("read");
    println!("after gate drop: write ok {} at {:?}, content {:?}", ok, at - t0, String::from_utf8_lossy(&now));
    let _ = std::fs::remove_dir_all(&d);
    assert!(!(ok && now == b"after!"), "a held write was let through unkept when the gate closed");
}

#[test]
#[ignore]
fn qa_b_mmap_made_before_hold_bypasses_as_root() {
    let d = scratch("mmap");
    let p = d.join("held.txt");
    std::fs::write(&p, b"before").expect("write");
    let g = Gate::new().expect("gate");
    let other = d.join("other.txt");
    std::fs::write(&other, b"o").expect("write");
    let of = std::fs::File::open(&other).expect("open other");
    g.hold(&of).expect("hold other");
    let (ready, go) = (d.join("ready"), d.join("go"));
    let py = format!("import mmap,os,time\nf=open('{p}','r+b'); m=mmap.mmap(f.fileno(),0)\nopen('{r}','w').close()\n\
while not os.path.exists('{g}'): time.sleep(0.01)\nm[0:6]=b'HOSTIL'; m.flush()\n",
        p = p.display(), r = ready.display(), g = go.display());
    let mut child = std::process::Command::new("python3").arg("-c").arg(py).spawn().expect("python3");
    while !ready.exists() { std::thread::sleep(Duration::from_millis(10)); }
    let f = std::fs::File::open(&p).expect("open");
    g.hold(&f).expect("hold");
    std::fs::write(&go, b"").expect("go");
    let mut seen = 0;
    for _ in 0..20 {
        let v = g.next(Some(Duration::from_millis(100))).expect("next");
        seen += v.len();
        for h in v { let _ = g.answer(h, true); }
        if let Ok(Some(_)) = child.try_wait() { break; }
    }
    let st = child.wait().expect("wait");
    g.release(&f).expect("release");
    g.release(&of).expect("release other");
    let now = std::fs::read(&p).expect("read");
    println!("child {:?} held events {} content now {:?}", st, seen, String::from_utf8_lossy(&now));
    let _ = std::fs::remove_dir_all(&d);
    assert!(seen > 0 || now == b"before", "a mapping made before hold() wrote the held file with no event");
}

// ---- KIMB-USERMGR 2k, fault 3 proof: which pre-content mark gates a file of a folder opened after the mark,
// before any hold of the file itself (by descriptor, read and write): a folder mark (FAN_EVENT_ON_CHILD) or a
// filesystem mark; and what each costs per open+read ----

fn pc_gate(d: &Path, fs: bool) -> nix::sys::fanotify::Fanotify {
    use nix::sys::fanotify::*;
    let g = Fanotify::init(InitFlags::FAN_CLASS_PRE_CONTENT | InitFlags::FAN_CLOEXEC | InitFlags::FAN_NONBLOCK
        | InitFlags::FAN_UNLIMITED_QUEUE, EventFFlags::O_RDONLY | EventFFlags::O_LARGEFILE).expect("init");
    let (how, extra) = if fs { (MarkFlags::FAN_MARK_FILESYSTEM, MaskFlags::empty()) }
        else { (MarkFlags::FAN_MARK_INODE, MaskFlags::FAN_EVENT_ON_CHILD) };
    g.mark(MarkFlags::FAN_MARK_ADD | how, MaskFlags::from_bits_retain(0x0010_0000 | 0x0001_0000) | extra,
        nix::fcntl::AT_FDCWD, Some(d)).expect("mark");
    g
}

/// Answers every access with allow, counting open-perm (n[0]) and pre-access (n[1]) events, until `stop`.
fn allow_all(g: nix::sys::fanotify::Fanotify, n: Arc<[AtomicUsize; 2]>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    use nix::sys::fanotify::*;
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let mut fds = [nix::poll::PollFd::new(std::os::fd::AsFd::as_fd(&g), nix::poll::PollFlags::POLLIN)];
            let _ = nix::poll::poll(&mut fds, nix::poll::PollTimeout::from(20u8));
            for ev in g.read_events().unwrap_or_default() {
                let m = ev.mask().bits();
                if m & 0x0001_0000 != 0 { n[0].fetch_add(1, Ordering::SeqCst); }
                // nix's mask() truncates bits it does not know, FAN_PRE_ACCESS among them: any other event is one.
                if m & 0x0001_0000 == 0 { n[1].fetch_add(1, Ordering::SeqCst); }
                if let Some(fd) = ev.fd() {
                    let _ = g.write_response(FanotifyResponse::new(fd, Response::FAN_ALLOW));
                }
            }
        }
    })
}

fn early_fd_events(fs: bool) -> (usize, usize, usize, Duration, Duration) {
    let d = scratch(if fs { "pcfs" } else { "pcdir" });
    let p = d.join("held.txt");
    std::fs::write(&p, b"before").expect("write");
    let n: Arc<[AtomicUsize; 2]> = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let stop = Arc::new(AtomicBool::new(false));
    let t = allow_all(pc_gate(&d, fs), n.clone(), stop.clone());
    std::thread::sleep(Duration::from_millis(50));
    let mut early = std::fs::OpenOptions::new().read(true).write(true).open(&p).expect("early open");
    std::thread::sleep(Duration::from_millis(100));
    let opens = n[0].load(Ordering::SeqCst);
    let c0 = n[1].load(Ordering::SeqCst);
    early.write_all(b"HOSTILE").expect("write");
    std::thread::sleep(Duration::from_millis(100));
    let by_write = n[1].load(Ordering::SeqCst) - c0;
    let c1 = n[1].load(Ordering::SeqCst);
    let mut b = [0u8; 4];
    let _ = std::io::Seek::seek(&mut early, std::io::SeekFrom::Start(0));
    let _ = std::io::Read::read(&mut early, &mut b);
    std::thread::sleep(Duration::from_millis(100));
    let by_read = n[1].load(Ordering::SeqCst) - c1;
    let side = scratch(if fs { "pcfs_side" } else { "pcdir_side" });
    let q = side.join("free.txt");
    std::fs::write(&q, b"x").expect("write");
    let time = |f: &Path| { let t0 = Instant::now(); for _ in 0..2000 { let _ = std::fs::read(f); } t0.elapsed() };
    let (marked, other) = (time(&p), time(&q));
    stop.store(true, Ordering::SeqCst);
    let _ = t.join();
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&side);
    (opens, by_write, by_read, marked, other)
}

#[test]
#[ignore]
fn um2k_folder_mark_gates_a_file_opened_before_its_hold_as_root() {
    let f = early_fd_events(false);
    let s = early_fd_events(true);
    println!("folder mark: open-perm {} pre-access by write {} by read {}; 2000 reads in it {:?}, beside it {:?}", f.0, f.1, f.2, f.3, f.4);
    println!("filesystem mark: open-perm {} pre-access by write {} by read {}; 2000 reads in it {:?}, beside it {:?}", s.0, s.1, s.2, s.3, s.4);
    assert!(f.1 > 0, "a write through a descriptor opened after the folder mark raised no pre-access event: \
        folder {:?} filesystem {:?}", f, s);
}

// ---- KIMB-USERMGR 2k, B-F4 proof: the handle /proc/self/fdinfo shows for an inode mark (no unsafe, no
// name_to_handle_at) is the handle an entry event carries for that folder, on ext4 (home) and on tmpfs ----

fn fdinfo_handle_matches_event(d: &Path) -> String {
    use nix::sys::fanotify::*;
    let g = Fanotify::init(InitFlags::FAN_CLASS_NOTIF | InitFlags::FAN_CLOEXEC | InitFlags::FAN_NONBLOCK
        | InitFlags::from_bits_retain(0x1e00), EventFFlags::O_RDONLY).expect("init");
    g.mark(MarkFlags::FAN_MARK_ADD | MarkFlags::FAN_MARK_FILESYSTEM,
        MaskFlags::from_bits_retain(0x100 | 0x4000_0000), nix::fcntl::AT_FDCWD, Some(d)).expect("mark fs");
    // An ignore-only inode mark raises no event; it is there only to be listed.
    g.mark(MarkFlags::FAN_MARK_ADD | MarkFlags::FAN_MARK_INODE | MarkFlags::FAN_MARK_IGNORED_MASK
        | MarkFlags::FAN_MARK_IGNORED_SURV_MODIFY, MaskFlags::FAN_OPEN, nix::fcntl::AT_FDCWD, Some(d)).expect("mark inode");
    let ino = std::fs::metadata(d).expect("stat").ino();
    let fdn = std::os::fd::AsRawFd::as_raw_fd(&std::os::fd::AsFd::as_fd(&g));
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fdn)).expect("fdinfo");
    let line = info.lines().find(|l| l.starts_with(&format!("fanotify ino:{:x} ", ino))).unwrap_or("").to_string();
    let field = |k: &str| line.split(' ').find_map(|w| w.strip_prefix(k)).unwrap_or("").to_string();
    let (kind, hex) = (field("fhandle-type:"), field("f_handle:"));
    std::fs::create_dir(d.join("sub")).expect("mkdir");
    std::thread::sleep(Duration::from_millis(50));
    let mut buf = vec![0u8; 8192];
    let n = nix::unistd::read(std::os::fd::AsFd::as_fd(&g), &mut buf).expect("read");
    let mut out = Vec::new();
    let _ = parse_events(&buf[..n], &mut out);
    let ev = out.iter().find_map(|c| match c {
        Change::Entry { dir, .. } => Some(dir.clone()),
        _ => None,
    }).expect("an entry event");
    let ev_hex: String = ev.handle.iter().map(|b| format!("{:02x}", b)).collect();
    let ev_kind = format!("{:x}", ev.kind);
    let same = kind == ev_kind && hex == ev_hex;
    format!("{} fdinfo type {} handle {} | event type {} handle {} | same {} | line {:?}",
        d.display(), kind, hex, ev_kind, ev_hex, same, line)
}

#[test]
#[ignore]
fn um2k_fdinfo_handle_is_the_event_handle_as_root() {
    let d = scratch("fdh_ext4");
    let a = fdinfo_handle_matches_event(&d);
    let t = scratch("fdh_tmpfs");
    assert!(sh(&format!("mount -t tmpfs none {}", t.display())), "mount tmpfs");
    let b = fdinfo_handle_matches_event(&t);
    let _ = sh(&format!("umount {}", t.display()));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&t);
    println!("handle: {}\nhandle: {}", a, b);
    assert!(a.contains("same true") && b.contains("same true"), "{}\n{}", a, b);
}
