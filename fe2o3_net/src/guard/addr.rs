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
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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
        Ipv4Addr,
        SocketAddr,
    },
    sync::{
        Arc,
        RwLock,
        atomic::{
            AtomicUsize,
            Ordering,
        },
    },
    time::{
        Duration,
        SystemTime,
    },
};

/// Per-address state in the guard state machine.
#[derive(Clone, Debug)]
pub enum AddressState<const N: usize> {
    Monitor(RingTimer<N>),    // watching the rate only, nothing dropped
    Throttle {    // a request closer than `tint_min` to the last is dropped
        reqs:       RingTimer<N>,
        tint_min:   Duration,
        start:      SystemTime,
        sunset:     Duration,    // cooldown, after which the address returns to Monitor
    },
    Blacklist {    // blocked outright
        since:  SystemTime,
        reason: BlacklistReason,
    },
    Whitelist,    // always allowed through
}

impl<const N: usize> Default for AddressState<N> {
    fn default() -> Self {
        Self::Monitor(RingTimer::default())
    }
}

impl<const N: usize> AddressState<N> {
    /// One of "monitor", "throttle", "blacklist" or "whitelist".
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
    AutoRateLimit,    // the state machine moved it there after sustained abuse
    Manual,           // administrator action
}

/// Per-address log: state, counters, and caller-supplied extension data `D`.
#[derive(Clone, Debug)]
pub struct AddressLog<
    const N: usize,
    D: Clone + Debug + Default,
> {
    pub ip:             Option<IpAddr>,     // held so a snapshot need not reverse a shard key
    pub state:          AddressState<N>,
    pub throttle_cnt:   u16,                // throttling episodes so far
    pub first_seen:     SystemTime,
    pub last_seen:      SystemTime,
    pub total_reqs:     u64,
    // Live connections from this address right now, held behind an `Arc` so a
    // `ConnPermit` handed to a spawned task can decrement it on drop without
    // re-acquiring the shard lock, and so an eviction of the log cannot strand
    // an in-flight permit. This is a *concurrency* gauge, orthogonal to the
    // *rate* window above.
    pub conns:          Arc<AtomicUsize>,
    // When this address last crossed into an offending state (throttled or auto-
    // blacklisted), so a decay can relax it after a quiet spell. `None` once it
    // has decayed or if it has never offended. Distinct from `last_seen`, which a
    // still-active address refreshes on every request: the whole point is to
    // relax an address that is *alive but no longer offending*.
    pub offended_at:    Option<SystemTime>,
    pub data:           D,                  // caller-supplied extension payload
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
            conns:          Arc::new(AtomicUsize::new(0)),
            offended_at:    None,
            data:           D::default(),
        }
    }
}

/// Decision returned by the guard check APIs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardDecision {
    Allow,
    Throttled,                   // inside an active throttle window
    Blocked(BlacklistReason),
}

impl GuardDecision {
    pub fn should_drop(&self) -> bool {
        !matches!(self, Self::Allow)
    }
}

/// Aggregate tallies across all known addresses, suitable for a dashboard chip row.
#[derive(Clone, Copy, Debug, Default)]
pub struct GuardCounts {
    pub monitor:        usize,
    pub throttle:       usize,
    pub blacklist:      usize,
    pub whitelist:      usize,
    pub total:          usize,    // distinct addresses observed
    pub total_reqs:     u64,      // across every address
}

/// One row in a guard snapshot table.
#[derive(Clone, Debug)]
pub struct GuardEntry {
    pub ip:             IpAddr,          // a row appears only where the log stored one
    pub state:          &'static str,    // "monitor", "throttle", "blacklist" or "whitelist"
    pub throttle_cnt:   u16,
    pub total_reqs:     u64,
    pub first_seen:     SystemTime,
    pub last_seen:      SystemTime,
}

/// Snapshot of the guard: counts plus up to `max` per-address entries.
#[derive(Clone, Debug, Default)]
pub struct GuardSnapshot {
    pub counts:     GuardCounts,
    pub entries:    Vec<GuardEntry>,    // capped by the caller
}

