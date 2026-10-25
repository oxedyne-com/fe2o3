//! The JSON wire form of the sync engine's records, written once so the gateway that sends a
//! [`Delta`] and the page that applies it read the same shape (`jdat` feature).
//!
//! ```text
//! Cursor   {uid_validity, highest_uid, highest_modseq?, polls, backlog: "1:5"}
//! Snapshot {mailbox, cursor?, held?: "1:40,42", window?: "30:42"}
//! Budget   {max_count?, max_bytes?, bodies?}
//! Delta    {mailbox, tier: "qresync"|"condstore"|"full"|"", reset?: {old, new},
//!           new: [{uid, flags, internal_date, size, raw, whole, link_mid?, link_gm?}],
//!           flags: [{uid, flags}], vanished: "3,7", cursor, more}
//! Entry    {id, mailbox, uid_validity, uid, message_id?, size?, year?,
//!           action: "flags"|"move"|"archive"|"trash"|"junk"|"not_junk"|"expunge",
//!           add?: [..], remove?: [..], to?: "inbox"|"role:<role>"|"path:<name>"}
//! ActReply {id: "<decimal>", outcome: "done"|"gone"|"superseded"|"lost"|"stuck"|"error", detail?}
//! ```
//! A UID set travels as its IMAP wire form, so 10^5 held UIDs cost bytes. `raw` is standard
//! base64, and may be left out by a reader that keeps the bytes elsewhere (the page hands the
//! delta to its wasm apply without them). An entry's `id` is read as a number or a decimal
//! string; a reply's is written as a string, since a JavaScript number holds only 53 bits.
//!
//! Decoding is strict where a silent default would lose something: a UID set that does not
//! parse, an unknown action or target, a number out of range or a flag that is not a string is
//! an error naming the field, and a pending entry left out of a rebase would be a press undone.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::sync::{
    ActOutcome,
    Action,
    Budget,
    Cursor,
    Delta,
    Entry,
    FlagState,
    LinkKey,
    NewMessage,
    Reset,
    Snapshot,
    SpecialUse,
    Target,
    Tier,
    UidSet,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_text::base64;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FIELDS                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// The value under `key`, or `None` when it is absent or `null`.
fn field<'a>(d: &'a Dat, key: &str) -> Option<&'a Dat> {
    match d.map_get(&dat!(key)) {
        Ok(Some(Dat::Empty)) | Ok(None) | Err(_)    => None,
        Ok(Some(v))                                 => Some(v),
    }
}

fn is_map(d: &Dat) -> bool {
    matches!(d, Dat::Map(_) | Dat::OrdMap(_))
}

/// A whole number under `key` within `0..=max`; absent is 0.
fn whole(d: &Dat, key: &str, max: u64) -> Outcome<u64> {
    let v = match field(d, key) {
        Some(v) => v,
        None    => return Ok(0),
    };
    match v.get_i64() {
        Some(n) if n >= 0 && (n as u64) <= max  => Ok(n as u64),
        Some(n) => Err(err!("'{}' is {}, outside 0 to {}.", key, n, max; Invalid, Input, Range)),
        None    => Err(err!("'{}' must be a whole number, not {:?}.", key, v.kind(); Invalid, Input)),
    }
}

fn u32_of(d: &Dat, key: &str) -> Outcome<u32> {
    Ok(res!(whole(d, key, u32::MAX as u64)) as u32)
}

/// A string under `key`; absent is empty.
fn text(d: &Dat, key: &str) -> Outcome<String> {
    match field(d, key) {
        None                => Ok(String::new()),
        Some(Dat::Str(s))   => Ok(s.clone()),
        Some(v)             => Err(err!("'{}' must be a string, not {:?}.", key, v.kind(); Invalid, Input)),
    }
}

fn need_text(d: &Dat, key: &str) -> Outcome<String> {
    let s = res!(text(d, key));
    if s.is_empty() {
        return Err(err!("'{}' is missing.", key; Missing, Input));
    }
    Ok(s)
}

