//! Tests for `fe2o3_sys::inotify` against the real kernel: real directories, real `mv`
//! and `rm`, a real queue overflow and a real descriptor limit. The oracle for what is
//! watched is `/proc/self/fdinfo/<fd>`, the kernel's own list of an instance's watches,
//! compared with the inodes `stat` reports for the directories that should be watched.
//! Only the per-user limits are never reached for real, since Syncthing and the desktop
//! share them; the budget stands in for the watch count and `prlimit` for the descriptors.

#![cfg(all(target_os = "linux", feature = "inotify"))]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::inotify::{
    Limit,
    Opened,
    Renamed,
    Wake,
    Watch,
};

use std::{
    collections::BTreeSet,
    fs::{
        self,
        File,
    },
    os::fd::AsRawFd,
    path::{
        Path,
        PathBuf,
    },
    process::Command,
    sync::{
        Mutex,
        MutexGuard,
    },
    time::{
        Duration,
        Instant,
    },
};

const CHILD:    &str        = "FE2O3_SYS_INO_CHILD";
const LONG:     Duration    = Duration::from_secs(5);
const QUIET:    Duration    = Duration::from_millis(300);

static LOCK: Mutex<()> = Mutex::new(());

// Every test counts watches in this process, so they take turns.
fn serial() -> MutexGuard<'static, ()> {
    match LOCK.lock() {
        Ok(g)   => g,
        Err(p)  => p.into_inner(),
    }
}

fn io<T>(r: std::io::Result<T>, what: &str) -> Outcome<T> {
    match r {
        Ok(v)   => Ok(v),
        Err(e)  => Err(err!("{}: {}.", what, e; IO, File)),
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Outcome<Self> {
        let dir = std::env::temp_dir().join(format!("fe2o3-sys-ino-t-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        res!(io(fs::create_dir_all(&dir), "make the scratch directory"));
        Ok(Self(dir))
    }

    fn root(&self) -> PathBuf {
        self.0.join("root")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A test may leave a directory it closed to itself.
        let _ = Command::new("chmod").arg("-R").arg("u+rwx").arg(&self.0).status();
        let _ = fs::remove_dir_all(&self.0);
    }
}

// The directories of a test tree, made under the scratch directory.
fn mkdirs(base: &Path, dirs: &[&str]) -> Outcome<()> {
    for d in dirs {
        res!(io(fs::create_dir_all(base.join(d)), d));
    }
    Ok(())
}

fn touch(path: &Path) -> Outcome<()> {
    res!(io(File::create(path), "create a file"));
    Ok(())
}

fn sh(dir: &Path, script: &str) -> Outcome<()> {
    let out = res!(io(Command::new("sh").arg("-c").arg(script).current_dir(dir).output(), script));
    if !out.status.success() {
        return Err(err!(
            "`{}` failed: {}", script, String::from_utf8_lossy(&out.stderr); Test));
    }
    Ok(())
}

// Directories a tree's owner would not read, by any name along the path.
fn admit(path: &[u8]) -> bool {
    !path.split(|b| *b == b'/').any(|c|
        matches!(c, b".git" | b".ore" | b"target" | b"skip"))
}

fn all(_: &[u8]) -> bool {
    true
}

fn open<F>(root: &Path, budget: usize, admit: F) -> Outcome<Watch>
    where F: FnMut(&[u8]) -> bool,
{
    match res!(Watch::open(root, budget, admit)) {
        Opened::Watching(w)     => Ok(w),
        Opened::Exhausted(l)    => Err(err!("Expected a watch, but the tree is exhausted: {:?}.", l; Test)),
    }
}

fn fdinfo(fd: i32) -> Outcome<String> {
    io(fs::read_to_string(format!("/proc/self/fdinfo/{}", fd)), "read fdinfo")
}

// The inodes the kernel says an instance watches.
fn watched(w: &Watch) -> Outcome<BTreeSet<u64>> {
    let body = res!(fdinfo(w.as_raw_fd()));
    let mut set = BTreeSet::new();
    for line in body.lines().filter(|l| l.starts_with("inotify wd:")) {
        let tok = res!(line.split_whitespace().find(|t| t.starts_with("ino:"))
            .ok_or_else(|| err!("No ino in fdinfo line {:?}.", line; Test)));
        set.insert(res!(io(u64::from_str_radix(&tok[4..], 16).map_err(|e|
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)), "parse an inode")));
    }
    Ok(set)
}

// The inodes `stat` reports for the given paths.
fn inodes(paths: &[PathBuf]) -> Outcome<BTreeSet<u64>> {
    let out = res!(io(Command::new("stat").arg("-c").arg("%i").args(paths).output(), "stat"));
    if !out.status.success() {
        return Err(err!("stat failed: {}", String::from_utf8_lossy(&out.stderr); Test));
    }
    let mut set = BTreeSet::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        set.insert(res!(io(line.trim().parse::<u64>().map_err(|e|
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)), "parse a stat inode")));
    }
    Ok(set)
}

