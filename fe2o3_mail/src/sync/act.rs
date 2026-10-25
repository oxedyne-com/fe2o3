//! Acting on the server for a journal entry, with every outcome definite.
//!
//! - A flag change is a `+FLAGS`/`-FLAGS` delta, never an absolute list. On a UID the folder no
//!   longer holds it is [`ActOutcome::Gone`]: definite, not retried, and not an alarm.
//! - A move whose source UID is gone is `Done` when the target holds the message's `Message-ID`,
//!   `Superseded` when any other folder does (another device filed it first), and `Lost` only
//!   when none does.
//! - A `Message-ID` match is never trusted from `SEARCH` alone: each is confirmed by fetching
//!   that header field ([`find_message_id`]).
//! - On a UIDVALIDITY that is no longer the entry's, the message is re-found by `Message-ID` and
//!   size, and only a unique match is acted on; otherwise the entry is `Stuck`.
//! - Without `MOVE`, a move is `UID COPY`, `+FLAGS \Deleted` and `UID EXPUNGE`; without `UIDPLUS`
//!   the action refuses, because a plain `EXPUNGE` would remove other `\Deleted` messages too.

use crate::sync::{
    bare_message_id,
    header_message_id,
    Caps,
    MailSession,
    Roles,
    SelectWith,
    SpecialUse,
    UidSet,
    FetchWhat,
    AppendUid,
};

use oxedyne_fe2o3_core::prelude::*;


/// Where a move goes: a role, resolved by its attribute, or a path the user chose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    Inbox,
    Role(SpecialUse),
    Path(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Flags { add: Vec<String>, remove: Vec<String> },
    Move { to: Target },
    Archive,    // by the provider's rule
    Trash,
    Junk,
    NotJunk,    // back to the inbox
    Expunge,
}

impl Action {
    /// Does the action take the message out of its folder?
    pub fn removes(&self) -> bool {
        !matches!(self, Self::Flags { .. })
    }
}

/// One journal entry, as the device recorded it at the press.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Entry {
    pub id:             u64,
    pub mailbox:        String,
    pub uid_validity:   u32,
    pub uid:            u32,
    pub message_id:     String,         // empty where the message has none
    pub size:           u32,            // RFC822.SIZE, which with the Message-ID re-finds it
    pub year:           Option<u16>,    // of its Date, for a yearly archive
    pub action:         Action,
}

