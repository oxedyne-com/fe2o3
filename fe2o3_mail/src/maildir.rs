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
//!   dovecot-uidvalidity            8-byte hex marker (mtime is canon)
//!   dovecot-uidvalidity.<hex>      empty companion file
//!   dovecot-uidlist                "<uid> <filename>" map for UID stability
//!   subscriptions                  one folder name per line
//! ```
//!
//! UIDs are assigned monotonically per folder. On open, the store
//! reads the existing `dovecot-uidlist` if present, scans `cur` and
//! `new` for any files not yet listed, assigns them fresh UIDs, and
//! rewrites the file. Subsequent appends increment `uidnext` and
//! append a new entry. UIDVALIDITY comes from the
//! `dovecot-uidvalidity.<hex>` filename or, failing that, the current
//! Unix time at first open.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::mail::store::{
    FolderName,
    FolderStatus,
    MailStore,
    MailUser,
    MessageFlags,
    MessageMeta,
    MessageUid,
};

use std::{
    collections::BTreeMap,
    fs::{
        self,
        File,
    },
    io::{
        BufRead,
        BufReader,
        Read,
        Write,
    },
    path::{
        Path,
        PathBuf,
    },
    sync::Arc,
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};


/// File name of the dovecot-compatible UID list.
const UIDLIST_NAME: &str = "dovecot-uidlist";

/// File name prefix of the dovecot-compatible UID validity marker.
const UIDVALIDITY_PREFIX: &str = "dovecot-uidvalidity";

/// File name of the subscription list.
const SUBSCRIPTIONS_NAME: &str = "subscriptions";

/// File written into a sub-mailbox to mark it as a Maildir folder.
const MAILDIRFOLDER_NAME: &str = "maildirfolder";


/// Maildir-backed `MailStore`.
///
/// Holds a single root directory (typically `/var/mail/vhosts`) and
/// nothing else: every operation derives its on-disk paths from the
/// `MailUser::delivery_key` it is handed.
#[derive(Clone, Debug)]
pub struct MaildirStore {
    /// Filesystem root holding `<domain>/<local>/` per-user trees.
    root: Arc<PathBuf>,
    /// Hostname appended to generated message filenames.
    hostname: Arc<String>,
}

impl MaildirStore {
    /// Build a new store rooted at `root`. The directory must exist.
    pub fn new(root: PathBuf, hostname: impl Into<String>) -> Outcome<Self> {
        if !root.is_dir() {
            return Err(err!(
                "MaildirStore root {:?} is not a directory.", root;
                Init, Invalid, Path));
        }
        Ok(Self {
            root:       Arc::new(root),
            hostname:   Arc::new(hostname.into()),
        })
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

    /// Read the UID validity for a folder, or generate and persist one.
    fn read_or_init_uidvalidity(folder_dir: &Path) -> Outcome<u32> {
        // Look for `dovecot-uidvalidity.<hex>` -- the hex suffix is
        // canonical.
        if let Ok(rd) = fs::read_dir(folder_dir) {
            for entry in rd.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if let Some(rest) = name.strip_prefix(&fmt!("{}.", UIDVALIDITY_PREFIX)) {
                    if let Ok(n) = u32::from_str_radix(rest, 16) {
                        return Ok(n);
                    }
                }
            }
        }
        // Fall back to the contents of dovecot-uidvalidity if present.
        let plain = folder_dir.join(UIDVALIDITY_PREFIX);
        if let Ok(s) = fs::read_to_string(&plain) {
            let s = s.trim();
            if let Ok(n) = u32::from_str_radix(s, 16) {
                return Ok(n);
            }
            if let Ok(n) = s.parse::<u32>() {
                return Ok(n);
            }
        }
        // Generate.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(1);
        let marker = folder_dir.join(fmt!("{}.{:08x}", UIDVALIDITY_PREFIX, now));
        let _ = File::create(&marker);
        let mut p = match File::create(&plain) {
            Ok(f) => f,
            Err(e) => return Err(err!(e,
                "Writing {:?}.", plain; IO, File, Write)),
        };
        let _ = p.write_all(fmt!("{:08x}", now).as_bytes());
        Ok(now)
    }

