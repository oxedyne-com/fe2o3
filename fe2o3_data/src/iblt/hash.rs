//! Internal hash primitive used by the IBLT.
//!
//! Deterministic, seeded, and portable: a splitmix64 finaliser composed over
//! 8-byte chunks of the key. Not cryptographically secure -- adversarial
//! callers should pre-hash their keys with a keyed cryptographic hash (SipHash,
//! BLAKE3) before feeding them to the IBLT. The interior hash here only has to
//! be pseudo-random enough that the `k` cell selections are independent and
//! uniformly distributed over `num_cells`, which the splitmix64 mixer provides
//! comfortably for non-adversarial inputs.

/// A splitmix64 avalanche step. Deterministic, parameter-free, reversible.
pub(crate) fn mix64(mut x: u64) -> u64 {
	x ^= x >> 30;
	x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
	x ^= x >> 27;
	x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
	x ^= x >> 31;
	x
}

/// Hashes a byte slice under a seed, producing a 64-bit value.
///
/// Processes the key in 8-byte chunks, XOR-accumulating each chunk into the
/// running state and mixing between chunks. The final state is XORed with the
/// length to distinguish inputs that differ only in trailing zero bytes, then
/// mixed once more.
pub(crate) fn hash_bytes(bytes: &[u8], seed: u64) -> u64 {
	let mut h = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
	h = mix64(h);
	for chunk in bytes.chunks(8) {
		let mut buf = [0u8; 8];
		buf[..chunk.len()].copy_from_slice(chunk);
		h ^= u64::from_le_bytes(buf);
		h = mix64(h);
	}
	h ^= bytes.len() as u64;
	mix64(h)
}

/// Two-output hash used for double-hashing. Returns `(h1, h2)` where `h1` is
/// the primary hash used for the purity-check fingerprint and the first cell
/// index, and `h2` is the step used for subsequent cell indices.
pub(crate) fn hash_pair(bytes: &[u8], seed: u64) -> (u64, u64) {
	let h1 = hash_bytes(bytes, seed);
	// Second seed differs deterministically from the first. Mixing the seed
	// through a fixed constant keeps the two outputs independent without
	// exposing a second seed parameter.
	let h2 = hash_bytes(bytes, seed ^ 0x5851_f42d_4c95_7f2d);
	(h1, h2)
}
