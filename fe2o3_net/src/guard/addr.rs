//! Generic per-IP rate-limiting and blacklisting guard.
//!
//! `AddressGuard` tracks a per-IP state machine -- `Monitor` then `Throttle` then
//! `Blacklist`, plus a manual `Whitelist` escape hatch -- driven by a sliding window of
//! request timestamps. Each caller-visible primitive returns an `Outcome<GuardDecision>`
//! describing whether the request should be allowed through, throttled, or blocked.
//!
//! The guard is transport-agnostic; HTTPS, SMTP, SHIELD's UDP wire protocol or any other
//! caller can plug it into its accept path. Protocol-specific extensions (for example
//! SHIELD's handshake-sequence check) are layered on top via the low-level
//! [`AddressGuard::update_log`] helper, which exposes the per-address log under the same
//! shard lock acquired by the rate-limit check.

use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedyne_fe2o3_data::ring::RingTimer;
use oxedyne_fe2o3_hash::map::ShardMap;
use oxedyne_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};

use std::{
    clone::Clone,
    fmt::Debug,
    net::{
        IpAddr,
        SocketAddr,
    },
    sync::RwLock,
    time::{
        Duration,
        SystemTime,
    },
};

/// Per-address state in the guard state machine.
#[derive(Clone, Debug)]
pub enum AddressState<const N: usize> {
    /// Passively observing request rate; nothing dropped.
    Monitor(RingTimer<N>),
    /// Actively throttling; requests closer than `tint_min` apart are dropped.
    Throttle {
        /// Ring of throttled request timestamps.
        reqs:       RingTimer<N>,
        /// Minimum interval between allowed requests.
        tint_min:   Duration,
        /// When the throttle episode began.
        start:      SystemTime,
        /// Cooldown after which the address returns to `Monitor`.
        sunset:     Duration,
    },
    /// Address is blocked outright.
    Blacklist {
        /// When the address was blacklisted.
        since:  SystemTime,
        /// Why the address was blacklisted.
        reason: BlacklistReason,
    },
    /// Address is always allowed through.
    Whitelist,
}

impl<const N: usize> Default for AddressState<N> {
    fn default() -> Self {
        Self::Monitor(RingTimer::default())
    }
}

impl<const N: usize> AddressState<N> {
    /// Short label used by dashboards, logs and snapshot listings.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Monitor(_)    => "monitor",
            Self::Throttle{..}  => "throttle",
            Self::Blacklist{..} => "blacklist",
            Self::Whitelist     => "whitelist",
        }
    }
}

/// Reason an address is in the `Blacklist` state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlacklistReason {
    /// Guard state machine transitioned the address automatically after sustained rate abuse.
    AutoRateLimit,
    /// Administrator action.
    Manual,
}

/// Per-address log: state, counters, and caller-supplied extension data `D`.
#[derive(Clone, Debug)]
pub struct AddressLog<
    const N: usize,
    D: Clone + Debug + Default,
> {
    /// The IP this log belongs to, stored here so snapshots can emit human-readable rows
    /// without reverse-resolving hashed shard keys.
    pub ip:             Option<IpAddr>,
    /// Guard state machine state.
    pub state:          AddressState<N>,
    /// Number of throttling episodes this address has been through.
    pub throttle_cnt:   u16,
    /// When the address was first observed.
    pub first_seen:     SystemTime,
    /// When the address was most recently observed.
    pub last_seen:      SystemTime,
    /// Total requests observed for this address.
    pub total_reqs:     u64,
    /// Caller-supplied extension payload.
    pub data:           D,
}

impl<
    const N: usize,
    D: Clone + Debug + Default,
>
    Default for AddressLog<N, D>
{
    fn default() -> Self {
        let now = SystemTime::now();
        Self {
            ip:             None,
            state:          AddressState::default(),
            throttle_cnt:   0,
            first_seen:     now,
            last_seen:      now,
            total_reqs:     0,
            data:           D::default(),
        }
    }
}

/// Decision returned by the guard check APIs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardDecision {
    /// Let the request through.
    Allow,
    /// Request is within an active throttle window; drop it.
    Throttled,
    /// Address is blacklisted; drop the request.
    Blocked(BlacklistReason),
}

