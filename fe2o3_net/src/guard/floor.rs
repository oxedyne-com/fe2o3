//! A minimum interval between two accepted calls under one string key.
//!
//! Lifted from `/api/debug-trace` in Daimond's gateway, where it kept one account and device
//! from posting a diagnostic trace more than once every two seconds. `addr::AddressGuard` rates
//! an address and escalates to a blacklist, `user::UserGuard` holds trust state and
//! `nonce::NonceTracker` refuses a replay, so none of them answers "has this key spoken within
//! the last N milliseconds", which is a floor and not a window.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    sync::Mutex,
};


/// Remembers when each key was last admitted, and refuses a key that returns too soon.
///
/// The caller supplies the clock, so a test can say "1.5 seconds later" without waiting. A
/// refused call leaves the record alone: a client hammering the floor does not hold it open.
/// Entries are let go once they are older than the time to live, but only when the map has grown
/// past a bound, so a quiet guard never walks its own map.
#[derive(Debug)]
pub struct KeyFloor {
    min_ms:     u64,
    ttl_ms:     u64,
    prune_at:   usize,
    last:       Mutex<HashMap<String, u64>>, // key -> millisecond of the last admitted call
}

impl KeyFloor {

    pub const DEFAULT_TTL_MS:   u64     = 3_600_000;
    pub const DEFAULT_PRUNE_AT: usize   = 4_096;

    pub fn new(min_ms: u64) -> Self {
        Self {
            min_ms,
            ttl_ms:     Self::DEFAULT_TTL_MS,
            prune_at:   Self::DEFAULT_PRUNE_AT,
            last:       Mutex::new(HashMap::new()),
        }
    }

    /// How long an entry is kept before it may be pruned. It must outlast the floor, or a live
    /// key would be forgotten mid-interval.
    pub fn with_ttl_ms(mut self, ttl_ms: u64) -> Self {
        self.ttl_ms = ttl_ms.max(self.min_ms);
        self
    }

    /// The size the map may reach before the stale entries are swept out.
    pub fn with_prune_at(mut self, prune_at: usize) -> Self {
        self.prune_at = prune_at;
        self
    }

    /// Is the key far enough past its last admitted call? Records the call when it is.
    pub fn admit(&self, key: &str, now_ms: u64) -> Outcome<bool> {
        let mut map = lock_mutex!(self.last);
        if map.len() > self.prune_at {
            let ttl = self.ttl_ms;
            map.retain(|_, &mut last| now_ms.saturating_sub(last) < ttl);
        }
        if let Some(&last) = map.get(key) {
            if now_ms.saturating_sub(last) < self.min_ms {
                return Ok(false);
            }
        }
        map.insert(key.to_string(), now_ms);
        Ok(true)
    }

    /// How many keys are held.
    pub fn len(&self) -> Outcome<usize> {
        let map = lock_mutex!(self.last);
        Ok(map.len())
    }
}
