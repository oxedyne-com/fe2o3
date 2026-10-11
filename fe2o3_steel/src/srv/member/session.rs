//! Server-side sessions for a site's signed-in members, kept in the vhost's Ozone store.
//!
//! A session survives a restart, is ended at once by a sign-out or a revocation, and cannot be
//! recovered from a read of the store: the browser holds a 32-byte random token, and the store holds
//! only its SHA3-256 hash, as the key `<prefix>s/<hex hash>` -> `{ principal, data, created,
//! last_seen }`. One process owns the store, so the RAM map in front of it is the authority while
//! the process runs, and `Sessions::load` rebuilds it from one prefix scan at startup.
//!
//! # Audit against the Daimond gateway's sessions (2026-10-11)
//!
//! The gateway (`daimond/gateway/src/schema.rs` `sess:` keys, `handlers/{auth_verify,common}.rs`)
//! and Steel's admin cookie (`srv/admin/session.rs`) were read first. Each behaviour, kept or
//! changed:
//!
//! - **Token form: changed.** The gateway mints a 32-character random string and stores it raw
//!   as the key `sess:<token>`, so anyone who reads the store or a dump of it holds every live
//!   session. Here the token is 32 random bytes and only its hash is stored.
//! - **What is stored: changed.** The gateway keeps `{ account_id, issued_ts, exp }`. Here the
//!   record holds the principal as bytes, an app's own `data`, `created` and `last_seen`; expiry
//!   is computed from the config rather than frozen into the record, so a changed window applies
//!   to every session at once.
//! - **Expiry: kept.** An idle window plus an absolute ceiling, the ceiling enforced on read, not
//!   only by the writer, is the gateway's design and is right.
//! - **Slide throttle: kept.** The gateway writes a slide only when it moves `exp` by more than
//!   `SESSION_SLIDE_MIN_SECS`; here `last_seen` is written at most once per `touch_secs`. The RAM
//!   copy is exact, so a restart costs a session at most `touch_secs` of idle time.
//! - **No resurrection: kept.** The gateway's slide re-reads under a lock so that a sign-out
//!   between read and write stays a sign-out. Here every write happens under the state lock and
//!   only for a session the RAM map still holds, so a touch cannot write back an ended session.
//! - **Per-principal cap: added.** The gateway has none, so a principal's sessions grow without
//!   bound until each one expires. Here the oldest is dropped past `per_principal`.
//! - **Revocation: added.** The gateway ends one session by its token; it cannot end every
//!   session of one account. `end_all` and `end_id` do.
//! - **Sweep: added.** The gateway deletes an expired record only when its own token is next
//!   presented, so an abandoned session's record stays forever. `sweep` removes every expired one,
//!   and `load` drops those it finds.
//! - **Logging: kept.** The gateway logs the account on sign-in and never the token; here the log
//!   names a session by the first eight hex characters of its hash and never the token.
//! - **The admin cookie: not reused.** Stateless and encrypted under a key that rotates on restart,
//!   it cannot survive a restart or be revoked singly, which members need.
//!
//! The gateway's store is `O3db` with the same type parameters as `VhostDb`, so a migration needs
//! only its `Store` to hand over its `Arc<RwLock<GwDb>>` and a `Uid`, and the live `sess:` records
//! to be dropped (forcing one sign-in), since a raw token cannot become a hashed one without being
//! presented. Steel's own WebSocket member login (`sess_meta:<sid>`) also stores its id raw and is
//! a candidate for the same move.

pub use super::{
    Clock,
    Token,
    TOKEN_LEN,
};

use super::{
    Hash32,
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
    collections::{
        BTreeSet,
        HashMap,
    },
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
};

#[derive(Clone, Debug)]
pub struct SessionCfg {
    pub prefix:         String, // keys are `<prefix>s/<hex hash>`
    pub idle_secs:      u64,
    pub absolute_secs:  u64,
    pub per_principal:  usize,  // at least 1
    pub touch_secs:     u64,    // least gap between two `last_seen` writes
}

impl Default for SessionCfg {
    fn default() -> Self {
        Self {
            prefix:         fmt!("member/"),
            idle_secs:      14 * 86_400,
            absolute_secs:  90 * 86_400,
            per_principal:  10,
            touch_secs:     600,
        }
    }
}

impl SessionCfg {
    pub fn check(&self) -> Outcome<()> {
        if self.idle_secs == 0 || self.absolute_secs == 0 {
            return Err(err!(
                "SessionCfg: idle_secs ({}) and absolute_secs ({}) must both be above zero.",
                self.idle_secs, self.absolute_secs;
                Invalid, Input));
        }
        if self.per_principal == 0 {
            return Err(err!("SessionCfg: per_principal must be at least 1."; Invalid, Input));
        }
        Ok(())
    }

