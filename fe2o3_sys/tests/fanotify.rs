// The root arms are `#[ignore]`d: run them as root (CAP_SYS_ADMIN) on Linux 6.14+, in the VM.
#![cfg(all(target_os = "linux", feature = "fanotify"))]

use oxedyne_fe2o3_sys::fanotify::{
    parse_events,
    Access,
    Change,
    fdinfo_handle,
    Entries,
    Fid,
    Gate,
    Handle,
    Op,
};

use std::{
    ffi::OsString,
    io::{
        Read,
        Seek,
        SeekFrom,
    },
    os::fd::AsFd,
    path::PathBuf,
    time::Duration,
};

/// The test records' own handle layout (type 1, inode then generation): the parser keeps it opaque.
fn ino(f: &Fid) -> Option<u64> {
    let b = &f.handle.bytes;
    if b.len() >= 4 { Some(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as u64) } else { None }
}

fn rec(kind: u8, ino: u32, name: &str) -> Vec<u8> {
    let mut b = vec![kind, 0, 0, 0];
    b.extend_from_slice(&7i32.to_ne_bytes());
    b.extend_from_slice(&9i32.to_ne_bytes());
    b.extend_from_slice(&8u32.to_ne_bytes());
    b.extend_from_slice(&1i32.to_ne_bytes()); // FILEID_INO32_GEN
    b.extend_from_slice(&ino.to_ne_bytes());
    b.extend_from_slice(&3u32.to_ne_bytes()); // generation
    b.extend_from_slice(name.as_bytes());
    b.push(0);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    let n = b.len() as u16;
    b[2..4].copy_from_slice(&n.to_ne_bytes());
    b
}

fn event(mask: u64, recs: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = recs.concat();
    let mut b = Vec::new();
    b.extend_from_slice(&((24 + body.len()) as u32).to_ne_bytes());
    b.extend_from_slice(&[3, 0]);
    b.extend_from_slice(&24u16.to_ne_bytes());
    b.extend_from_slice(&mask.to_ne_bytes());
    b.extend_from_slice(&(-1i32).to_ne_bytes());
    b.extend_from_slice(&42i32.to_ne_bytes());
    b.extend_from_slice(&body);
    b
}

#[test]
fn test_fanotify_fdinfo_handle_is_read_from_its_mark_line() {
    let info = "pos:\t0\nflags:\t02004000\nfanotify flags:1e00 event-flags:8000\n\
        fanotify ino:80d93 sdev:800001 mflags:0 mask:0 ignored_mask:20 fhandle-bytes:8 fhandle-type:1 f_handle:930d080001000000\n";
    let h = fdinfo_handle(info, 0x80d93, 0x800001).expect("handle");
    assert_eq!((h.kind, h.bytes), (1, vec![0x93, 0x0d, 0x08, 0, 1, 0, 0, 0]));
    // The same inode number on another device is not it.
    assert!(fdinfo_handle(info, 0x80d93, 0x800002).is_err());
    // A filesystem without handles lists none.
    assert!(fdinfo_handle("fanotify ino:5 sdev:1a mflags:0 mask:0 ignored_mask:20 fhandle-bytes:0 \
        fhandle-type:ff f_handle:\n", 5, 0x1a).is_err());
}