// Exactly the root and the named directories hold watches, as the kernel lists them.
fn assert_placed(w: &Watch, root: &Path, rel: &[&str]) -> Outcome<()> {
    let mut paths = vec![root.to_path_buf()];
    for r in rel {
        paths.push(root.join(r));
    }
    let want = res!(inodes(&paths));
    let got = res!(watched(w));
    assert_eq!(got, want, "kernel's watch list against the admitted directories");
    assert_eq!(w.watches(), want.len(), "the watcher's own count");
    Ok(())
}

// Every watch held by every inotify instance of this process.
fn process_watches() -> Outcome<usize> {
    let mut n = 0;
    for ent in res!(io(fs::read_dir("/proc/self/fd"), "list descriptors")) {
        let ent = res!(io(ent, "read a descriptor entry"));
        match fs::read_link(ent.path()) {
            Ok(t) if t == Path::new("anon_inode:inotify") => {
                let body = res!(io(fs::read_to_string(
                    format!("/proc/self/fdinfo/{}", ent.file_name().to_string_lossy())), "read fdinfo"));
                n += body.lines().filter(|l| l.starts_with("inotify wd:")).count();
            },
            _ => (),
        }
    }
    Ok(n)
}

fn ren(from: &str, to: &str, dir: bool) -> Renamed {
    Renamed { from: from.as_bytes().to_vec(), to: to.as_bytes().to_vec(), dir }
}

fn moved(pairs: Vec<Renamed>) -> Wake {
    Wake::Moved(pairs)
}

// 30 directories to be watched, and the ones that are not.
fn big_tree(root: &Path) -> Outcome<Vec<String>> {
    let mut admitted = Vec::new();
    for i in 0..5 {
        admitted.push(format!("d{}", i));
        for j in 0..5 {
            admitted.push(format!("d{}/s{}", i, j));
        }
    }
    for a in &admitted {
        res!(io(fs::create_dir_all(root.join(a)), a));
    }
    res!(mkdirs(root, &[".git/objects/pack", ".ore/store", "target/debug/deps", "d0/target/inner"]));
    res!(mkdirs(root.parent().unwrap_or(root), &["outside/sub"]));
    res!(sh(root, "ln -s d1 link_in && ln -s ../outside link_out && ln -s ../outside/sub deep_link"));
    Ok(admitted)
}

#[test]
fn placement_is_exactly_the_admitted_directories() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("place"));
    let root = s.root();
    res!(mkdirs(&root, &[""]));
    let admitted = res!(big_tree(&root));
    assert_eq!(admitted.len(), 30);
    let mut asked: Vec<Vec<u8>> = Vec::new();
    let w = res!(open(&root, 8_192, |p| { asked.push(p.to_vec()); admit(p) }));
    let rel: Vec<&str> = admitted.iter().map(|a| a.as_str()).collect();
    res!(assert_placed(&w, &root, &rel));
    // The predicate is never asked about the root, nor about anything inside a directory
    // it refused, nor about a link, whatever the link leads to.
    assert!(asked.iter().all(|p| !p.is_empty()));
    assert!(asked.iter().all(|p| p.split(|b| *b == b'/').skip(1).all(|c| c != b"inner")));
    assert!(asked.iter().all(|p| !p.starts_with(b".git/") && !p.starts_with(b"target/")));
    assert!(asked.iter().all(|p| p != b"link_in" && p != b"link_out" && p != b"deep_link"));
    assert!(asked.contains(&b"target".to_vec()) && asked.contains(&b"d0/target".to_vec()));
    // Nothing the links lead to is watched, so writing through them is silent.
    let mut w = w;
    res!(sh(&root, "touch link_out/a link_out/sub/b deep_link/c link_in/d"));
    assert_eq!(res!(w.wait(QUIET, admit)), moved(Vec::new()), "link_in/d is inside the tree");
    res!(sh(&root, "touch link_out/e deep_link/f"));
    assert_eq!(res!(w.wait(QUIET, admit)), Wake::Timeout);
    res!(sh(&root, "touch .git/x .ore/y target/z d0/target/w"));
    assert_eq!(res!(w.wait(QUIET, admit)), Wake::Timeout);
    res!(assert_placed(&w, &root, &rel));
    Ok(())
}

