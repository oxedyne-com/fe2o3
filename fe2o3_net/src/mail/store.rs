//! Mailbox storage trait.
//!
//! `MailStore` is the abstraction the SMTP and IMAP servers use to persist
//! and retrieve messages. The trait deliberately operates on raw RFC 5322
//! message bytes rather than a parsed `EmailMessage`: IMAP `FETCH BODY[]`
//! must return the original bytes byte-for-byte, and SMTP `DATA` already
//! delivers a fully-formed message blob.
//!
//! Implementations are expected to be cheap to clone (typically via an
//! internal `Arc`) so a long-running server can hand a store to every
//! connection task without contention.

use oxedyne_fe2o3_core::prelude::*;

use std::time::SystemTime;


/// One IMAP folder name as it appears on the wire.
///
/// Folder names use the IMAP modified UTF-7 encoding on the wire but every
/// implementation in Hematite stores them as plain UTF-8. The value here is
/// always the user-facing form -- conversion to/from modified UTF-7 happens
/// at the IMAP wire layer.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FolderName(pub String);

impl FolderName {
    /// Wrap a raw string as a folder name.
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }

    /// Borrow the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable per-message identifier inside a folder.
///
/// IMAP requires monotonically increasing 32-bit unique identifiers per
/// folder (RFC 3501 §2.3.1.1). Each `MailStore` implementation is
/// responsible for honouring that contract.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MessageUid(pub u32);

/// Per-message flags carried by IMAP `FETCH FLAGS` and set by `STORE`.
///
/// Limited to the small set Thunderbird actually uses on a steady-state
/// session. Custom keywords are out of scope for the MVP.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MessageFlags {
    /// `\Seen` -- the user has read the message.
    pub seen:       bool,
    /// `\Answered` -- the user replied to the message.
    pub answered:   bool,
    /// `\Flagged` -- starred / flagged.
    pub flagged:    bool,
    /// `\Deleted` -- marked for `EXPUNGE`.
    pub deleted:    bool,
    /// `\Draft`.
    pub draft:      bool,
    /// `\Recent` -- newly delivered into the folder, as defined by RFC 3501.
    /// Cleared by the next session that opens the folder R/W.
    pub recent:     bool,
}

impl MessageFlags {
    /// Render the flag set as the space-separated IMAP atom list inside
    /// `(... )`, e.g. `\Seen \Flagged`.
    pub fn to_imap_list(&self) -> String {
        let mut out = String::new();
        let mut push = |s: &str| {
            if !out.is_empty() { out.push(' '); }
            out.push_str(s);
        };
        if self.seen      { push("\\Seen"); }
        if self.answered  { push("\\Answered"); }
        if self.flagged   { push("\\Flagged"); }
        if self.deleted   { push("\\Deleted"); }
        if self.draft     { push("\\Draft"); }
        if self.recent    { push("\\Recent"); }
        out
    }

    /// Returns true if `flag` (without the leading backslash) is set.
    pub fn has(&self, flag: &str) -> bool {
        match flag {
            "\\Seen"     | "Seen"     => self.seen,
            "\\Answered" | "Answered" => self.answered,
            "\\Flagged"  | "Flagged"  => self.flagged,
            "\\Deleted"  | "Deleted"  => self.deleted,
            "\\Draft"    | "Draft"    => self.draft,
            "\\Recent"   | "Recent"   => self.recent,
            _ => false,
        }
    }

    /// Set or clear a single flag by name.
    pub fn set(&mut self, flag: &str, on: bool) {
        match flag {
            "\\Seen"     | "Seen"     => self.seen     = on,
            "\\Answered" | "Answered" => self.answered = on,
            "\\Flagged"  | "Flagged"  => self.flagged  = on,
            "\\Deleted"  | "Deleted"  => self.deleted  = on,
            "\\Draft"    | "Draft"    => self.draft    = on,
            "\\Recent"   | "Recent"   => self.recent   = on,
            _ => (),
        }
    }
}

/// Lightweight per-message metadata used by IMAP `FETCH` and `STATUS`.
///
/// Holds everything the server needs to answer FETCH ENVELOPE, FLAGS,
/// INTERNALDATE, RFC822.SIZE and UID without re-reading the raw message
/// bytes from disk.
#[derive(Clone, Debug)]
pub struct MessageMeta {
    /// IMAP UID inside the folder.
    pub uid:        MessageUid,
    /// On-disk byte size of the raw message.
    pub size:       u64,
    /// Time the server received and stored the message (RFC 3501 §2.3.3).
    pub internal:   SystemTime,
    /// Current flag set.
    pub flags:      MessageFlags,
}