    fn key_prefix(&self) -> String {
        fmt!("{}s/", self.prefix)
    }

    fn key(&self, hash: &Hash32) -> Dat {
        Dat::Str(fmt!("{}{}", self.key_prefix(), hex(hash)))
    }
}

/// A live session as a caller sees it. `id` is the hex hash of the token: it names the session,
/// for a list of a member's devices say, and cannot be used to present it.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id:         String,
    pub principal:  Vec<u8>,
    pub data:       Dat,
    pub created:    u64,    // Unix seconds
    pub last_seen:  u64,    // Unix seconds
}

#[derive(Clone)]
struct Entry {
    principal:      Vec<u8>,
    data:           Dat,
    created:        u64,
    last_seen:      u64,
    stored_seen:    u64, // `last_seen` as the store holds it
}

impl Entry {
    fn to_dat(&self, last_seen: u64) -> Dat {
        let mut m = DaticleMap::new();
        m.insert(dat!("principal"),     Dat::BU64(self.principal.clone()));
        m.insert(dat!("data"),          self.data.clone());
        m.insert(dat!("created"),       Dat::U64(self.created));
        m.insert(dat!("last_seen"),     Dat::U64(last_seen));
        Dat::Map(m)
    }

    fn from_dat(d: &Dat) -> Outcome<Self> {
        let m = match d {
            Dat::Map(m) => m,
            _           => return Err(err!(
                "A member session record is not a map: {:?}.", d.kind();
                Invalid, Input, Decode)),
        };
        let principal = match m.get(&dat!("principal")).and_then(|p| p.bytes_ref()) {
            Some(b) => b.clone(),
            None    => return Err(err!(
                "A member session record has no principal bytes."; Missing, Input, Decode)),
        };
        let num = |name: &str| -> Outcome<u64> {
            match m.get(&dat!(name)) {
                Some(Dat::U64(n))   => Ok(*n),
                other               => Err(err!(
                    "A member session record's '{}' is not a u64: {:?}.", name, other;
                    Invalid, Input, Decode)),
            }
        };
        let created = res!(num("created"));
        let last_seen = res!(num("last_seen"));
        Ok(Self {
            principal,
            data:           m.get(&dat!("data")).cloned().unwrap_or(Dat::Empty),
            created,
            last_seen,
            stored_seen:    last_seen,
        })
    }

    fn session(&self, hash: &Hash32) -> Session {
        Session {
            id:         hex(hash),
            principal:  self.principal.clone(),
            data:       self.data.clone(),
            created:    self.created,
            last_seen:  self.last_seen,
        }
    }
}

#[derive(Default)]
struct State {
    by_hash:        HashMap<Hash32, Entry>,
    by_principal:   HashMap<Vec<u8>, BTreeSet<(u64, Hash32)>>, // oldest first
}

impl State {
    fn add(&mut self, hash: Hash32, e: Entry) {
        self.by_principal.entry(e.principal.clone()).or_default().insert((e.created, hash));
        self.by_hash.insert(hash, e);
    }

    fn remove(&mut self, hash: &Hash32) -> Option<Entry> {
        let e = match self.by_hash.remove(hash) {
            Some(e) => e,
            None    => return None,
        };
        let empty = match self.by_principal.get_mut(&e.principal) {
            Some(set) => {
                set.remove(&(e.created, *hash));
                set.is_empty()
            }
            None => false,
        };
        if empty {
            self.by_principal.remove(&e.principal);
        }
        Some(e)
    }
}

/// The sessions of one vhost's members: an in-RAM map in front of the store.
///
/// Every write is awaited until the store confirms it durable (`Database::insert` and `delete`
/// wait on `Responder::recv_store_ack` and `recv_delete_ack`), and is made under the state lock,
/// so the store and the map change together and two requests cannot interleave a change.
pub struct Sessions {
    cfg:    SessionCfg,
    clock:  Clock,
    state:  Mutex<State>,
}

impl Sessions {
    /// Rebuilds the sessions from one scan of `cfg.prefix`, dropping any that have expired. For
    /// startup only: the scan walks the whole store.
    pub fn load(db: &(Arc<RwLock<VhostDb>>, Uid), cfg: SessionCfg) -> Outcome<Self> {
        Self::load_with_clock(db, cfg, Clock::System)
    }