impl GuardDecision {
    /// True if the guard says the request must be dropped.
    pub fn should_drop(&self) -> bool {
        !matches!(self, Self::Allow)
    }
}

/// Aggregate tallies across all known addresses, suitable for a dashboard chip row.
#[derive(Clone, Copy, Debug, Default)]
pub struct GuardCounts {
    /// Addresses currently in the Monitor state.
    pub monitor:        usize,
    /// Addresses currently in the Throttle state.
    pub throttle:       usize,
    /// Addresses currently in the Blacklist state.
    pub blacklist:      usize,
    /// Addresses currently in the Whitelist state.
    pub whitelist:      usize,
    /// Total distinct addresses observed.
    pub total:          usize,
    /// Total requests recorded across all addresses.
    pub total_reqs:     u64,
}

/// One row in a guard snapshot table.
#[derive(Clone, Debug)]
pub struct GuardEntry {
    /// IP address (only emitted when the log stored one).
    pub ip:             IpAddr,
    /// State label ("monitor" / "throttle" / "blacklist" / "whitelist").
    pub state:          &'static str,
    /// Number of throttling episodes this address has been through.
    pub throttle_cnt:   u16,
    /// Total requests for this address.
    pub total_reqs:     u64,
    /// First time the address was observed.
    pub first_seen:     SystemTime,
    /// Most recent observation.
    pub last_seen:      SystemTime,
}

/// Snapshot of the guard: counts plus up to `max` per-address entries.
#[derive(Clone, Debug, Default)]
pub struct GuardSnapshot {
    /// Aggregate tallies.
    pub counts:     GuardCounts,
    /// Per-address rows, capped by the caller.
    pub entries:    Vec<GuardEntry>,
}

/// Generic per-address guard.
#[derive(Debug)]
pub struct AddressGuard<
    const C: usize, // ShardMap capacity.
    M: MapMut<HashForm, AddressLog<N, D>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static,
    const S: usize, // Hasher salt length.
    const N: usize, // Request timer ring length.
    D: Clone + Debug + Default,
> {
    /// Shard map of per-address logs keyed by a hash of the IP octets.
    pub amap:           ShardMap<C, S, AddressLog<N, D>, M, H>,
    /// Maximum average requests per second permitted in `Monitor` state.
    pub arps_max:       u64,
    /// Minimum interval between allowed requests in `Throttle` state.
    pub tint_min:       Duration,
    /// Base throttle cooldown duration.
    pub tsunset_base:   Duration,
    /// Upper bound on jitter added to `tsunset_base` to spread cooldown expiries.
    pub tsunset_spread: Duration,
    /// Number of throttling episodes after which the address is blacklisted.
    pub blist_cnt:      u16,
}

impl<
    const C: usize,
    M: MapMut<HashForm, AddressLog<N, D>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static,
    const S: usize,
    const N: usize,
    D: Clone + Debug + Default,
