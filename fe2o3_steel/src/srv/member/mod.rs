//! What a site keeps for its signed-in members, as opposed to its operators (`srv::admin`).
//!
//! `session` and `invite` share one secret form: 32 random bytes the member holds, of which the
//! store keeps only the SHA3-256 hash, so a read of the store yields nothing that can be presented.

pub mod invite;
pub mod session;

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_text::base2x;

use std::{
    fmt,
    sync::{
        atomic::{
            AtomicU64,
            Ordering,
        },
        Arc,
    },
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

pub const TOKEN_LEN: usize = 32; // bytes

type Hash32 = [u8; TOKEN_LEN];

/// The secret a member holds, in a cookie for a session or in a link for an invite. It never
/// prints: neither `Debug` nor any formatting shows its bytes, and only `to_hex` gives them out.
pub struct Token(Hash32);

impl Token {
    fn generate() -> Self {
        let mut b = [0u8; TOKEN_LEN];
        Rand::fill_u8(&mut b);
        Self(b)
    }

    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }

    pub fn from_hex(s: &str) -> Outcome<Self> {
        if s.len() != TOKEN_LEN * 2 {
            return Err(err!(
                "A member token must be {} hex characters, this one has {}.",
                TOKEN_LEN * 2, s.len();
                Invalid, Input));
        }
        let v = res!(base2x::HEX.from_str(&s.to_ascii_uppercase())); // either case
        let b: Hash32 = res!(v.try_into().map_err(|_| err!(
            "A member token did not decode to {} bytes.", TOKEN_LEN;
            Invalid, Input)));
        Ok(Self(b))
    }

    fn hash(&self) -> Outcome<Hash32> {
        let h = HashScheme::new_sha3_256().hash(&[&self.0], []);
        let v = h.as_hashform().as_vec();
        let b: Hash32 = res!(v.try_into().map_err(|_| err!(
            "A SHA3-256 hash was not {} bytes.", TOKEN_LEN;
            Unexpected, Size)));
        Ok(b)
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token(..)")
    }
}

/// Where a member store reads the time. A test steps a `Manual` clock to cross an expiry.
#[derive(Clone, Debug)]
pub enum Clock {
    System,
    Manual(Arc<AtomicU64>), // Unix seconds
}

impl Clock {
    pub fn now(&self) -> Outcome<u64> {
        match self {
            Self::System    => Ok(res!(SystemTime::now().duration_since(UNIX_EPOCH),
                System, Invalid).as_secs()),
            Self::Manual(t) => Ok(t.load(Ordering::SeqCst)),
        }
    }
}

fn hex(b: &[u8]) -> String {
    base2x::HEX.to_string(b)
}

fn hash_of_key(s: &str) -> Option<Hash32> {
    if s.len() != TOKEN_LEN * 2 {
        return None;
    }
    match base2x::HEX.from_str(&s.to_ascii_uppercase()) {
        Ok(v)   => v.try_into().ok(),
        Err(_)  => None,
    }
}

fn short(hash: &Hash32) -> String {
    let mut s = hex(hash);
    s.truncate(8);
    s
}