    pub fn load_with_clock(
        db:     &(Arc<RwLock<VhostDb>>, Uid),
        cfg:    SessionCfg,
        clock:  Clock,
    )
        -> Outcome<Self>
    {
        res!(cfg.check());
        let now = res!(clock.now());
        let (db_arc, uid) = db;
        let mut state = State::default();
        let mut dropped = 0usize;
        {
            let dbr = lock_read!(db_arc);
            let keys = res!(dbr.scan(&ScanOpts::with_str_prefix(cfg.key_prefix()), None));
            for (key, _, _) in keys {
                let name = match &key {
                    Dat::Str(s) => s.clone(),
                    _           => continue,
                };
                let hash = match hash_of_key(&name[cfg.key_prefix().len()..]) {
                    Some(h) => h,
                    None    => {
                        warn!("member session: load: '{}' is not a session key; left alone.", name);
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
                        warn!("member session: load: {} is unreadable and is dropped: {}",
                            short(&hash), e);
                        res!(dbr.delete(&key, uid.clone(), None));
                        dropped += 1;
                        continue;
                    }
                };
                if expired(&cfg, &e, now) {
                    res!(dbr.delete(&key, uid.clone(), None));
                    dropped += 1;
                    continue;
                }
                state.add(hash, e);
            }
        }
        info!("member session: loaded {} live under '{}', dropped {} expired or unreadable.",
            state.by_hash.len(), cfg.prefix, dropped);
        Ok(Self {
            cfg,
            clock,
            state: Mutex::new(state),
        })
    }

    pub fn cfg(&self) -> &SessionCfg { &self.cfg }

    /// Opens a session for `principal`, first ending its oldest where it already holds
    /// `per_principal`.
    pub fn start(
        &self,
        db:         &(Arc<RwLock<VhostDb>>, Uid),
        principal:  &[u8],
        data:       Dat,
    )
        -> Outcome<Token>
    {
        let now = res!(self.clock.now());
        let token = Token::generate();
        let hash = res!(token.hash());
        let e = Entry {
            principal:      principal.to_vec(),
            data,
            created:        now,
            last_seen:      now,
            stored_seen:    now,
        };
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let dbr = lock_read!(db_arc);
        // The cap is made room for before the write, so a failed delete leaves no new session.
        loop {
            let oldest = match state.by_principal.get(principal) {
                Some(set) if set.len() >= self.cfg.per_principal => match set.iter().next() {
                    Some((_, h))    => *h,
                    None            => break,
                },
                _ => break,
            };
            res!(dbr.delete(&self.cfg.key(&oldest), uid.clone(), None));
            state.remove(&oldest);
            debug!("member session: {} ended, its principal's oldest past the cap of {}.",
                short(&oldest), self.cfg.per_principal);
        }
        res!(dbr.insert(self.cfg.key(&hash), e.to_dat(now), uid.clone(), None));
        state.add(hash, e);
        debug!("member session: {} started.", short(&hash));
        Ok(token)
    }

    /// The live session the token names, if any. An expired one is ended here; a live one has
    /// `last_seen` moved to now, written through at most once per `touch_secs`.
    pub fn check(
        &self,
        db:     &(Arc<RwLock<VhostDb>>, Uid),
        token:  &Token,
    )
        -> Outcome<Option<Session>>
    {
        let now = res!(self.clock.now());
        let hash = res!(token.hash());
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let e = match state.by_hash.get(&hash) {
            Some(e) => e.clone(),
            None    => return Ok(None),
        };
        if expired(&self.cfg, &e, now) {
            state.remove(&hash);
            // Best effort: the map no longer holds it, so it is ended for this process, and a
            // record left behind is dropped by the next sweep or load.
            let dbr = lock_read!(db_arc);
            if let Err(err) = dbr.delete(&self.cfg.key(&hash), uid.clone(), None) {
                warn!("member session: {} expired but its record could not be deleted: {}",
                    short(&hash), err);
            }
            return Ok(None);
        }
        let mut stored_seen = e.stored_seen;
        if now >= e.stored_seen.saturating_add(self.cfg.touch_secs) {
            // Best effort too: a touch that could not be written costs this session up to
            // `touch_secs` of idle time after a restart, not the request in hand.
            let dbr = lock_read!(db_arc);
            match dbr.insert(self.cfg.key(&hash), e.to_dat(now), uid.clone(), None) {
                Ok(_)       => stored_seen = now,
                Err(err)    => warn!("member session: {} could not record its use: {}",
                    short(&hash), err),
            }
        }
        let live = match state.by_hash.get_mut(&hash) {
            Some(live) => {
                live.last_seen = now;
                live.stored_seen = stored_seen;
                live.clone()
            }
            None => return Ok(None),
        };
        Ok(Some(live.session(&hash)))
    }

