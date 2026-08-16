//! One process at a time, said out loud.
//!
//! An Ozone store has no arbitration between processes. Two of them opening one
//! store both write into the live file of each zone, interleaving records and
//! losing each other's, and neither is told: the writes are appended by separate
//! file handles at separate offsets, and the index each keeps is its own. The
//! reader that comes afterwards finds a file its index does not account for.
//!
//! Nothing in the database can make that safe, because the two processes share
//! no channel to co-ordinate over. What can be done is to refuse the second one,
//! by name, at the moment it opens rather than at the moment it corrupts.
//!
//! # What is held, and how
//!
//! A `holder` file in the store's root directory, carrying the process
//! identifier of whoever holds it. It is created with `create_new`, which is one
//! atomic operation, so two processes racing to open a fresh store cannot both
//! win.
//!
//! An existing file is not simply obeyed. A process that is killed leaves its
//! holder behind, and a store nobody can open again until a file is deleted by
//! hand would be worse than no guard at all. So the identifier is checked
//! against the process table: a holder that is gone is taken over, and only a
//! holder that is running refuses.
//!
//! # What this does not do
//!
//! It does not make concurrent access work, and it is not a mutex. It converts
//! silent corruption into a refusal that names the process in the way, which is
//! the whole of the claim.
//!
//! It is also advisory across machines: a store on a shared filesystem, opened
//! from two hosts, holds two process identifiers that mean nothing to each
//! other. Ozone is not built for that and this does not make it so.

use oxedyne_fe2o3_core::prelude::*;

use std::fs;
use std::io::Write;
use std::path::{
    Path,
    PathBuf,
};


/// Name of the file recording which process holds a store.
pub const HOLDER_FILE: &str = "holder";


/// The claim one process has on one store, released when it is dropped.
///
/// Exactly one of these exists per open store however many handles share it,
/// for the reason [`crate::db::O3db`] keeps one shutdown record behind an
/// `Arc`: a claim per handle would be released by the first handle to go.
#[derive(Debug)]
pub struct Holder {
    /// The file carrying the claim.
    path:   PathBuf,
    /// The process this claim was written for, so a claim taken by somebody
    /// else in the meantime is not deleted on the way out.
    pid:    u32,
}

impl Holder {

    /// Claims a store for this process, or says who has it.
    ///
    /// Fails where the store is held by a process that is still running, naming
    /// that process and this file, because those are the two things a person
    /// needs in order to do anything about it.
    pub fn take(db_root: &Path)
        -> Outcome<Self>
    {
        let path = db_root.join(HOLDER_FILE);
        let pid = std::process::id();
        // Two attempts at most: the first may find a holder, and the second runs
        // only after that holder has been established as gone and removed. A
        // third would mean another process is racing this one for an abandoned
        // store, and the one that got there first should keep it.
        for attempt in 0..2 {
            match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    res!(file.write_all(fmt!("{}", pid).as_bytes()));
                    res!(file.flush());
                    return Ok(Self { path, pid });
                },
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(err!(e,
                    "The holder file {:?} could not be written.", path;
                IO, File, Write)),
            }
            let held = res!(Self::read_pid(&path));
            if let Some(other) = held {
                if res!(Self::is_running(other)) {
                    // Two readings, and they want opposite actions, so the
                    // message separates them rather than leaving somebody to
                    // guess at the wrong hour. A holder left by a process that
                    // died is taken over silently above and never reaches here;
                    // what reaches here is a live process, which is either the
                    // holder or an unrelated program that was given a reused
                    // identifier after a hard kill.
                    return Err(err!(
                        "The Ozone store at {:?} is held by process {}, which is running, \
                        so this process will not open it: two writers into one store lose \
                        each other's writes.\n\
                        \x20 If process {} is the program holding this store, stop it and \
                        try again.\n\
                        \x20 If it is not, this claim is stale -- an identifier is reused \
                        after a hard kill -- and removing {:?} is safe. \
                        /proc/{}/cmdline says which program it is.",
                        db_root, other, other, path, other;
                    Invalid, Conflict, File, Exists));
                }
            }
            // The holder is gone, or the file says nothing a process identifier
            // can be read from, which a half-written file would. Either way
            // nobody is using it.
            if attempt == 0 {
                match fs::remove_file(&path) {
                    Ok(())	=> (),
                    // Somebody else cleared it first, which is the outcome wanted.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(err!(e,
                        "The stale holder file {:?} could not be removed.", path;
                    IO, File, Write)),
                }
            }
        }
        Err(err!(
            "The Ozone store at {:?} could not be claimed: its holder file {:?} was taken \
            by another process while this one was clearing it.", db_root, path;
        Conflict, File, Exists))
    }

    /// Reads the process identifier a holder file carries, if it carries one.
    fn read_pid(path: &Path)
        -> Outcome<Option<u32>>
    {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            // Gone between the create and the read, which means nobody holds it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err!(e,
                "The holder file {:?} could not be read.", path;
            IO, File, Read)),
        };
        Ok(text.trim().parse::<u32>().ok())
    }

    /// Reports whether a process is running.
    ///
    /// Read from the process table rather than by signalling, so that a process
    /// belonging to another user answers as truthfully as one of this user's.
    fn is_running(pid: u32)
        -> Outcome<bool>
    {
        Ok(Path::new(&fmt!("/proc/{}", pid)).exists())
    }
}