#[test]
fn a_directory_closed_to_the_process_is_skipped() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("locked"));
    let root = s.root();
    res!(mkdirs(&root, &["open/in", "locked/in"]));
    res!(sh(&root, "chmod 000 locked"));
    let w = res!(open(&root, 8_192, all));
    res!(assert_placed(&w, &root, &["open", "open/in"]));
    Ok(())
}

#[test]
fn a_tree_made_in_a_flash_is_placed_whole() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("race"));
    let root = s.root();
    res!(mkdirs(&root, &[""]));
    let mut w = res!(open(&root, 8_192, all));
    res!(sh(&root, "mkdir -p a/b/c && touch a/b/c/f"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    res!(assert_placed(&w, &root, &["a", "a/b", "a/b/c"]));
    // The tree built in that flash is as live as any other.
    res!(sh(&root, "touch a/b/c/g"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    Ok(())
}

#[test]
fn a_file_renamed_in_place_is_paired() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("mvfile"));
    let root = s.root();
    res!(mkdirs(&root, &["d"]));
    res!(touch(&root.join("x")));
    let mut w = res!(open(&root, 8_192, all));
    res!(sh(&root, "mv x y"));
    assert_eq!(res!(w.wait(LONG, all)), moved(vec![ren("x", "y", false)]));
    Ok(())
}

#[test]
fn a_file_moved_into_a_subdirectory_is_paired() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("mvinto"));
    let root = s.root();
    res!(mkdirs(&root, &["d"]));
    res!(touch(&root.join("x")));
    let mut w = res!(open(&root, 8_192, all));
    res!(sh(&root, "mv x d/y"));
    assert_eq!(res!(w.wait(LONG, all)), moved(vec![ren("x", "d/y", false)]));
    Ok(())
}

#[test]
fn a_directory_rename_moves_the_prefix_of_what_follows() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("mvdir"));
    let root = s.root();
    res!(mkdirs(&root, &["d/sub"]));
    res!(touch(&root.join("d/z")));
    let mut w = res!(open(&root, 8_192, all));
    res!(sh(&root, "mv d e"));
    assert_eq!(res!(w.wait(LONG, all)), moved(vec![ren("d", "e", true)]));
    res!(assert_placed(&w, &root, &["e", "e/sub"]));
    // The watch on the moved directory reports under its new name.
    res!(sh(&root, "mv e/z e/w"));
    assert_eq!(res!(w.wait(LONG, all)), moved(vec![ren("e/z", "e/w", false)]));
    res!(sh(&root, "mv e/sub/ e/sub2 && touch e/sub2/q"));
    assert_eq!(res!(w.wait(LONG, all)), moved(vec![ren("e/sub", "e/sub2", true)]));
    res!(assert_placed(&w, &root, &["e", "e/sub2"]));
    Ok(())
}

#[test]
fn renames_in_one_read_each_name_the_paths_of_their_moment() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("seq"));
    let root = s.root();
    res!(mkdirs(&root, &["d", "a", "b"]));
    res!(touch(&root.join("d/z")));
    res!(touch(&root.join("b/f")));
    let mut w = res!(open(&root, 8_192, all));
    // Both moves are queued before the first read.
    res!(sh(&root, "mv d e && mv e/z e/w"));
    assert_eq!(res!(w.wait(LONG, all)),
        moved(vec![ren("d", "e", true), ren("e/z", "e/w", false)]));
    // A swap through a third name, then a file renamed inside what is now `a`.
    res!(sh(&root, "mv a t && mv b a && mv t b && mv a/f a/g"));
    assert_eq!(res!(w.wait(LONG, all)), moved(vec![
        ren("a", "t", true),
        ren("b", "a", true),
        ren("t", "b", true),
        ren("a/f", "a/g", false),
    ]));
    res!(assert_placed(&w, &root, &["e", "a", "b"]));
    Ok(())
}