#[test]
fn test_fanotify_parses_entry_rename_and_overflow() {
    let mut bytes = event(0x4000_0100, &[rec(2, 11, "new"), rec(1, 12, "")]);
    bytes.extend(event(0x1000_0000, &[rec(10, 11, "HEAD.lock"), rec(12, 13, "HEAD"), rec(1, 14, "")]));
    bytes.extend(event(0x4000, &[]));
    // Made and removed, merged by the kernel into one event of no known order: Recheck.
    bytes.extend(event(0x0000_0300, &[rec(2, 11, "tmp"), rec(1, 15, "")]));
    let mut out = Vec::new();
    parse_events(&bytes, &mut out).expect("parse");
    assert_eq!(out.len(), 4);
    match &out[0] {
        Change::Entry { op, dir, name, to, target, dir_entry } => {
            assert_eq!((*op, ino(dir), name.clone(), to.is_none(), *dir_entry),
                (Op::Create, Some(11), OsString::from("new"), true, true));
            assert_eq!(target.as_ref().and_then(ino), Some(12));
            assert_eq!(dir.fsid, [7, 9]);
        },
        c => panic!("{:?}", c),
    }
    match &out[1] {
        Change::Entry { op, name, to: Some((tdir, tname)), dir_entry: false, .. } => {
            assert_eq!((*op, name.clone(), ino(tdir), tname.clone()),
                (Op::Rename, OsString::from("HEAD.lock"), Some(13), OsString::from("HEAD")));
        },
        c => panic!("{:?}", c),
    }
    assert_eq!(out[2], Change::Lost);
    let ops: Vec<(Op, Option<u64>)> = out[3..].iter().map(|c| match c {
        Change::Entry { op, target, .. } => (*op, target.as_ref().and_then(ino)),
        c => panic!("{:?}", c),
    }).collect();
    assert_eq!(ops, vec![(Op::Recheck, Some(15))]);
    // A length past the buffer is refused, not read past.
    let mut bad = event(0x100, &[rec(2, 1, "x")]);
    bad[0] = 200;
    assert!(parse_events(&bad, &mut Vec::new()).is_err());
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fe2o3_fan_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn drain(e: &Entries, want: usize) -> Vec<Change> {
    let mut v = Vec::new();
    for _ in 0..40 {
        v.extend(e.next(Some(Duration::from_millis(50))).expect("next"));
        if v.len() >= want {
            break;
        }
    }
    v
}

#[test]
#[ignore]
fn test_fanotify_entries_as_root() {
    let d = scratch("ent");
    let e = Entries::filesystem(&d).expect("watch");
    let dh = e.handle_of(std::fs::File::open(&d).expect("open").as_fd()).expect("handle of d");
    std::fs::create_dir(d.join("a")).expect("mkdir");
    let a = std::fs::File::open(d.join("a")).expect("open a");
    let ah = e.handle_of(a.as_fd()).expect("handle of a");
    std::fs::rename(d.join("a"), d.join("b")).expect("rename");
    std::fs::write(d.join("HEAD.lock"), b"ref").expect("write");
    std::fs::rename(d.join("HEAD.lock"), d.join("HEAD")).expect("rename");
    std::fs::remove_dir(d.join("b")).expect("rmdir");
    let v: Vec<Change> = drain(&e, 5).into_iter().filter(|c| match c {
        Change::Entry { dir, .. } => dir.handle == dh,
        Change::Lost => true,
    }).collect();
    let seen: Vec<(Op, String, Option<String>, bool, Option<Handle>)> = v.iter().map(|c| match c {
        Change::Entry { op, name, to, dir_entry, target, .. } => (*op, name.to_string_lossy().to_string(),
            to.as_ref().map(|t| t.1.to_string_lossy().to_string()), *dir_entry,
            target.as_ref().map(|t| t.handle.clone())),
        Change::Lost => panic!("lost"),
    }).collect();
    assert!(seen.contains(&(Op::Create, "a".into(), None, true, Some(ah.clone()))), "{:?}", seen);
    assert!(seen.contains(&(Op::Rename, "a".into(), Some("b".into()), true, Some(ah.clone()))), "{:?}", seen);
    assert!(seen.iter().any(|s| s.0 == Op::Rename && s.1 == "HEAD.lock" && s.2.as_deref() == Some("HEAD")
        && !s.3), "{:?}", seen);
    assert!(seen.contains(&(Op::Delete, "b".into(), None, true, Some(ah.clone()))), "{:?}", seen);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
#[ignore]
fn test_fanotify_overflow_is_lost_as_root() {
    let q = "/proc/sys/fs/fanotify/max_queued_events";
    let was = std::fs::read_to_string(q).expect("read sysctl");
    std::fs::write(q, "16").expect("set sysctl");
    let d = scratch("ovf");
    let e = Entries::filesystem(&d);
    std::fs::write(q, was.trim()).expect("restore sysctl");
    let e = e.expect("watch");
    for i in 0..64 {
        std::fs::create_dir(d.join(format!("d{}", i))).expect("mkdir");
    }
    let v = drain(&e, 17);
    assert!(v.contains(&Change::Lost), "no overflow among {} events", v.len());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
#[ignore]
fn test_fanotify_gate_holds_first_access_as_root() {
    let d = scratch("gate");
    let p = d.join("held.txt");
    std::fs::write(&p, b"before").expect("write");
    let g = Gate::new().expect("gate");
    // An O_PATH descriptor: the holder never opens the file it holds (QA watch A-F1).
    let f = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).custom_flags(nix::fcntl::OFlag::O_PATH.bits()).open(&p).expect("open O_PATH")
    };
    g.hold(&f).expect("hold");
    let p2 = p.clone();
    let writer = std::thread::spawn(move || std::fs::write(&p2, b"after!"));
    let held = loop {
        let mut v = g.next(Some(Duration::from_secs(5))).expect("next");
        if let Some(Access::Held(h)) = v.pop() {
            break h;
        }
    };
    // The writer waits: the file still holds its old bytes, read through the event's descriptor.
    let fd = held.fd().expect("fd").try_clone_to_owned().expect("dup");
    let mut copy = std::fs::File::from(fd);
    copy.seek(SeekFrom::Start(0)).expect("seek");
    let mut s = String::new();
    copy.read_to_string(&mut s).expect("read");
    assert_eq!(s, "before");
    g.release(&f).expect("release");
    held.answer(true).expect("allow");
    writer.join().expect("join").expect("write after allow");
    assert_eq!(std::fs::read_to_string(&p).expect("read"), "after!");
    // Released: the next access is not held.
    std::fs::read(&p).expect("read again");
    assert!(g.next(Some(Duration::from_millis(200))).expect("next").is_empty());
    let _ = std::fs::remove_dir_all(&d);
}
