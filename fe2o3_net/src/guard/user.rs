use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedyne_fe2o3_hash::map::ShardMap;
use oxedyne_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    clone::Clone,
    collections::BTreeSet,
    fmt::Debug,
    sync::RwLock,
    //time::{
    //    Duration,
    //    SystemTime,
    //},
};

/// Trust classification the server holds for a given user.
#[derive(Clone, Debug)]
pub enum UserState {
    /// User has not yet been classified.
    Unknown,
    /// User is barred; their packets are dropped.
    Blacklist, // No soup for you.
    /// User is explicitly trusted and allowed through.
    Whitelist, // Come on through.
}

impl Default for UserState {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Per-user record combining trust state with a caller-supplied data payload.
#[derive(Clone, Debug, Default)]
pub struct UserLog<
    D: Clone + Debug + Default, // user supplied data container
> {
    pub state:  UserState,      // current trust classification
    pub items:  ItemWindow,     // what the item quota has counted
    pub data:   D,              // application-specific payload
}

/// The distinct items one user has touched in one window of the caller's clock,
/// for [`UserGuard::admit_item`].
#[derive(Clone, Debug, Default)]
pub struct ItemWindow {
    pub window: u64,                // the window the items were counted in
    pub items:  BTreeSet<Vec<u8>>,  // distinct items, at most the quota's limit
}

/// What [`UserGuard::admit_item`] made of one request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaDecision {
    Fresh,      // a new item, counted, within the limit
    Repeat,     // already counted this window, so free
    Exempt,     // a whitelisted user, never counted
    Exhausted,  // a new item past the limit, refused
    Blocked,    // a blacklisted user, refused
}

impl QuotaDecision {
    /// Is the request to be served?
    pub fn allowed(&self) -> bool {
        match self {
            Self::Fresh | Self::Repeat | Self::Exempt   => true,
            Self::Exhausted | Self::Blocked             => false,
        }
    }
}

/// Sharded, concurrent guard tracking per-user trust state.
///
/// User records are held in a [`ShardMap`] keyed by user identifier, giving
/// concurrent access across `C` shards without a single global lock.
#[derive(Debug)]
pub struct UserGuard<
    // ShardMap
    const C: usize, // Capacity (maximum number of bins).
    M: MapMut<HashForm, UserLog<D>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
    const S: usize, // Key hasher salt length.
    // AddressData
    D: Clone + Debug + Default, // user supplied data container
> {
    /// Sharded map from user key to that user's log.
    pub umap: ShardMap<C, S, UserLog<D>, M, H>,
}

impl<
    // ShardMap
    const C: usize, // Capacity (maximum number of bins).
    M: MapMut<HashForm, UserLog<D>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
    const S: usize, // Key hasher salt length.
    // AddressData
    D: Clone + Debug + Default, // user supplied data container
