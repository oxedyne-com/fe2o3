//! The hybrid logical clock: causal order across skewed nodes, the guard against a peer far ahead,
//! monotonicity under a wall that steps back, the counter's wall bump, and an encoding whose byte
//! order is the stamps' order.

use oxedyne_fe2o3_core::{
    prelude::*,
    time::hlc::{
        DELTA_MS_DEFAULT,
        Hlc,
        HlcClock,
        WallClock,
    },
};

use std::sync::{
    Arc,
    atomic::{
        AtomicU64,
        Ordering,
    },
};


const T0:	u64 = 1_760_659_200_000;	// 17 Oct 2025, ms
const MIN:	i64 = 60_000;
const DAY:	u64 = 86_400_000;

// A node's view of a shared true time, off by `skew` ms.
#[derive(Debug)]
struct Skewed {
    t:		Arc<AtomicU64>,
    skew:	i64,
}

impl WallClock for Skewed {
    fn wall_ms(&self) -> Outcome<u64> {
        Ok((self.t.load(Ordering::SeqCst) as i64 + self.skew) as u64)
    }
}

// A wall the test sets directly, free to step back.
#[derive(Debug, Default)]
struct Manual(AtomicU64);

impl Manual {
    fn set(&self, ms: u64) { self.0.store(ms, Ordering::SeqCst) }
}