#[test]
fn a_directory_moved_out_loses_its_watches() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("out"));
    let root = s.root();
    res!(mkdirs(&root, &["d/sub", "keep"]));
    let mut w = res!(open(&root, 8_192, all));
    res!(assert_placed(&w, &root, &["d", "d/sub", "keep"]));
    res!(sh(&s.0, "mv root/d out"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    res!(assert_placed(&w, &root, &["keep"]));
    // Writes where it went are not the tree's business.
    res!(sh(&s.0, "touch out/f out/sub/g"));
    assert_eq!(res!(w.wait(QUIET, all)), Wake::Timeout);
    res!(assert_placed(&w, &root, &["keep"]));
    Ok(())
}

#[test]
fn a_directory_moved_in_is_watched_whole() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("in"));
    let root = s.root();
    res!(mkdirs(&root, &[""]));
    res!(mkdirs(&s.0, &["in/p/q", "in/r"]));
    let mut w = res!(open(&root, 8_192, all));
    res!(sh(&s.0, "mv in root/d2"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    res!(assert_placed(&w, &root, &["d2", "d2/p", "d2/p/q", "d2/r"]));
    res!(sh(&root, "touch d2/p/q/f"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    Ok(())
}

#[test]
fn a_directory_renamed_to_an_excluded_name_is_released() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("toskip"));
    let root = s.root();
    res!(mkdirs(&root, &["d/sub", "skip"]));
    let mut w = res!(open(&root, 8_192, admit));
    res!(assert_placed(&w, &root, &["d", "d/sub"]));
    // Both ends of this rename are watched, so the kernel pairs it, but the new name is
    // one the tree's owner does not read.
    res!(sh(&root, "mv d target"));
    assert_eq!(res!(w.wait(LONG, admit)), moved(vec![ren("d", "target", true)]));
    res!(assert_placed(&w, &root, &[]));
    res!(sh(&root, "touch target/f target/sub/g"));
    assert_eq!(res!(w.wait(QUIET, admit)), Wake::Timeout);
    // And back to a name it reads, which watches the directory afresh.
    res!(sh(&root, "mv target d"));
    assert_eq!(res!(w.wait(LONG, admit)), moved(vec![ren("target", "d", true)]));
    res!(assert_placed(&w, &root, &["d", "d/sub"]));
    // Into an unwatched directory only the departure is seen, and nothing pairs.
    res!(sh(&root, "mv d skip/d"));
    assert_eq!(res!(w.wait(LONG, admit)), moved(Vec::new()));
    res!(assert_placed(&w, &root, &[]));
    res!(sh(&root, "touch skip/d/f skip/d/sub/g"));
    assert_eq!(res!(w.wait(QUIET, admit)), Wake::Timeout);
    res!(sh(&root, "mv skip/d d"));
    assert_eq!(res!(w.wait(LONG, admit)), moved(Vec::new()));
    res!(assert_placed(&w, &root, &["d", "d/sub"]));
    Ok(())
}

#[test]
fn a_directory_deleted_and_made_again_is_watched_afresh() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("again"));
    let root = s.root();
    res!(mkdirs(&root, &["d/old", "e/deep/er"]));
    let mut w = res!(open(&root, 8_192, all));
    res!(sh(&root, "rmdir d/old d && mkdir d d/n"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    res!(assert_placed(&w, &root, &["d", "d/n", "e", "e/deep", "e/deep/er"]));
    res!(sh(&root, "rm -r e"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    res!(assert_placed(&w, &root, &["d", "d/n"]));
    Ok(())
}

#[test]
fn a_flooded_queue_reads_as_look_now_and_the_tree_is_placed_again() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("flood"));
    let root = s.root();
    res!(mkdirs(&root, &["pre"]));
    let mut w = res!(open(&root, 8_192, all));
    let max: usize = res!(io(fs::read_to_string("/proc/sys/fs/inotify/max_queued_events"), "read the queue size"))
        .trim().parse().unwrap_or(16_384);
    // Each file is two events, a create and a close, so this overruns the queue twice.
    for i in 0..(max + 1_000) {
        res!(touch(&root.join(format!("f{}", i))));
    }
    // Made once the queue is full, so the kernel drops what announces them.
    res!(sh(&root, "mkdir -p late/sub && touch late/sub/f"));
    assert_eq!(res!(w.wait(LONG, all)), Wake::LookNow);
    res!(assert_placed(&w, &root, &["pre", "late", "late/sub"]));
    assert_eq!(res!(w.wait(QUIET, all)), Wake::Timeout);
    res!(sh(&root, "touch late/sub/g"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    Ok(())
}

#[test]
fn a_budget_too_small_for_the_tree_releases_it_at_open() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("budget-open"));
    let root = s.root();
    res!(mkdirs(&root, &["a/b", "c/d", "e/f", "g"]));
    assert_eq!(res!(process_watches()), 0);
    // Root and seven directories, against a budget of five.
    match res!(Watch::open(&root, 5, all)) {
        Opened::Exhausted(Limit::Budget) => (),
        other => return Err(err!("Expected Exhausted(Budget), got {:?}.", other; Test)),
    }
    assert_eq!(res!(process_watches()), 0, "no instance of this process keeps a watch");
    // The same tree fits a budget of eight, to the watch.
    let w = res!(open(&root, 8, all));
    assert_eq!(w.watches(), 8);
    assert_eq!(res!(process_watches()), 8);
    Ok(())
}

#[test]
fn a_tree_that_outgrows_its_budget_is_released_and_stays_released() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("budget-wait"));
    let root = s.root();
    res!(mkdirs(&root, &["a", "b"]));
    let mut w = res!(open(&root, 5, all));
    assert_eq!(res!(watched(&w)).len(), 3);
    res!(mkdirs(&root, &["c", "d", "e/f"]));
    assert_eq!(res!(w.wait(LONG, all)), Wake::Exhausted(Limit::Budget));
    assert_eq!(res!(watched(&w)).len(), 0, "the kernel lists no watch");
    assert_eq!(w.watches(), 0);
    // It is a tree's last word; nothing is placed behind the caller's back.
    res!(sh(&root, "touch c/x"));
    assert_eq!(res!(w.wait(QUIET, all)), Wake::Exhausted(Limit::Budget));
    assert_eq!(res!(watched(&w)).len(), 0);
    Ok(())
}