/// An RAII grant of one concurrent connection from an address.
///
/// Returned by [`AddressGuard::acquire`] and held for the lifetime of the
/// connection it admits, typically moved into the task that serves it. The two
/// counters it holds -- the per-address one and the guard-wide one -- both fall
/// in `Drop`, so a task that panics is not counted against its address for ever
/// after, and neither counter needs the shard lock to be released.
#[derive(Debug)]
pub struct ConnPermit {
    per_ip: Arc<AtomicUsize>,
    total:  Arc<AtomicUsize>,
}

impl Drop for ConnPermit {
    fn drop(&mut self) {
        self.per_ip.fetch_sub(1, Ordering::AcqRel);
        self.total.fetch_sub(1, Ordering::AcqRel);
    }
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
    // Per-address logs, keyed by a hash of the IP octets.
    pub amap:           ShardMap<C, S, AddressLog<N, D>, M, H>,
    pub arps_max:       u64,         // maximum average requests per second in Monitor
    pub tint_min:       Duration,    // minimum interval between requests in Throttle
    pub tsunset_base:   Duration,    // base throttle cooldown
    pub tsunset_spread: Duration,    // jitter ceiling on it, to spread the expiries
    pub blist_cnt:      u16,         // throttling episodes before blacklisting
    // Maximum concurrent connections from any one address; 0 disables the cap,
    // which is the inert default. A concurrency bound orthogonal to the rate
    // window, closing the slow-hold shape a rate limiter alone cannot see.
    pub conn_max:       usize,
    // Live connections across every address right now, a guard-wide gauge kept
    // in step by `acquire` and `ConnPermit::drop`. Read for a health body.
    pub live_total:     Arc<AtomicUsize>,
    // How long an alive-but-quiet record keeps its throttle history before it
    // decays: an address past this long with no fresh offence has its throttle
    // count reset and an auto-blacklist or throttle relaxed to Monitor. 0
    // disables the decay, which is the inert default -- but a running process
    // then never forgets a one-off burst, so a shared NAT address that was heavy
    // once stays near the blacklist threshold until a restart.
    pub decay_after:    Duration,
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
    fn ip_bytes(addr: &IpAddr) -> Vec<u8> {
        match addr {
            IpAddr::V4(a) => a.octets().to_vec(),
            IpAddr::V6(a) => a.octets().to_vec(),
        }
    }

    /// The jitter is coarse and deterministic, taken from the system clock: its
    /// purpose is to stop cooldowns expiring together, not to be unguessable.
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

    /// An address not seen before is inserted in `Monitor` and allowed through.
    pub fn check(&self, addr: &IpAddr) -> Outcome<GuardDecision> {
        let (decision, _) = res!(self.update_log(addr, |_log, _new| Ok(())));
        Ok(decision)
    }

