//! Applying a [`Delta`] to a device's cache, over a [`SyncStore`] (OPFS in a browser, a directory
//! natively).
//!
//! Every message is held under its (uidvalidity, uid) [`MsgKey`], which is also its Maildir
//! name, so a delta applied twice -- after a crash before its cursor was stored -- puts nothing
//! twice and changes nothing the second time. The cursor is stored last, once every write has
//! landed.

use crate::sync::{
    act::{
        Action,
        Entry,
    },
    Delta,
    Cursor,
};

use oxedyne_fe2o3_core::prelude::*;


/// A held message's identity: unique per folder, and never reused within a generation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MsgKey {
    pub uid_validity:   u32,
    pub uid:            u32,
}

/// A device's cache of its folders. One generation per folder is current, and at most one older
/// generation is set aside while a reset rebuilds.
pub trait SyncStore {
    /// The UIDVALIDITY of the current generation, `None` for a folder never synced.
    fn generation(&self, mailbox: &str) -> Outcome<Option<u32>>;
    /// Sets the current generation aside (dropping any older one set aside before it) and starts
    /// an empty one under `uid_validity`.
    fn begin_generation(&mut self, mailbox: &str, uid_validity: u32) -> Outcome<()>;
    /// Drops the generation set aside; nothing where there is none.
    fn end_generation(&mut self, mailbox: &str) -> Outcome<()>;
    fn flags(&self, mailbox: &str, key: MsgKey) -> Outcome<Option<Vec<String>>>;
    /// Holds a message, or replaces a header-only copy with a whole one. Was anything written?
    fn put(&mut self, mailbox: &str, key: MsgKey, flags: &[String], raw: &[u8], whole: bool)
        -> Outcome<bool>;
    fn change_flags(&mut self, mailbox: &str, key: MsgKey, add: &[String], remove: &[String])
        -> Outcome<()>;
    /// Was it held?
    fn remove(&mut self, mailbox: &str, key: MsgKey) -> Outcome<bool>;
    fn cursor(&self, mailbox: &str) -> Outcome<Option<Cursor>>;
    fn set_cursor(&mut self, mailbox: &str, cursor: &Cursor) -> Outcome<()>;
}

/// What one apply changed; all zero for a replay.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Applied {
    pub added:      usize,
    pub changed:    usize,
    pub removed:    usize,
    pub reset:      bool,
}

/// Applies `d` to `store`, rebased under the device's `pending` journal: a flag that a pending
/// entry has changed keeps its local value, and a message a pending entry has moved away is
/// left alone, until that entry's outcome arrives. A delta never undoes a press just made.
pub fn apply<T: SyncStore>(store: &mut T, d: &Delta, pending: &[Entry]) -> Outcome<Applied> {
    let mb = d.mailbox.as_str();
    let uv = d.cursor.uid_validity;
    let mut out = Applied::default();
    let held = res!(store.generation(mb));
    if held != Some(uv) {
        // A reset, or the first sync. A delta for an older generation than the one held, which
        // only a stale replay could carry, must not be mixed into it.
        match (held, d.reset) {
            (None, _)                               => (),
            (Some(h), Some(r)) if h == r.old        => out.reset = true,
            (Some(h), _) => return Err(err!(
                "The delta for {} is for UIDVALIDITY {}, and the device holds {}: it is not \
                applied.", mb, uv, h; Mismatch, Input)),
        }
        res!(store.begin_generation(mb, uv));
    }

    let mine: Vec<&Entry> = pending.iter()
        .filter(|e| e.mailbox == d.mailbox && e.uid_validity == uv)
        .collect();
    let moved = |uid: u32| mine.iter().any(|e| e.uid == uid && e.action.removes());

    for m in &d.new {
        if moved(m.uid) {
            continue;
        }
        let flags = rebase_flags(&m.flags, m.uid, &mine);
        let key = MsgKey { uid_validity: uv, uid: m.uid };
        if res!(store.put(mb, key, &flags, &m.raw, m.whole)) {
            out.added += 1;
        }
    }
    for f in &d.flags {
        if moved(f.uid) {
            continue;
        }
        let key = MsgKey { uid_validity: uv, uid: f.uid };
        let local = match res!(store.flags(mb, key)) {
            Some(l) => l,
            None    => continue,
        };
        let want = rebase_flags(&f.flags, f.uid, &mine);
        let add: Vec<String> = want.iter().filter(|w| !has_flag(&local, w)).cloned().collect();
        let remove: Vec<String> = local.iter().filter(|l| !has_flag(&want, l)).cloned().collect();
        if !add.is_empty() || !remove.is_empty() {
            res!(store.change_flags(mb, key, &add, &remove));
            out.changed += 1;
        }
    }
    for uid in d.vanished.iter() {
        if res!(store.remove(mb, MsgKey { uid_validity: uv, uid })) {
            out.removed += 1;
        }
    }
    if !d.more {
        res!(store.end_generation(mb));
    }
    res!(store.set_cursor(mb, &d.cursor));
    Ok(out)
}

/// The server's `flags` for `uid` with every pending flag change for it laid over them, in
/// journal order.
pub fn rebase_flags(flags: &[String], uid: u32, pending: &[&Entry]) -> Vec<String> {
    let mut out: Vec<String> = flags.to_vec();
    for e in pending.iter().filter(|e| e.uid == uid) {
        if let Action::Flags { add, remove } = &e.action {
            out.retain(|f| !has_flag(remove, f));
            for a in add {
                if !has_flag(&out, a) {
                    out.push(a.clone());
                }
            }
        }
    }
    out
}

/// Flags compare without regard to case (RFC 3501 2.3.2).
fn has_flag(set: &[String], f: &str) -> bool {
    set.iter().any(|s| s.eq_ignore_ascii_case(f))
}


// ── Maildir names ───────────────────────────────────────────────────

/// The system flags a Maildir name carries in its `:2,` info, in the ASCII order it requires.
const INFO: [(char, &str); 5] = [
    ('D', "\\Draft"),
    ('F', "\\Flagged"),
    ('R', "\\Answered"),
    ('S', "\\Seen"),
    ('T', "\\Deleted"),
];

/// `<uidvalidity>.<uid>:2,<info>`: the key is the unique part, so a renamed flag change and a
/// replayed put land on the same file.
pub fn maildir_name(key: MsgKey, flags: &[String]) -> String {
    let info: String = INFO.iter()
        .filter(|(_, f)| has_flag(flags, f))
        .map(|(c, _)| *c)
        .collect();
    fmt!("{}.{}:2,{}", key.uid_validity, key.uid, info)
}

/// The key and system flags of a name [`maildir_name`] wrote; `None` for any other name.
pub fn parse_maildir_name(name: &str) -> Option<(MsgKey, Vec<String>)> {
    let (base, info) = name.split_once(":2,")?;
    let (uv, uid) = base.split_once('.')?;
    let key = MsgKey {
        uid_validity:   uv.parse().ok()?,
        uid:            uid.parse().ok()?,
    };
    let flags = INFO.iter()
        .filter(|(c, _)| info.contains(*c))
        .map(|(_, f)| f.to_string())
        .collect();
    Some((key, flags))
}