>
    AddressGuard<C, M, H, S, N, D>
{
    /// Convert an IP address to its raw octets as a byte vector.
    fn ip_bytes(addr: &IpAddr) -> Vec<u8> {
        match addr {
            IpAddr::V4(a) => a.octets().to_vec(),
            IpAddr::V6(a) => a.octets().to_vec(),
        }
    }

    /// Compute a sunset duration with coarse, deterministic jitter derived from the system
    /// clock. The purpose is anti-coordination of cooldown expiries; cryptographic randomness
    /// is unnecessary.
    fn sunset(&self) -> Duration {
        if self.tsunset_spread.is_zero() {
            return self.tsunset_base;
        }
        let spread = self.tsunset_spread.as_nanos() as u64;
        let now = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as u64,
            Err(_) => 0,
        };
        self.tsunset_base + Duration::from_nanos(now % spread)
    }

    /// Check an IP against the guard and update its log. If the address is unknown it is
    /// inserted in the `Monitor` state and `Allow` is returned.
    pub fn check(&self, addr: &IpAddr) -> Outcome<GuardDecision> {
        let (decision, _) = res!(self.update_log(addr, |_log, _new| Ok(())));
        Ok(decision)
    }

    /// Low-level primitive: resolve the per-address log, run the state-machine step, and
    /// invoke `extra` while still holding the shard write lock so callers can compose their
    /// own checks on top of the generic rate-limit. Returns the state-machine decision plus
    /// whatever `extra` returned.
    ///
    /// `extra` is passed a mutable reference to the log and a boolean indicating whether the
    /// log was newly created by this call.
    pub fn update_log<F, T>(
        &self,
        addr:   &IpAddr,
        extra:  F,
    )
        -> Outcome<(GuardDecision, T)>
    where
        F: FnOnce(&mut AddressLog<N, D>, bool) -> Outcome<T>,
    {
        let key = self.amap.key(&Self::ip_bytes(addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        let mut unlocked_map = lock_write!(locked_map);
        let now = SystemTime::now();

        // Fast path for new entries: insert a fresh Monitor log, record the first request,
        // run the caller's extra hook and return.
        if unlocked_map.get(&key).is_none() {
            let mut log = AddressLog::<N, D>::default();
            log.ip          = Some(*addr);
            log.first_seen  = now;
            log.last_seen   = now;
            log.total_reqs  = 1;
            if let AddressState::Monitor(ref mut reqs) = log.state {
                reqs.update();
            }
            let extra_val = res!(extra(&mut log, true));
            unlocked_map.insert(key, log);
            return Ok((GuardDecision::Allow, extra_val));
        }

        // Existing entry: update, run state machine, run extra hook, all under one lock.
        let sunset = self.sunset();
        let decision = {
            let log = match unlocked_map.get_mut(&key) {
                Some(l) => l,
                None => return Err(err!(
                    "Address log for {} vanished between contains and get_mut.", addr;
                Bug, Missing)),
            };
            log.last_seen = now;
            log.total_reqs = log.total_reqs.saturating_add(1);
            self.evaluate(log, sunset)
        };
        let extra_val = match unlocked_map.get_mut(&key) {
            Some(l) => res!(extra(l, false)),
            None => return Err(err!(
                "Address log for {} vanished mid-update.", addr;
            Bug, Missing)),
        };
        Ok((decision, extra_val))
    }

    /// Run the Monitor -> Throttle -> Blacklist state-machine step for a known entry.
    fn evaluate(
        &self,
        log:    &mut AddressLog<N, D>,
        sunset: Duration,
    )
        -> GuardDecision
    {
        // Sunset expired throttled addresses back to Monitor before this step.
        if let AddressState::Throttle{ start, sunset: cool, .. } = &log.state {
            if let Ok(elapsed) = start.elapsed() {
                if elapsed >= *cool {
                    log.state = AddressState::Monitor(RingTimer::default());
                }
            }
        }

        match &mut log.state {
            AddressState::Monitor(ref mut reqs) => {
                reqs.update();
                if reqs.avg_rps() > self.arps_max {
                    let next_cnt = log.throttle_cnt.saturating_add(1);
                    if next_cnt >= self.blist_cnt {
                        log.state = AddressState::Blacklist {
                            since:  SystemTime::now(),
                            reason: BlacklistReason::AutoRateLimit,
                        };
                        log.throttle_cnt = next_cnt;
                        return GuardDecision::Blocked(BlacklistReason::AutoRateLimit);
                    }
                    log.state = AddressState::Throttle {
                        reqs:       RingTimer::default(),
                        tint_min:   self.tint_min,
                        start:      SystemTime::now(),
                        sunset,
                    };
                    log.throttle_cnt = next_cnt;
                    return GuardDecision::Throttled;
                }
                GuardDecision::Allow
            },
            AddressState::Throttle{ ref mut reqs, tint_min, .. } => {
                reqs.update();
                if reqs.last_duration() < *tint_min {
                    return GuardDecision::Throttled;
                }
                GuardDecision::Allow
            },
            AddressState::Blacklist{ reason, .. } => {
                GuardDecision::Blocked(*reason)
            },
            AddressState::Whitelist => GuardDecision::Allow,
        }
    }

    /// Acquire the shard lock and key for a socket address. Kept for callers that want to
    /// drive the map themselves without going through `update_log`.
    pub fn get_locked_map(
        &self,
        addr: &SocketAddr,
    )
        -> Outcome<(HashForm, &RwLock<M>)>
    {
        let ip_addr = addr.ip();
        let key = self.amap.key(&Self::ip_bytes(&ip_addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        Ok((key, locked_map))
    }

    /// Force an IP into the Whitelist state. Creates the log if missing.
    pub fn whitelist(&self, addr: &IpAddr) -> Outcome<()> {
        let key = self.amap.key(&Self::ip_bytes(addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        let mut unlocked_map = lock_write!(locked_map);
        match unlocked_map.get_mut(&key) {
            Some(log) => log.state = AddressState::Whitelist,
            None => {
                let mut log = AddressLog::<N, D>::default();
                log.ip    = Some(*addr);
                log.state = AddressState::Whitelist;
                unlocked_map.insert(key, log);
            }
        }
        Ok(())
    }

    /// Force an IP into the Blacklist state with `BlacklistReason::Manual`.
    pub fn blacklist(&self, addr: &IpAddr) -> Outcome<()> {
        let key = self.amap.key(&Self::ip_bytes(addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        let mut unlocked_map = lock_write!(locked_map);
        let bl = AddressState::Blacklist {
            since:  SystemTime::now(),
            reason: BlacklistReason::Manual,
        };
        match unlocked_map.get_mut(&key) {
            Some(log) => log.state = bl,
            None => {
                let mut log = AddressLog::<N, D>::default();
                log.ip    = Some(*addr);
                log.state = bl;
                unlocked_map.insert(key, log);
            }
        }
        Ok(())
    }

    /// Reset an IP to the default Monitor state and zero its throttle count.
    pub fn unblock(&self, addr: &IpAddr) -> Outcome<()> {
        let key = self.amap.key(&Self::ip_bytes(addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        let mut unlocked_map = lock_write!(locked_map);
        if let Some(log) = unlocked_map.get_mut(&key) {
            log.state = AddressState::default();
            log.throttle_cnt = 0;
        }
        Ok(())
    }

    /// Lookup the current state label for an address, without mutation. Returns `None` if
    /// the address has never been observed.
    pub fn peek(&self, addr: &IpAddr) -> Outcome<Option<&'static str>> {
        let key = self.amap.key(&Self::ip_bytes(addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        let unlocked_map = lock_read!(locked_map);
        Ok(unlocked_map.get(&key).map(|l| l.state.label()))
    }

    /// Aggregate tallies across all known addresses. O(number of addresses).
    pub fn counts(&self) -> Outcome<GuardCounts> {
        let mut c = GuardCounts::default();
        for i in 0..self.amap.n {
            if let Some(locked_map) = self.amap.shards[i].as_ref() {
                let unlocked = lock_read!(locked_map);
                for (_k, log) in unlocked.iter() {
                    c.total += 1;
                    c.total_reqs = c.total_reqs.saturating_add(log.total_reqs);
                    match log.state {
                        AddressState::Monitor(_)    => c.monitor   += 1,
                        AddressState::Throttle{..}  => c.throttle  += 1,
                        AddressState::Blacklist{..} => c.blacklist += 1,
                        AddressState::Whitelist     => c.whitelist += 1,
                    }
                }
            }
        }
        Ok(c)
    }

    /// Snapshot of counts plus up to `max` per-address entries. Entries are emitted in no
    /// particular order; callers that want a stable view should sort on the returned
    /// `Vec<GuardEntry>` themselves.
    pub fn snapshot(&self, max: usize) -> Outcome<GuardSnapshot> {
        let mut snap = GuardSnapshot::default();
        for i in 0..self.amap.n {
            if let Some(locked_map) = self.amap.shards[i].as_ref() {
                let unlocked = lock_read!(locked_map);
                for (_k, log) in unlocked.iter() {
                    snap.counts.total += 1;
                    snap.counts.total_reqs = snap.counts.total_reqs.saturating_add(log.total_reqs);
                    match log.state {
                        AddressState::Monitor(_)    => snap.counts.monitor   += 1,
                        AddressState::Throttle{..}  => snap.counts.throttle  += 1,
                        AddressState::Blacklist{..} => snap.counts.blacklist += 1,
                        AddressState::Whitelist     => snap.counts.whitelist += 1,
                    }
                    if snap.entries.len() < max {
                        if let Some(ip) = log.ip {
                            snap.entries.push(GuardEntry {
                                ip,
                                state:          log.state.label(),
                                throttle_cnt:   log.throttle_cnt,
                                total_reqs:     log.total_reqs,
                                first_seen:     log.first_seen,
                                last_seen:      log.last_seen,
                            });
                        }
                    }
                }
            }
        }
        Ok(snap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxedyne_fe2o3_hash::{
        hash::HashScheme,
        map::ShardMap,
    };
    use oxedyne_fe2o3_iop_hash::api::HashForm;

    use std::{
        collections::BTreeMap,
        net::Ipv4Addr,
    };

    const N: usize = 16; // Short ring so tests can saturate it quickly.

    type TestGuard = AddressGuard<
        4,                                      // C: shards
        BTreeMap<HashForm, AddressLog<N, ()>>,  // M: inner map
        HashScheme,                             // H: hasher
        8,                                      // S: salt len
        N,                                      // N: ring length
        (),                                     // D: no extension data
    >;

    fn make_guard(rps_max: u64, blist_cnt: u16) -> TestGuard {
        let salt = [1u8; 8];
        AddressGuard {
            amap: ShardMap::<4, 8, AddressLog<N, ()>, BTreeMap<HashForm, AddressLog<N, ()>>, HashScheme>::new(
                4,
                salt,
                BTreeMap::new(),
                HashScheme::try_from("Seahash").expect("seahash scheme"),
            ).expect("shard map"),
            arps_max:       rps_max,
            tint_min:       Duration::from_millis(10),
            tsunset_base:   Duration::from_millis(50),
            tsunset_spread: Duration::ZERO,
            blist_cnt,
        }
    }

    #[test]
    fn check_allows_first_request() {
        let guard = make_guard(100, 5);
        let addr: IpAddr = Ipv4Addr::new(10, 0, 0, 1).into();
        let d = guard.check(&addr).expect("check");
        assert_eq!(d, GuardDecision::Allow);
        let counts = guard.counts().expect("counts");
        assert_eq!(counts.total, 1);
        assert_eq!(counts.monitor, 1);
    }

    #[test]
    fn manual_blacklist_blocks_subsequent_checks() {
        let guard = make_guard(100, 5);
        let addr: IpAddr = Ipv4Addr::new(10, 0, 0, 2).into();
        guard.blacklist(&addr).expect("blacklist");
        let d = guard.check(&addr).expect("check");
        match d {
            GuardDecision::Blocked(BlacklistReason::Manual) => (),
            other => panic!("expected Blocked(Manual), got {:?}", other),
        }
    }

    #[test]
    fn unblock_restores_monitor() {
        let guard = make_guard(100, 5);
        let addr: IpAddr = Ipv4Addr::new(10, 0, 0, 3).into();
        guard.blacklist(&addr).expect("blacklist");
        guard.unblock(&addr).expect("unblock");
        let d = guard.check(&addr).expect("check");
        assert_eq!(d, GuardDecision::Allow);
    }

    #[test]
    fn whitelist_overrides_rate_limit() {
        let guard = make_guard(1, 5); // very tight
        let addr: IpAddr = Ipv4Addr::new(10, 0, 0, 4).into();
        guard.whitelist(&addr).expect("whitelist");
        // Drive many rapid checks; all should be Allow.
        for _ in 0..64 {
            assert_eq!(guard.check(&addr).expect("check"), GuardDecision::Allow);
        }
    }

    #[test]
    fn snapshot_reports_per_address_entries() {
        let guard = make_guard(100, 5);
        let a: IpAddr = Ipv4Addr::new(10, 0, 0, 5).into();
        let b: IpAddr = Ipv4Addr::new(10, 0, 0, 6).into();
        guard.check(&a).expect("a");
        guard.check(&b).expect("b");
        let snap = guard.snapshot(16).expect("snapshot");
        assert_eq!(snap.counts.total, 2);
        assert_eq!(snap.entries.len(), 2);
        assert!(snap.entries.iter().any(|e| e.ip == a));
        assert!(snap.entries.iter().any(|e| e.ip == b));
    }
}
