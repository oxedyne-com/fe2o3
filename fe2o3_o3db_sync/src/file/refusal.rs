//! The refusal marker (2026-10-10, QA F1).  The kernel reports a write-back error to the first
//! caller that asks and to no later one, so an open refused over such an error, or a run stopped
//! by one, cannot rely on the next open being told again: a restart by systemd, or another
//! program's `syncfs` on the same filesystem, would consume the report, and the open after it
//! would replay tails that may not be on the disk.  The marker makes the refusal outlast the
//! report.  Only an operator clears it, and clearing it says that the files in doubt were checked,
//! or that the store was restored.

use crate::{
    prelude::*,
    comm::channels::Cut,
    file::zdir::{
        sync_dir,
        write_whole,
    },
    test::hooks,
};

use oxedyne_fe2o3_core::file::ExclusiveLock;

use std::{
    path::{
        Path,
        PathBuf,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

/// The marker in a store's root that refuses every open of the store while it stands.
pub struct Refusal;

impl Refusal {
    pub fn path(root: &Path) -> PathBuf {
        root.join(constant::REFUSAL_FILENAME)
    }

    /// Leaves the marker, naming `why` and every file in doubt in `cuts`, durably: through a
    /// synced temporary file, renamed, and the root synced.  The cut lines are the fault
    /// record's, so the marker alone can stand in for a record that was lost or never written.
    pub fn write(root: &Path, why: &str, cuts: &[Cut]) -> Outcome<PathBuf> {
        if hooks::fault_record_fails() {
            return Err(err!("The refusal marker could not be written (test::hooks)."; IO, Write));
        }
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut text = fmt!("when {}\nwhy {}\n", secs, why.replace('\n', " "));
        for c in cuts {
            text.push_str(&c.line());
            text.push('\n');
        }
        write_whole(root, constant::REFUSAL_FILENAME, &text, None)
    }

    /// Leaves the marker for the open refused by `e`, and returns `e` saying whether it was left.
    pub(crate) fn refuse(root: &Path, e: Error<ErrTag>) -> Error<ErrTag> {
        match Self::write(root, &e.plain(), &[]) {
            Ok(path) => err!(e,
                "The refusal is kept in {:?}, so every later open of the store is refused too, \
                since the kernel will not report the error again.  Once the files in doubt are \
                checked, or the store is restored, clear it with `Refusal::clear` \
                (`o3db_verify --clear-refusal --o3db DIR`).", path;
                Init, File, Fatal),
            Err(w) => err!(e,
                "CRITICAL: the refusal marker could not be written either ({}), so a later open \
                may be told nothing of the error and replay what is not on the disk.  Do not \
                open the store at {:?} again before its files are checked, or it is restored.",
                w, root;
                Init, File, Fatal),
        }
    }

    /// Refuses the open of the store at `root` while its marker stands.
    pub(crate) fn check(root: &Path) -> Outcome<()> {
        let path = Self::path(root);
        if !path.exists() {
            return Ok(());
        }
        let why = match std::fs::read_to_string(&path) {
            Ok(text)    => text.lines()
                .find_map(|l| l.strip_prefix("why ").map(String::from))
                .unwrap_or_default(),
            Err(e)      => fmt!("(the marker cannot be read: {})", e),
        };
        Err(err!(
            "The store at {:?} refuses to open while {:?} stands: {}  The kernel reports a \
            write-back error only once, so this open could not see it again and would replay \
            what may not be on the disk.  Once the files in doubt are checked, or the store is \
            restored, clear the refusal with `Refusal::clear` (`o3db_verify --clear-refusal \
            --o3db DIR`).", root, path, why;
            Init, File, Fatal))
    }

    /// The operator's verb: removes the marker, durably, so the store opens again.  Calling it
    /// says that the files in doubt were checked, or that the store was restored; nothing here
    /// checks either.  The marker's cuts are first folded into the fault record, durably, so the
    /// next open still cuts every file in doubt even when the record had been lost (QA r2 F2).
    /// It takes the store's lock, and refuses while another open holds the store, whose files
    /// may still change.  Returns whether there was a marker.
    pub fn clear(root: &Path) -> Outcome<bool> {
        let path = Self::path(root);
        if !path.exists() {
            return Ok(false);
        }
        let lock_path = root.join(constant::STORE_LOCK_FILENAME);
        let _lock = match res!(ExclusiveLock::try_acquire(&lock_path)) {
            Some(l) => l,
            None => return Err(err!(
                "The refusal of the Ozone store at {:?} is not cleared: the store is held by \
                another open, in another process or this one ({:?} is locked), whose files may \
                still change.  Stop the one holding it, then clear.", root, lock_path;
                Init, Lock, Conflict)),
        };
        let marker = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(err!(e,
                "Cannot read the refusal marker {:?} to fold its cuts into the fault record.", path;
                IO, File, Read)),
        };
        let folded = res!(Self::fold(root, &marker));
        match std::fs::remove_file(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(err!(e,
                "Cannot remove the refusal marker {:?}.", path;
                IO, File, Write)),
        }
        res!(sync_dir(root));
        warn!(sync_log::stream(),
            "The refusal of the store at {:?} was cleared: the operator holds that the files in \
            doubt were checked, or that the store was restored.  The next open cuts these files \
            in doubt (zone, file number, data and index marks): {:?}", root, folded);
        Ok(true)
    }

    // Copies the cut lines of the marker text `marker` that the fault record lacks into the
    // record, written whole, and returns every cut line the record now holds.
    fn fold(root: &Path, marker: &str) -> Outcome<Vec<String>> {
        let cuts: Vec<&str> = marker.lines().filter(|l| l.starts_with("cut ")).collect();
        let rpath = root.join(constant::FAULT_FILENAME);
        let mut text = match std::fs::read_to_string(&rpath) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if cuts.is_empty() {
                    return Ok(Vec::new());
                }
                // The record was lost, or never written: the marker's own lines head it.
                let mut head = String::new();
                for l in marker.lines() {
                    if let Some(w) = l.strip_prefix("when ") {
                        head.push_str(&fmt!("when {}\n", w));
                    } else if let Some(w) = l.strip_prefix("why ") {
                        head.push_str(&fmt!("error {}\n", w));
                    }
                }
                head
            },
            Err(e) => return Err(err!(e,
                "Cannot read the fault record {:?} to fold the refusal marker's cuts into it, so \
                the marker is kept.", rpath;
                IO, File, Read)),
        };
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        let mut added = false;
        for l in &cuts {
            if !text.lines().any(|r| r == *l) {
                text.push_str(l);
                text.push('\n');
                added = true;
            }
        }
        if added {
            if let Err(e) = write_whole(root, constant::FAULT_FILENAME, &text, None) {
                return Err(err!(e,
                    "Cannot fold the refusal marker's cuts into the fault record {:?}, so the \
                    marker is kept.", rpath;
                    IO, File, Write));
            }
        }
        Ok(text.lines().filter(|l| l.starts_with("cut ")).map(String::from).collect())
    }
}