fn boolean(d: &Dat, key: &str) -> Outcome<bool> {
    match field(d, key) {
        None                => Ok(false),
        Some(Dat::Bool(b))  => Ok(*b),
        Some(v)             => Err(err!("'{}' must be true or false, not {:?}.", key, v.kind(); Invalid, Input)),
    }
}

/// A list of strings under `key`; absent is empty.
fn strings(d: &Dat, key: &str) -> Outcome<Vec<String>> {
    match field(d, key) {
        None    => Ok(Vec::new()),
        Some(v) => v.get_string_list().ok_or_else(||
            err!("'{}' must be a list of strings.", key; Invalid, Input)),
    }
}

fn list<'a>(d: &'a Dat, key: &str) -> Outcome<&'a [Dat]> {
    match field(d, key) {
        None                => Ok(&[]),
        Some(Dat::List(l))  => Ok(l),
        Some(v)             => Err(err!("'{}' must be a list, not {:?}.", key, v.kind(); Invalid, Input)),
    }
}

/// A UID set in its IMAP wire form under `key`; absent or empty is the empty set.
fn uid_set(d: &Dat, key: &str) -> Outcome<UidSet> {
    let s = res!(text(d, key));
    if s.is_empty() {
        return Ok(UidSet::new());
    }
    match UidSet::parse(&s) {
        Ok(u)   => Ok(u),
        Err(e)  => Err(err!(e, "'{}' is not a UID set: '{}'.", key, s; Invalid, Input)),
    }
}

fn set_dat(s: &UidSet) -> Dat {
    dat!(fmt!("{}", s))
}

fn strings_dat(f: &[String]) -> Dat {
    Dat::List(f.iter().map(|x| dat!(x.clone())).collect())
}

