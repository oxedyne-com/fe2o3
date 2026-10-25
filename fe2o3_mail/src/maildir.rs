//! Maildir-backed implementation of `MailStore`.
//!
//! Storage layout follows the Dovecot/Maildir++ convention so a freshly
//! deployed Hematite instance can take over an existing Dovecot
//! mailbox tree without copying data:
//!
//! ```text
//! <root>/<domain>/<local>/
//!   cur/                INBOX cur (already-seen messages)
//!   new/                INBOX new (unprocessed deliveries)
//!   tmp/                INBOX tmp (writes in progress)
//!   .Sent/cur/          sub-folder using Maildir++ '.' prefix
//!   .Sent/new/
//!   ...
//!   dovecot-uidvalidity            8-byte hex, the latest UIDVALIDITY issued
//!   dovecot-uidvalidity.<hex>      empty companion file, exactly one
//!   dovecot-uidlist                "3 V<validity> N<next>" then "<uid> :<key>" rows
//!   hematite-folder.lock           flock'd across every change to the folder
//!   hematite-subscriptions.lock    flock'd across a change to subscriptions
//!   subscriptions                  one folder name per line
//! ```
//!
//! 2026-10-26 (F2a): a UID names one message for as long as its
//! UIDVALIDITY lasts. Every reading of a folder, delivery, flag change
//! and expunge holds an exclusive `flock` on the folder's lock file, so
//! IMAP sessions and SMTP delivery, in threads or in processes, never
//! interleave a read-modify-write of the uidlist. The uidlist is
//! replaced whole (temporary file, fsync, rename), a failed write is an
//! error, UIDNEXT only moves forward, and a uidlist that is missing or
//! damaged is answered with a new, larger UIDVALIDITY rather than a
//! silent renumbering under the old one.
//!
//! A deliberate UIDVALIDITY bump is the same path: with every process
//! that serves the folder stopped, remove its `dovecot-uidlist`. The
//! next reading issues a larger UIDVALIDITY, leaves its one marker, and
//! numbers every message afresh from 1 in delivery (file name) order;
//! `a_uidvalidity_bump_leaves_one_marker_and_numbers_afresh` holds it so.

use oxedyne_fe2o3_core::{
    prelude::*,
    file::{
        ExclusiveLock,
        SaveMode,
        save_atomic,
        sync_parent_dir,
    },
};
use oxedyne_fe2o3_net::mail::store::{
    FlagUpdate,
    FolderName,
    FolderStatus,
    MailStore,
    MailUser,
    MessageFlags,
    MessageMeta,
    MessageUid,
};

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fs::{
        self,
        File,
        OpenOptions,
    },
    io::{
        ErrorKind,
        Read,
        Write,
    },
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        atomic::{
            AtomicU64,
            Ordering,
        },
    },
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};


const UIDLIST_NAME:         &str    = "dovecot-uidlist";
const UIDVALIDITY_PREFIX:   &str    = "dovecot-uidvalidity";
const SUBSCRIPTIONS_NAME:   &str    = "subscriptions";
const MAILDIRFOLDER_NAME:   &str    = "maildirfolder";      // marks a Maildir++ sub-folder
const LOCK_NAME:            &str    = "hematite-folder.lock";
const SUBS_LOCK_NAME:       &str    = "hematite-subscriptions.lock"; // at the user root
const LOCK_WAIT:            Duration = Duration::from_secs(30);     // then a Timeout error
const NAME_TRIES:           u64     = 16;                   // fresh message names tried in tmp/
static NAME_SEQ:            AtomicU64 = AtomicU64::new(0);  // the Maildir `Q<n>` delivery counter


/// Maildir-backed `MailStore`.
///
/// Holds a single root directory (typically `/var/mail/vhosts`) and
/// nothing else: every operation derives its on-disk paths from the
/// `MailUser::delivery_key` it is handed.
#[derive(Clone, Debug)]
pub struct MaildirStore {
    root:       Arc<PathBuf>,   // holds the `<domain>/<local>/` trees
    hostname:   Arc<String>,    // in every message name, `/` and `:` escaped
    lock_wait:  Duration,       // longest wait for a folder's or the subscriptions' lock
}

