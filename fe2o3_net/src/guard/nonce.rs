//! Moved here from `fe2o3_steel`'s signed admin login on 2026-09-23, so that
//! any protocol whose signers choose their own nonces refuses a replay the same
//! way. Each pair is forgotten a window after it was first shown, by the
//! caller's clock. A verifier that issues its nonces itself, as
//! `presentation::verify` does, marks the issued challenge spent instead: that
//! mark lasts exactly as long as the challenge, where a window opened at the
//! showing can close while the challenge still stands if the clock steps back
//! between the issue and the showing.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    time::Duration,
};


/// Refuses a second showing of the same `(scope, nonce)` pair inside `window`.
///
/// Each pair is remembered with the time it was first shown, and entries older
/// than the window are evicted lazily on each record, so no background thread
/// is needed. A scope is whatever the caller keys replays by: a signer's id, an
/// audience, or nothing at all. Sized for challenge-response rates, not as a
/// general-purpose rate limiter.
#[derive(Debug)]
pub struct NonceTracker {
    seen:   HashMap<(Vec<u8>, [u8; 32]), u64>,  // pair -> unix seconds first shown
    window: Duration,
}

impl NonceTracker {

    pub fn new(window: Duration) -> Self {
        Self {
            seen:   HashMap::new(),
            window,
        }
    }

    /// Records the pair at `now` (unix seconds), or refuses it if it was
    /// already shown inside the window.
    pub fn record(
        &mut self,
        scope:  &[u8],
        nonce:  &[u8; 32],
        now:    u64,
    )
        -> Outcome<()>
    {
        self.evict_expired(now);
        let key = (scope.to_vec(), *nonce);
        if self.seen.contains_key(&key) {
            return Err(err!(
                "Nonce already seen in this scope inside the {} s replay window.",
                self.window.as_secs();
                Invalid, Security, Duplicate));
        }
        self.seen.insert(key, now);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    fn evict_expired(&mut self, now: u64) {
        let window_secs = self.window.as_secs();
        self.seen.retain(|_, ts| now.saturating_sub(*ts) <= window_secs);
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_tracker_accepts_distinct_and_rejects_repeat() -> Outcome<()> {
        let mut t = NonceTracker::new(Duration::from_secs(60));
        let scope = b"alice".to_vec();
        let n1 = [0x11u8; 32];
        let n2 = [0x22u8; 32];
        res!(t.record(&scope, &n1, 1000));
        res!(t.record(&scope, &n2, 1000));
        assert!(t.record(&scope, &n1, 1000).is_err(),
            "re-presenting the same nonce inside the window must fail");
        Ok(())
    }

    #[test]
    fn nonce_tracker_evicts_after_window() -> Outcome<()> {
        let mut t = NonceTracker::new(Duration::from_secs(60));
        let scope = b"alice".to_vec();
        let n = [0x33u8; 32];
        res!(t.record(&scope, &n, 1000));
        // Same scope and nonce, but 61 seconds later: eviction runs on the
        // insert and the record succeeds.
        res!(t.record(&scope, &n, 1061));
        Ok(())
    }

    #[test]
    fn nonce_tracker_keeps_scopes_apart() -> Outcome<()> {
        let mut t = NonceTracker::new(Duration::from_secs(60));
        let a = b"alice".to_vec();
        let b = b"bob".to_vec();
        let n = [0x44u8; 32];
        res!(t.record(&a, &n, 1000));
        // Different scope, same nonce: allowed.
        res!(t.record(&b, &n, 1000));
        Ok(())
    }
}
