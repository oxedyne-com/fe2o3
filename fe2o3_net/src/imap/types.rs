//! The plain data the IMAP client returns and a sync engine holds: a mailbox's LIST entry and
//! special-use role, the state a SELECT leaves, a fetched message, and the UIDPLUS response codes.
//!
//! Tokio-free, so the folder sync engine in `fe2o3_mail::sync` can hold them in a browser; the
//! client re-exports every one, so `imap::client::FetchWhat` and the rest still resolve (F3).

use crate::imap::uid_set::UidSet;


/// One mailbox as reported by `LIST`.
#[derive(Clone, Debug)]
pub struct MailboxInfo {
    pub name:       String,         // as the server spells it
    pub delimiter:  Option<char>,   // `None` for a flat namespace
    pub attrs:      Vec<String>,    // e.g. `\HasChildren`, `\Noselect`, `\Sent`
}

impl MailboxInfo {
    /// A mailbox that exists only to hold children cannot itself be selected.
    pub fn selectable(&self) -> bool {
        !self.attrs.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect"))
    }

    /// The RFC 6154 special-use role, where the server declared one. This is how a client
    /// recognises the Sent folder on a mailbox whose
    /// names are localised -- Gmail spells it "[Gmail]/Gesendet" for a German
    /// account, but the attribute is `\Sent` everywhere.
    pub fn special_use(&self) -> Option<SpecialUse> {
        for a in &self.attrs {
            let role = match a.to_uppercase().as_str() {
                "\\SENT"     => Some(SpecialUse::Sent),
                "\\DRAFTS"   => Some(SpecialUse::Drafts),
                "\\TRASH"    => Some(SpecialUse::Trash),
                "\\JUNK"     => Some(SpecialUse::Junk),
                "\\ARCHIVE"  => Some(SpecialUse::Archive),
                "\\ALL"      => Some(SpecialUse::All),
                "\\FLAGGED"  => Some(SpecialUse::Flagged),
                _            => None,
            };
            if role.is_some() {
                return role;
            }
        }
        None
    }
}

/// The special-use roles of RFC 6154, one per mailbox at most.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialUse {
    Sent,
    Drafts,
    Trash,      // deleted messages await expunge
    Junk,       // spam
    Archive,    // filed out of the inbox
    All,        // every message however filed, Gmail's "All Mail"
    Flagged,    // starred or flagged
}

/// The state of a mailbox after `SELECT` or `EXAMINE`.
///
/// `uid_validity` is the one field a synchronising caller must persist:
/// if it changes, every UID it has cached is meaningless and the mailbox
/// must be re-read from scratch.
#[derive(Clone, Debug, Default)]
pub struct MailboxStatus {
    pub name:           String,
    pub exists:         u32,            // messages present
    pub recent:         u32,            // messages flagged `\Recent`
    pub uid_validity:   u32,            // UID namespace generation
    pub uid_next:       u32,            // the UID the next arrival will be given
    pub flags:          Vec<String>,    // defined in this mailbox
    pub read_only:      bool,           // `EXAMINE`, or a server that downgraded the `SELECT`
    // RFC 7162. `None` from a server without CONDSTORE in use, and for a mailbox it answers
    // `NOMODSEQ`, which keeps no mod-sequences.
    pub highest_modseq: Option<u64>,
    pub vanished:       UidSet,                 // QRESYNC: known UIDs expunged since the modseq
    pub changed:        Vec<FetchedMessage>,    // QRESYNC: flags changed since the modseq
}

/// What a `SELECT` or `EXAMINE` asks for beyond the mailbox itself (RFC 7162).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SelectWith {
    #[default]
    Plain,
    CondStore,  // `(CONDSTORE)`: the mailbox's HIGHESTMODSEQ, and a MODSEQ on every fetch
    // `(QRESYNC (uid_validity modseq known))`: the reply carries the flags changed since `modseq`
    // and those of `known` (every UID, where `None`) that have vanished. A server whose
    // UIDVALIDITY is no longer `uid_validity` ignores the rest, and the caller starts again.
    QResync {
        uid_validity:   u32,
        modseq:         u64,
        known:          Option<UidSet>,
    },
}

/// A mailbox's counts by `STATUS`, without selecting it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FolderStatus {
    pub messages:       u32,
    pub uid_next:       u32,
    pub uid_validity:   u32,
    pub unseen:         u32,
    pub highest_modseq: Option<u64>,    // asked for only where CONDSTORE is offered
}

/// What a `UID FETCH ... (CHANGEDSINCE n)` returned.
#[derive(Clone, Debug, Default)]
pub struct FetchChanged {
    pub messages:   Vec<FetchedMessage>,    // changed since the modseq
    pub vanished:   UidSet,                 // QRESYNC only: expunged since the modseq
}

/// How much of each message to pull down. Where the mailbox keeps mod-sequences each fetch also
/// asks for `MODSEQ`, and where the server offers `X-GM-EXT-1`, for Gmail's message ID and labels;
/// `MessageId` alone asks for nothing it does not name, being a check that must stay a few bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchWhat {
    Flags,      // UID and flags, the cheapest poll of what changed
    Meta,       // and internal date and size; no body at all
    Headers,    // and the RFC 5322 header block
    Full,       // and the whole message
    MessageId,  // UID, size and the `Message-ID` field alone, to confirm a search's match
}

impl FetchWhat {
    /// `BODY.PEEK` rather than `BODY`, so reading a message does not silently mark it `\Seen` --
    /// a sync should be invisible to whoever is reading the mailbox elsewhere.
    pub fn items(&self) -> &'static str {
        match self {
            Self::Flags     => "UID FLAGS",
            Self::Meta      => "UID FLAGS INTERNALDATE RFC822.SIZE",
            Self::Headers   => "UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER]",
            Self::Full      => "UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[]",
            Self::MessageId => "UID RFC822.SIZE BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)]",
        }
    }
}

/// One message returned by `UID FETCH`.
#[derive(Clone, Debug, Default)]
pub struct FetchedMessage {
    pub seq:            u32,            // sequence number in the current mailbox view
    pub uid:            u32,            // stable within the mailbox's current `uid_validity`
    pub flags:          Vec<String>,    // e.g. `\Seen`, `\Answered`
    // `INTERNALDATE` exactly as the server gave it, e.g. `01-Jan-2026 09:15:00 +0000`. Left as
    // the server's string, because only the caller knows what calendar it wants it in.
    pub internal_date:  String,
    pub size:           u32,            // `RFC822.SIZE`, which may exceed `body.len()`
    pub body:           Vec<u8>,        // whole message, header block, or empty, per FetchWhat
    pub modseq:         Option<u64>,    // RFC 7162 `MODSEQ`, where the mailbox keeps them
    pub gm_msgid:       Option<u64>,    // Gmail's `X-GM-MSGID`, stable across its labels
    pub gm_labels:      Vec<String>,    // Gmail's `X-GM-LABELS`, e.g. `\Inbox`, `Important`
}

/// Where `UID COPY` or `UID MOVE` put each message, from the `UIDPLUS` `COPYUID` response code.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CopyUid {
    pub uid_validity:   u32,                // of the destination
    pub pairs:          Vec<(u32, u32)>,    // source UID, destination UID
}

/// The new message's place, from the `UIDPLUS` `APPENDUID` response code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppendUid {
    pub uid_validity:   u32,
    pub uid:            u32,
}