impl Default for Action {
    fn default() -> Self {
        Self::Flags { add: Vec::new(), remove: Vec::new() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActOutcome {
    Done,
    Gone,                   // the UID is no longer in the folder: a flag change has nothing to do
    Superseded(String),     // another device filed it first, here
    Lost,                   // found in no folder at all, to be shown, never dropped
    Stuck(String),          // cannot act without guessing; kept until the user decides
}

/// The account's server, which fixes its archive and its Sent filing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provider {
    Steel,
    Gmail,
    Office365,
    Generic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveRule {
    Yearly,     // `<Archive>/<year>` of the message's Date, created when absent
    AllFlat,    // a move to the `\All` role, which drops Gmail's Inbox label
    Role,       // the `\Archive` role, flat
}

/// Who puts a sent message's copy in Sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SentFiling {
    Append,         // the client, by `APPEND`, after a confirmed search finds no copy
    // The server's SMTP does, asynchronously, so a search just after the send can miss a copy
    // about to appear: never `APPEND`, which would double Sent.
    ServerFiles,
}

impl Provider {
    pub fn archive_rule(&self) -> ArchiveRule {
        match self {
            Self::Gmail                 => ArchiveRule::AllFlat,
            Self::Office365             => ArchiveRule::Role,
            Self::Steel | Self::Generic => ArchiveRule::Yearly,
        }
    }

    pub fn sent_filing(&self) -> SentFiling {
        match self {
            Self::Gmail | Self::Office365   => SentFiling::ServerFiles,
            Self::Steel | Self::Generic     => SentFiling::Append,
        }
    }
}

/// How a sent message's copy was filed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Filed {
    Server,                     // the server files it; confirmed later by a sync of Sent
    Already,                    // a confirmed copy was there
    Appended(Option<AppendUid>),
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ACTING                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Carries out one journal entry. An `Err` is a transport failure, which the journal may retry;
/// every outcome is definite.
pub async fn act<S: MailSession>(
    s:          &mut S,
    caps:       &Caps,
    roles:      &Roles,
    provider:   Provider,
    e:          &Entry,
)
    -> Outcome<ActOutcome>
{
    // Where the message goes, resolved before anything is touched.
    let dest = match &e.action {
        Action::Flags { .. } | Action::Expunge => None,
        other => match res!(destination(s, roles, provider, e, other).await) {
            Ok(d)       => Some(d),
            Err(why)    => return Ok(ActOutcome::Stuck(why)),
        },
    };

    let st = res!(s.select(&e.mailbox, &SelectWith::Plain).await);
    let uid = if st.uid_validity == e.uid_validity {
        Some(e.uid)
    } else {
        // The UIDs the entry was made under are void: re-find the message, only uniquely.
        if e.message_id.is_empty() {
            return Ok(ActOutcome::Stuck(fmt!("Cannot identify the message: {} was renumbered \
                and it has no Message-ID.", e.mailbox)));
        }
        let found: Vec<u32> = res!(find_message_id(s, &e.message_id).await).into_iter()
            .filter(|(_, size)| *size == e.size)
            .map(|(uid, _)| uid)
            .collect();
        match found.len() {
            1 => Some(found[0]),
            n => return Ok(ActOutcome::Stuck(fmt!("Cannot identify the message: {} was renumbered \
                and {} of its messages have this one's Message-ID and size.", e.mailbox, n))),
        }
    };
    let present = match uid {
        Some(u) => res!(s.uid_fetch(&UidSet::from([u]), FetchWhat::Flags).await)
            .iter().any(|m| m.uid == u),
        None    => false,
    };

    match (&e.action, dest, present, uid) {
        (Action::Flags { add, remove }, _, true, Some(u)) => {
            let add: Vec<&str> = add.iter().map(|f| f.as_str()).collect();
            let remove: Vec<&str> = remove.iter().map(|f| f.as_str()).collect();
            res!(s.uid_store_flags(&UidSet::from([u]), &add, &remove).await);
            Ok(ActOutcome::Done)
        }
        (Action::Flags { .. }, _, _, _) | (Action::Expunge, _, false, _) => Ok(ActOutcome::Gone),
        (Action::Expunge, _, true, Some(u)) => {
            if !caps.uidplus {
                return Ok(ActOutcome::Stuck(no_uid_expunge()));
            }
            let one = UidSet::from([u]);
            res!(s.uid_store_flags(&one, &["\\Deleted"], &[]).await);
            res!(s.uid_expunge(&one).await);
            Ok(ActOutcome::Done)
        }
        (_, Some(to), true, Some(u)) => {
            if to == e.mailbox {
                return Ok(ActOutcome::Done);
            }
            let one = UidSet::from([u]);
            if caps.mov {
                res!(s.uid_move(&one, &to).await);
            } else if caps.uidplus {
                res!(s.uid_copy(&one, &to).await);
                res!(s.uid_store_flags(&one, &["\\Deleted"], &[]).await);
                res!(s.uid_expunge(&one).await);
            } else {
                return Ok(ActOutcome::Stuck(no_uid_expunge()));
            }
            Ok(ActOutcome::Done)
        }
        (_, Some(to), _, _) => locate(s, roles, e, &to).await,
        (_, None, _, _) => Err(err!("Entry {}: a move with no destination.", e.id; Bug)),
    }
}

/// The source no longer holds the message: did the move happen, did another device file it, or
/// is it gone from everywhere?
async fn locate<S: MailSession>(s: &mut S, roles: &Roles, e: &Entry, to: &str)
    -> Outcome<ActOutcome>
{
    if e.message_id.is_empty() {
        return Ok(ActOutcome::Stuck(fmt!("The message has left {} and has no Message-ID to find \
            it by.", e.mailbox)));
    }
    if roles.exists(to) {
        res!(s.select(to, &SelectWith::Plain).await);
        if !res!(find_message_id(s, &e.message_id).await).is_empty() {
            return Ok(ActOutcome::Done);
        }
    }
    let others: Vec<String> = roles.selectable()
        .filter(|m| *m != to && *m != e.mailbox)
        .map(|m| m.to_string())
        .collect();
    for mb in others {
        res!(s.select(&mb, &SelectWith::Plain).await);
        if !res!(find_message_id(s, &e.message_id).await).is_empty() {
            return Ok(ActOutcome::Superseded(mb));
        }
    }
    Ok(ActOutcome::Lost)
}

/// The mailbox an action moves to, created where the provider's rule needs one that is absent;
/// the inner `Err` says why it cannot be resolved without a guess.
async fn destination<S: MailSession>(
    s:          &mut S,
    roles:      &Roles,
    provider:   Provider,
    e:          &Entry,
    action:     &Action,
)
    -> Outcome<std::result::Result<String, String>>
{
    let role = |r: SpecialUse| roles.role(r).map(|n| n.to_string())
        .ok_or_else(|| fmt!("The account has no {:?} folder.", r));
    let to = match action {
        Action::Move { to: Target::Inbox } | Action::NotJunk => Ok(roles.inbox.clone()),
        Action::Move { to: Target::Role(r) }    => role(*r),
        Action::Move { to: Target::Path(p) }    => Ok(p.clone()),
        Action::Trash                           => role(SpecialUse::Trash),
        Action::Junk                            => role(SpecialUse::Junk),
        Action::Archive => match provider.archive_rule() {
            ArchiveRule::AllFlat    => role(SpecialUse::All),
            ArchiveRule::Role       => Ok(roles.role(SpecialUse::Archive)
                .unwrap_or("Archive").to_string()),
            ArchiveRule::Yearly     => {
                let year = match e.year {
                    Some(y) => y,
                    None    => return Ok(Err(fmt!("The message has no Date to archive it by."))),
                };
                let base = roles.role(SpecialUse::Archive).unwrap_or("Archive");
                let delim = roles.mailboxes.iter()
                    .find(|m| m.name == base)
                    .and_then(|m| m.delimiter)
                    .or(roles.delimiter)
                    .unwrap_or('/');
                Ok(fmt!("{}{}{}", base, delim, year))
            }
        },
        Action::Flags { .. } | Action::Expunge  => Err(fmt!("Not a move.")),
    };
    if let Ok(name) = &to {
        if !roles.exists(name) {
            if let Action::Archive = action {
                res!(s.create(name).await);
            }
        }
    }
    Ok(to)
}

fn no_uid_expunge() -> String {
    fmt!("The server offers neither MOVE nor UIDPLUS, and a plain EXPUNGE would remove other \
        deleted messages too.")
}

/// The UIDs and sizes of the messages in the selected folder whose `Message-ID` is `mid`. The
/// server's `SEARCH` gives candidates only, since a server may match a substring or anything at
/// all; each is confirmed by fetching its `Message-ID` field, a few bytes.
pub async fn find_message_id<S: MailSession>(s: &mut S, mid: &str) -> Outcome<Vec<(u32, u32)>> {
    let bare = bare_message_id(mid);
    if bare.is_empty() || bare.chars().any(|c| c.is_control() || c == '"' || c == '\\') {
        return Err(err!("The Message-ID {:?} cannot be searched for: empty, or with a control \
            character, quote or backslash.", mid; Invalid, Input));
    }
    let uids = res!(s.uid_search(&fmt!("HEADER Message-ID \"{}\"", bare)).await);
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let asked = UidSet::from(&uids);
    Ok(res!(s.uid_fetch(&asked, FetchWhat::MessageId).await).into_iter()
        .filter(|m| asked.contains(m.uid) && header_message_id(&m.body).as_deref() == Some(bare))
        .map(|m| (m.uid, m.size))
        .collect())
}

/// Files a sent message's copy in Sent by the provider's rule. Only an `Append` account is
/// searched, and only a confirmed absence is appended to.
pub async fn file_sent<S: MailSession>(
    s:          &mut S,
    roles:      &Roles,
    provider:   Provider,
    mid:        &str,
    raw:        &[u8],
)
    -> Outcome<Filed>
{
    if provider.sent_filing() == SentFiling::ServerFiles {
        return Ok(Filed::Server);
    }
    let sent = res!(roles.role(SpecialUse::Sent)
        .ok_or_else(|| err!("The account has no folder with the \\Sent role."; Missing)));
    let sent = sent.to_string();
    res!(s.select(&sent, &SelectWith::Plain).await);
    if !res!(find_message_id(s, mid).await).is_empty() {
        return Ok(Filed::Already);
    }
    Ok(Filed::Appended(res!(s.append(&sent, &["\\Seen"], raw).await)))
}
