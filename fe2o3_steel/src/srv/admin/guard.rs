//! Per-IP rate-limit / blacklist guard for Steel.
//!
//! A thin wrapper around `fe2o3_net::guard::addr::AddressGuard` that fixes the generic
//! parameters to Steel's defaults and exposes a `new_shared` builder. Referenced from
//! `AdminState`, fed by the TCP accept loop in `srv/server.rs`, and rendered by the admin
//! dashboard's Security view.
//!
//! The guard is intentionally wired in the TCP accept path rather than deeper in the HTTPS
//! handler so a blacklisted attacker costs the server only a SYN/ACK -- no TLS handshake,
//! no HTTP parse, no application dispatch.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::{
    hash::HashScheme,
    map::ShardMap,
};
use oxedyne_fe2o3_iop_hash::api::HashForm;
use oxedyne_fe2o3_net::guard::addr::{
    AddressGuard,
    AddressLog,
};

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::Duration,
};

pub const GUARD_SHARDS:                 usize = 16;
pub const GUARD_RING:                   usize = 64;
pub const GUARD_SALT_LEN:               usize = 8;
// Fixed salt bytes for the shard hasher, static because the guard map is
// in-memory only.
pub const GUARD_SALT: [u8; GUARD_SALT_LEN] = [
    0x9a, 0x5b, 0x11, 0xe7, 0xaa, 0x3c, 0x80, 0x42,
];

// Guard thresholds an operator may override from the `addr_guard` config block.
// The request ceiling is more permissive than shield's 30 because HTTP clients
// burst heavily on page loads.
pub const DEFAULT_RPS_MAX:              u64 = 50;
pub const DEFAULT_TINT_MIN:             Duration = Duration::from_millis(100);
pub const DEFAULT_TSUNSET_BASE:         Duration = Duration::from_secs(60);
pub const DEFAULT_TSUNSET_SPREAD:       Duration = Duration::from_secs(240);
pub const DEFAULT_BLIST_CNT:            u16 = 6;

pub const DEFAULT_SNAPSHOT_CAP:         usize = 256;

/// The caller-supplied extension payload is `()`: Steel does not need to carry
/// shield-style proof-of-work negotiation on top of the state machine.
pub type SteelAddressGuard = AddressGuard<
    GUARD_SHARDS,
    BTreeMap<HashForm, AddressLog<GUARD_RING, ()>>,
    HashScheme,
    GUARD_SALT_LEN,
    GUARD_RING,
    (),
>;

/// Deserialised from the `addr_guard` block in Steel's `ServerConfig` and applied
/// when the guard is constructed at startup.
///
/// Every field has a meaningful default (the `DEFAULT_*` consts above), so a
/// deployment that omits the `addr_guard` block altogether gets the same
/// thresholds as the pre-config version of this module.
#[derive(Clone, Debug)]
pub struct AddrGuardSettings {
    pub rps_max:            u64,        // average requests per second before downgrade to Throttle
    pub tint_min:           Duration,   // minimum interval between allowed requests while throttled
    pub tsunset_base:       Duration,   // base throttle cooldown
    pub tsunset_spread:     Duration,   // jitter added to `tsunset_base`, spreading cooldown expiry
    pub blist_cnt:          u16,        // throttle episodes before auto-blacklisting
}

impl Default for AddrGuardSettings {
    fn default() -> Self {
        Self {
            rps_max:        DEFAULT_RPS_MAX,
            tint_min:       DEFAULT_TINT_MIN,
            tsunset_base:   DEFAULT_TSUNSET_BASE,
            tsunset_spread: DEFAULT_TSUNSET_SPREAD,
            blist_cnt:      DEFAULT_BLIST_CNT,
        }
    }
}

pub fn new_shared() -> Outcome<Arc<SteelAddressGuard>> {
    new_shared_with(AddrGuardSettings::default())
}

/// The shard count, ring length and hasher salt are fixed compile-time
/// parameters; only the runtime thresholds are operator adjustable.
pub fn new_shared_with(settings: AddrGuardSettings) -> Outcome<Arc<SteelAddressGuard>> {
    let amap = res!(ShardMap::<
        GUARD_SHARDS,
        GUARD_SALT_LEN,
        AddressLog<GUARD_RING, ()>,
        BTreeMap<HashForm, AddressLog<GUARD_RING, ()>>,
        HashScheme,
    >::new(
        GUARD_SHARDS as u32,
        GUARD_SALT,
        BTreeMap::new(),
        res!(HashScheme::try_from("Seahash")),
    ));
    let guard = AddressGuard {
        amap,
        arps_max:       settings.rps_max,
        tint_min:       settings.tint_min,
        tsunset_base:   settings.tsunset_base,
        tsunset_spread: settings.tsunset_spread,
        blist_cnt:      settings.blist_cnt,
    };
    Ok(Arc::new(guard))
}