/// A message on disk with the UID the folder's uidlist gives it.
#[derive(Clone, Debug)]
struct Entry {
    meta:   MessageMeta,
    path:   PathBuf,
    key:    String,     // the file name less its `:2,` flags, the uidlist key
}

/// One reading of a folder, taken under its lock.
#[derive(Debug)]
struct FolderView {
    entries:        Vec<Entry>, // UID order
    uid_validity:   u32,
    uid_next:       u32,
    recent:         u32,
}

impl FolderView {
    fn find(&self, uid: MessageUid) -> Option<&Entry> {
        match self.entries.binary_search_by_key(&uid.0, |e| e.meta.uid.0) {
            Ok(i)   => self.entries.get(i),
            Err(_)  => None,
        }
    }

    fn messages(&self) -> Vec<MessageMeta> {
        self.entries.iter().map(|e| e.meta.clone()).collect()
    }
}

/// What a folder's `dovecot-uidlist` holds.
#[derive(Debug, PartialEq)]
enum Uidlist {
    Missing,
    Garbled,
    Valid {
        validity:   u32,
        next:       u32,
        map:        BTreeMap<String, u32>,
    },
}

impl MaildirStore {
    /// Build a new store rooted at `root`. The directory must exist.
    pub fn new(root: PathBuf, hostname: impl Into<String>) -> Outcome<Self> {
        if !root.is_dir() {
            return Err(err!(
                "MaildirStore root {:?} is not a directory.", root;
                Init, Invalid, Path));
        }
        // The Maildir specification's escapes, since `/` would make a path and `:` the flags.
        let hostname = hostname.into().replace('/', "\\057").replace(':', "\\072");
        Ok(Self {
            root:       Arc::new(root),
            hostname:   Arc::new(hostname),
            lock_wait:  LOCK_WAIT,
        })
    }

    /// The longest any call waits for a lock (30 s by default) before it fails with a
    /// `Timeout` error, so a stuck holder is reported rather than hanging every session and
    /// delivery on the folder.
    pub fn with_lock_wait(mut self, wait: Duration) -> Self {
        self.lock_wait = wait;
        self
    }

    /// Resolve the absolute on-disk path for a user's mailbox tree.
    fn user_root(&self, user: &MailUser) -> PathBuf {
        // delivery_key is a relative path under root, set by
        // PasswdFileUserStore. Falls back to "<domain>/<local>" if
        // empty.
        if user.delivery_key.is_empty() {
            self.root.join(&user.domain).join(&user.local)
        } else {
            self.root.join(&user.delivery_key)
        }
    }

    /// Resolve the absolute on-disk path for one folder. INBOX is the
    /// user root itself; every other name lives at `.<name>` under the
    /// root, with `/` separators in the IMAP name turned into `.`.
    fn folder_path(&self, user: &MailUser, folder: &FolderName) -> PathBuf {
        let user_root = self.user_root(user);
        if folder.as_str().eq_ignore_ascii_case("INBOX") {
            user_root
        } else {
            let mapped = folder.as_str().replace('/', ".");
            user_root.join(fmt!(".{}", mapped))
        }
    }

    /// Map an on-disk Maildir++ subdirectory back to an IMAP-friendly
    /// folder name (`.Sent.Archive` → `Sent/Archive`).
    fn folder_from_subdir(name: &str) -> String {
        let stripped = name.strip_prefix('.').unwrap_or(name);
        stripped.replace('.', "/")
    }

    /// Ensure cur/new/tmp exist under `dir`.
    fn ensure_subdirs(dir: &Path) -> Outcome<()> {
        for sub in ["cur", "new", "tmp"] {
            let p = dir.join(sub);
            if !p.exists() {
                if let Err(e) = fs::create_dir_all(&p) {
                    return Err(err!(e,
                        "Creating Maildir subdir {:?}.", p;
                        IO, File, Init));
                }
            }
        }
        Ok(())
    }

    /// Waits for and takes the folder's lock, which every reader and writer of its uidlist and
    /// of its `cur/` and `new/` holds, in this process or another.
    fn lock_folder(&self, folder_dir: &Path) -> Outcome<ExclusiveLock> {
        res!(Self::ensure_subdirs(folder_dir));
        ExclusiveLock::acquire_within(&folder_dir.join(LOCK_NAME), self.lock_wait)
    }

