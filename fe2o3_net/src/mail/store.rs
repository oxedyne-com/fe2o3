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
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::time::SystemTime;


/// Runs the synchronous `f` on tokio's blocking pool and awaits it, so a store or user
/// lookup that waits on a lock or the disk stalls only its own session, never a runtime
/// worker and every other task queued on it.
#[cfg(feature = "async")]
pub async fn blocking<T, F>(f: F) -> Outcome<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Outcome<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result)  => result,
        Err(e)      => Err(err!(e, "A blocking mail store call panicked or was cancelled."; Thread)),
    }
}


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

    /// Which system flag `flag` names, with or without its leading backslash and in any
    /// case (RFC 3501 §2.3.2), as an index into [`Self::SYSTEM`].
    fn system_index(flag: &str) -> Option<usize> {
        let name = flag.strip_prefix('\\').unwrap_or(flag);
        Self::SYSTEM.iter().position(|s| s.eq_ignore_ascii_case(name))
    }

    const SYSTEM: [&'static str; 6] = ["Seen", "Answered", "Flagged", "Deleted", "Draft", "Recent"];

    fn field_mut(&mut self, i: usize) -> Option<&mut bool> {
        match i {
            0 => Some(&mut self.seen),
            1 => Some(&mut self.answered),
            2 => Some(&mut self.flagged),
            3 => Some(&mut self.deleted),
            4 => Some(&mut self.draft),
            5 => Some(&mut self.recent),
            _ => None,
        }
    }

    /// Is `flag` set? Named with or without its leading backslash, in any case; anything
    /// outside the known set reads as clear.
    pub fn has(&self, flag: &str) -> bool {
        match Self::system_index(flag) {
            Some(0) => self.seen,
            Some(1) => self.answered,
            Some(2) => self.flagged,
            Some(3) => self.deleted,
            Some(4) => self.draft,
            Some(5) => self.recent,
            _       => false,
        }
    }

    /// A name outside the known set is silently ignored, since a client may
    /// `STORE` a custom keyword the MVP does not carry.
    pub fn set(&mut self, flag: &str, on: bool) {
        if let Some(b) = Self::system_index(flag).and_then(|i| self.field_mut(i)) {
            *b = on;
        }
    }
}

/// A flag change as the client asked for it (RFC 3501 §6.4.6 `+FLAGS`, `-FLAGS`, `FLAGS`),
/// applied to the stored flags at the moment of writing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlagUpdate {
    Add(MessageFlags),
    Remove(MessageFlags),
    Replace(MessageFlags),
}

impl FlagUpdate {
    /// The flags after this change to `cur`. `\Recent` belongs to the session and no STORE
    /// alters it.
    pub fn apply(&self, cur: MessageFlags) -> MessageFlags {
        let (m, op): (&MessageFlags, fn(bool, bool) -> bool) = match self {
            Self::Add(m)        => (m, |c, x| c || x),
            Self::Remove(m)     => (m, |c, x| c && !x),
            Self::Replace(m)    => (m, |_, x| x),
        };
        MessageFlags {
            seen:       op(cur.seen,        m.seen),
            answered:   op(cur.answered,    m.answered),
            flagged:    op(cur.flagged,     m.flagged),
            deleted:    op(cur.deleted,     m.deleted),
            draft:      op(cur.draft,       m.draft),
            recent:     cur.recent,
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
    // Addresses besides its own that the account may send as: the identities a mail client
    // sends through this one login. Empty for most accounts.
    pub send_as:        Vec<String>,
}

impl MailUser {
    pub fn address(&self) -> String {
        fmt!("{}@{}", self.local, self.domain)
    }

    /// May this account send mail as `address`? Its own address and those listed in
    /// `send_as`, compared without regard to case.
    pub fn may_send_as(&self, address: &str) -> bool {
        let a = address.trim().to_lowercase();
        a == self.address().to_lowercase()
            || self.send_as.iter().any(|s| s.trim().to_lowercase() == a)
    }
}

/// Every method takes a `MailUser`, so one store hosts many accounts.
///
/// The trait is deliberately synchronous: the IMAP server runs each call through
/// [`blocking`], on tokio's blocking pool, so a call that waits (a Maildir folder
/// read queues behind the folder's flock) holds no runtime worker. Async would force every implementation through
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

    /// [`Self::append`] for a delivery, which a sender retries after a temporary failure: a
    /// message the folder already holds, the same Message-ID and the same fields and body as its
    /// sender set them (the fields added in transit aside, as
    /// [`crate::email::header::sender_parts`] reads them), is not filed again, and the UID of
    /// the copy there is answered, so a retry after a partial failure duplicates nothing. A
    /// different message reusing a Message-ID, and a message without one, are filed. IMAP
    /// APPEND files what the client sends, and does not come this way.
    fn deliver(
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

    /// The stored bytes exactly, since `FETCH BODY[]` must reproduce them. `None` when the
    /// message is gone, expunged by another session.
    fn fetch_bytes(
        &self,
        user:   &MailUser,
        folder: &FolderName,
        uid:    MessageUid,
    )
        -> Outcome<Option<Vec<u8>>>;

    /// Hands `f` the stored bytes of each message of `uids`, in the order given. A UID the
    /// folder no longer holds is skipped. A store that can read many messages under one
    /// look at the folder overrides this; the default fetches each in turn.
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
        for uid in uids {
            if let Some(bytes) = res!(self.fetch_bytes(user, folder, *uid)) {
                res!(f(*uid, &bytes));
            }
        }
        Ok(())
    }

    /// Applies `op` to the message's flags as they stand when the store holds the folder,
    /// not as a session last read them, so a flag another session set meanwhile survives.
    /// `None` when the message is gone, expunged by another session.
    fn update_flags(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        uid:        MessageUid,
        op:         FlagUpdate,
    )
        -> Outcome<Option<MessageFlags>>;

    /// [`Self::update_flags`] for each of `uids`, the answers in the same order. A store that
    /// can should do it under one hold of the folder; a caller bounds `uids`, since no one else
    /// reaches the folder meanwhile.
    fn update_flags_many(
        &self,
        user:       &MailUser,
        folder:     &FolderName,
        uids:       &[MessageUid],
        op:         FlagUpdate,
    )
        -> Outcome<Vec<Option<MessageFlags>>>
    {
        let mut out = Vec::with_capacity(uids.len());
        for uid in uids {
            out.push(res!(self.update_flags(user, folder, *uid, op)));
        }
        Ok(out)
    }

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
