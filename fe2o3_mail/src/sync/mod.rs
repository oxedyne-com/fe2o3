//! The folder sync engine: what changed in one IMAP folder since the device last looked, as a
//! bounded [`Delta`] the device applies to its cache.
//!
//! The engine is written once, against [`MailSession`], so the gateway's `ImapClient` and a
//! browser's tunnel drive the same algorithm. It owns no socket, no clock and no runtime, and
//! builds for `wasm32-unknown-unknown` with the crate's default features off.
//!
//! A sync works by capability tier ([`Tier`]):
//! - **QRESYNC** (RFC 7162): `SELECT (QRESYNC ...)` returns the changed flags and the `VANISHED`
//!   UIDs directly;
//! - **CONDSTORE**: `UID FETCH held (FLAGS) (CHANGEDSINCE m)`, then `UID SEARCH UID held` for
//!   what vanished;
//! - **full reconcile** (Steel, Office 365): `UID FETCH (FLAGS)` over the window on every poll and
//!   over the whole held set every `sweep_every`th, so a message read elsewhere long ago does not
//!   keep a stale state here; `UID SEARCH ALL` against the held set for what vanished.
//!
//! Each new message says whether it is an arrival ([`NewMessage::arrival`]): only what came after
//! the folder became known to the device is, never what a capped first read or a rebuild left in
//! the backlog, so a trigger that fires on arrivals does not fire on old mail.
//!
//! The device advances its [`Cursor`] only once the delta's writes have landed ([`apply`]), so a
//! crash replays a delta and never skips one, and a replayed apply changes nothing. A UIDVALIDITY
//! change makes the delta a [`Reset`]: the device sets its one old generation aside, rebuilds,
//! and drops the old one when the rebuild completes.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod act;
pub mod apply;
#[cfg(feature = "server")]
pub mod imap;
#[cfg(feature = "jdat")]
pub mod wire;

pub use act::{
    act,
    file_sent,
    find_message_id,
    ActOutcome,
    Action,
    ArchiveRule,
    Entry,
    Filed,
    Provider,
    SentFiling,
    Target,
};
pub use apply::{
    apply,
    maildir_name,
    parse_maildir_name,
    rebase_flags,
    Applied,
    MsgKey,
    SyncStore,
};

use crate::message;

use oxedyne_fe2o3_core::prelude::*;
pub use oxedyne_fe2o3_net::imap::{
    types::{
        AppendUid,
        CopyUid,
        FetchChanged,
        FetchWhat,
        FetchedMessage,
        MailboxInfo,
        MailboxStatus,
        SelectWith,
        SpecialUse,
    },
    uid_set::UidSet,
};

use std::collections::BTreeMap;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SESSION                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// One logged-in IMAP conversation, as the engine needs it: the wire verbs and nothing of policy,
/// so that every rule (tiers, the Message-ID confirmation, the outcomes of an action) lives in
/// the engine, once, whichever transport carries it.
///
/// `async fn` without a `Send` bound on purpose: a browser's tunnel is single-threaded, and the
/// engine is only ever driven by one task. A UID the server does not hold is passed over in a
/// fetch, a store or an expunge, never an error.
#[allow(async_fn_in_trait)]
pub trait MailSession {
    async fn capabilities(&mut self) -> Outcome<Vec<String>>;
    async fn list(&mut self) -> Outcome<Vec<MailboxInfo>>;
    async fn create(&mut self, mailbox: &str) -> Outcome<()>;
    /// `SELECT`; a QRESYNC select must have QRESYNC enabled first, which the session does.
    async fn select(&mut self, mailbox: &str, with: &SelectWith) -> Outcome<MailboxStatus>;
    async fn uid_fetch(&mut self, uids: &UidSet, what: FetchWhat) -> Outcome<Vec<FetchedMessage>>;
    async fn uid_fetch_changed(&mut self, uids: &UidSet, what: FetchWhat, modseq: u64)
        -> Outcome<FetchChanged>;
    async fn uid_search(&mut self, criteria: &str) -> Outcome<Vec<u32>>;
    /// `+FLAGS` and `-FLAGS` only; there is no absolute set.
    async fn uid_store_flags(&mut self, uids: &UidSet, add: &[&str], remove: &[&str]) -> Outcome<()>;
    async fn uid_copy(&mut self, uids: &UidSet, to: &str) -> Outcome<Option<CopyUid>>;
    async fn uid_move(&mut self, uids: &UidSet, to: &str) -> Outcome<Option<CopyUid>>;
    async fn uid_expunge(&mut self, uids: &UidSet) -> Outcome<()>;
    async fn append(&mut self, mailbox: &str, flags: &[&str], raw: &[u8]) -> Outcome<Option<AppendUid>>;
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CAPABILITIES AND ROLES                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// What the engine needs to know of a server's `CAPABILITY` list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Caps {
    pub qresync:    bool,
    pub condstore:  bool,   // true wherever `qresync` is
    pub mov:        bool,   // RFC 6851 `MOVE`
    pub uidplus:    bool,   // RFC 4315, for `UID EXPUNGE`
    pub gmail:      bool,   // `X-GM-EXT-1`, so `X-GM-MSGID` is fetched
}

