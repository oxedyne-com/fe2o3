//! Length padding, so that a ciphertext's size says little about its plaintext's.
//!
//! [`padme_len`] is Padmé, Algorithm 1 of Nikitin et al., "Reducing Metadata Leakage from
//! Encrypted Files and Communication with PURBs" (PoPETs 2019, arXiv 1806.03160, §4.4). A padded
//! length keeps no more significant bits in its mantissa than its exponent holds, so the lengths
//! up to M fall into O(log log M) buckets, and the overhead is below 2^(E−S)/L ≈ 1/(2 log₂ L):
//! at most +11.1% (9 bytes to 10), and falling as L grows.
//!
//! [`PadRule`] frames a body as `u32 big-endian length ‖ body ‖ zeros` and pads the whole frame
//! by the rule a [`PadConfig`] names, never below its floor; [`unpad`] needs no rule, so a
//! reader is not tied to the writer's configuration. Pad before sealing and unpad after
//! opening: the padding hides a length only inside an authenticated cipher.
//!
//! The JS twin is `fe2o3_crypto/js/pad.mjs`; both check `tests/data/padme_vectors.txt`, made
//! by an oracle written from the paper alone (`tests/data/gen_padme_vectors.py`).

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
};

/// Bytes of the length prefix. // fixed: the frame format, a u32 big-endian body length
pub const PREFIX_LEN: usize = 4;

/// Padmé's padded length for `len` bytes. Below 2, where ⌊log₂ L⌋ is undefined, the length
/// stands, as the paper's buckets begin at 2.
pub fn padme_len(len: u64) -> Outcome<u64> {
    if len < 2 {
        return Ok(len);
    }
    let e = 63 - len.leading_zeros() as u64;    // ⌊log₂ L⌋, the exponent
    let s = 64 - e.leading_zeros() as u64;      // ⌊log₂ E⌋ + 1, the bits that hold E
    let z = e - s;                              // low bits forced to zero
    let m = (1u64 << z) - 1;
    match len.checked_add(m) {
        Some(v) => Ok(v & !m),
        None    => Err(err!("Padmé cannot pad a length of {} within 64 bits.", len;
            Overflow, Size)),
    }
}

/// How a frame's length is rounded up, each never below `floor` bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PadRule {
    Padme       { floor: u64 },                 // Padmé buckets, sizes visible to ~12%
    PowerOfTwo  { floor: u64 },                 // next power of two, up to +100%
    Block       { floor: u64, block: u64 },     // whole blocks, e.g. constant 1 MiB frames
}

impl PadRule {

    /// The rounded length of a frame of `len` bytes, the prefix included.
    pub fn round(&self, len: u64) -> Outcome<u64> {
        let (got, floor) = match *self {
            Self::Padme { floor } => (res!(padme_len(len)), floor),
            Self::PowerOfTwo { floor } => {
                let p = if len <= 1 { 1 } else {
                    match len.checked_next_power_of_two() {
                        Some(p) => p,
                        None    => return Err(err!(
                            "No power of two above {} fits in 64 bits.", len; Overflow, Size)),
                    }
                };
                (p, floor)
            }
            Self::Block { floor, block } => {
                if block == 0 {
                    return Err(err!("A block pad rule needs a block of at least one byte.";
                        Invalid, Input, Configuration));
                }
                let n = len / block + if len % block == 0 { 0 } else { 1 };
                match n.checked_mul(block) {
                    Some(v) => (v, floor),
                    None    => return Err(err!(
                        "{} bytes in blocks of {} overflow 64 bits.", len, block; Overflow, Size)),
                }
            }
        };
        Ok(got.max(floor))
    }

    /// The length of the frame [`Self::pad`] makes for a body of `body_len` bytes.
    pub fn framed_len(&self, body_len: u64) -> Outcome<u64> {
        if body_len > u32::MAX as u64 {
            return Err(err!("A body of {} bytes is over the frame's u32 length prefix.", body_len;
                TooBig, Size));
        }
        self.round(PREFIX_LEN as u64 + body_len)
    }

    /// `body` framed and padded: its length as a big-endian u32, the body, then zeros.
    pub fn pad(&self, body: &[u8]) -> Outcome<Vec<u8>> {
        let total = res!(self.framed_len(body.len() as u64));
        let total = match usize::try_from(total) {
            Ok(t)  => t,
            Err(_) => return Err(err!("A padded frame of {} bytes does not fit in memory here.",
                total; TooBig, Size)),
        };
        let mut v = Vec::with_capacity(total);
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(body);
        v.resize(total, 0);
        Ok(v)
    }
}

/// The body inside a frame [`PadRule::pad`] made. Refused when the frame is shorter than its
/// prefix, the prefix claims more than the frame holds, or the fill is not all zeros.
pub fn unpad(frame: &[u8]) -> Outcome<&[u8]> {
    if frame.len() < PREFIX_LEN {
        return Err(err!("A padded frame of {} bytes is shorter than its {}-byte length prefix.",
            frame.len(), PREFIX_LEN; Invalid, Input, Size));
    }
    let n = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    let end = match PREFIX_LEN.checked_add(n) {
        Some(e) if e <= frame.len() => e,
        _ => return Err(err!("A padded frame of {} bytes claims a body of {} bytes.",
            frame.len(), n; Invalid, Input, Size)),
    };
    if frame[end..].iter().any(|b| *b != 0) {
        return Err(err!("A padded frame's fill after its {}-byte body is not all zeros.", n;
            Invalid, Input));
    }
    Ok(&frame[PREFIX_LEN..end])
}

/// The padding an app's configuration names: `rule` is `padme`, `pow2` or `block`; `floor` is
/// the least frame in bytes; `block` is the block size, read only by `block`.
#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct PadConfig {
    pub rule:   String,
    pub floor:  u64,
    pub block:  u64,
}

impl Default for PadConfig {
    fn default() -> Self {
        Self {
            rule:   "padme".to_string(),
            floor:  256,
            block:  1_048_576, // 1 MiB
        }
    }
}

impl Config for PadConfig {
    fn check_and_fix(&mut self) -> Outcome<()> {
        let _ = res!(self.rule());
        Ok(())
    }
}

impl PadConfig {

    /// The rule this configuration names, refused when the name is unknown or a block is empty.
    pub fn rule(&self) -> Outcome<PadRule> {
        let floor = self.floor;
        match self.rule.as_str() {
            "padme" => Ok(PadRule::Padme { floor }),
            "pow2"  => Ok(PadRule::PowerOfTwo { floor }),
            "block" => {
                if self.block == 0 {
                    return Err(err!("The pad rule 'block' needs 'block' of at least one byte.";
                        Invalid, Input, Configuration));
                }
                Ok(PadRule::Block { floor, block: self.block })
            }
            other => Err(err!("The pad rule {:?} is not one of padme, pow2 or block.", other;
                Invalid, Input, Configuration)),
        }
    }
}