>
    UserGuard<C, M, H, S, D>
{
    /// Updates state for given address and returns whether the packet should be dropped.
    pub fn drop_packet<
        const UIDL: usize,
        UID: NumIdDat<UIDL>,
    >(
        &self,
        uid:            &UID,
        accept_unknown: bool,
    )
        -> Outcome<bool>
    {
        let (key, locked_map) = res!(self.get_locked_map(uid));
        let mut unlocked_map = lock_write!(locked_map);
        match unlocked_map.get_mut(&key) {
            Some(_ulog) => {
                // TODO examine user log
            },
            None => {
                if accept_unknown { 
                    let ulog = UserLog::default();
                    unlocked_map.insert(key, ulog);
                } else {
                    return Ok(true);
                }
            },
        }
        Ok(false)
    }

    /// Resolves a user identifier to its shard key and the [`RwLock`] guarding
    /// the shard that would hold that user's log.
    pub fn get_locked_map<
        const UIDL: usize,
        UID: NumIdDat<UIDL>,
    >(
        &self,
        uid: &UID,
    )
        -> Outcome<(HashForm, &RwLock<M>)>
    {
        self.get_locked_map_by_bytes(&uid.to_byte_array())
    }

    /// As [`Self::get_locked_map`], for a user named by bytes of any length: a
    /// hex identifier, a name, a key. The shard map hashes whatever it is given,
    /// so a fixed-width numeric id is one caller's choice, not the guard's.
    pub fn get_locked_map_by_bytes(&self, user: &[u8]) -> Outcome<(HashForm, &RwLock<M>)> {
        let key = self.umap.key(user);
        let locked_map = res!(self.umap.get_shard_using_hash(&key));
        Ok((key, locked_map))
    }

    /// Count `item` against a quota of `limit` distinct items per window for
    /// `user`, recording it when it is admitted.
    ///
    /// `window` is the caller's own clock divided into windows, so a test can
    /// move it and two guards can share one: a request in a window other than
    /// the one the user's items were counted in starts the count again. An item
    /// already counted in the window is free, because answering it again tells
    /// the user nothing new. A whitelisted user is exempt and a blacklisted one
    /// refused, as the address guard treats them. A user not seen before is
    /// added as `Unknown`.
    pub fn admit_item(
        &self,
        user:   &[u8],
        item:   &[u8],
        window: u64,
        limit:  usize,
    )
        -> Outcome<QuotaDecision>
    {
        let (key, locked_map) = res!(self.get_locked_map_by_bytes(user));
        let mut unlocked_map = lock_write!(locked_map);
        if unlocked_map.get(&key).is_none() {
            unlocked_map.insert(key.clone(), UserLog::default());
        }
        let ulog = match unlocked_map.get_mut(&key) {
            Some(l) => l,
            None => return Err(err!(
                "The log just inserted for a user is not in its shard."; Bug, Missing)),
        };
        match ulog.state {
            UserState::Whitelist    => return Ok(QuotaDecision::Exempt),
            UserState::Blacklist    => return Ok(QuotaDecision::Blocked),
            UserState::Unknown      => (),
        }
        let items = &mut ulog.items;
        if items.window != window {
            items.window = window;
            items.items.clear();
        }
        if items.items.contains(item) {
            return Ok(QuotaDecision::Repeat);
        }
        if items.items.len() >= limit {
            return Ok(QuotaDecision::Exhausted);
        }
        items.items.insert(item.to_vec());
        Ok(QuotaDecision::Fresh)
    }

    /// Forget every `Unknown` user whose counted items belong to a window before
    /// `window`, so the guard holds only the users active in the current one.
    ///
    /// A log goes whole, payload and all, as the address guard's
    /// `sweep_idle` drops a `Monitor` record, so a caller keeping a payload it
    /// needs should not sweep. A log that never counted an item, and a
    /// whitelisted or blacklisted user, is kept: a trust decision outlives the
    /// window.
    pub fn sweep_windows(&self, window: u64) -> Outcome<usize> {
        let mut evicted = 0usize;
        for i in 0..self.umap.n {
            if let Some(locked_map) = self.umap.shards[i].as_ref() {
                let mut unlocked = lock_write!(locked_map);
                unlocked.retain(|_k, ulog| {
                    let stale = !ulog.items.items.is_empty() && ulog.items.window < window;
                    let transient = matches!(ulog.state, UserState::Unknown);
                    let drop_it = stale && transient;
                    if drop_it {
                        evicted += 1;
                    }
                    !drop_it
                });
            }
        }
        Ok(evicted)
    }
    //pub fn get_user_log<'a>(&'a self, uid: &'a U) -> Option<&'a UserLog<D>> {
    //    self.umap.get(uid)
    //}

    //pub fn get_user_log_mut<'a>(&'a mut self, uid: &'a U) -> Option<&'a mut UserLog<D>> {
    //    self.umap.get_mut(uid)
    //}
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxedyne_fe2o3_hash::hash::HashScheme;

    use std::collections::BTreeMap;

    type TestGuard = UserGuard<
        4,                                  // C: shards
        BTreeMap<HashForm, UserLog<()>>,    // M: inner map
        HashScheme,                         // H: hasher
        8,                                  // S: salt length
        (),                                 // D: no payload
    >;

    fn make_guard() -> Outcome<TestGuard> {
        Ok(UserGuard {
            umap: res!(ShardMap::new(
                4,
                [7u8; 8],
                BTreeMap::new(),
                res!(HashScheme::try_from("Seahash")),
            )),
        })
    }

    /// Distinct items are counted up to the limit, a repeat is free, and the
    /// first item past the limit is refused while a counted one is still served.
    #[test]
    fn test_admit_item_limit_00() -> Outcome<()> {
        let guard = res!(make_guard());
        let user = b"0123456789";
        for i in 0u8..3 {
            assert_eq!(res!(guard.admit_item(user, &[i], 1, 3)), QuotaDecision::Fresh);
        }
        assert_eq!(res!(guard.admit_item(user, &[3], 1, 3)), QuotaDecision::Exhausted);
        assert_eq!(res!(guard.admit_item(user, &[1], 1, 3)), QuotaDecision::Repeat,
            "an item counted this window is answered again for free");
        assert!(QuotaDecision::Repeat.allowed());
        assert!(!QuotaDecision::Exhausted.allowed());
        Ok(())
    }

    /// A new window starts the count again.
    #[test]
    fn test_admit_item_window_roll_00() -> Outcome<()> {
        let guard = res!(make_guard());
        let user = b"abcdef0123";
        assert_eq!(res!(guard.admit_item(user, b"a", 5, 1)), QuotaDecision::Fresh);
        assert_eq!(res!(guard.admit_item(user, b"b", 5, 1)), QuotaDecision::Exhausted);
        assert_eq!(res!(guard.admit_item(user, b"b", 6, 1)), QuotaDecision::Fresh,
            "the next window has its own budget");
        assert_eq!(res!(guard.admit_item(user, b"a", 6, 1)), QuotaDecision::Exhausted,
            "and an item counted in the last one is new in this one");
        Ok(())
    }

    /// One user's budget is not another's.
    #[test]
    fn test_admit_item_per_user_00() -> Outcome<()> {
        let guard = res!(make_guard());
        assert_eq!(res!(guard.admit_item(b"one", b"x", 1, 1)), QuotaDecision::Fresh);
        assert_eq!(res!(guard.admit_item(b"one", b"y", 1, 1)), QuotaDecision::Exhausted);
        assert_eq!(res!(guard.admit_item(b"two", b"y", 1, 1)), QuotaDecision::Fresh,
            "a second user is unaffected by the first's spend");
        Ok(())
    }

    /// A whitelisted user is never counted, and a blacklisted one is refused.
    #[test]
    fn test_admit_item_trust_00() -> Outcome<()> {
        let guard = res!(make_guard());
        for (user, state) in [
            (&b"white"[..], UserState::Whitelist),
            (&b"black"[..], UserState::Blacklist),
        ] {
            let (key, locked) = res!(guard.get_locked_map_by_bytes(user));
            let mut map = lock_write!(locked);
            map.insert(key, UserLog { state, ..UserLog::default() });
        }
        for _ in 0..3 {
            assert_eq!(res!(guard.admit_item(b"white", b"i", 1, 0)), QuotaDecision::Exempt);
        }
        assert_eq!(res!(guard.admit_item(b"black", b"i", 1, 10)), QuotaDecision::Blocked);
        Ok(())
    }

    /// A sweep forgets the users whose items are from an earlier window, and
    /// keeps the current ones and every trust decision.
    #[test]
    fn test_sweep_windows_00() -> Outcome<()> {
        let guard = res!(make_guard());
        res!(guard.admit_item(b"old", b"i", 1, 5));
        res!(guard.admit_item(b"new", b"i", 2, 5));
        {
            let (key, locked) = res!(guard.get_locked_map_by_bytes(b"barred"));
            let mut map = lock_write!(locked);
            map.insert(key, UserLog { state: UserState::Blacklist, ..UserLog::default() });
        }
        assert_eq!(res!(guard.sweep_windows(2)), 1, "only the stale user goes");
        let count = |user: &[u8]| -> Outcome<bool> {
            let (key, locked) = res!(guard.get_locked_map_by_bytes(user));
            let map = lock_read!(locked);
            Ok(map.get(&key).is_some())
        };
        assert!(!res!(count(b"old")));
        assert!(res!(count(b"new")));
        assert!(res!(count(b"barred")));
        Ok(())
    }
}