    /// Reads the folder and brings its uidlist up to date: a UID for each file not yet listed,
    /// none for a file gone. `_lock` is the folder's own, which the caller holds throughout.
    fn read_folder(
        folder_dir:     &Path,
        clear_recent:   bool,
        _lock:          &ExclusiveLock,
    )
        -> Outcome<FolderView>
    {
        // Files in new/ are RECENT, and are moved to cur/ when the recent flag is being
        // cleared (a SELECT, not an EXAMINE).
        let mut files: Vec<(String, PathBuf, bool)> = Vec::new(); // name, path, recent
        for sub in ["cur", "new"] {
            let dir = folder_dir.join(sub);
            let rd = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(e) => return Err(err!(e,
                    "Reading the Maildir directory {:?}.", dir;
                    IO, File, Read)),
            };
            for entry in rd {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => return Err(err!(e,
                        "Reading an entry of the Maildir directory {:?}.", dir;
                        IO, File, Read)),
                };
                let path = entry.path();
                if !path.is_file() { continue; }
                let name = entry.file_name().to_string_lossy().into_owned();
                if sub == "new" && clear_recent {
                    let with_flags = if name.contains(":2,") { name.clone() } else { fmt!("{}:2,", name) };
                    let target = folder_dir.join("cur").join(&with_flags);
                    match fs::rename(&path, &target) {
                        Ok(()) => files.push((with_flags, target, false)),
                        Err(e) => {
                            warn!("Failed to move {:?} -> {:?}: {}", path, target, e);
                            files.push((name, path, true));
                        },
                    }
                } else {
                    files.push((name, path, sub == "new"));
                }
            }
        }

        let uidlist_path = folder_dir.join(UIDLIST_NAME);
        let mut changed = false;
        let (validity, mut next, mut map) = match res!(read_uidlist(&uidlist_path)) {
            Uidlist::Valid { validity, next, map } if validity != 0 => (validity, next, map),
            Uidlist::Valid { next, map, .. } => {
                // Written by this store before F2a, which kept the validity only in the
                // `dovecot-uidvalidity` files: the UIDs stand under the validity issued there.
                changed = true;
                let v = match stored_uidvalidity(folder_dir) {
                    Some(v) => v,
                    None => {
                        let v = fresh_uidvalidity(None);
                        res!(record_uidvalidity(folder_dir, v));
                        v
                    },
                };
                (v, next, map)
            },
            damage => {
                // Nothing says which UIDs were issued under the old validity, so every message
                // is numbered afresh under a new and larger one, which tells every client to
                // forget what it holds.
                let prev = stored_uidvalidity(folder_dir);
                let v = fresh_uidvalidity(prev);
                if damage == Uidlist::Garbled || prev.is_some() {
                    warn!("The uidlist {:?} is {}: UIDVALIDITY {:?} -> {}.",
                        uidlist_path,
                        if damage == Uidlist::Garbled { "damaged" } else { "missing" },
                        prev, v);
                }
                res!(record_uidvalidity(folder_dir, v));
                changed = true;
                (v, 1, BTreeMap::new())
            },
        };
        for uid in map.values() {
            if *uid >= next {
                next = uid.saturating_add(1);
                changed = true;
            }
        }

        // New files take UIDs in name order, which is delivery order for names this store makes.
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut present: BTreeSet<String> = BTreeSet::new();
        let mut entries: Vec<Entry> = Vec::with_capacity(files.len());
        for (name, path, recent) in &files {
            let key = key_of(name);
            if present.contains(&key) { continue; } // one message seen in both cur/ and new/
            let uid = match map.get(&key) {
                Some(u) => *u,
                None => {
                    if next == u32::MAX {
                        return Err(err!(
                            "The folder {:?} has issued every UID under UIDVALIDITY {}.",
                            folder_dir, validity;
                            Overflow, Range));
                    }
                    let u = next;
                    next += 1;
                    map.insert(key.clone(), u);
                    changed = true;
                    u
                },
            };
            present.insert(key.clone());
            entries.push(Entry {
                meta:   file_to_meta(path, name, uid, *recent),
                path:   path.clone(),
                key,
            });
        }
        let listed = map.len();
        map.retain(|k, _| present.contains(k));
        if map.len() != listed { changed = true; }
        if changed {
            res!(write_uidlist(&uidlist_path, validity, next, &map));
        }

        entries.sort_by_key(|e| e.meta.uid.0);
        let recent = entries.iter().filter(|e| e.meta.flags.recent).count() as u32;
        Ok(FolderView {
            entries,
            uid_validity:   validity,
            uid_next:       next,
            recent,
        })
    }

    /// The folder read under its lock, which is released before this returns.
    fn view(&self, folder_dir: &Path, clear_recent: bool) -> Outcome<FolderView> {
        let lock = res!(self.lock_folder(folder_dir));
        Self::read_folder(folder_dir, clear_recent, &lock)
    }

    /// Creates a file in `tmp_dir` under a name no other delivery uses, this process's or
    /// another's: `<secs>.M<usec>P<pid>Q<n>.<host>`, opened with `create_new` so that a
    /// clash is refused rather than overwritten.
    fn create_unique(&self, tmp_dir: &Path) -> Outcome<(String, PathBuf, File)> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let pid = std::process::id();
        for _ in 0..NAME_TRIES {
            let q = NAME_SEQ.fetch_add(1, Ordering::Relaxed);
            let unique = fmt!("{}.M{}P{}Q{}.{}",
                now.as_secs(), now.subsec_micros(), pid, q, self.hostname);
            let path = tmp_dir.join(&unique);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => return Ok((unique, path, f)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(err!(e,
                    "Creating the delivery file {:?}.", path;
                    IO, File, Write)),
            }
        }
        Err(err!(
            "Every one of {} fresh message names in {:?} was already taken.", NAME_TRIES, tmp_dir;
            IO, File, Exists))
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MailStore IMPLEMENTATION                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

