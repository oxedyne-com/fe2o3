//! Moved here from `fe2o3_steel`'s signed admin login on 2026-09-23, so that
//! any protocol whose signers choose their own nonces refuses a replay the same
//! way. A verifier that issues its nonces itself, as `presentation::verify`
//! does, marks the issued challenge spent instead, which needs no clock.
//!
//! 2026-09-23, from the presentation audit: a pair was held for the window
//! after its first showing, while a freshness check of the same window accepts
//! a command stamped up to the window ahead, so a command stamped 110 s ahead
//! replayed 121 s after it was first shown. A pair is now held until the window
//! after the later of its stamp and its first showing.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::{
        BTreeSet,
        HashMap,
    },
    time::Duration,
};


type Pair = (Vec<u8>, [u8; 32]); // (scope, nonce)

/// Refuses a second showing of the same `(scope, nonce)` pair while a freshness
/// check with the same window could still accept the command that carries it.
///
/// A scope is whatever the caller keys replays by, such as a signer's id. Each
/// pair is held until `max(stamp, now) + window`, where `stamp` is the time the
/// signer put on the command and `now` the time it was first shown, both unix
/// seconds: a check that accepts stamps within the window either side of now
/// accepts this command until `stamp + window`, and no later. Pairs are let go
/// in the order they lapse as later pairs arrive, so no thread of its own is
/// needed.
///
/// Two limits make it fail closed. Once the caller's clock has read later than
/// a pair's `stamp + window`, the tracker cannot say whether it has already let
/// that pair go, so after a clock steps back it refuses such a pair until the
/// clock catches up. And it holds at most `max_pairs` pairs, refusing the next
/// until some lapse.
#[derive(Debug)]
pub struct NonceTracker {
    held:       HashMap<Pair, u64>,     // pair -> unix second it is held until
    by_lapse:   BTreeSet<(u64, Pair)>,  // the same pairs, soonest to lapse first
    window:     Duration,
    max_pairs:  usize,
    latest:     u64,                    // the latest `now` the caller has shown
}

impl NonceTracker {

    pub const MAX_PAIRS: usize = 1 << 16;

    pub fn new(window: Duration) -> Self {
        Self {
            held:       HashMap::new(),
            by_lapse:   BTreeSet::new(),
            window,
            max_pairs:  Self::MAX_PAIRS,
            latest:     0,
        }
    }

    /// Caps the pairs held at once, at least one.
    pub fn with_max_pairs(mut self, max_pairs: usize) -> Self {
        self.max_pairs = max_pairs.max(1);
        self
    }