// Re-run by the test below under a descriptor limit, in one of two modes.
fn exhausted_child(mode: &str) -> Outcome<()> {
    let s = res!(Scratch::new(&format!("emfile-{}", mode)));
    let root = s.root();
    res!(mkdirs(&root, &["a/b", "c"]));
    let mut held = Vec::new();
    while let Ok(f) = File::open("/dev/null") {
        held.push(f);
    }
    if mode == "list" {
        // One descriptor left: enough for the instance, not for listing a directory.
        held.pop();
    }
    let opened = res!(Watch::open(&root, 8_192, all));
    drop(held);
    match opened {
        Opened::Exhausted(Limit::Instances) => Ok(()),
        other => Err(err!("Expected Exhausted(Instances), got {:?}.", other; Test)),
    }
}

#[test]
fn running_out_of_descriptors_reads_as_instances() -> Outcome<()> {
    if let Ok(mode) = std::env::var(CHILD) {
        return exhausted_child(&mode);
    }
    let _g = serial();
    let exe = res!(io(std::env::current_exe(), "find this test binary"));
    for mode in ["init", "list"] {
        // A real EMFILE from the kernel, under a real descriptor limit.
        let out = res!(io(Command::new("prlimit")
            .arg("--nofile=64:64")
            .arg(&exe)
            .args(["--exact", "running_out_of_descriptors_reads_as_instances",
                "--test-threads=1", "--nocapture"])
            .env(CHILD, mode)
            .output(), "run prlimit"));
        assert!(out.status.success(),
            "child in mode {} failed:\n{}\n{}", mode,
            String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"),
            "child in mode {} ran no test:\n{}", mode, String::from_utf8_lossy(&out.stdout));
    }
    Ok(())
}

