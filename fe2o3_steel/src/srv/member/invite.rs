//! Single-use invites for a site's members, kept in the vhost's Ozone store.
//!
//! An invite is a link carrying a 32-byte random token; the store holds only its SHA3-256 hash, as
//! the key `<prefix>i/<hex hash>` -> `{ payload, issued, exp }`, where `payload` is whatever the
//! app needs to act on the invite (who it is for, what it grants). Taking an invite reads and
//! deletes it under one lock, so of two requests presenting the same link only one is answered
//! with its payload.
//!
//! # Audit against the Daimond gateway's single-use codes (2026-10-11)
//!
//! The gateway has two: device pairing codes (`schema.rs` `pair:` keys, `put_pairing` and
//! `take_pairing`, used by `handlers/pair.rs`) and beta passcodes (`pcode:` keys,
//! `Store::redeem_passcode`, reached through `handlers/passcode.rs` and the redeem route that
//! `handlers/account.rs` tests). `handlers/invite.rs` composes and mails an invitation and keeps
//! no code of its own. Each behaviour, kept or changed:
//!
//! - **Code form: changed.** Both store the code raw as the key, so a read of the store hands
//!   over every open code. Here the token is 32 random bytes and only its hash is stored.
//! - **Single use: changed.** `take_pairing` reads, then deletes outside any lock and ignores the
//!   delete's result, so two concurrent takes can both return the bundle, and a failed delete
//!   leaves the code redeemable. `redeem_passcode` is right: its check and its write sit under
//!   the store's write lock. Here `take` holds the state lock across the read and a delete that
//!   is awaited until durable, and fails rather than answer when the delete fails.
//! - **Expiry: kept.** Both keep an expiry in the record and refuse past it; so does this.
//! - **Consumption by deletion: kept from pairing, not from passcodes.** A passcode is kept after
//!   use, marked spent, so its label still joins telemetry to a name. That is a beta's need, not
//!   an invite's: here the app records the outcome on the person it creates, and the invite goes.
//! - **Cap on open codes: added.** Neither has one; a pairing code is minted on every request of
//!   a signed-in account. Here `max_open` refuses the next invite.
//! - **Sweep: added.** An expired pairing code that is never presented stays forever, and so does
//!   a passcode. `sweep` removes every expired invite, and `load` drops those it finds.
//! - **Revocation by subject: added.** Neither can withdraw the codes of one account. `purge`
//!   deletes every invite whose payload a predicate picks, so revoking a person takes their open
//!   add-device invites with them.
//! - **Logging: kept.** Both log that a code was used and never the code; here an invite is named
//!   by the first eight hex characters of its hash.
//!
//! A migration of `pair:` needs only the gateway's `Arc<RwLock<GwDb>>` and a `Uid`; open codes
//! live ten minutes, so they can simply lapse across the switch.

use super::{
    Clock,
    Hash32,
    Token,
    hash_of_key,
    hex,
    short,
};
use crate::{
    app::server::VhostDb,
    srv::id::Uid,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::{
    Database,
    ScanOpts,
};
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
    collections::HashMap,
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
};

#[derive(Clone, Debug)]
pub struct InviteCfg {
    pub prefix:     String, // keys are `<prefix>i/<hex hash>`
    pub ttl_secs:   u64,
    pub max_open:   usize,  // at least 1
}

impl Default for InviteCfg {
    fn default() -> Self {
        Self {
            prefix:     fmt!("member/"),
            ttl_secs:   7 * 86_400,
            max_open:   20,
        }
    }
}

impl InviteCfg {
    pub fn check(&self) -> Outcome<()> {
        if self.ttl_secs == 0 {
            return Err(err!("InviteCfg: ttl_secs must be above zero."; Invalid, Input));
        }
        if self.max_open == 0 {
            return Err(err!("InviteCfg: max_open must be at least 1."; Invalid, Input));
        }
        Ok(())
    }

    fn key_prefix(&self) -> String {
        fmt!("{}i/", self.prefix)
    }

    fn key(&self, hash: &Hash32) -> Dat {
        Dat::Str(fmt!("{}{}", self.key_prefix(), hex(hash)))
    }
}

/// An open invite as a caller sees it. `id` is the hex hash of the token: it names the invite,
/// for a list of those outstanding say, and cannot be used to take it.
#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
    pub id:         String,
    pub payload:    Dat,
    pub issued:     u64,    // Unix seconds
    pub exp:        u64,    // Unix seconds
}

#[derive(Clone)]
struct Entry {
    payload:    Dat,
    issued:     u64,
    exp:        u64,
}

impl Entry {
    fn to_dat(&self) -> Dat {
        let mut m = DaticleMap::new();
        m.insert(dat!("payload"),   self.payload.clone());
        m.insert(dat!("issued"),    Dat::U64(self.issued));
        m.insert(dat!("exp"),       Dat::U64(self.exp));
        Dat::Map(m)
    }

