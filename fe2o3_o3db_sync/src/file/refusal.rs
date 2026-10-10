//! The refusal marker (2026-10-10, QA F1).  The kernel reports a write-back error to the first
//! caller that asks and to no later one, so an open refused over such an error, or a run stopped
//! by one, cannot rely on the next open being told again: a restart by systemd, or another
//! program's `syncfs` on the same filesystem, would consume the report, and the open after it
//! would replay tails that may not be on the disk.  The marker makes the refusal outlast the
//! report.  Only an operator clears it, and clearing it says that the files in doubt were checked,
//! or that the store was restored.

use crate::{
    prelude::*,
    file::zdir::{
        sync_dir,
        write_whole,
    },
    test::hooks,
};

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

    /// Leaves the marker, naming `why`, durably: through a synced temporary file, renamed, and the
    /// root synced.
    pub fn write(root: &Path, why: &str) -> Outcome<PathBuf> {
        if hooks::fault_record_fails() {
            return Err(err!("The refusal marker could not be written (test::hooks)."; IO, Write));
        }
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let text = fmt!("when {}\nwhy {}\n", secs, why.replace('\n', " "));
        write_whole(root, constant::REFUSAL_FILENAME, &text, None)
    }

    /// Leaves the marker for the open refused by `e`, and returns `e` saying whether it was left.
    pub(crate) fn refuse(root: &Path, e: Error<ErrTag>) -> Error<ErrTag> {
        match Self::write(root, &fmt!("{}", e)) {
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
    /// checks either.  A fault record left beside the marker still cuts its files at the next
    /// open.  Returns whether there was a marker.
    pub fn clear(root: &Path) -> Outcome<bool> {
        let path = Self::path(root);
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
            doubt were checked, or that the store was restored.", root);
        Ok(true)
    }
}