fn num_dat(n: u64) -> Dat {
    Dat::I64(n as i64)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CURSOR, SNAPSHOT, BUDGET                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// `highest_modseq` of 0 is read as none, which is what a server's 0 means.
pub fn cursor_from(c: &Dat) -> Outcome<Cursor> {
    if !is_map(c) {
        return Err(err!("A cursor must be an object, not {:?}.", c.kind(); Invalid, Input));
    }
    let modseq = res!(whole(c, "highest_modseq", i64::MAX as u64));
    Ok(Cursor {
        uid_validity:   res!(u32_of(c, "uid_validity")),
        highest_uid:    res!(u32_of(c, "highest_uid")),
        highest_modseq: if modseq > 0 { Some(modseq) } else { None },
        polls:          res!(u32_of(c, "polls")),
        backlog:        res!(uid_set(c, "backlog")),
    })
}

pub fn cursor_dat(c: &Cursor) -> Dat {
    let mut p = vec![
        (dat!("uid_validity"),  num_dat(c.uid_validity as u64)),
        (dat!("highest_uid"),   num_dat(c.highest_uid as u64)),
        (dat!("polls"),         num_dat(c.polls as u64)),
        (dat!("backlog"),       set_dat(&c.backlog)),
    ];
    if let Some(m) = c.highest_modseq {
        p.push((dat!("highest_modseq"), num_dat(m)));
    }
    create_dat_ordmap(p)
}

/// A folder as a device holds it; no `cursor` is a folder never synced.
pub fn snapshot_from(f: &Dat) -> Outcome<Snapshot> {
    let mailbox = res!(need_text(f, "mailbox"));
    let cursor = match field(f, "cursor") {
        Some(c) => match cursor_from(c) {
            Ok(c)   => c,
            Err(e)  => return Err(err!(e, "The cursor of '{}' cannot be read.", mailbox; Invalid, Input)),
        },
        None    => Cursor::default(),
    };
    let sets = (uid_set(f, "held"), uid_set(f, "window"));
    match sets {
        (Ok(held), Ok(window))  => Ok(Snapshot { mailbox, cursor, held, window }),
        (Err(e), _) | (_, Err(e)) => Err(err!(e, "The folder '{}' cannot be read.", mailbox; Invalid, Input)),
    }
}

pub fn snapshot_dat(s: &Snapshot) -> Dat {
    create_dat_ordmap(vec![
        (dat!("mailbox"),   dat!(s.mailbox.clone())),
        (dat!("cursor"),    cursor_dat(&s.cursor)),
        (dat!("held"),      set_dat(&s.held)),
        (dat!("window"),    set_dat(&s.window)),
    ])
}

/// The budget a request asks for, within `limit`: its count and bytes may only lower the limit's,
/// and `bodies` is the request's alone. `None` is the limit without bodies.
pub fn budget_within(b: Option<&Dat>, limit: &Budget) -> Outcome<Budget> {
    let b = match b {
        Some(b) if is_map(b)    => b,
        Some(Dat::Empty) | None => return Ok(Budget { bodies: false, ..*limit }),
        Some(v) => return Err(err!("'budget' must be an object, not {:?}.", v.kind(); Invalid, Input)),
    };
    let count = res!(whole(b, "max_count", u32::MAX as u64)) as usize;
    let bytes = res!(whole(b, "max_bytes", i64::MAX as u64));
    Ok(Budget {
        max_count:  if count > 0 { count.min(limit.max_count) } else { limit.max_count },
        max_bytes:  if bytes > 0 { bytes.min(limit.max_bytes) } else { limit.max_bytes },
        bodies:     res!(boolean(b, "bodies")),
        ..*limit
    })
}

pub fn budget_dat(b: &Budget) -> Dat {
    create_dat_ordmap(vec![
        (dat!("max_count"), num_dat(b.max_count as u64)),
        (dat!("max_bytes"), num_dat(b.max_bytes)),
        (dat!("bodies"),    Dat::Bool(b.bodies)),
    ])
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DELTA                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

fn tier_word(t: Option<Tier>) -> &'static str {
    match t {
        Some(Tier::QResync)     => "qresync",
        Some(Tier::CondStore)   => "condstore",
        Some(Tier::Full)        => "full",
        None                    => "",
    }
}

fn tier_of(w: &str) -> Outcome<Option<Tier>> {
    Ok(match w {
        "qresync"   => Some(Tier::QResync),
        "condstore" => Some(Tier::CondStore),
        "full"      => Some(Tier::Full),
        ""          => None,
        other       => return Err(err!("Unknown tier '{}'.", other; Invalid, Input)),
    })
}

fn new_from(m: &Dat) -> Outcome<NewMessage> {
    let uid = res!(u32_of(m, "uid"));
    if uid == 0 {
        return Err(err!("A new message needs its 'uid'."; Missing, Input));
    }
    let raw = res!(text(m, "raw"));
    let raw = match base64::decode(&raw) {
        Ok(r)   => r,
        Err(e)  => return Err(err!(e, "The 'raw' of UID {} is not base64.", uid; Invalid, Input)),
    };
    let gm = res!(text(m, "link_gm"));
    let mid = res!(text(m, "link_mid"));
    let link = if !gm.is_empty() {
        match gm.parse::<u64>() {
            Ok(n)   => Some(LinkKey::GmMsgId(n)),
            Err(_)  => return Err(err!("The 'link_gm' of UID {} is not a number: '{}'.",
                            uid, gm; Invalid, Input)),
        }
    } else if !mid.is_empty() {
        Some(LinkKey::MessageId(mid))
    } else {
        None
    };
    Ok(NewMessage {
        uid,
        flags:          res!(strings(m, "flags")),
        internal_date:  res!(text(m, "internal_date")),
        size:           res!(u32_of(m, "size")),
        raw,
        whole:          res!(boolean(m, "whole")),
        link,
    })
}

fn new_dat(m: &NewMessage) -> Dat {
    let mut p = vec![
        (dat!("uid"),           num_dat(m.uid as u64)),
        (dat!("flags"),         strings_dat(&m.flags)),
        (dat!("internal_date"), dat!(m.internal_date.clone())),
        (dat!("size"),          num_dat(m.size as u64)),
        (dat!("raw"),           dat!(base64::encode(&m.raw))),
        (dat!("whole"),         Dat::Bool(m.whole)),
    ];
    match &m.link {
        Some(LinkKey::MessageId(s)) => p.push((dat!("link_mid"), dat!(s.clone()))),
        Some(LinkKey::GmMsgId(n))   => p.push((dat!("link_gm"), dat!(fmt!("{}", n)))),
        None                        => (),
    }
    create_dat_ordmap(p)
}

fn delta_body(d: &Dat, mailbox: String) -> Outcome<Delta> {
    let cursor = match field(d, "cursor") {
        Some(c) => res!(cursor_from(c)),
        None    => return Err(err!("The delta has no cursor."; Missing, Input)),
    };
    if cursor.uid_validity == 0 {
        return Err(err!("The delta's cursor has no UIDVALIDITY."; Missing, Input));
    }
    let reset = match field(d, "reset") {
        Some(r) if is_map(r) => Some(Reset {
            old: res!(u32_of(r, "old")),
            new: res!(u32_of(r, "new")),
        }),
        Some(v) => return Err(err!("'reset' must be an object, not {:?}.", v.kind(); Invalid, Input)),
        None    => None,
    };
    let mut new = Vec::new();
    for m in res!(list(d, "new")) {
        new.push(res!(new_from(m)));
    }
    let mut flags = Vec::new();
    for f in res!(list(d, "flags")) {
        flags.push(FlagState {
            uid:    res!(u32_of(f, "uid")),
            flags:  res!(strings(f, "flags")),
        });
    }
    Ok(Delta {
        mailbox,
        tier:       res!(tier_of(&res!(text(d, "tier")))),
        reset,
        new,
        flags,
        vanished:   res!(uid_set(d, "vanished")),
        cursor,
        more:       res!(boolean(d, "more")),
    })
}

/// One folder's delta. A delta that carries an `error` in place of its changes (the gateway's
/// answer for a folder the server refused) is an error here, with that message.
pub fn delta_from(d: &Dat) -> Outcome<Delta> {
    let mailbox = res!(need_text(d, "mailbox"));
    let refused = res!(text(d, "error"));
    if !refused.is_empty() {
        return Err(err!("The folder '{}' was not synced: {}", mailbox, refused; Input));
    }
    match delta_body(d, mailbox.clone()) {
        Ok(x)   => Ok(x),
        Err(e)  => Err(err!(e, "The delta for '{}' cannot be read.", mailbox; Invalid, Input)),
    }
}

pub fn delta_dat(d: &Delta) -> Dat {
    let mut p = vec![
        (dat!("mailbox"),   dat!(d.mailbox.clone())),
        (dat!("tier"),      dat!(tier_word(d.tier))),
    ];
    if let Some(r) = d.reset {
        p.push((dat!("reset"), create_dat_ordmap(vec![
            (dat!("old"), num_dat(r.old as u64)),
            (dat!("new"), num_dat(r.new as u64)),
        ])));
    }
    let flags = d.flags.iter().map(|f| create_dat_ordmap(vec![
        (dat!("uid"),   num_dat(f.uid as u64)),
        (dat!("flags"), strings_dat(&f.flags)),
    ])).collect();
    p.extend(vec![
        (dat!("new"),       Dat::List(d.new.iter().map(new_dat).collect())),
        (dat!("flags"),     Dat::List(flags)),
        (dat!("vanished"),  set_dat(&d.vanished)),
        (dat!("cursor"),    cursor_dat(&d.cursor)),
        (dat!("more"),      Dat::Bool(d.more)),
    ]);
    create_dat_ordmap(p)
}

/// The place of a folder that could not be synced, in a response's `deltas`.
pub fn refused_dat(mailbox: &str, why: &str) -> Dat {
    create_dat_ordmap(vec![
        (dat!("mailbox"),   dat!(mailbox.to_string())),
        (dat!("error"),     dat!(why.to_string())),
    ])
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ENTRY AND REPLY                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

fn role_word(r: SpecialUse) -> &'static str {
    match r {
        SpecialUse::Sent        => "sent",
        SpecialUse::Drafts      => "drafts",
        SpecialUse::Trash       => "trash",
        SpecialUse::Junk        => "junk",
        SpecialUse::Archive     => "archive",
        SpecialUse::All         => "all",
        SpecialUse::Flagged     => "flagged",
    }
}

/// `inbox`, `role:<role>` or `path:<name>`; a role an action cannot target is none.
pub fn target_of(to: &str) -> Option<Target> {
    if to.eq_ignore_ascii_case("inbox") {
        return Some(Target::Inbox);
    }
    if let Some(p) = to.strip_prefix("path:") {
        return if p.is_empty() { None } else { Some(Target::Path(p.to_string())) };
    }
    let role = match to.strip_prefix("role:") {
        Some(r) => r,
        None    => return None,
    };
    let r = match role {
        "sent"      => SpecialUse::Sent,
        "drafts"    => SpecialUse::Drafts,
        "trash"     => SpecialUse::Trash,
        "junk"      => SpecialUse::Junk,
        "archive"   => SpecialUse::Archive,
        "all"       => SpecialUse::All,
        "flagged"   => SpecialUse::Flagged,
        _           => return None,
    };
    Some(Target::Role(r))
}

pub fn target_word(t: &Target) -> String {
    match t {
        Target::Inbox   => fmt!("inbox"),
        Target::Path(p) => fmt!("path:{}", p),
        Target::Role(r) => fmt!("role:{}", role_word(*r)),
    }
}

/// An entry's `id`, as a number or a decimal string.
fn entry_id(e: &Dat) -> Outcome<u64> {
    match field(e, "id") {
        Some(Dat::Str(s)) => s.parse::<u64>().map_err(|_|
            err!("The entry id '{}' is not a number.", s; Invalid, Input)),
        Some(v) => match v.get_i64() {
            Some(n) if n >= 0   => Ok(n as u64),
            _                   => Err(err!("The entry id must be a whole number."; Invalid, Input)),
        },
        None    => Err(err!("An entry needs its 'id'."; Missing, Input)),
    }
}

fn entry_body(e: &Dat, id: u64) -> Outcome<Entry> {
    let mailbox = res!(need_text(e, "mailbox"));
    let uid = res!(u32_of(e, "uid"));
    let uid_validity = res!(u32_of(e, "uid_validity"));
    if uid == 0 || uid_validity == 0 {
        return Err(err!("It needs its 'uid' and 'uid_validity'."; Missing, Input));
    }
    let action = match res!(text(e, "action")).as_str() {
        "flags"     => {
            let (add, remove) = (res!(strings(e, "add")), res!(strings(e, "remove")));
            if add.is_empty() && remove.is_empty() {
                return Err(err!("It changes no flag."; Missing, Input));
            }
            Action::Flags { add, remove }
        }
        "move"      => {
            let to = res!(text(e, "to"));
            Action::Move { to: res!(target_of(&to).ok_or_else(|| err!(
                "It moves to '{}': 'to' must be inbox, role:<role> or path:<name>.", to; Invalid, Input))) }
        }
        "archive"   => Action::Archive,
        "trash"     => Action::Trash,
        "junk"      => Action::Junk,
        "not_junk"  => Action::NotJunk,
        "expunge"   => Action::Expunge,
        other       => return Err(err!("Its action '{}' is unknown.", other; Invalid, Input)),
    };
    let year = res!(whole(e, "year", u16::MAX as u64)) as u16;
    Ok(Entry {
        id,
        mailbox,
        uid_validity,
        uid,
        message_id: res!(text(e, "message_id")),
        size:       res!(u32_of(e, "size")),
        year:       if year > 0 { Some(year) } else { None },
        action,
    })
}

/// One journal entry. It is all or nothing: an entry that cannot be read is never run or rebased
/// in part.
pub fn entry_from(e: &Dat) -> Outcome<Entry> {
    let id = res!(entry_id(e));
    match entry_body(e, id) {
        Ok(x)   => Ok(x),
        Err(x)  => Err(err!(x, "Entry {} cannot be read.", id; Invalid, Input)),
    }
}

pub fn entry_dat(e: &Entry) -> Dat {
    let mut p = vec![
        (dat!("id"),            num_dat(e.id)),
        (dat!("mailbox"),       dat!(e.mailbox.clone())),
        (dat!("uid_validity"),  num_dat(e.uid_validity as u64)),
        (dat!("uid"),           num_dat(e.uid as u64)),
    ];
    if !e.message_id.is_empty() {
        p.push((dat!("message_id"), dat!(e.message_id.clone())));
    }
    p.push((dat!("size"), num_dat(e.size as u64)));
    if let Some(y) = e.year {
        p.push((dat!("year"), num_dat(y as u64)));
    }
    let word = match &e.action {
        Action::Flags { add, remove } => {
            p.push((dat!("add"),    strings_dat(add)));
            p.push((dat!("remove"), strings_dat(remove)));
            "flags"
        }
        Action::Move { to } => {
            p.push((dat!("to"), dat!(target_word(to))));
            "move"
        }
        Action::Archive => "archive",
        Action::Trash   => "trash",
        Action::Junk    => "junk",
        Action::NotJunk => "not_junk",
        Action::Expunge => "expunge",
    };
    p.push((dat!("action"), dat!(word)));
    create_dat_ordmap(p)
}

/// What became of one entry: a definite outcome, or `Err` with why it could not be run, so it is
/// kept and sent again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActReply {
    pub id:         u64,
    pub outcome:    std::result::Result<ActOutcome, String>,
}