    /// Claim one concurrent connection slot for an address.
    ///
    /// Returns `Some(permit)` when the address is under its concurrency cap, and
    /// `None` when it is at or over it -- the caller then drops the connection.
    /// The permit decrements both the per-address and the guard-wide live count
    /// when it is dropped, so it must be held for exactly as long as the
    /// connection lives, which is what moving it into the serving task achieves.
    ///
    /// Orthogonal to [`Self::check`]: `check` is the *rate* limit run before the
    /// handshake, `acquire` is the *concurrency* limit that bounds how many
    /// connections one address may hold open at once. A distributed flood is
    /// caught by the rate window; a slow-hold from a single source is caught
    /// here. A `conn_max` of 0 never refuses, only counts.
    pub fn acquire(&self, addr: &IpAddr) -> Outcome<Option<ConnPermit>> {
        let key = self.amap.key(&Self::ip_bytes(addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        let counter = {
            let mut unlocked_map = lock_write!(locked_map);
            match unlocked_map.get(&key).map(|log| log.conns.clone()) {
                Some(c) => c,
                None => {
                    let mut log = AddressLog::<N, D>::default();
                    log.ip = Some(*addr);
                    let c = log.conns.clone();
                    unlocked_map.insert(key, log);
                    c
                },
            }
        };
        // The increment is speculative and rolled back on refusal, so a rejected
        // connection leaves the count exactly where it was.
        let now = counter.fetch_add(1, Ordering::AcqRel) + 1;
        if self.conn_max != 0 && now > self.conn_max {
            counter.fetch_sub(1, Ordering::AcqRel);
            return Ok(None);
        }
        self.live_total.fetch_add(1, Ordering::AcqRel);
        Ok(Some(ConnPermit {
            per_ip: counter,
            total:  self.live_total.clone(),
        }))
    }

    /// Live connections across every address right now.
    pub fn live_conns(&self) -> usize {
        self.live_total.load(Ordering::Acquire)
    }

    /// Prove in-process that the guard's state machine is armed, without any
    /// external probe that a live guard would itself blacklist.
    ///
    /// Blacklists a documentation address (TEST-NET-1, `192.0.2.1`, RFC 5737),
    /// confirms it is then blocked, releases it, and confirms it is allowed
    /// again. A `true` result is what a health body reports as `guard_selftest`.
    /// Leaves one monitor entry for that reserved address behind, which is inert.
    pub fn self_test(&self) -> bool {
        let ip: IpAddr = Ipv4Addr::new(192, 0, 2, 1).into();
        if self.blacklist(&ip).is_err() {
            return false;
        }
        let blocked = matches!(self.check(&ip), Ok(d) if d.should_drop());
        let _ = self.unblock(&ip);
        let allowed = matches!(self.check(&ip), Ok(GuardDecision::Allow));
        blocked && allowed
    }

    /// `extra` runs while the shard write lock is still held, so a caller can
    /// compose its own check on top of the generic rate limit without a second
    /// acquisition. Its boolean argument says whether this call created the log.
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

    /// One Monitor -> Throttle -> Blacklist step, for an entry already present.
    fn evaluate(
        &self,
        log:    &mut AddressLog<N, D>,
        sunset: Duration,
    )
        -> GuardDecision
    {
        // Decay first: an address that has served the decay window with no fresh
        // offence has its throttle history forgiven and an auto-blacklist or
        // throttle relaxed to Monitor, so a one-off burst does not hold a shared
        // NAT address near the blacklist threshold for the life of the process.
        // A manual blacklist and a whitelist are operator decisions and never
        // decay. This runs on a live request, so it relaxes an alive-but-quiet
        // record, which the idle sweep (that evicts a *dead* record) does not.
        if !self.decay_after.is_zero() {
            let quiet_enough = log.offended_at
                .and_then(|t| t.elapsed().ok())
                .map(|since| since >= self.decay_after)
                .unwrap_or(false);
            if quiet_enough {
                let auto_black = matches!(log.state,
                    AddressState::Blacklist { reason: BlacklistReason::AutoRateLimit, .. });
                let throttled = matches!(log.state, AddressState::Throttle { .. });
                if auto_black || throttled {
                    log.state = AddressState::Monitor(RingTimer::default());
                }
                log.throttle_cnt = 0;
                log.offended_at = None;
            }
        }

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
                        log.offended_at = Some(SystemTime::now());
                        return GuardDecision::Blocked(BlacklistReason::AutoRateLimit);
                    }
                    log.state = AddressState::Throttle {
                        reqs:       RingTimer::default(),
                        tint_min:   self.tint_min,
                        start:      SystemTime::now(),
                        sunset,
                    };
                    log.throttle_cnt = next_cnt;
                    log.offended_at = Some(SystemTime::now());
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

    /// For a caller that wants to drive the map itself rather than go through
    /// `update_log`.
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

    /// Creates the log where the address has not been seen before.
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

    /// The reason recorded is `Manual`, and the log is created where the address
    /// has not been seen before.
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

    /// Back to `Monitor`, with the throttle count zeroed, so the address starts
    /// again from nothing.
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

    /// Reads without mutating, unlike `check`. `None` where the address has never
    /// been observed.
    pub fn peek(&self, addr: &IpAddr) -> Outcome<Option<&'static str>> {
        let key = self.amap.key(&Self::ip_bytes(addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        let unlocked_map = lock_read!(locked_map);
        Ok(unlocked_map.get(&key).map(|l| l.state.label()))
    }

    /// Walks every address, so the cost is linear in the number of them.
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

    /// Counts plus up to `max` per-address entries, in no particular order: a
    /// caller wanting a stable view sorts the entries itself.
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

    /// Evict idle `Monitor` records, returning how many were dropped.
    ///
    /// A distributed flood is a great many addresses each making one request
    /// below the rate floor: every one mints a `Monitor` log that is never
    /// throttled and so, without this, is never reclaimed -- the memory-
    /// exhaustion shape the rate window cannot itself close. This drops a record
    /// only when all three hold: it is in `Monitor` (a transient rate-watching
    /// state, not an operator `Whitelist` nor an active `Throttle`/`Blacklist`
    /// carrying a security decision that must outlive it until its own sunset);
    /// it has no live connection (`conns == 0`, so eviction can never reset a
    /// per-IP concurrency cap out from under a connection still holding a
    /// `ConnPermit`); and it has not been seen within `idle`. A re-created record
    /// starts clean, which for a `Monitor` address reaches the identical
    /// decision, so nothing is lost by forgetting it.
    pub fn sweep_idle(&self, idle: Duration) -> Outcome<usize> {
        let now = SystemTime::now();
        let mut evicted = 0usize;
        for i in 0..self.amap.n {
            if let Some(locked_map) = self.amap.shards[i].as_ref() {
                let mut unlocked = lock_write!(locked_map);
                unlocked.retain(|_k, log| {
                    let stale = now.duration_since(log.last_seen)
                        .map(|age| age > idle)
                        .unwrap_or(false);
                    let quiet = log.conns.load(Ordering::Acquire) == 0;
                    let transient = matches!(log.state, AddressState::Monitor(_));
                    let drop_it = stale && quiet && transient;
                    if drop_it {
                        evicted += 1;
                    }
                    !drop_it
                });
            }
        }
        Ok(evicted)
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
            conn_max:       0,
            live_total:     Arc::new(AtomicUsize::new(0)),
            decay_after:    Duration::ZERO,
        }
    }

    fn make_conn_guard(conn_max: usize) -> TestGuard {
        let mut g = make_guard(1_000_000, 1_000);
        g.conn_max = conn_max;
        g
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
    fn concurrency_admits_up_to_the_cap_and_refuses_beyond() {
        let guard = make_conn_guard(3);
        let addr: IpAddr = Ipv4Addr::new(10, 0, 1, 1).into();
        // Three admitted, held live.
        let p1 = guard.acquire(&addr).expect("acquire 1");
        let p2 = guard.acquire(&addr).expect("acquire 2");
        let p3 = guard.acquire(&addr).expect("acquire 3");
        assert!(p1.is_some() && p2.is_some() && p3.is_some(),
            "the first N connections must be admitted");
        assert_eq!(guard.live_conns(), 3);
        // The N+1th is refused while the first three are still held.
        let p4 = guard.acquire(&addr).expect("acquire 4");
        assert!(p4.is_none(), "the N+1th connection must be refused");
        assert_eq!(guard.live_conns(), 3, "a refusal must not count");
    }

    #[test]
    fn concurrency_releases_on_drop() {
        let guard = make_conn_guard(2);
        let addr: IpAddr = Ipv4Addr::new(10, 0, 1, 2).into();
        let p1 = guard.acquire(&addr).expect("acquire 1");
        {
            let p2 = guard.acquire(&addr).expect("acquire 2");
            assert!(p2.is_some());
            // At the cap with p1 and p2 held: a third is refused, and the refusal
            // does not count.
            assert!(guard.acquire(&addr).expect("acquire 3").is_none(),
                "at the cap, a third is refused");
            assert_eq!(guard.live_conns(), 2);
        }
        // p2 dropped at the end of the block: a slot is free again.
        assert_eq!(guard.live_conns(), 1, "dropping a permit must free its slot");
        let p4 = guard.acquire(&addr).expect("acquire 4");
        assert!(p4.is_some(), "a released slot must be reusable");
        // p1 and p4 held.
        assert_eq!(guard.live_conns(), 2);
        drop(p1);
        // Only p4 remains.
        assert_eq!(guard.live_conns(), 1);
    }

    #[test]
    fn concurrency_is_isolated_per_address() {
        let guard = make_conn_guard(1);
        let a: IpAddr = Ipv4Addr::new(10, 0, 1, 3).into();
        let b: IpAddr = Ipv4Addr::new(10, 0, 1, 4).into();
        let _pa = guard.acquire(&a).expect("acquire a").expect("a admitted");
        // a is at its cap, but b is unaffected.
        assert!(guard.acquire(&a).expect("acquire a2").is_none(),
            "a's second connection is refused");
        assert!(guard.acquire(&b).expect("acquire b").is_some(),
            "a second address must be unaffected by the first's cap");
    }

    #[test]
    fn a_zero_cap_never_refuses_but_still_counts() {
        let guard = make_conn_guard(0);
        let addr: IpAddr = Ipv4Addr::new(10, 0, 1, 5).into();
        let mut held = Vec::new();
        for _ in 0..100 {
            held.push(guard.acquire(&addr).expect("acquire").expect("admitted"));
        }
        assert_eq!(guard.live_conns(), 100);
        held.clear();
        assert_eq!(guard.live_conns(), 0);
    }

    #[test]
    fn self_test_reports_armed() {
        let guard = make_guard(100, 5);
        assert!(guard.self_test(), "a constructed guard must report armed");
    }

    #[test]
    fn throttle_history_decays_after_a_quiet_window() {
        // A high rate ceiling so the decayed record does not immediately re-offend
        // on the very check that relaxes it; the decay is what is under test.
        let mut guard = make_guard(1_000_000, 3);
        guard.decay_after = Duration::from_millis(20);
        let addr: IpAddr = Ipv4Addr::new(10, 0, 4, 1).into();

        // Plant a record auto-blacklisted well in the past, with no fresh offence:
        // an address that was heavy once and has since gone quiet.
        let old = SystemTime::now()
            .checked_sub(Duration::from_millis(200))
            .expect("time before now");
        guard.update_log(&addr, |log, _new| {
            log.state = AddressState::Blacklist {
                since:  old,
                reason: BlacklistReason::AutoRateLimit,
            };
            log.throttle_cnt = 3;
            log.offended_at = Some(old);
            Ok(())
        }).expect("plant blacklisted record");

        // The next check sees the quiet window has elapsed: the auto-blacklist is
        // relaxed and the throttle history forgiven, so the address is allowed.
        let decision = guard.check(&addr).expect("check");
        assert_eq!(decision, GuardDecision::Allow,
            "a quiet auto-blacklisted address must decay back to allowed");

        // A manual blacklist is an operator decision and must NOT decay.
        let banned: IpAddr = Ipv4Addr::new(10, 0, 4, 2).into();
        guard.update_log(&banned, |log, _new| {
            log.state = AddressState::Blacklist {
                since:  old,
                reason: BlacklistReason::Manual,
            };
            log.offended_at = Some(old);
            Ok(())
        }).expect("plant manual blacklist");
        assert!(matches!(guard.check(&banned), Ok(d) if d.should_drop()),
            "a manual blacklist must survive the decay window");
    }

    #[test]
    fn sweep_evicts_idle_monitor_records_but_spares_live_and_blacklisted() {
        let guard = make_conn_guard(4);
        let idle: IpAddr = Ipv4Addr::new(10, 0, 2, 1).into();
        let live: IpAddr = Ipv4Addr::new(10, 0, 2, 2).into();
        let banned: IpAddr = Ipv4Addr::new(10, 0, 2, 3).into();

        // An idle Monitor record: acquired then released, so conns == 0.
        drop(guard.acquire(&idle).expect("acquire idle"));
        // A live record still holding a connection.
        let _held = guard.acquire(&live).expect("acquire live").expect("admitted");
        // A blacklisted record, which carries a security decision.
        guard.blacklist(&banned).expect("blacklist");

        // Everything is younger than an hour, so an hour-idle sweep evicts nothing.
        assert_eq!(guard.sweep_idle(Duration::from_secs(3600)).expect("sweep"), 0,
            "a sweep must spare records seen within the idle window");

        // A zero idle window makes every past-seen record stale: the idle Monitor
        // one goes, the live one is spared (conns > 0), the blacklisted one is
        // spared (not Monitor).
        let evicted = guard.sweep_idle(Duration::ZERO).expect("sweep");
        assert_eq!(evicted, 1, "only the idle Monitor record must be evicted");
        // The live cap is intact: its slot was never reset by an eviction.
        assert_eq!(guard.live_conns(), 1);
        // The blacklist survived the sweep.
        assert!(matches!(guard.check(&banned), Ok(d) if d.should_drop()),
            "a blacklisted address must not be forgotten by an idle sweep");
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