/// Per-folder summary returned by `MailStore::folder_status`.
#[derive(Clone, Debug, Default)]
pub struct FolderStatus {
    /// Total number of messages currently in the folder (post-expunge).
    pub exists:         u32,
    /// Number of messages with the `\Recent` flag.
    pub recent:         u32,
    /// Number of messages without the `\Seen` flag.
    pub unseen:         u32,
    /// UID validity value -- changes whenever the UID space is reset.
    pub uid_validity:   u32,
    /// UID that the next appended message will receive.
    pub uid_next:       u32,
}

/// One IMAP user identity, as scoped by `MailStore`.
///
/// A `MailUser` is the result of `UserStore::authenticate` (see
/// [`crate::mail::user`]). Different `MailStore` backends may key off
/// different fields -- a Maildir store usually only needs `delivery_dir`,
/// while an Ozone-backed store would key off the user id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailUser {
    /// Local part (left of `@`) the user authenticated as.
    pub local:          String,
    /// Domain (right of `@`).
    pub domain:         String,
    /// Filesystem path or opaque key identifying this user's mailbox root,
    /// chosen by the `UserStore` and consumed by the `MailStore`.
    pub delivery_key:   String,
}

impl MailUser {
    /// Render the user as `local@domain`.
    pub fn address(&self) -> String {
        fmt!("{}@{}", self.local, self.domain)
    }
}

/// Mailbox storage abstraction.
///
/// All methods take a `MailUser` so a single store can host multiple
/// accounts. Implementations decide how to map the user to physical storage
/// -- a Maildir-backed store will turn `delivery_key` into a directory
/// path, an Ozone-backed store will use it as a database key prefix.
///
/// The trait is intentionally synchronous: the IMAP and SMTP servers wrap
/// each call in `tokio::task::spawn_blocking` so the underlying I/O does
/// not block the runtime. Async would force every implementation through
/// `Pin<Box<dyn Future>>` for no practical gain on a single-host mailbox.
pub trait MailStore: Clone + Send + Sync + 'static {
    /// Ensure the user's storage is initialised on disk (folders, INBOX,
    /// etc.). Idempotent.
    fn ensure_user(&self, user: &MailUser) -> Outcome<()>;

    /// Append a fully-formed RFC 5322 message to a folder. Returns the
    /// freshly assigned UID. Implementations must allocate UIDs
    /// monotonically per folder.
    fn append(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        bytes:      &[u8],
        flags:      MessageFlags,
        internal:   Option<SystemTime>,
    )
        -> Outcome<MessageUid>;

    /// List every folder this user has, recursively.
    fn list_folders(&self, user: &MailUser) -> Outcome<Vec<FolderName>>;

    /// Status counters for one folder (EXISTS, RECENT, UNSEEN,
    /// UIDVALIDITY, UIDNEXT).
    fn folder_status(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<FolderStatus>;

    /// Enumerate every message in the folder in UID order. `read_only`
    /// controls whether the `\Recent` flag is cleared on the messages
    /// returned (RFC 3501 §6.3.1: SELECT clears, EXAMINE does not).
    fn list_messages(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        read_only:  bool,
    )
        -> Outcome<Vec<MessageMeta>>;

    /// Read the raw RFC 5322 bytes for one message.
    fn fetch_bytes(
        &self,
        user:   &MailUser,
        folder: &FolderName,
        uid:    MessageUid,
    )
        -> Outcome<Vec<u8>>;

    /// Replace the flag set for one message. Returns the new flag set
    /// (which may differ from the input if the implementation enforces
    /// invariants like always-clear `\Recent`).
    fn set_flags(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        uid:        MessageUid,
        flags:      MessageFlags,
    )
        -> Outcome<MessageFlags>;

    /// Permanently remove every message in the folder whose `\Deleted`
    /// flag is set. Returns the list of UIDs that were expunged, in the
    /// order they were removed -- IMAP requires sending an `EXPUNGE`
    /// untagged response for each one.
    fn expunge(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<Vec<MessageUid>>;

    /// Create a folder if it does not already exist.
    fn create_folder(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<()>;

    /// Subscribe a folder. Hematite-side bookkeeping only; the trait does
    /// nothing fancy here, but Thunderbird issues `LSUB` and expects to
    /// see what it `SUBSCRIBE`d earlier.
    fn subscribe(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<()>;

    /// List every subscribed folder for this user.
    fn list_subscribed(&self, user: &MailUser) -> Outcome<Vec<FolderName>>;
}