impl Drop for Holder {
    /// Releases the claim, and leaves alone one that is no longer this
    /// process's.
    ///
    /// A claim taken over by somebody else means this process was believed dead,
    /// which happens when a process identifier is reused. Deleting the file then
    /// would strip a live holder of its claim, so the identifier is checked
    /// before the file goes.
    fn drop(&mut self) {
        if let Ok(Some(pid)) = Self::read_pid(&self.path) {
            if pid != self.pid {
                return;
            }
        }
        let _ = fs::remove_file(&self.path);
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A second claim on a store this process holds is refused, and says who
    /// holds it.
    #[test]
    fn a_store_this_process_holds_is_not_handed_out_again() -> Outcome<()> {
        let dir = std::env::temp_dir().join(fmt!(
            "o3db_holder_{}_{}", std::process::id(), 1));
        res!(fs::create_dir_all(&dir));
        let first = res!(Holder::take(&dir));
        let again = Holder::take(&dir);
        assert!(again.is_err(), "one store was claimed by two holders");
        let said = fmt!("{}", res!(again.err().ok_or_else(|| err!(
            "a refusal carried no error"; Test, Missing))).plain());
        assert!(said.contains(&fmt!("{}", std::process::id())),
            "the refusal names the holding process: {}", said);
        assert!(said.contains(HOLDER_FILE),
            "and the file to look at: {}", said);
        drop(first);
        // Released, so the next caller has it.
        let third = Holder::take(&dir);
        assert!(third.is_ok(), "a released store was not claimable");
        drop(third);
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    /// A holder left behind by a process that is gone is taken over rather than
    /// obeyed, since a store nobody can reopen would be worse than no guard.
    #[test]
    fn a_holder_left_by_a_dead_process_is_taken_over() -> Outcome<()> {
        let dir = std::env::temp_dir().join(fmt!(
            "o3db_holder_{}_{}", std::process::id(), 2));
        res!(fs::create_dir_all(&dir));
        // Process identifiers are bounded, so one past the maximum names nothing
        // and cannot come to name something while the test runs.
        let path = dir.join(HOLDER_FILE);
        res!(fs::write(&path, b"4294967295"));
        let taken = Holder::take(&dir);
        assert!(taken.is_ok(), "a stale holder was obeyed rather than taken over");
        assert_eq!(res!(Holder::read_pid(&path)), Some(std::process::id()),
            "the holder file names this process afterwards");
        drop(taken);
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }

    /// A holder file holding nothing readable is treated as abandoned, which is
    /// what a process killed midway through writing one leaves.
    #[test]
    fn a_half_written_holder_is_treated_as_abandoned() -> Outcome<()> {
        let dir = std::env::temp_dir().join(fmt!(
            "o3db_holder_{}_{}", std::process::id(), 3));
        res!(fs::create_dir_all(&dir));
        res!(fs::write(dir.join(HOLDER_FILE), b""));
        let taken = Holder::take(&dir);
        assert!(taken.is_ok(), "an unreadable holder was obeyed");
        drop(taken);
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }
}