    /// Open and refresh the per-folder UID list, returning every
    /// known message in UID order.
    fn read_messages(
        &self,
        folder_dir: &Path,
        clear_recent: bool,
    )
        -> Outcome<(Vec<MessageMeta>, u32, u32)>
    {
        res!(Self::ensure_subdirs(folder_dir));

        // Walk cur/ and new/ to find every file currently present.
        // Files in new/ are RECENT and will be moved to cur/ if we
        // are clearing the recent flag (a SELECT, not EXAMINE).
        let mut cur_files: Vec<(String, PathBuf)> = Vec::new();
        let mut new_files: Vec<(String, PathBuf)> = Vec::new();
        for sub in ["cur", "new"] {
            let dir = folder_dir.join(sub);
            if let Ok(rd) = fs::read_dir(&dir) {
                for entry in rd.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if sub == "cur" {
                            cur_files.push((name, path));
                        } else {
                            new_files.push((name, path));
                        }
                    }
                }
            }
        }

        // If clearing recent, move files from new/ into cur/ adding
        // the `:2,` flag suffix so they parse cleanly.
        if clear_recent {
            for (name, path) in std::mem::take(&mut new_files) {
                let with_flags = if name.contains(":2,") {
                    name.clone()
                } else {
                    fmt!("{}:2,", name)
                };
                let target = folder_dir.join("cur").join(&with_flags);
                if let Err(e) = fs::rename(&path, &target) {
                    warn!("Failed to move {:?} -> {:?}: {}", path, target, e);
                    cur_files.push((name, path));
                } else {
                    cur_files.push((with_flags, target));
                }
            }
        }

        let recent_count = new_files.len();
        let mut all_files: Vec<(String, PathBuf, bool)> = Vec::new();
        for (n, p) in &cur_files { all_files.push((n.clone(), p.clone(), false)); }
        for (n, p) in &new_files { all_files.push((n.clone(), p.clone(), true)); }

        // Read the existing uid list.
        let uidlist_path = folder_dir.join(UIDLIST_NAME);
        let mut uid_by_name: BTreeMap<String, u32> = BTreeMap::new();
        let mut uid_next: u32 = 1;
        if let Ok(file) = File::open(&uidlist_path) {
            let reader = BufReader::new(file);
            let mut header_seen = false;
            for line in reader.lines().flatten() {
                if !header_seen {
                    header_seen = true;
                    // Extract `N<num>` if present.
                    for tok in line.split_whitespace() {
                        if let Some(rest) = tok.strip_prefix('N') {
                            if let Ok(n) = rest.parse::<u32>() {
                                uid_next = n;
                            }
                        }
                    }
                    if line.contains(' ') && !line.starts_with(char::is_alphabetic) {
                        // Treat as data row, not header.
                        if let Some((u, n)) = parse_uid_line(&line) {
                            uid_by_name.insert(n, u);
                            if u >= uid_next { uid_next = u + 1; }
                        }
                    }
                    continue;
                }
                if let Some((u, n)) = parse_uid_line(&line) {
                    uid_by_name.insert(n, u);
                    if u >= uid_next { uid_next = u + 1; }
                }
            }
        }

        // Assign UIDs to any new files. dovecot-uidlist keys by the
        // unique part of the filename (everything before `:2,` if
        // present), which we mirror here so a flag change does not
        // re-allocate a UID.
        let mut by_uid: BTreeMap<u32, MessageMeta> = BTreeMap::new();
        let mut keys_seen: Vec<String> = Vec::new();
        for (name, path, recent) in &all_files {
            let key = key_of(name);
            keys_seen.push(key.clone());
            let uid = match uid_by_name.get(&key) {
                Some(u) => *u,
                None => {
                    let u = uid_next;
                    uid_next = uid_next.saturating_add(1);
                    uid_by_name.insert(key.clone(), u);
                    u
                }
            };
            let meta = file_to_meta(path, name, uid, *recent);
            by_uid.insert(uid, meta);
        }

        // Rewrite the uid list with current state. Drop any entries
        // for files that no longer exist on disk.
        let mut keep: BTreeMap<String, u32> = BTreeMap::new();
        for k in &keys_seen {
            if let Some(u) = uid_by_name.get(k) {
                keep.insert(k.clone(), *u);
            }
        }
        let _ = write_uidlist(&uidlist_path, &keep, uid_next);

        let messages: Vec<MessageMeta> = by_uid.into_values().collect();
        Ok((messages, uid_next, recent_count as u32))
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
        res!(Self::ensure_subdirs(&root));
        let _ = Self::read_or_init_uidvalidity(&root);
        // Pre-create the special-use folders so IMAP clients can
        // discover them via LIST + SPECIAL-USE attributes without
        // having to issue CREATE first. This unblocks Thunderbird's
        // "save sent messages on the server" behaviour, which
        // otherwise silently falls back to Local Folders when the
        // target Sent folder does not exist on the server.
        for f in ["Sent", "Drafts", "Trash", "Junk", "Archive"] {
            let _ = self.create_folder(user, &FolderName::new(f));
            let _ = self.subscribe(user, &FolderName::new(f));
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

        // Generate a unique filename. Format:
        // `<unix>.M<usec>P<pid>.<host>:2,<flags>`
        let now = SystemTime::now();
        let secs = now.duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let usec = now.duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_micros())
            .unwrap_or(0);
        let pid = std::process::id();
        let unique = fmt!("{}.M{}P{}.{}", secs, usec, pid, self.hostname);
        let flag_suffix = flag_suffix(flags);
        let filename = fmt!("{}:2,{}", unique, flag_suffix);

        // Write to tmp then rename into cur/.
        let tmp_path = folder_dir.join("tmp").join(&filename);
        let cur_path = folder_dir.join("cur").join(&filename);
        {
            let mut f = match File::create(&tmp_path) {
                Ok(f) => f,
                Err(e) => return Err(err!(e,
                    "Creating tmp file {:?}.", tmp_path;
                    IO, File, Write)),
            };
            if let Err(e) = f.write_all(bytes) {
                return Err(err!(e,
                    "Writing tmp file {:?}.", tmp_path;
                    IO, File, Write));
            }
            if let Err(e) = f.sync_all() {
                warn!("sync_all on {:?}: {}", tmp_path, e);
            }
        }
        if let Err(e) = fs::rename(&tmp_path, &cur_path) {
            return Err(err!(e,
                "Renaming {:?} -> {:?}.", tmp_path, cur_path;
                IO, File, Write));
        }
        // Best-effort: set the file mtime so INTERNALDATE round-trips.
        if let Some(t) = internal {
            let _ = filetime_set(&cur_path, t);
        }

        // Reload UID list to allocate the new UID.
        let (msgs, _next, _rec) = res!(self.read_messages(&folder_dir, false));
        let mut uid = MessageUid(0);
        for m in &msgs {
            if cur_path.file_name().map(|f| f.to_string_lossy().into_owned())
                == Some(filename.clone())
            {
                uid = m.uid;
                break;
            }
        }
        // If the loop did not find it (because the cur entry was named
        // differently after we moved it), grab the maximum UID just
        // assigned.
        if uid.0 == 0 {
            if let Some(m) = msgs.last() { uid = m.uid; }
        }
        Ok(uid)
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
        let folder_dir = self.folder_path(user, folder);
        let uidvalidity = res!(Self::read_or_init_uidvalidity(&folder_dir));
        let (msgs, next, recent) = res!(self.read_messages(&folder_dir, false));
        let unseen = msgs.iter().filter(|m| !m.flags.seen).count() as u32;
        Ok(FolderStatus {
            exists:         msgs.len() as u32,
            recent,
            unseen,
            uid_validity:   uidvalidity,
            uid_next:       next,
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
        let folder_dir = self.folder_path(user, folder);
        let (msgs, _next, _rec) = res!(self.read_messages(&folder_dir, !read_only));
        Ok(msgs)
    }

    fn fetch_bytes(
        &self,
        user:   &MailUser,
        folder: &FolderName,
        uid:    MessageUid,
    )
        -> Outcome<Vec<u8>>
    {
        let folder_dir = self.folder_path(user, folder);
        let (msgs, _next, _rec) = res!(self.read_messages(&folder_dir, false));
        let meta = match msgs.iter().find(|m| m.uid == uid) {
            Some(m) => m,
            None => return Err(err!(
                "No message with UID {} in folder {}.",
                uid.0, folder.as_str();
                Missing, Input)),
        };
        // The on-disk path is encoded in MessageMeta via the size /
        // internal date alone -- we need to walk the directory again
        // and locate the file by UID. Use the uidlist key.
        let uidlist_path = folder_dir.join(UIDLIST_NAME);
        let key = read_uidlist_key_for(&uidlist_path, uid.0);
        let key = match key {
            Some(k) => k,
            None => return Err(err!(
                "UID {} present in cache but not in {:?}.", uid.0, uidlist_path;
                Bug, Missing)),
        };
        // Look in cur/ and new/ for any file whose key matches.
        for sub in ["cur", "new"] {
            if let Ok(rd) = fs::read_dir(folder_dir.join(sub)) {
                for entry in rd.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if key_of(&name) == key {
                        let mut f = match File::open(entry.path()) {
                            Ok(f) => f,
                            Err(e) => return Err(err!(e,
                                "Opening {:?}.", entry.path();
                                IO, File, Read)),
                        };
                        let mut bytes = Vec::with_capacity(meta.size as usize);
                        if let Err(e) = f.read_to_end(&mut bytes) {
                            return Err(err!(e,
                                "Reading {:?}.", entry.path();
                                IO, File, Read));
                        }
                        return Ok(bytes);
                    }
                }
            }
        }
        Err(err!(
            "Could not locate file for UID {} on disk.", uid.0;
            Missing, IO, File))
    }

    fn set_flags(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        uid:        MessageUid,
        flags:      MessageFlags,
    )
        -> Outcome<MessageFlags>
    {
        let folder_dir = self.folder_path(user, folder);
        let uidlist_path = folder_dir.join(UIDLIST_NAME);
        let key = match read_uidlist_key_for(&uidlist_path, uid.0) {
            Some(k) => k,
            None => return Err(err!(
                "UID {} not in {:?}.", uid.0, uidlist_path; Missing)),
        };
        // Locate the current file.
        for sub in ["cur", "new"] {
            if let Ok(rd) = fs::read_dir(folder_dir.join(sub)) {
                for entry in rd.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if key_of(&name) != key { continue; }
                    // Build the new filename with updated flag suffix.
                    let new_name = fmt!("{}:2,{}", key, flag_suffix(flags));
                    let new_path = folder_dir.join("cur").join(&new_name);
                    if let Err(e) = fs::rename(entry.path(), &new_path) {
                        return Err(err!(e,
                            "Renaming for flag update {:?} -> {:?}.",
                            entry.path(), new_path;
                            IO, File, Write));
                    }
                    return Ok(flags);
                }
            }
        }
        Err(err!(
            "No file on disk for UID {}.", uid.0; Missing, IO, File))
    }

    fn expunge(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<Vec<MessageUid>>
    {
        let folder_dir = self.folder_path(user, folder);
        let (msgs, _next, _rec) = res!(self.read_messages(&folder_dir, false));
        let mut removed: Vec<MessageUid> = Vec::new();
        for m in &msgs {
            if !m.flags.deleted { continue; }
            let key = match read_uidlist_key_for(&folder_dir.join(UIDLIST_NAME), m.uid.0) {
                Some(k) => k,
                None => continue,
            };
            for sub in ["cur", "new"] {
                if let Ok(rd) = fs::read_dir(folder_dir.join(sub)) {
                    for entry in rd.flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if key_of(&name) == key {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
                }
            }
            removed.push(m.uid);
        }
        // Refresh uidlist.
        let _ = self.read_messages(&folder_dir, false);
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
                let _ = File::create(&marker);
            }
        }
        let _ = Self::read_or_init_uidvalidity(&folder_dir);
        Ok(())
    }

    fn subscribe(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<()>
    {
        let path = self.user_root(user).join(SUBSCRIPTIONS_NAME);
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
        if !current.iter().any(|x| x == &imap_name) {
            current.push(imap_name);
        }
        let mut f = match File::create(&path) {
            Ok(f) => f,
            Err(e) => return Err(err!(e,
                "Writing {:?}.", path; IO, File, Write)),
        };
        for line in &current {
            let _ = f.write_all(line.as_bytes());
            let _ = f.write_all(b"\n");
        }
        Ok(())
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

/// Parse a single dovecot-uidlist data row into `(uid, key)`. The
/// first line of the file is a header (`3 V0 N7 [G<guid>]`) that
/// happens to look superficially like a data row -- we reject it by
/// requiring the second token to begin with `':'` (uidlist v3
/// format) or a digit (timestamp prefix used in older formats).
fn parse_uid_line(line: &str) -> Option<(u32, String)> {
    let line = line.trim();
    let mut it = line.splitn(2, ' ');
    let uid: u32 = ok!(ok!(it.next()).parse().ok());
    let rest = ok!(it.next());
    let first = ok!(rest.chars().next());
    if first != ':' && !first.is_ascii_digit() {
        return None;
    }
    let name = match rest.find(':') {
        Some(i) => &rest[i + 1..],
        None => rest,
    };
    Some((uid, key_of(name)))
}

/// Look up the uidlist key for a given UID without keeping the whole
/// file in memory.
fn read_uidlist_key_for(path: &Path, uid: u32) -> Option<String> {
    let file = ok!(File::open(path).ok());
    let reader = BufReader::new(file);
    let mut header_seen = false;
    for line in reader.lines().flatten() {
        if !header_seen {
            header_seen = true;
            // Header lines start with a digit (version) but may also
            // be a data line in older formats.
            if let Some((u, k)) = parse_uid_line(&line) {
                if u == uid { return Some(k); }
            }
            continue;
        }
        if let Some((u, k)) = parse_uid_line(&line) {
            if u == uid { return Some(k); }
        }
    }
    None
}

/// Rewrite the dovecot-uidlist with a fresh map.
fn write_uidlist(path: &Path, map: &BTreeMap<String, u32>, uidnext: u32) -> Outcome<()> {
    let mut f = match File::create(path) {
        Ok(f) => f,
        Err(e) => return Err(err!(e,
            "Writing {:?}.", path; IO, File, Write)),
    };
    let _ = writeln!(f, "3 V0 N{}", uidnext);
    let mut by_uid: BTreeMap<u32, String> = BTreeMap::new();
    for (k, u) in map { by_uid.insert(*u, k.clone()); }
    for (u, k) in &by_uid {
        let _ = writeln!(f, "{} :{}", u, k);
    }
    Ok(())
}

/// Set a file's mtime to the given SystemTime. Best effort.
fn filetime_set(path: &Path, t: SystemTime) -> Outcome<()> {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Use the libc utimes via std::fs::File and set_modified.
    let f = match File::options().write(true).open(path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };
    let _ = f.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(secs));
    Ok(())
}