    /// Records the pair, or refuses it if it is still held, if the tracker can
    /// no longer vouch for its stamp, or if the tracker is full.
    ///
    /// # Arguments
    ///
    /// * `stamp` - the unix second the signer put on the command.
    /// * `now` - the caller's clock, the one its freshness check read.
    pub fn record(
        &mut self,
        scope:  &[u8],
        nonce:  &[u8; 32],
        stamp:  u64,
        now:    u64,
    )
        -> Outcome<()>
    {
        let window = self.window.as_secs();
        self.latest = self.latest.max(now);
        self.let_go(self.latest);
        let pair = (scope.to_vec(), *nonce);
        if self.held.contains_key(&pair) {
            return Err(err!(
                "Nonce already seen in this scope while a command carrying it could still \
                pass a {} s freshness check.", window;
                Invalid, Security, Duplicate));
        }
        // A pair with this stamp could have been held and let go already, if
        // the clock has read past its window before stepping back.
        if stamp.saturating_add(window) < self.latest {
            return Err(err!(
                "A command stamped {} is refused, since the clock has read {}, past the {} s \
                window in which the tracker vouches for a stamp.", stamp, self.latest, window;
                Invalid, Security, Order));
        }
        if self.held.len() >= self.max_pairs {
            return Err(err!(
                "The nonce tracker holds {} pairs, its most, so no more are recorded until \
                some lapse.", self.held.len();
                Excessive, Size));
        }
        let until = stamp.max(now).saturating_add(window);
        self.by_lapse.insert((until, pair.clone()));
        self.held.insert(pair, until);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Lets go of every pair held until before `now`, soonest first.
    fn let_go(&mut self, now: u64) {
        while let Some((until, _)) = self.by_lapse.first() {
            if *until >= now {
                break;
            }
            if let Some((_, pair)) = self.by_lapse.pop_first() {
                self.held.remove(&pair);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> NonceTracker {
        NonceTracker::new(Duration::from_secs(60))
    }

    #[test]
    fn nonce_tracker_accepts_distinct_and_rejects_repeat() -> Outcome<()> {
        let mut t = tracker();
        let scope = b"alice".to_vec();
        let n1 = [0x11u8; 32];
        let n2 = [0x22u8; 32];
        res!(t.record(&scope, &n1, 1000, 1000));
        res!(t.record(&scope, &n2, 1000, 1000));
        assert!(t.record(&scope, &n1, 1000, 1000).is_err(),
            "re-presenting the same nonce inside the window must fail");
        Ok(())
    }

    #[test]
    fn nonce_tracker_evicts_after_window() -> Outcome<()> {
        let mut t = tracker();
        let scope = b"alice".to_vec();
        let n = [0x33u8; 32];
        res!(t.record(&scope, &n, 1000, 1000));
        // Same scope and nonce, freshly stamped 61 seconds later: the first
        // record has lapsed, and this one is recorded.
        res!(t.record(&scope, &n, 1061, 1061));
        Ok(())
    }

    #[test]
    fn nonce_tracker_keeps_scopes_apart() -> Outcome<()> {
        let mut t = tracker();
        let a = b"alice".to_vec();
        let b = b"bob".to_vec();
        let n = [0x44u8; 32];
        res!(t.record(&a, &n, 1000, 1000));
        // Different scope, same nonce: allowed.
        res!(t.record(&b, &n, 1000, 1000));
        Ok(())
    }

    /// A command stamped ahead of the clock is fresh until its stamp plus the
    /// window, so its pair is held that long, not the window from its first
    /// showing.
    #[test]
    fn nonce_tracker_holds_a_future_stamp_to_its_own_window() -> Outcome<()> {
        let mut t = tracker();
        let n = [0x55u8; 32];
        res!(t.record(b"ops", &n, 1055, 1000));
        assert!(t.record(b"ops", &n, 1055, 1061).is_err(), "held past the window from its showing");
        assert!(t.record(b"ops", &n, 1055, 1115).is_err(), "held to its stamp plus the window");
        res!(t.record(b"ops", &n, 1116, 1116));
        Ok(())
    }

    /// A command stamped behind the clock is held for the window from its first
    /// showing, the later of the two.
    #[test]
    fn nonce_tracker_holds_a_past_stamp_from_its_showing() -> Outcome<()> {
        let mut t = tracker();
        let n = [0x66u8; 32];
        res!(t.record(b"ops", &n, 950, 1000));
        assert!(t.record(b"ops", &n, 950, 1060).is_err(), "held for the window from its showing");
        Ok(())
    }

    /// Once the clock has read past a stamp's window, a pair with that stamp is
    /// refused even after the clock steps back, since it may have been let go.
    #[test]
    fn nonce_tracker_refuses_what_it_can_no_longer_vouch_for() -> Outcome<()> {
        let mut t = tracker();
        let n = [0x77u8; 32];
        res!(t.record(b"ops", &n, 1000, 1000));
        res!(t.record(b"ops", &[0x78u8; 32], 2000, 2000)); // clock runs ahead; the first lapses
        assert!(t.record(b"ops", &n, 1000, 1010).is_err(), "replayed after the clock stepped back");
        assert!(t.record(b"ops", &[0x79u8; 32], 1010, 1010).is_err(),
            "a new stamp the tracker cannot vouch for, until the clock catches up");
        res!(t.record(b"ops", &[0x79u8; 32], 1950, 1950));
        Ok(())
    }

    /// At its cap the tracker refuses rather than let go of a pair still held,
    /// and records again once pairs lapse.
    #[test]
    fn nonce_tracker_refuses_at_its_cap() -> Outcome<()> {
        let mut t = tracker().with_max_pairs(2);
        res!(t.record(b"ops", &[1u8; 32], 1000, 1000));
        res!(t.record(b"ops", &[2u8; 32], 1000, 1000));
        assert!(t.record(b"ops", &[3u8; 32], 1000, 1000).is_err(), "full");
        req!(t.len(), 2);
        res!(t.record(b"ops", &[3u8; 32], 1061, 1061));
        req!(t.len(), 1);
        Ok(())
    }
}