impl WallClock for Manual {
    fn wall_ms(&self) -> Outcome<u64> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

// Deterministic, so a failure repeats.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

// Messages pass at random between the nodes while true time advances.  Every receipt must stamp
// above what was sent, every stamp above the node's previous one, and no receipt may fault.
fn chain(clocks: &mut [HlcClock<Skewed>], t: &Arc<AtomicU64>, steps: usize, seed: u64) -> Outcome<()> {
    let mut rng = Lcg(seed);
    let mut last: Vec<Hlc> = clocks.iter().map(|c| c.last()).collect();
    for i in 0..steps {
        t.fetch_add(rng.next(40), Ordering::SeqCst);
        let n = clocks.len() as u64;
        let a = rng.next(n) as usize;
        let b = (a + 1 + rng.next(n - 1) as usize) % clocks.len();
        let sent = res!(clocks[a].now());
        if sent <= last[a] {
            return Err(err!("Step {}: node {} stamped {} after {}.", i, a, sent, last[a]; Test, Order));
        }
        last[a] = sent;
        let got = res!(clocks[b].recv(&sent));
        if let Some(f) = got.fault {
            return Err(err!("Step {}: node {} faulted on {}: {}.", i, b, sent, f; Test, Unexpected));
        }
        if got.stamp <= sent || got.stamp <= last[b] {
            return Err(err!(
                "Step {}: node {} received {} and stamped {} (its previous {}).",
                i, b, sent, got.stamp, last[b];
            Test, Order));
        }
        last[b] = got.stamp;
        // A local event after the receipt follows it too.
        let after = res!(clocks[b].now());
        if after <= got.stamp {
            return Err(err!("Step {}: node {} stamped {} after receiving into {}.", i, b, after, got.stamp;
                Test, Order));
        }
        last[b] = after;
    }
    Ok(())
}

fn nodes(t: &Arc<AtomicU64>, skews: &[i64]) -> Vec<HlcClock<Skewed>> {
    skews.iter().enumerate().map(|(i, s)| {
        HlcClock::new(Skewed { t: t.clone(), skew: *s }, 0xa0 + i as u64)
    }).collect()
}

#[test]
fn skewed_nodes_order_causally_once_offset_corrected() -> Outcome<()> {
    let t = Arc::new(AtomicU64::new(T0));
    let skews = [5 * MIN, -5 * MIN, 0, 3 * MIN];
    let mut clocks = nodes(&t, &skews);
    // Offsets measured against the reference, each off by up to a few hundred ms.
    let errs = [250, -300, 0, 120];
    for (c, (s, e)) in clocks.iter_mut().zip(skews.iter().zip(errs.iter())) {
        c.set_offset_ms(-*s + *e);
    }
    res!(chain(&mut clocks, &t, 5_000, 7));
    Ok(())
}

#[test]
fn nodes_within_delta_order_causally_with_no_offset() -> Outcome<()> {
    let t = Arc::new(AtomicU64::new(T0));
    let mut clocks = nodes(&t, &[25_000, -25_000, 0]);
    res!(chain(&mut clocks, &t, 5_000, 11));
    Ok(())
}

#[test]
fn uncorrected_five_minute_skew_is_a_fault() -> Outcome<()> {
    let t = Arc::new(AtomicU64::new(T0));
    let mut clocks = nodes(&t, &[5 * MIN, -5 * MIN]);
    let sent = res!(clocks[0].now());
    let got = res!(clocks[1].recv(&sent));
    match got.fault {
        Some(f) if f.ahead_ms == 10 * MIN as u64 => Ok(()),
        other => Err(err!("A peer 10 min ahead answered fault {:?}.", other; Test, Mismatch)),
    }
}

#[test]
fn a_peer_two_days_ahead_is_ignored_and_reported() -> Outcome<()> {
    let wall = Arc::new(Manual::default());
    wall.set(T0);
    let mut c = HlcClock::new(wall.clone(), 1);
    let before = res!(c.now());
    let remote = Hlc::new(T0 + 2 * DAY, 9, 2);
    let got = res!(c.recv(&remote));
    let f = match got.fault {
        Some(f) => f,
        None    => return Err(err!("A stamp two days ahead was merged into {}.", got.stamp; Test, Missing)),
    };
    if f.remote != remote || f.local_ms != T0 || f.ahead_ms != 2 * DAY || f.delta_ms != DELTA_MS_DEFAULT {
        return Err(err!("The fault reads {:?}.", f; Test, Mismatch));
    }
    // Only the counter stepped.
    if got.stamp != Hlc::new(T0, before.ctr + 1, 1) {
        return Err(err!("The receipt stamped {} after {}.", got.stamp, before; Test, Mismatch));
    }
    for _ in 0..100 {
        let s = res!(c.now());
        if s.wall_ms > T0 + DELTA_MS_DEFAULT {
            return Err(err!("now() reached {} past local + delta.", s; Test, Excessive));
        }
    }
    // A peer exactly delta ahead is believed.
    let near = Hlc::new(T0 + DELTA_MS_DEFAULT, 0, 3);
    let got = res!(c.recv(&near));
    if got.fault.is_some() || got.stamp <= near {
        return Err(err!("A peer at the bound gave {:?}.", got; Test, Mismatch));
    }
    Ok(())
}

#[test]
fn delta_is_configurable() -> Outcome<()> {
    let wall = Manual::default();
    wall.set(T0);
    let mut c = HlcClock::new(&wall, 1).with_delta_ms(1_000);
    let got = res!(c.recv(&Hlc::new(T0 + 1_001, 0, 2)));
    if got.fault.is_none() {
        return Err(err!("1001 ms ahead under a 1000 ms bound was merged."; Test, Missing));
    }
    let got = res!(c.recv(&Hlc::new(T0 + 1_000, 0, 2)));
    if got.fault.is_some() {
        return Err(err!("1000 ms ahead under a 1000 ms bound faulted."; Test, Unexpected));
    }
    Ok(())
}

#[test]
fn monotone_under_a_wall_that_steps_back() -> Outcome<()> {
    let wall = Manual::default();
    wall.set(T0);
    let mut c = HlcClock::new(&wall, 1);
    let mut prev = res!(c.now());
    let walls = [T0 + 50, T0 - 10_000, T0 - 10_000, T0 + 20, T0 + 51, T0 + 100];
    for w in walls {
        wall.set(w);
        let s = res!(c.now());
        if s <= prev {
            return Err(err!("At wall {} the clock stamped {} after {}.", w, s, prev; Test, Order));
        }
        prev = s;
    }
    // A re-measured offset that moves the corrected wall back a minute changes nothing either.
    c.set_offset_ms(-MIN);
    for _ in 0..10 {
        let s = res!(c.now());
        if s <= prev || s.wall_ms != T0 + 100 {
            return Err(err!("After the offset stepped back the clock stamped {} after {}.", s, prev;
                Test, Order));
        }
        prev = s;
    }
    Ok(())
}

#[test]
fn a_spent_counter_bumps_the_wall() -> Outcome<()> {
    let wall = Manual::default();
    wall.set(T0);
    let mut c = HlcClock::new(&wall, 1).with_last(Hlc::new(T0, u32::MAX - 1, 0));
    let a = res!(c.now());
    let b = res!(c.now());
    if a != Hlc::new(T0, u32::MAX, 1) || b != Hlc::new(T0 + 1, 0, 1) {
        return Err(err!("The counter's end gave {} then {}.", a, b; Test, Mismatch));
    }
    // A merge at the counter's end bumps the same way.
    let got = res!(c.recv(&Hlc::new(T0 + 1, u32::MAX, 2)));
    if got.stamp != Hlc::new(T0 + 2, 0, 1) {
        return Err(err!("A merge at the counter's end gave {}.", got.stamp; Test, Mismatch));
    }
    // Only a wall with nowhere to go is an error.
    let mut c = HlcClock::new(&wall, 1).with_last(Hlc::new(u64::MAX, u32::MAX, 0));
    match c.now() {
        Err(_)  => Ok(()),
        Ok(s)   => Err(err!("A spent wall and counter stamped {}.", s; Test, Unexpected)),
    }
}

#[test]
fn encoding_order_is_stamp_order() -> Outcome<()> {
    let mut rng = Lcg(3);
    let pick = |r: &mut Lcg| -> Hlc {
        // Few distinct values per field, so ties at each level are common.
        Hlc::new(T0 + r.next(4), r.next(3) as u32 * 0x0100_0001, r.next(3) * 0x0100_0000_0000_0001)
    };
    for _ in 0..5_000 {
        let a = pick(&mut rng);
        let b = pick(&mut rng);
        if a.to_bytes().cmp(&b.to_bytes()) != a.cmp(&b) {
            return Err(err!("{} and {} order differently as bytes.", a, b; Test, Order));
        }
        if res!(Hlc::from_bytes(&a.to_bytes())) != a {
            return Err(err!("{} does not survive its encoding.", a; Test, Mismatch));
        }
        let shown = a.to_string();
        if res!(shown.parse::<Hlc>()) != a {
            return Err(err!("{} does not parse back from '{}'.", a, shown; Test, Mismatch));
        }
    }
    if Hlc::from_bytes(&[0u8; 19]).is_ok() || "12.3".parse::<Hlc>().is_ok() || "x.1@0".parse::<Hlc>().is_ok() {
        return Err(err!("A malformed stamp was accepted."; Test, Unexpected));
    }
    if Hlc::new(1, 2, 0xa1).to_string() != "1.2@00000000000000a1" {
        return Err(err!("The shown form is {}.", Hlc::new(1, 2, 0xa1); Test, Mismatch));
    }
    Ok(())
}
