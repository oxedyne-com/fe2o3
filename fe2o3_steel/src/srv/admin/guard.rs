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

/// Number of shards the guard's per-address map is split over.
pub const GUARD_SHARDS:                 usize = 16;
/// Length of the per-address request-timestamp ring.
pub const GUARD_RING:                   usize = 64;
/// Salt length used by the shard hasher.
pub const GUARD_SALT_LEN:               usize = 8;
/// Fixed salt bytes for the shard hasher. Static because the guard map is in-memory only.
pub const GUARD_SALT: [u8; GUARD_SALT_LEN] = [
    0x9a, 0x5b, 0x11, 0xe7, 0xaa, 0x3c, 0x80, 0x42,
];

/// Default maximum average requests per second a Monitor address may sustain before
/// being downgraded. HTTP clients burst heavily on page loads, so the default is more
/// permissive than shield's 30.
pub const DEFAULT_RPS_MAX:              u64 = 50;
/// Default minimum spacing between permitted requests while throttled.
pub const DEFAULT_TINT_MIN:             Duration = Duration::from_millis(100);
/// Default base throttle cooldown.
pub const DEFAULT_TSUNSET_BASE:         Duration = Duration::from_secs(60);
/// Default upper bound on jitter added to `DEFAULT_TSUNSET_BASE`.
pub const DEFAULT_TSUNSET_SPREAD:       Duration = Duration::from_secs(240);
/// Default number of throttle episodes before automatic blacklisting.
pub const DEFAULT_BLIST_CNT:            u16 = 6;

/// Default cap on the number of entries returned by a dashboard snapshot.
pub const DEFAULT_SNAPSHOT_CAP:         usize = 256;

/// Fully concrete type alias for the address guard Steel uses. The caller-supplied
/// extension payload is `()`: Steel does not need to carry shield-style proof-of-work
/// negotiation on top of the state machine.
pub type SteelAddressGuard = AddressGuard<
    GUARD_SHARDS,
    BTreeMap<HashForm, AddressLog<GUARD_RING, ()>>,
    HashScheme,
    GUARD_SALT_LEN,
    GUARD_RING,
    (),
>;

/// Construct a shared Steel address guard with the module defaults.
pub fn new_shared() -> Outcome<Arc<SteelAddressGuard>> {
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
        arps_max:       DEFAULT_RPS_MAX,
        tint_min:       DEFAULT_TINT_MIN,
        tsunset_base:   DEFAULT_TSUNSET_BASE,
        tsunset_spread: DEFAULT_TSUNSET_SPREAD,
        blist_cnt:      DEFAULT_BLIST_CNT,
    };
    Ok(Arc::new(guard))
}