impl MailStore for MaildirStore {

    fn ensure_user(&self, user: &MailUser) -> Outcome<()> {
        let root = self.user_root(user);
        if !root.exists() {
            if let Err(e) = fs::create_dir_all(&root) {
                return Err(err!(e,
                    "Creating user mailbox {:?}.", root;
                    IO, File, Init));
            }
        }
        res!(self.view(&root, false));
        // Pre-create the special-use folders so IMAP clients can
        // discover them via LIST + SPECIAL-USE attributes without
        // having to issue CREATE first. This unblocks Thunderbird's
        // "save sent messages on the server" behaviour, which
        // otherwise silently falls back to Local Folders when the
        // target Sent folder does not exist on the server.
        for f in ["Sent", "Drafts", "Trash", "Junk", "Archive"] {
            res!(self.create_folder(user, &FolderName::new(f)));
            res!(self.subscribe(user, &FolderName::new(f)));
        }
        Ok(())
    }

    fn append(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        bytes:      &[u8],
        flags:      MessageFlags,
        internal:   Option<SystemTime>,
    )
        -> Outcome<MessageUid>
    {
        let folder_dir = self.folder_path(user, folder);
        if !folder_dir.exists() {
            res!(self.create_folder(user, folder));
        }
        res!(Self::ensure_subdirs(&folder_dir));

        // Written in tmp/ outside the lock, so a large message holds up no reader.
        let (unique, tmp_path, mut f) = res!(self.create_unique(&folder_dir.join("tmp")));
        let written = (|| -> Outcome<()> {
            res!(f.write_all(bytes));
            // After the write, which sets the mtime, so INTERNALDATE round-trips.
            if let Some(t) = internal {
                if let Err(e) = f.set_modified(t) {
                    warn!("Could not set the INTERNALDATE of {:?}: {}.", tmp_path, e);
                }
            }
            res!(f.sync_all());
            Ok(())
        })();
        drop(f);
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp_path);
            return Err(err!(e,
                "Writing the delivery file {:?}.", tmp_path;
                IO, File, Write));
        }

        let filename = fmt!("{}:2,{}", unique, flag_suffix(flags));
        let cur_path = folder_dir.join("cur").join(&filename);
        let lock = match self.lock_folder(&folder_dir) {
            Ok(l) => l,
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(e);
            },
        };
        // Every writer of cur/ holds the lock, so nothing can take the name between the test
        // and the rename.
        if cur_path.exists() {
            let _ = fs::remove_file(&tmp_path);
            return Err(err!(
                "A message named {:?} is already in the folder.", cur_path;
                IO, File, Exists));
        }
        if let Err(e) = fs::rename(&tmp_path, &cur_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(err!(e,
                "Renaming {:?} -> {:?}.", tmp_path, cur_path;
                IO, File, Write));
        }
        // The data is on disk; the entry naming it must be too before the UID is given out,
        // or a power loss leaves a uidlist naming a message that SMTP acknowledged and lost.
        res!(sync_parent_dir(&cur_path));
        let view = res!(Self::read_folder(&folder_dir, false, &lock));
        match view.entries.iter().find(|e| e.key == unique) {
            Some(e) => Ok(e.meta.uid),
            None => Err(err!(
                "The message just filed as {:?} has no UID in {:?}.", cur_path, folder_dir;
                Bug, Missing)),
        }
    }

    fn list_folders(&self, user: &MailUser) -> Outcome<Vec<FolderName>> {
        let root = self.user_root(user);
        let mut out = vec![FolderName::new("INBOX")];
        if let Ok(rd) = fs::read_dir(&root) {
            for entry in rd.flatten() {
                if !entry.path().is_dir() { continue; }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') && name != "." && name != ".." {
                    out.push(FolderName::new(Self::folder_from_subdir(&name)));
                }
            }
        }
        Ok(out)
    }


    fn folder_status(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<FolderStatus>
    {
        let view = res!(self.view(&self.folder_path(user, folder), false));
        let unseen = view.entries.iter().filter(|e| !e.meta.flags.seen).count() as u32;
        Ok(FolderStatus {
            exists:         view.entries.len() as u32,
            recent:         view.recent,
            unseen,
            uid_validity:   view.uid_validity,
            uid_next:       view.uid_next,
        })
    }

    fn list_messages(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        read_only:  bool,
    )
        -> Outcome<Vec<MessageMeta>>
    {
        let view = res!(self.view(&self.folder_path(user, folder), !read_only));
        Ok(view.messages())
    }

    fn fetch_bytes(
        &self,
        user:   &MailUser,
        folder: &FolderName,
        uid:    MessageUid,
    )
        -> Outcome<Option<Vec<u8>>>
    {
        let folder_dir = self.folder_path(user, folder);
        let lock = res!(self.lock_folder(&folder_dir));
        let view = res!(Self::read_folder(&folder_dir, false, &lock));
        match view.find(uid) {
            Some(e) => Ok(Some(res!(read_message(&e.path, e.meta.size)))),
            None    => Ok(None),
        }
    }

    fn visit_bytes<F>(
        &self,
        user:   &MailUser,
        folder: &FolderName,
        uids:   &[MessageUid],
        mut f:  F,
    )
        -> Outcome<()>
        where F: FnMut(MessageUid, &[u8]) -> Outcome<()>
    {
        let folder_dir = self.folder_path(user, folder);
        let lock = res!(self.lock_folder(&folder_dir));
        let view = res!(Self::read_folder(&folder_dir, false, &lock));
        for uid in uids {
            if let Some(e) = view.find(*uid) {
                let bytes = res!(read_message(&e.path, e.meta.size));
                res!(f(*uid, &bytes));
            }
        }
        Ok(())
    }

    fn update_flags(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        uid:        MessageUid,
        op:         FlagUpdate,
    )
        -> Outcome<Option<MessageFlags>>
    {
        let folder_dir = self.folder_path(user, folder);
        let lock = res!(self.lock_folder(&folder_dir));
        let view = res!(Self::read_folder(&folder_dir, false, &lock));
        let entry = match view.find(uid) {
            Some(e) => e,
            None    => return Ok(None),
        };
        // The file's own name, read under the lock, is the current flag set.
        let flags = op.apply(entry.meta.flags);
        let new_path = folder_dir.join("cur").join(fmt!("{}:2,{}", entry.key, flag_suffix(flags)));
        if new_path != entry.path {
            if let Err(e) = fs::rename(&entry.path, &new_path) {
                return Err(err!(e,
                    "Renaming for flag update {:?} -> {:?}.", entry.path, new_path;
                    IO, File, Write));
            }
        }
        Ok(Some(flags))
    }

    fn expunge(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<Vec<MessageUid>>
    {
        let folder_dir = self.folder_path(user, folder);
        let lock = res!(self.lock_folder(&folder_dir));
        let view = res!(Self::read_folder(&folder_dir, false, &lock));
        let mut removed: Vec<MessageUid> = Vec::new();
        for e in &view.entries {
            if !e.meta.flags.deleted { continue; }
            match fs::remove_file(&e.path) {
                Ok(()) => removed.push(e.meta.uid),
                // Gone already, by a hand outside this store: it is expunged all the same.
                Err(err) if err.kind() == ErrorKind::NotFound => removed.push(e.meta.uid),
                // Still there, so it is not reported, and stays \Deleted for the next EXPUNGE.
                Err(err) => warn!("Could not expunge UID {} of {:?}, file {:?}: {}.",
                    e.meta.uid.0, folder_dir, e.path, err),
            }
        }
        if !removed.is_empty() {
            res!(Self::read_folder(&folder_dir, false, &lock));
        }
        Ok(removed)
    }

    fn create_folder(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<()>
    {
        let folder_dir = self.folder_path(user, folder);
        if let Err(e) = fs::create_dir_all(&folder_dir) {
            return Err(err!(e,
                "Creating folder {:?}.", folder_dir;
                IO, File, Init));
        }
        res!(Self::ensure_subdirs(&folder_dir));
        // Maildir++ marker.
        if !folder.as_str().eq_ignore_ascii_case("INBOX") {
            let marker = folder_dir.join(MAILDIRFOLDER_NAME);
            if !marker.exists() {
                if let Err(e) = File::create(&marker) {
                    return Err(err!(e,
                        "Creating the Maildir++ marker {:?}.", marker;
                        IO, File, Init));
                }
            }
        }
        // Issues the folder's UIDVALIDITY with its first uidlist.
        res!(self.view(&folder_dir, false));
        Ok(())
    }

    fn subscribe(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<()>
    {
        let user_root = self.user_root(user);
        let path = user_root.join(SUBSCRIPTIONS_NAME);
        // The list is read, added to and saved whole, so two at once would lose a name.
        let _lock = res!(ExclusiveLock::acquire_within(&user_root.join(SUBS_LOCK_NAME), self.lock_wait));
        let mut current: Vec<String> = Vec::new();
        if let Ok(s) = fs::read_to_string(&path) {
            for line in s.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    current.push(trimmed.to_string());
                }
            }
        }
        let imap_name = folder.as_str().to_string();
        if current.iter().any(|x| x == &imap_name) {
            return Ok(());
        }
        current.push(imap_name);
        let mut text = current.join("\n");
        text.push('\n');
        // Whole or not at all: a truncated file would drop every other subscription.
        save_atomic(&path, text.as_bytes(), SaveMode::Keep)
    }

    fn list_subscribed(&self, user: &MailUser) -> Outcome<Vec<FolderName>> {
        let path = self.user_root(user).join(SUBSCRIPTIONS_NAME);
        let mut out: Vec<FolderName> = Vec::new();
        // Always include INBOX.
        out.push(FolderName::new("INBOX"));
        if let Ok(s) = fs::read_to_string(&path) {
            for line in s.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                if trimmed.eq_ignore_ascii_case("INBOX") { continue; }
                // Dovecot writes a `V<TAB><version>` header at the
                // top of its subscriptions file. Skip any line that
                // starts with the literal `V` followed by whitespace
                // and a digit -- a real folder name will not look
                // like this.
                let mut it = trimmed.chars();
                if it.next() == Some('V') {
                    if let Some(c) = it.next() {
                        if c.is_whitespace() {
                            if let Some(d) = it.next() {
                                if d.is_ascii_digit() { continue; }
                            }
                        }
                    }
                }
                out.push(FolderName::new(trimmed));
            }
        }
        Ok(out)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HELPERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// Strip the `:2,FLAGS` suffix from a Maildir filename, returning the
/// stable unique part used as the UID list key.
fn key_of(name: &str) -> String {
    match name.find(":2,") {
        Some(i) => name[..i].to_string(),
        None => name.to_string(),
    }
}

/// Translate a `MessageFlags` set into its Maildir suffix (sorted).
fn flag_suffix(flags: MessageFlags) -> String {
    let mut s = String::new();
    if flags.draft     { s.push('D'); }
    if flags.flagged   { s.push('F'); }
    if flags.answered  { s.push('R'); }
    if flags.seen      { s.push('S'); }
    if flags.deleted   { s.push('T'); }
    s
}

/// Inverse of `flag_suffix`. Recognises the standard letters.
fn flags_from_suffix(suffix: &str) -> MessageFlags {
    let mut f = MessageFlags::default();
    for c in suffix.chars() {
        match c {
            'D' => f.draft = true,
            'F' => f.flagged = true,
            'R' => f.answered = true,
            'S' => f.seen = true,
            'T' => f.deleted = true,
            _ => (),
        }
    }
    f
}

/// Build a `MessageMeta` from a Maildir file path.
fn file_to_meta(path: &Path, name: &str, uid: u32, recent: bool) -> MessageMeta {
    let suffix = match name.rfind(":2,") {
        Some(i) => &name[i + 3..],
        None => "",
    };
    let mut flags = flags_from_suffix(suffix);
    flags.recent = recent;
    let md = fs::metadata(path).ok();
    let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
    let internal = md.and_then(|m| m.modified().ok()).unwrap_or(SystemTime::now());
    MessageMeta {
        uid: MessageUid(uid),
        size,
        internal,
        flags,
    }
}

fn read_message(path: &Path, size: u64) -> Outcome<Vec<u8>> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) => return Err(err!(e,
            "Opening {:?}.", path;
            IO, File, Read)),
    };
    let mut bytes = Vec::with_capacity(size as usize);
    if let Err(e) = f.read_to_end(&mut bytes) {
        return Err(err!(e,
            "Reading {:?}.", path;
            IO, File, Read));
    }
    Ok(bytes)
}