#[test]
fn a_still_tree_times_out_on_time() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("still"));
    let root = s.root();
    res!(mkdirs(&root, &["a/b"]));
    let mut w = res!(open(&root, 8_192, all));
    let t = Instant::now();
    assert_eq!(res!(w.wait(Duration::from_millis(300), all)), Wake::Timeout);
    let took = t.elapsed();
    println!("a wait of 300ms on a still tree took {:?}", took);
    assert!(took >= Duration::from_millis(300) && took < Duration::from_millis(500),
        "waited {:?}", took);
    // And a zero wait looks and returns.
    let t = Instant::now();
    assert_eq!(res!(w.wait(Duration::ZERO, all)), Wake::Timeout);
    assert!(t.elapsed() < Duration::from_millis(100));
    Ok(())
}

#[test]
fn writes_inside_an_excluded_directory_are_silent() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("excluded"));
    let root = s.root();
    res!(mkdirs(&root, &["skip/deep", "keep"]));
    let mut w = res!(open(&root, 8_192, admit));
    res!(assert_placed(&w, &root, &["keep"]));
    res!(sh(&root, "touch skip/a skip/deep/b && mv skip/a skip/deep/c && rm skip/deep/b"));
    assert_eq!(res!(w.wait(QUIET, admit)), Wake::Timeout);
    // A write in a watched directory does wake it, so the silence above was earned.
    res!(sh(&root, "touch keep/a"));
    assert_eq!(res!(w.wait(LONG, admit)), moved(Vec::new()));
    // A new directory the owner reads is watched; one the owner does not is seen and left.
    res!(sh(&root, "mkdir skip2 && mkdir target"));
    assert_eq!(res!(w.wait(LONG, admit)), moved(Vec::new()));
    res!(assert_placed(&w, &root, &["keep", "skip2"]));
    Ok(())
}

#[test]
fn deleting_the_root_reads_as_root_gone() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("rootgone"));
    let root = s.root();
    res!(mkdirs(&root, &["a/b", "c"]));
    let mut w = res!(open(&root, 8_192, all));
    res!(sh(&s.0, "rm -r root"));
    assert_eq!(res!(w.wait(LONG, all)), Wake::RootGone);
    assert_eq!(res!(watched(&w)).len(), 0);
    assert_eq!(w.watches(), 0);
    assert_eq!(res!(w.wait(QUIET, all)), Wake::RootGone);
    Ok(())
}

#[test]
fn moving_the_root_away_reads_as_root_gone() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("rootmoved"));
    let root = s.root();
    res!(mkdirs(&root, &["a"]));
    let mut w = res!(open(&root, 8_192, all));
    res!(sh(&s.0, "mv root elsewhere"));
    assert_eq!(res!(w.wait(LONG, all)), Wake::RootGone);
    assert_eq!(res!(watched(&w)).len(), 0);
    // A new directory at the old path is for a new watch to place.
    res!(mkdirs(&root, &["a"]));
    assert_eq!(res!(w.wait(QUIET, all)), Wake::RootGone);
    let w2 = res!(open(&root, 8_192, all));
    res!(assert_placed(&w2, &root, &["a"]));
    Ok(())
}

#[test]
fn a_root_named_by_a_link_is_watched_through_it() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("rootlink"));
    let root = s.root();
    res!(mkdirs(&root, &["a"]));
    res!(sh(&s.0, "ln -s root rootlink"));
    let mut w = res!(open(&s.0.join("rootlink"), 8_192, all));
    res!(assert_placed(&w, &root, &["a"]));
    res!(sh(&root, "touch a/f"));
    assert_eq!(res!(w.wait(LONG, all)), moved(Vec::new()));
    Ok(())
}

#[test]
fn a_root_that_is_not_a_directory_is_refused() -> Outcome<()> {
    let _g = serial();
    let s = res!(Scratch::new("badroot"));
    res!(touch(&s.0.join("file")));
    assert!(Watch::open(&s.0.join("file"), 8_192, all).is_err());
    assert!(Watch::open(&s.0.join("missing"), 8_192, all).is_err());
    assert_eq!(res!(process_watches()), 0);
    Ok(())
}