    fn from_dat(d: &Dat) -> Outcome<Self> {
        let m = match d {
            Dat::Map(m) => m,
            _           => return Err(err!(
                "A member invite record is not a map: {:?}.", d.kind();
                Invalid, Input, Decode)),
        };
        let num = |name: &str| -> Outcome<u64> {
            match m.get(&dat!(name)) {
                Some(Dat::U64(n))   => Ok(*n),
                other               => Err(err!(
                    "A member invite record's '{}' is not a u64: {:?}.", name, other;
                    Invalid, Input, Decode)),
            }
        };
        Ok(Self {
            payload:    m.get(&dat!("payload")).cloned().unwrap_or(Dat::Empty),
            issued:     res!(num("issued")),
            exp:        res!(num("exp")),
        })
    }

    fn expired(&self, now: u64) -> bool {
        now >= self.exp
    }

    fn invite(&self, hash: &Hash32) -> Invite {
        Invite {
            id:         hex(hash),
            payload:    self.payload.clone(),
            issued:     self.issued,
            exp:        self.exp,
        }
    }
}

/// The open invites of one vhost: an in-RAM map in front of the store.
///
/// As with `Sessions`, every write is awaited until the store confirms it durable and is made
/// under the state lock, and a record is deleted before its map entry, so an invite that could
/// not be deleted stays open rather than coming back after a restart.
pub struct Invites {
    cfg:    InviteCfg,
    clock:  Clock,
    state:  Mutex<HashMap<Hash32, Entry>>,
}

impl Invites {
    /// Rebuilds the invites from one scan of `cfg.prefix`, dropping any that have expired. For
    /// startup only: the scan walks the whole store.
    pub fn load(db: &(Arc<RwLock<VhostDb>>, Uid), cfg: InviteCfg) -> Outcome<Self> {
        Self::load_with_clock(db, cfg, Clock::System)
    }

    pub fn load_with_clock(
        db:     &(Arc<RwLock<VhostDb>>, Uid),
        cfg:    InviteCfg,
        clock:  Clock,
    )
        -> Outcome<Self>
    {
        res!(cfg.check());
        let now = res!(clock.now());
        let (db_arc, uid) = db;
        let mut state = HashMap::new();
        let mut dropped = 0usize;
        {
            let dbr = lock_read!(db_arc);
            let kp = cfg.key_prefix();
            let keys = res!(dbr.scan(&ScanOpts::with_str_prefix(kp.clone()), None));
            for (key, _, _) in keys {
                let name = match &key {
                    Dat::Str(s) => s.clone(),
                    _           => continue,
                };
                let hash = match hash_of_key(&name[kp.len()..]) {
                    Some(h) => h,
                    None    => {
                        warn!("member invite: load: '{}' is not an invite key; left alone.", name);
                        continue;
                    }
                };
                // A key the scan listed that reads as absent is a deleted record.
                let val = match res!(dbr.get(&key, None)) {
                    Some((v, _))    => v,
                    None            => continue,
                };
                let e = match Entry::from_dat(&val) {
                    Ok(e)   => e,
                    Err(e)  => {
                        warn!("member invite: load: {} is unreadable and is dropped: {}",
                            short(&hash), e);
                        res!(dbr.delete(&key, uid.clone(), None));
                        dropped += 1;
                        continue;
                    }
                };
                if e.expired(now) {
                    res!(dbr.delete(&key, uid.clone(), None));
                    dropped += 1;
                    continue;
                }
                state.insert(hash, e);
            }
        }
        info!("member invite: loaded {} open under '{}', dropped {} expired or unreadable.",
            state.len(), cfg.prefix, dropped);
        Ok(Self {
            cfg,
            clock,
            state: Mutex::new(state),
        })
    }

    pub fn cfg(&self) -> &InviteCfg { &self.cfg }

    /// Opens an invite carrying `payload`, valid for `ttl_secs`. Refused while `max_open` are
    /// open; the expired are swept first, so they never hold a place.
    pub fn issue(
        &self,
        db:         &(Arc<RwLock<VhostDb>>, Uid),
        payload:    Dat,
    )
        -> Outcome<Token>
    {
        let now = res!(self.clock.now());
        let token = Token::generate();
        let hash = res!(token.hash());
        let e = Entry {
            payload,
            issued: now,
            exp:    now.saturating_add(self.cfg.ttl_secs),
        };
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let dbr = lock_read!(db_arc);
        res!(self.sweep_locked(&mut state, &dbr, uid, now));
        if state.len() >= self.cfg.max_open {
            return Err(err!(
                "{} member invites are open, the most allowed; one must be taken, revoked or \
                expire before another is issued.", state.len();
                Excessive, Size));
        }
        res!(dbr.insert(self.cfg.key(&hash), e.to_dat(), uid.clone(), None));
        state.insert(hash, e);
        debug!("member invite: {} issued.", short(&hash));
        Ok(token)
    }