    /// Replaces the app data of the live session the token names, written through before the map
    /// changes, as a re-challenge records its time. Whether there was a live session. An expired
    /// one is left for `check` or the sweep to end; it is not revived.
    pub fn set_data(
        &self,
        db:     &(Arc<RwLock<VhostDb>>, Uid),
        token:  &Token,
        data:   Dat,
    )
        -> Outcome<bool>
    {
        let now = res!(self.clock.now());
        let hash = res!(token.hash());
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let mut e = match state.by_hash.get(&hash) {
            Some(e) if !expired(&self.cfg, e, now)  => e.clone(),
            _                                       => return Ok(false),
        };
        e.data = data;
        {
            let dbr = lock_read!(db_arc);
            res!(dbr.insert(self.cfg.key(&hash), e.to_dat(e.last_seen), uid.clone(), None));
        }
        e.stored_seen = e.last_seen;
        if let Some(live) = state.by_hash.get_mut(&hash) {
            *live = e;
        }
        debug!("member session: {} data replaced.", short(&hash));
        Ok(true)
    }

    /// Ends the session the token names. Whether there was one.
    pub fn end(
        &self,
        db:     &(Arc<RwLock<VhostDb>>, Uid),
        token:  &Token,
    )
        -> Outcome<bool>
    {
        let hash = res!(token.hash());
        self.end_hash(db, &hash)
    }

    /// Ends the session with this `Session::id`, for a member who revokes one device from
    /// another. Whether there was one.
    pub fn end_id(
        &self,
        db: &(Arc<RwLock<VhostDb>>, Uid),
        id: &str,
    )
        -> Outcome<bool>
    {
        match hash_of_key(id) {
            Some(hash)  => self.end_hash(db, &hash),
            None        => Ok(false),
        }
    }

    /// Ends every session of `principal`, and no other. How many there were.
    pub fn end_all(
        &self,
        db:         &(Arc<RwLock<VhostDb>>, Uid),
        principal:  &[u8],
    )
        -> Outcome<usize>
    {
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let hashes: Vec<Hash32> = match state.by_principal.get(principal) {
            Some(set)   => set.iter().map(|(_, h)| *h).collect(),
            None        => return Ok(0),
        };
        let dbr = lock_read!(db_arc);
        for hash in &hashes {
            // The record goes first: a session the store still held would come back at the
            // next load, and a revocation must not be undone by a restart.
            res!(dbr.delete(&self.cfg.key(hash), uid.clone(), None));
            state.remove(hash);
        }
        info!("member session: ended all {} sessions of one principal.", hashes.len());
        Ok(hashes.len())
    }

    /// Ends every expired session. How many.
    pub fn sweep(&self, db: &(Arc<RwLock<VhostDb>>, Uid)) -> Outcome<usize> {
        let now = res!(self.clock.now());
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        let gone: Vec<Hash32> = state.by_hash.iter()
            .filter(|(_, e)| expired(&self.cfg, e, now))
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
        debug!("member session: swept {} expired.", gone.len());
        Ok(gone.len())
    }

    /// The live sessions of `principal`, oldest first, as they stand without a touch.
    pub fn list(&self, principal: &[u8]) -> Outcome<Vec<Session>> {
        let now = res!(self.clock.now());
        let state = lock_mutex!(self.state);
        let mut out = Vec::new();
        if let Some(set) = state.by_principal.get(principal) {
            for (_, h) in set {
                if let Some(e) = state.by_hash.get(h) {
                    if !expired(&self.cfg, e, now) {
                        out.push(e.session(h));
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn len(&self) -> Outcome<usize> {
        Ok(lock_mutex!(self.state).by_hash.len())
    }

    fn end_hash(
        &self,
        db:     &(Arc<RwLock<VhostDb>>, Uid),
        hash:   &Hash32,
    )
        -> Outcome<bool>
    {
        let (db_arc, uid) = db;
        let mut state = lock_mutex!(self.state);
        if !state.by_hash.contains_key(hash) {
            return Ok(false);
        }
        let dbr = lock_read!(db_arc);
        // The record first, as in `end_all`: if it cannot be deleted the session stays live and
        // the caller is told, rather than its coming back at the next restart.
        res!(dbr.delete(&self.cfg.key(hash), uid.clone(), None));
        state.remove(hash);
        debug!("member session: {} ended.", short(hash));
        Ok(true)
    }
}

fn expired(cfg: &SessionCfg, e: &Entry, now: u64) -> bool {
    now >= e.last_seen.saturating_add(cfg.idle_secs)
        || now >= e.created.saturating_add(cfg.absolute_secs)
}