fn read_uidlist(path: &Path) -> Outcome<Uidlist> {
    match fs::read(path) {
        Ok(bytes)                                   => Ok(parse_uidlist(&bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound   => Ok(Uidlist::Missing),
        Err(e)                                      => Err(err!(e,
            "Reading the uidlist {:?}.", path;
            IO, File, Read)),
    }
}

/// Reads a version 3 dovecot-uidlist: a header `3 V<validity> N<next> ...` and then one
/// `<uid> [<extensions>] :<file name>` row per message, UIDs rising. Anything else, a file
/// cut short (every writer ends it with a newline) included, is `Garbled`.
fn parse_uidlist(bytes: &[u8]) -> Uidlist {
    let text = match std::str::from_utf8(bytes) {
        Ok(t)   => t,
        Err(_)  => return Uidlist::Garbled,
    };
    let body = match text.strip_suffix('\n') {
        Some(b) => b,
        None    => return Uidlist::Garbled,
    };
    let mut lines = body.split('\n');
    let mut toks = match lines.next() {
        Some(h) => h.split_whitespace(),
        None    => return Uidlist::Garbled,
    };
    if toks.next() != Some("3") { return Uidlist::Garbled; }
    let mut validity: Option<u32> = None;
    let mut next: Option<u32> = None;
    for t in toks {
        let (slot, digits) = match t.split_at_checked(1) {
            Some(("V", d)) => (&mut validity, d),
            Some(("N", d)) => (&mut next, d),
            _ => continue,
        };
        match digits.parse::<u32>() {
            Ok(n)   => *slot = Some(n),
            Err(_)  => return Uidlist::Garbled,
        }
    }
    let (validity, next) = match (validity, next) {
        (Some(v), Some(n))  => (v, n),
        _                   => return Uidlist::Garbled,
    };
    let mut map: BTreeMap<String, u32> = BTreeMap::new();
    let mut last = 0u32;
    for line in lines {
        let (uid, key) = match parse_uid_line(line) {
            Some(row)   => row,
            None        => return Uidlist::Garbled,
        };
        if uid <= last || map.insert(key, uid).is_some() {
            return Uidlist::Garbled;
        }
        last = uid;
    }
    Uidlist::Valid { validity, next, map }
}

/// Parse a single dovecot-uidlist data row, `<uid> [<extensions>] :<file name>`, into
/// `(uid, key)`.
fn parse_uid_line(line: &str) -> Option<(u32, String)> {
    let (uid, rest) = ok!(line.split_once(' '));
    let uid: u32 = ok!(uid.parse().ok());
    if uid == 0 { return None; }
    let name = &rest[ok!(rest.find(':')) + 1..];
    if name.is_empty() { return None; }
    Some((uid, key_of(name)))
}

/// Replaces the uidlist whole, so a reader or a crash finds the old list or the new one,
/// never an empty or partial file.
fn write_uidlist(
    path:       &Path,
    validity:   u32,
    next:       u32,
    map:        &BTreeMap<String, u32>,
)
    -> Outcome<()>
{
    let by_uid: BTreeMap<u32, &str> = map.iter().map(|(k, u)| (*u, k.as_str())).collect();
    let mut text = fmt!("3 V{} N{}\n", validity, next);
    for (u, k) in &by_uid {
        text.push_str(&fmt!("{} :{}\n", u, k));
    }
    res!(save_atomic(path, text.as_bytes(), SaveMode::Keep));
    Ok(())
}

/// The largest UIDVALIDITY the folder's `dovecot-uidvalidity` files record.
fn stored_uidvalidity(folder_dir: &Path) -> Option<u32> {
    let mut best: Option<u32> = None;
    let prefix = fmt!("{}.", UIDVALIDITY_PREFIX);
    if let Ok(rd) = fs::read_dir(folder_dir) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            if let Some(rest) = name.to_string_lossy().strip_prefix(&prefix) {
                best = best.max(u32::from_str_radix(rest, 16).ok());
            }
        }
    }
    if let Ok(s) = fs::read_to_string(folder_dir.join(UIDVALIDITY_PREFIX)) {
        let s = s.trim();
        best = best.max(u32::from_str_radix(s, 16).ok().or_else(|| s.parse().ok()));
    }
    best
}