    /// The open invite the token names, without using it up: for the page a link opens before
    /// the member acts on it.
    pub fn peek(&self, token: &Token) -> Outcome<Option<Invite>> {
        let now = res!(self.clock.now());
        let hash = res!(token.hash());
        let state = lock_mutex!(self.state);
        Ok(match state.get(&hash) {
            Some(e) if !e.expired(now)  => Some(e.invite(&hash)),
            _                           => None,
        })
    }

    /// Uses up the invite the token names, giving its payload; `None` for an invite that is
    /// unknown, already taken or expired. The read and the durable delete happen under one lock,
    /// so two takes of one token cannot both succeed, and a failed delete is an error rather than
    /// a payload handed out while the invite stays open.
    pub fn take(
        &self,
        db:     &(Arc<RwLock<VhostDb>>, Uid),
        token:  &Token,
    )
        -> Outcome<Option<Dat>>
    {
        let now = res!(self.clock.now());
        let hash = res!(token.hash());
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let expired = match state.get(&hash) {
            Some(e) => e.expired(now),
            None    => return Ok(None),
        };
        let dbr = lock_read!(db_arc);
        res!(dbr.delete(&self.cfg.key(&hash), uid.clone(), None));
        let e = match state.remove(&hash) {
            Some(e) => e,
            None    => return Ok(None),
        };
        if expired {
            debug!("member invite: {} presented after it expired; removed.", short(&hash));
            return Ok(None);
        }
        info!("member invite: {} taken.", short(&hash));
        Ok(Some(e.payload))
    }

    /// Withdraws the invite with this `Invite::id`. Whether there was one.
    pub fn revoke(
        &self,
        db: &(Arc<RwLock<VhostDb>>, Uid),
        id: &str,
    )
        -> Outcome<bool>
    {
        let hash = match hash_of_key(id) {
            Some(h) => h,
            None    => return Ok(false),
        };
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        if !state.contains_key(&hash) {
            return Ok(false);
        }
        let dbr = lock_read!(db_arc);
        res!(dbr.delete(&self.cfg.key(&hash), uid.clone(), None));
        state.remove(&hash);
        debug!("member invite: {} revoked.", short(&hash));
        Ok(true)
    }

    /// Withdraws every invite, open or expired, whose payload `pred` picks: revoking a person
    /// takes their open invites with them. How many.
    pub fn purge<F>(
        &self,
        db:     &(Arc<RwLock<VhostDb>>, Uid),
        pred:   F,
    )
        -> Outcome<usize>
        where F: Fn(&Dat) -> bool
    {
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let gone: Vec<Hash32> = state.iter()
            .filter(|(_, e)| pred(&e.payload))
            .map(|(h, _)| *h)
            .collect();
        if gone.is_empty() {
            return Ok(0);
        }
        let dbr = lock_read!(db_arc);
        for hash in &gone {
            res!(dbr.delete(&self.cfg.key(hash), uid.clone(), None));
            state.remove(hash);
        }
        info!("member invite: purged {}.", gone.len());
        Ok(gone.len())
    }

    /// Removes every expired invite. How many.
    pub fn sweep(&self, db: &(Arc<RwLock<VhostDb>>, Uid)) -> Outcome<usize> {
        let now = res!(self.clock.now());
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let dbr = lock_read!(db_arc);
        self.sweep_locked(&mut state, &dbr, uid, now)
    }

    /// How many invites are open and unexpired.
    pub fn open_count(&self) -> Outcome<usize> {
        let now = res!(self.clock.now());
        let state = lock_mutex!(self.state);
        Ok(state.values().filter(|e| !e.expired(now)).count())
    }

    /// The open, unexpired invites, oldest first.
    pub fn list(&self) -> Outcome<Vec<Invite>> {
        let now = res!(self.clock.now());
        let state = lock_mutex!(self.state);
        let mut out: Vec<Invite> = state.iter()
            .filter(|(_, e)| !e.expired(now))
            .map(|(h, e)| e.invite(h))
            .collect();
        out.sort_by(|a, b| (a.issued, &a.id).cmp(&(b.issued, &b.id)));
        Ok(out)
    }

    fn sweep_locked(
        &self,
        state:  &mut HashMap<Hash32, Entry>,
        dbr:    &VhostDb,
        uid:    &Uid,
        now:    u64,
    )
        -> Outcome<usize>
    {
        let gone: Vec<Hash32> = state.iter()
            .filter(|(_, e)| e.expired(now))
            .map(|(h, _)| *h)
            .collect();
        for hash in &gone {
            res!(dbr.delete(&self.cfg.key(hash), uid.clone(), None));
            state.remove(hash);
        }
        if !gone.is_empty() {
            debug!("member invite: swept {} expired.", gone.len());
        }
        Ok(gone.len())
    }
}