impl Caps {
    pub fn from_list(caps: &[String]) -> Self {
        let has = |n: &str| caps.iter().any(|c| c.eq_ignore_ascii_case(n));
        let qresync = has("QRESYNC");
        Self {
            qresync,
            condstore:  qresync || has("CONDSTORE"),
            mov:        has("MOVE"),
            uidplus:    has("UIDPLUS"),
            gmail:      has("X-GM-EXT-1"),
        }
    }

    pub fn tier(&self) -> Tier {
        match (self.qresync, self.condstore) {
            (true, _)       => Tier::QResync,
            (false, true)   => Tier::CondStore,
            (false, false)  => Tier::Full,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tier {
    QResync,
    CondStore,
    Full,
}

/// The folders of one account by RFC 6154 role. A role is found by its attribute only, never by
/// a folder's name, which is localised ("[Gmail]/Gesendet") and which a user may reuse.
#[derive(Clone, Debug, Default)]
pub struct Roles {
    pub inbox:      String,                         // as the server spells it
    pub roles:      BTreeMap<RoleKey, String>,      // role to mailbox name
    pub delimiter:  Option<char>,                   // the hierarchy delimiter of the first entry
    pub mailboxes:  Vec<MailboxInfo>,               // every mailbox, as listed
}

/// `SpecialUse` as a map key.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RoleKey(u8);

impl From<SpecialUse> for RoleKey {
    fn from(r: SpecialUse) -> Self {
        RoleKey(match r {
            SpecialUse::Sent    => 0,
            SpecialUse::Drafts  => 1,
            SpecialUse::Trash   => 2,
            SpecialUse::Junk    => 3,
            SpecialUse::Archive => 4,
            SpecialUse::All     => 5,
            SpecialUse::Flagged => 6,
        })
    }
}

impl Roles {
    /// The first mailbox to declare a role keeps it.
    pub fn from_list(list: &[MailboxInfo]) -> Self {
        let mut r = Self {
            inbox:      "INBOX".to_string(),
            delimiter:  list.iter().find_map(|m| m.delimiter),
            mailboxes:  list.to_vec(),
            ..Default::default()
        };
        for m in list {
            if m.name.eq_ignore_ascii_case("INBOX") {
                r.inbox = m.name.clone();
            }
            if let Some(role) = m.special_use() {
                r.roles.entry(role.into()).or_insert_with(|| m.name.clone());
            }
        }
        r
    }

    pub fn role(&self, role: SpecialUse) -> Option<&str> {
        self.roles.get(&role.into()).map(|s| s.as_str())
    }

    pub fn exists(&self, mailbox: &str) -> bool {
        self.mailboxes.iter().any(|m| m.name == mailbox)
    }

    /// Every mailbox that can be selected, the order the server listed them in.
    pub fn selectable(&self) -> impl Iterator<Item = &str> {
        self.mailboxes.iter().filter(|m| m.selectable()).map(|m| m.name.as_str())
    }

    /// Every folder a sync follows: the inbox, each role, and each subscribed-or-listed
    /// selectable folder, each named once.
    pub fn synced(&self) -> Vec<String> {
        let mut out = vec![self.inbox.clone()];
        for name in self.roles.values().map(|s| s.as_str()).chain(self.selectable()) {
            if !out.iter().any(|o| o == name) {
                out.push(name.to_string());
            }
        }
        out
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE REQUEST AND THE DELTA                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// Where a device's copy of one folder stands. It is the engine's to compute and the device's to
/// keep, and it is stored only once the delta that carried it has landed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Cursor {
    pub uid_validity:   u32,            // 0 for a folder never synced
    pub highest_uid:    u32,            // every UID up to here is held, or in `backlog`
    pub highest_modseq: Option<u64>,    // CONDSTORE and QRESYNC only
    pub polls:          u32,            // counts full-reconcile polls, for the sweep
    pub backlog:        UidSet,         // seen arriving, not yet fetched; newest are fetched first
    pub floor:          u32,            // the folder's last UID when it became known: none up to here arrived
}

/// What a device holds of one folder, sent with each sync.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub mailbox:    String,
    pub cursor:     Cursor,
    pub held:       UidSet,     // every UID held under `cursor.uid_validity`
    pub window:     UidSet,     // the held UIDs a full-reconcile poll re-reads every time
}

/// The bound on one request, so a mailbox of 10^5 messages costs many small requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budget {
    pub max_count:      usize,  // new messages per delta
    pub max_bytes:      u64,    // of bodies and headers per delta; a body past it comes as its header
    pub bodies:         bool,   // fetch whole new messages, where they fit
    pub sweep_every:    u32,    // a full-reconcile poll re-reads every held flag this often
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_count:      200,
            max_bytes:      4 * 1024 * 1024,
            bodies:         false,
            sweep_every:    10,
        }
    }
}