/// A UIDVALIDITY after `prev`: the clock in seconds, or one past `prev` when the clock is not
/// past it.
fn fresh_uidvalidity(prev: Option<u32>) -> u32 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(1);
    now.max(prev.map(|p| p.saturating_add(1)).unwrap_or(1))
}

/// Records `v` as the folder's latest UIDVALIDITY, so that a uidlist lost later is answered
/// with a larger one. Exactly one `dovecot-uidvalidity.<hex>` marker remains, the Dovecot
/// layout: a reader that takes the first marker it finds (this store before F2a) would
/// otherwise see an older validity over the new numbering.
fn record_uidvalidity(folder_dir: &Path, v: u32) -> Outcome<()> {
    res!(save_atomic(&folder_dir.join(UIDVALIDITY_PREFIX), fmt!("{:08x}", v).as_bytes(), SaveMode::Keep));
    let name = fmt!("{}.{:08x}", UIDVALIDITY_PREFIX, v);
    let marker = folder_dir.join(&name);
    match OpenOptions::new().write(true).create(true).truncate(false).open(&marker) {
        Ok(f) => if let Err(e) = f.sync_all() {
            return Err(err!(e,
                "Syncing the UIDVALIDITY marker {:?}.", marker;
                IO, File, Write));
        },
        Err(e) => return Err(err!(e,
            "Creating the UIDVALIDITY marker {:?}.", marker;
            IO, File, Write)),
    }
    let rd = match fs::read_dir(folder_dir) {
        Ok(rd) => rd,
        Err(e) => return Err(err!(e,
            "Reading the folder {:?} for older UIDVALIDITY markers.", folder_dir;
            IO, File, Read)),
    };
    let prefix = fmt!("{}.", UIDVALIDITY_PREFIX);
    for entry in rd.flatten() {
        let other = entry.file_name().to_string_lossy().into_owned();
        if other.starts_with(&prefix) && other != name {
            let p = entry.path();
            if let Err(e) = fs::remove_file(&p) {
                return Err(err!(e,
                    "Removing the superseded UIDVALIDITY marker {:?}.", p;
                    IO, File, Write));
            }
        }
    }
    sync_parent_dir(&marker)
}
