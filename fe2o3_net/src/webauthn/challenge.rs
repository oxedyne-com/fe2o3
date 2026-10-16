//! Stateless WebAuthn challenges.
//!
//! A challenge kept in a server-side map lets an unauthenticated flood of ceremony starts evict
//! another browser's challenge. Here nothing is kept at mint: the challenge is a random nonce,
//! and the browser carries it back in a short-lived pre-auth cookie together with its expiry, its
//! purpose (`create` or `get`) and an HMAC-SHA256 over all three under a per-process key. The only
//! server state is the used-set that refuses a second showing inside the window, a
//! `guard::nonce::NonceTracker`, bounded by its window and its cap. A restart makes a new key and
//! voids the open challenges, which at 120 s is acceptable.
//!
//! The used-set fails closed: once full it refuses every check until pairs lapse, so a site that
//! lets strangers check challenges should rate-limit that route by address.

use crate::{
    acme::jose::{
        base64url_decode,
        base64url_encode,
    },
    guard::nonce::NonceTracker,
    hmac::{
        hmac_sha256,
        verify_hmac_sha256,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};

use std::{
    sync::Mutex,
    time::Duration,
};

pub const TTL_SECS: u64 = 120;
pub const NONCE_LEN: usize = 32;

const DOMAIN: &[u8] = b"fe2o3 webauthn challenge v1";
const COOKIE_LEN: usize = 1 + 8 + NONCE_LEN + 32; // purpose, expiry, nonce, tag

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Purpose {
    Create, // navigator.credentials.create()
    Get,    // navigator.credentials.get()
}

impl Purpose {
    fn tag(self) -> u8 {
        match self {
            Self::Create    => b'c',
            Self::Get       => b'g',
        }
    }
}

/// Mints and checks the challenges of one process.
pub struct Challenges {
    key:    [u8; 32],
    used:   Mutex<NonceTracker>,
}

impl Challenges {
    /// A fresh random key, so a restart voids every open challenge.
    pub fn new() -> Self {
        let mut key = [0u8; 32];
        Rand::fill_u8(&mut key);
        Self::with_key(key)
    }

    pub fn with_key(key: [u8; 32]) -> Self {
        Self {
            key,
            used: Mutex::new(NonceTracker::new(Duration::from_secs(TTL_SECS))),
        }
    }

    /// Caps the used-set, at least one; `NonceTracker::MAX_PAIRS` by default.
    pub fn with_max_used(self, max: usize) -> Self {
        let used = match self.used.into_inner() {
            Ok(t)   => t,
            Err(p)  => p.into_inner(),
        };
        Self {
            key:    self.key,
            used:   Mutex::new(used.with_max_pairs(max)),
        }
    }

    /// A new challenge for `purpose`, valid until `now + TTL_SECS`: the raw challenge bytes for
    /// the browser's `publicKey.challenge`, and the value of the cookie that carries it back.
    /// Nothing is recorded.
    pub fn mint(&self, purpose: Purpose, now: u64) -> ([u8; NONCE_LEN], String) {
        let mut nonce = [0u8; NONCE_LEN];
        Rand::fill_u8(&mut nonce);
        let exp = now.saturating_add(TTL_SECS);
        let mut b = Vec::with_capacity(COOKIE_LEN);
        b.push(purpose.tag());
        b.extend_from_slice(&exp.to_be_bytes());
        b.extend_from_slice(&nonce);
        let tag = hmac_sha256(&self.key, &signed(&b));
        b.extend_from_slice(&tag);
        (nonce, base64url_encode(&b))
    }

    /// Accepts `client_challenge` once: the cookie must carry this process's tag, be unexpired at
    /// `now`, be for `purpose`, and carry exactly this challenge, and its nonce must not have been
    /// accepted before.
    pub fn check(
        &self,
        cookie:             &str,
        client_challenge:   &[u8],
        purpose:            Purpose,
        now:                u64,
    )
        -> Outcome<()>
    {
        let b = res!(base64url_decode(cookie));
        if b.len() != COOKIE_LEN {
            return Err(err!(
                "A WebAuthn challenge cookie must decode to {} bytes, this one is {}.",
                COOKIE_LEN, b.len();
                Invalid, Input, Size));
        }
        let (body, tag) = b.split_at(COOKIE_LEN - 32);
        if !verify_hmac_sha256(&self.key, &signed(body), tag) {
            return Err(err!(
                "A WebAuthn challenge cookie's tag does not verify under this process's key: \
                tampered, or minted before a restart.";
                Invalid, Security, Mismatch));
        }
        if body[0] != purpose.tag() {
            return Err(err!(
                "A WebAuthn challenge minted for one ceremony was presented for {:?}.", purpose;
                Invalid, Security, Mismatch));
        }
        let mut eb = [0u8; 8];
        eb.copy_from_slice(&body[1..9]);
        let exp = u64::from_be_bytes(eb);
        if now >= exp {
            return Err(err!(
                "A WebAuthn challenge expired at {}, and it is now {}.", exp, now;
                Invalid, Security, Timeout));
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&body[9..]);
        if client_challenge != &nonce[..] {
            return Err(err!(
                "The challenge the browser signed is not the one its cookie carries.";
                Invalid, Security, Mismatch));
        }
        let mut used = lock_mutex!(self.used);
        // Held until the later of now and the mint time, plus the window: at least to `exp`.
        res!(used.record(&[body[0]], &nonce, exp.saturating_sub(TTL_SECS), now));
        Ok(())
    }

    /// How many accepted nonces the used-set holds.
    pub fn used_len(&self) -> Outcome<usize> {
        Ok(lock_mutex!(self.used).len())
    }
}

impl Default for Challenges {
    fn default() -> Self {
        Self::new()
    }
}

fn signed(body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(DOMAIN.len() + body.len());
    m.extend_from_slice(DOMAIN);
    m.extend_from_slice(body);
    m
}