/// The UIDVALIDITY changed: everything held under `old` is void.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reset {
    pub old:    u32,
    pub new:    u32,
}

/// How copies of one message are linked across folders, Gmail's All Mail above all.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LinkKey {
    GmMsgId(u64),       // only where the server advertises `X-GM-EXT-1`
    MessageId(String),  // the general rule, without angle brackets
}

/// A message that arrived since the cursor.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NewMessage {
    pub uid:            u32,
    pub flags:          Vec<String>,
    pub internal_date:  String,
    pub size:           u32,
    pub raw:            Vec<u8>,    // the whole message where `whole`, else its header block
    pub whole:          bool,
    pub link:           Option<LinkKey>,
    pub arrival:        bool,       // came after the folder became known, so a trigger may fire on it
}

/// A held message's flags as the server now has them. [`apply`] turns them into `+`/`-` changes
/// against the device's copy, rebased under its pending journal.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FlagState {
    pub uid:    u32,
    pub flags:  Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Delta {
    pub mailbox:    String,
    pub tier:       Option<Tier>,       // the tier this delta was read by
    pub reset:      Option<Reset>,
    pub new:        Vec<NewMessage>,    // newest first
    pub flags:      Vec<FlagState>,
    pub vanished:   UidSet,             // held UIDs the server no longer has
    pub cursor:     Cursor,             // to be stored only once this delta has landed
    pub more:       bool,               // call again with `cursor` for the rest
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE ENGINE                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// One bounded sync of one folder.
pub async fn sync_folder<S: MailSession>(
    s:      &mut S,
    caps:   &Caps,
    snap:   &Snapshot,
    budget: &Budget,
)
    -> Outcome<Delta>
{
    let cur = &snap.cursor;
    let fresh = cur.uid_validity == 0;
    let tier = caps.tier();
    let with = match (tier, cur.highest_modseq) {
        (Tier::QResync, Some(m)) if !fresh => SelectWith::QResync {
            uid_validity:   cur.uid_validity,
            modseq:         m,
            known:          if snap.held.is_empty() { None } else { Some(snap.held.clone()) },
        },
        (Tier::QResync, _) | (Tier::CondStore, _) => SelectWith::CondStore,
        (Tier::Full, _) => SelectWith::Plain,
    };
    let st = res!(s.select(&snap.mailbox, &with).await);
    if st.uid_validity == 0 {
        return Err(err!("The server gave {} no UIDVALIDITY, so nothing held from it could be \
            trusted.", snap.mailbox; Invalid, Input, Network));
    }

    let mut d = Delta {
        mailbox:    snap.mailbox.clone(),
        ..Default::default()
    };
    // A changed UIDVALIDITY voids every UID held: start again from nothing under the new one.
    let (held, base) = if fresh || st.uid_validity != cur.uid_validity {
        if !fresh {
            d.reset = Some(Reset { old: cur.uid_validity, new: st.uid_validity });
        }
        (UidSet::new(), Cursor { uid_validity: st.uid_validity, ..Default::default() })
    } else {
        (snap.held.clone(), cur.clone())
    };

    // Changed flags and vanished UIDs, by tier.
    let mut polls = base.polls;
    if !held.is_empty() {
        let (used, flags, gone) = match (&with, cur.highest_modseq, st.highest_modseq) {
            (SelectWith::QResync { .. }, _, Some(_)) => {
                (Tier::QResync, st.changed.clone(), st.vanished.intersection(&held))
            }
            (_, Some(was), Some(now)) if tier != Tier::Full => {
                if was == now {
                    (Tier::CondStore, Vec::new(), UidSet::new())
                } else {
                    let fc = res!(s.uid_fetch_changed(&held, FetchWhat::Flags, was).await);
                    let present = res!(s.uid_search(&fmt!("UID {}", held)).await);
                    let gone = held.difference(&UidSet::from(&present[..]));
                    (Tier::CondStore, fc.messages, gone)
                }
            }
            // No mod-sequences to go by, from the server or yet from this device.
            _ => {
                let sweep = budget.sweep_every <= 1 || polls % budget.sweep_every == 0;
                let set = if sweep { held.clone() } else { snap.window.intersection(&held) };
                polls = polls.wrapping_add(1);
                let flags = if set.is_empty() {
                    Vec::new()
                } else {
                    res!(s.uid_fetch(&set, FetchWhat::Flags).await)
                };
                let present = res!(s.uid_search("ALL").await);
                let gone = held.difference(&UidSet::from(&present[..]));
                (Tier::Full, flags, gone)
            }
        };
        d.tier = Some(used);
        d.flags = flags.into_iter()
            .filter(|m| held.contains(m.uid) && !gone.contains(m.uid))
            .map(|m| FlagState { uid: m.uid, flags: m.flags })
            .collect();
        d.vanished = gone;
    } else {
        d.tier = Some(match &with {
            SelectWith::QResync { .. }  => Tier::QResync,
            SelectWith::CondStore       => Tier::CondStore,
            SelectWith::Plain           => Tier::Full,
        });
        // A first read counts as a sweep: every flag it holds is fresh.
        if with == SelectWith::Plain {
            polls = polls.wrapping_add(1);
        }
    }

    // New messages: what arrived above the cursor, and what an earlier delta left in the backlog.
    let backlog = base.backlog.clone();
    let mut ask = backlog.clone();
    if st.uid_next == 0 || st.uid_next > base.highest_uid.saturating_add(1) {
        ask = ask.union(&UidSet::from_open(base.highest_uid.saturating_add(1)));
    }
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    if !ask.is_empty() {
        // `n:*` past the last UID answers with the last message (RFC 3501 6.4.8): drop it.
        for m in res!(s.uid_fetch(&ask, FetchWhat::Meta).await) {
            if m.uid > base.highest_uid || backlog.contains(m.uid) {
                sizes.push((m.uid, m.size));
            }
        }
    }
    sizes.sort();
    sizes.dedup_by_key(|p| p.0);
    let highest = sizes.last().map(|p| p.0.max(base.highest_uid)).unwrap_or(base.highest_uid);
    // Arrivals are told from the floor: the folder's last UID when this device first read it,
    // or rebuilt it under a new UIDVALIDITY. What was already there is fetched, a capped first
    // read's remainder later from the backlog, without being an arrival; what came after is one,
    // however long it waits in the backlog. UIDNEXT is read at the select, so a message delivered
    // between the select and the fetch counts as arriving.
    let floor = if fresh || d.reset.is_some() {
        if st.uid_next > 0 { st.uid_next - 1 } else { highest }
    } else {
        base.floor
    };
    // A backlog UID the server no longer returns was expunged before it was fetched.
    let mut backlog = UidSet::from(&sizes.iter().map(|p| p.0).collect::<Vec<_>>()[..]);

    // Newest first, within the count; whole bodies within the byte budget, else headers.
    let mut whole = UidSet::new();
    let mut heads: Vec<(u32, u32)> = Vec::new();
    let mut bytes = 0u64;
    for (uid, size) in sizes.iter().rev().take(budget.max_count) {
        if budget.bodies && bytes + (*size as u64) <= budget.max_bytes {
            bytes += *size as u64;
            whole.insert(*uid);
        } else {
            heads.push((*uid, *size));
        }
    }
    let mut got = Vec::new();
    if !whole.is_empty() {
        got.extend(res!(s.uid_fetch(&whole, FetchWhat::Full).await).into_iter()
            .filter(|m| whole.contains(m.uid))
            .map(|m| (m, true)));
    }
    // Headers count against the same budget. A header's length is unknown until it arrives but
    // never exceeds its message's size, so each batch is sized by that bound against what is
    // left, and what is left is then charged with what actually came. The first message of a
    // request moves however big its header, so no request is empty while any is due.
    let mut asked = whole.clone();
    let mut i = 0;
    while i < heads.len() {
        let left = budget.max_bytes.saturating_sub(bytes);
        let mut batch = UidSet::new();
        let mut bound = 0u64;
        while i < heads.len() {
            let size = heads[i].1 as u64;
            if bound + size > left && !(batch.is_empty() && asked.is_empty()) {
                break;
            }
            bound += size;
            batch.insert(heads[i].0);
            i += 1;
        }
        if batch.is_empty() {
            break;
        }
        for m in res!(s.uid_fetch(&batch, FetchWhat::Headers).await) {
            if batch.contains(m.uid) {
                bytes += m.body.len() as u64;
                got.push((m, false));
            }
        }
        asked = asked.union(&batch);
    }
    got.sort_by(|a, b| b.0.uid.cmp(&a.0.uid));
    got.dedup_by(|a, b| a.0.uid == b.0.uid);
    for (m, w) in got {
        d.new.push(NewMessage {
            uid:            m.uid,
            link:           link_key(&m, caps.gmail),
            flags:          m.flags,
            internal_date:  m.internal_date,
            size:           m.size,
            raw:            m.body,
            whole:          w,
            arrival:        m.uid > floor && !held.contains(m.uid),
        });
    }
    // What was asked for and not returned was expunged meanwhile; either way it leaves the backlog.
    backlog = backlog.difference(&asked);

    d.more = !backlog.is_empty();
    d.cursor = Cursor {
        uid_validity:   st.uid_validity,
        highest_uid:    highest,
        highest_modseq: st.highest_modseq,
        polls,
        backlog,
        floor,
    };
    Ok(d)
}

/// How this copy is linked to its others: Gmail's message ID only where the server advertises
/// `X-GM-EXT-1`, otherwise the `Message-ID` field. `None` for a message with neither.
pub fn link_key(m: &FetchedMessage, gmail: bool) -> Option<LinkKey> {
    if gmail {
        if let Some(id) = m.gm_msgid {
            return Some(LinkKey::GmMsgId(id));
        }
    }
    header_message_id(&m.body).map(LinkKey::MessageId)
}

/// The `Message-ID` a header block (or whole message) names, without its angle brackets.
pub fn header_message_id(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    message::headers(&text).into_iter()
        .find(|(n, _)| n == "message-id")
        .map(|(_, v)| bare_message_id(&v).to_string())
        .filter(|v| !v.is_empty())
}

/// A Message-ID without its angle brackets or surrounding space.
pub fn bare_message_id(mid: &str) -> &str {
    let t = mid.trim();
    t.strip_prefix('<').and_then(|t| t.strip_suffix('>')).unwrap_or(t).trim()
}
