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
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::time::SystemTime;


/// Always the plain UTF-8, user-facing form. Folder names travel the wire in
/// IMAP modified UTF-7, and the conversion happens at the wire layer.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FolderName(pub String);

impl FolderName {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// IMAP requires these to increase monotonically within a folder (RFC 3501
/// §2.3.1.1); each `MailStore` implementation honours that itself.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MessageUid(pub u32);

/// The small set Thunderbird uses on a steady-state session. Custom keywords
/// are out of scope for the MVP.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MessageFlags {
    pub seen:       bool,
    pub answered:   bool,
    pub flagged:    bool,
    pub deleted:    bool,   // marked for EXPUNGE
    pub draft:      bool,
    pub recent:     bool,   // cleared by the next session opening the folder R/W
}

impl MessageFlags {
    /// The space-separated atom list only, e.g. `\Seen \Flagged`; the caller
    /// supplies the surrounding parentheses.
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

    /// Is `flag` set? Named with or without its leading backslash; anything
    /// outside the known set reads as clear.
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

    /// A name outside the known set is silently ignored, since a client may
    /// `STORE` a custom keyword the MVP does not carry.
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

/// Answers FETCH FLAGS, INTERNALDATE, RFC822.SIZE and UID without re-reading
/// the raw message bytes.
#[derive(Clone, Debug)]
pub struct MessageMeta {
    pub uid:        MessageUid,
    pub size:       u64,        // raw message, bytes
    pub internal:   SystemTime, // when the server stored it, RFC 3501 §2.3.3
    pub flags:      MessageFlags,
}

#[derive(Clone, Debug, Default)]
pub struct FolderStatus {
    pub exists:         u32,    // messages present, post-expunge
    pub recent:         u32,    // messages flagged \Recent
    pub unseen:         u32,    // messages without \Seen
    pub uid_validity:   u32,    // changes whenever the UID space is reset
    pub uid_next:       u32,    // UID the next appended message will receive
}

/// The result of `UserStore::authenticate` (see [`crate::mail::user`]). Which
/// field a backend keys off is its own business: a Maildir store needs only
/// the delivery key, an Ozone-backed store would key off the user id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailUser {
    pub local:          String, // left of `@`, as authenticated
    pub domain:         String, // right of `@`
    pub delivery_key:   String, // mailbox root: a path or opaque key, set by the UserStore
}

impl MailUser {
    pub fn address(&self) -> String {
        fmt!("{}@{}", self.local, self.domain)
    }
}

/// Every method takes a `MailUser`, so one store hosts many accounts.
///
/// The trait is deliberately synchronous: the IMAP and SMTP servers wrap each
/// call in `tokio::task::spawn_blocking` so the underlying I/O does not block
/// the runtime. Async would force every implementation through
/// `Pin<Box<dyn Future>>` for no practical gain on a single-host mailbox.
pub trait MailStore: Clone + Send + Sync + 'static {
    /// Creates the folders, INBOX included. Idempotent.
    fn ensure_user(&self, user: &MailUser) -> Outcome<()>;

    /// `bytes` must be a fully-formed RFC 5322 message; the UID returned is
    /// allocated monotonically within the folder.
    fn append(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        bytes:      &[u8],
        flags:      MessageFlags,
        internal:   Option<SystemTime>,
    )
        -> Outcome<MessageUid>;

    /// Recursively.
    fn list_folders(&self, user: &MailUser) -> Outcome<Vec<FolderName>>;

    fn folder_status(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<FolderStatus>;

    /// In UID order. `read_only` withholds the clearing of `\Recent` on the
    /// messages returned (RFC 3501 §6.3.1: SELECT clears, EXAMINE does not).
    fn list_messages(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        read_only:  bool,
    )
        -> Outcome<Vec<MessageMeta>>;

    /// The stored bytes exactly, since `FETCH BODY[]` must reproduce them.
    fn fetch_bytes(
        &self,
        user:   &MailUser,
        folder: &FolderName,
        uid:    MessageUid,
    )
        -> Outcome<Vec<u8>>;

    /// The flag set returned may differ from the one given, where the
    /// implementation enforces an invariant such as always-clear `\Recent`.
    fn set_flags(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        uid:        MessageUid,
        flags:      MessageFlags,
    )
        -> Outcome<MessageFlags>;

    /// Removes every message flagged `\Deleted`, and returns their UIDs in
    /// removal order, since IMAP wants one untagged `EXPUNGE` per UID in that
    /// same order.
    fn expunge(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<Vec<MessageUid>>;

    /// Idempotent.
    fn create_folder(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<()>;

    /// Bookkeeping only, but it has to persist: Thunderbird issues `LSUB` and
    /// expects back what it subscribed to earlier.
    fn subscribe(
        &self,
        user:   &MailUser,
        folder: &FolderName,
    )
        -> Outcome<()>;

    fn list_subscribed(&self, user: &MailUser) -> Outcome<Vec<FolderName>>;
}