pub fn reply_dat(r: &ActReply) -> Dat {
    let (word, detail) = match &r.outcome {
        Ok(ActOutcome::Done)            => ("done",       None),
        Ok(ActOutcome::Gone)            => ("gone",       None),
        Ok(ActOutcome::Superseded(at))  => ("superseded", Some(at)),
        Ok(ActOutcome::Lost)            => ("lost",       None),
        Ok(ActOutcome::Stuck(why))      => ("stuck",      Some(why)),
        Err(why)                        => ("error",      Some(why)),
    };
    let mut p = vec![
        (dat!("id"),        dat!(fmt!("{}", r.id))),
        (dat!("outcome"),   dat!(word)),
    ];
    if let Some(d) = detail {
        p.push((dat!("detail"), dat!(d.clone())));
    }
    create_dat_ordmap(p)
}

pub fn reply_from(d: &Dat) -> Outcome<ActReply> {
    let id = res!(entry_id(d));
    let detail = res!(text(d, "detail"));
    let outcome = match res!(text(d, "outcome")).as_str() {
        "done"          => Ok(ActOutcome::Done),
        "gone"          => Ok(ActOutcome::Gone),
        "superseded"    => Ok(ActOutcome::Superseded(detail)),
        "lost"          => Ok(ActOutcome::Lost),
        "stuck"         => Ok(ActOutcome::Stuck(detail)),
        "error"         => Err(detail),
        other           => return Err(err!("The outcome '{}' of entry {} is unknown.",
                            other, id; Invalid, Input)),
    };
    Ok(ActReply { id, outcome })
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE JDAT TRAITS                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

macro_rules! wire_traits {
    ($t:ty, $from:ident, $to:expr) => {
        impl ToDat for $t {
            fn to_dat(&self) -> Outcome<Dat> {
                Ok($to(self))
            }
        }
        impl FromDat for $t {
            fn from_dat(dat: Dat) -> Outcome<Self> {
                $from(&dat)
            }
        }
    };
}

wire_traits!(Cursor,    cursor_from,    cursor_dat);
wire_traits!(Snapshot,  snapshot_from,  snapshot_dat);
wire_traits!(Delta,     delta_from,     delta_dat);
wire_traits!(Entry,     entry_from,     entry_dat);
wire_traits!(ActReply,  reply_from,     reply_dat);
