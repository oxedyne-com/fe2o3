//! Content-addressed storage (CAS): fixed-size and content-defined chunking,
//! with SHA-256 content addressing, for opaque byte payloads.
//!
//! This module underpins large, syncable payloads that must not be shipped
//! whole. A payload is split into chunks; each chunk is addressed by
//! the SHA-256 of its bytes, and an ordered [`Manifest`] of those addresses
//! reconstructs it. A store keyed by content address then holds a chunk once
//! however many manifests reference it, and a consumer fetches only the chunks
//! it lacks. That is what lets a large corpus be used from a device too small
//! to hold it whole: the device keeps a working-set cache and pulls the rest on
//! demand.
//!
//! # Why SHA-256, not SHA-3
//!
//! The canonical caller is a browser client that computes chunk addresses with
//! the Web Crypto API and a gateway that re-verifies them before it accepts a
//! chunk. Web Crypto offers SHA-256 but not SHA-3, so SHA-256 is the one
//! function both sides compute identically. See
//! [`oxedyne_fe2o3_hash::sha256`], which exists for exactly this reason. The
//! distributed-Ozone digest hash (`dist::storage`) has a different job -- peer
//! divergence detection among Rust nodes -- and is chosen there separately.
//!
//! # What this module does not do
//!
//! Encryption is the caller's concern. For a *content-blind* store the caller
//! encrypts each chunk before handing it here, so the address is over
//! ciphertext and the store never sees plaintext; deduplication is therefore
//! within one caller's keyspace, never across callers.
//!
//! # Two chunkers
//!
//! [`Chunker`] cuts at fixed offsets: simple, and enough to break the
//! whole-payload ceiling, but every boundary is an offset rather than a place in
//! the content, so inserting one byte near the front shifts every boundary after
//! it and the whole payload re-uploads.
//!
//! [`CdcChunker`] cuts where the content says to. A FastCDC-style gear rolling
//! hash reads the payload and declares a boundary wherever the hash of the
//! preceding bytes hits a mask, so an insertion moves only the boundaries around
//! it: the chunks either side keep their addresses and are never re-sent. That
//! is the refinement the fixed chunker was a first cut for, and it arrived
//! without changing [`Manifest`] or [`Cas`] -- both chunkers return the same
//! manifest shape, and a store cannot tell which produced what it holds.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::sha256;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::{
	HashMap,
	HashSet,
};
use std::sync::Mutex;


/// Length in bytes of a content address, a SHA-256 digest.
pub const ADDR_LEN: usize = 32;

/// Default chunk size, 256 KiB. Large enough that the per-chunk manifest
/// overhead stays a small fraction of a multi-megabyte payload, small enough
/// that an edit confined to one region re-uploads little.
pub const DEFAULT_CHUNK_SIZE: usize = 256 * 1024;

/// Default minimum content-defined chunk size, 64 KiB.
///
/// A floor stops a run of unlucky hash hits producing a swarm of tiny chunks,
/// each of which costs an address in every manifest that names it.
pub const DEFAULT_MIN_CHUNK_SIZE: usize = 64 * 1024;

/// Default maximum content-defined chunk size, 1 MiB.
///
/// A ceiling bounds the damage when the hash finds no boundary at all, which is
/// what happens across a long run of identical bytes.
pub const DEFAULT_MAX_CHUNK_SIZE: usize = 1024 * 1024;

/// Seed for the gear table's generator.
///
/// The table must be identical on every machine and in every release, because a
/// changed table changes every boundary and so every address, which would make
/// a store's existing chunks unreachable. Fixing the seed here, and deriving the
/// table from it rather than shipping a literal, is what pins it. The value is
/// the golden-ratio constant splitmix64 conventionally uses.
const GEAR_SEED: u64 = 0x9e37_79b9_7f4a_7c15;

/// Normalisation level: how many bits the mask tightens below the average chunk
/// size and loosens above it.
///
/// Plain gear chunking gives an exponential spread of chunk sizes, so short
/// chunks dominate and the tail is long. Cutting less readily before the average
/// and more readily after it pulls the spread in towards the average without
/// forcing boundaries at fixed offsets. Two is the level the FastCDC paper
/// settles on.
const NORM_LEVEL: u32 = 2;

/// The 256-entry gear table, one random u64 per byte value.
///
/// Generated at compile time from [`GEAR_SEED`] by splitmix64, so there is no
/// dependency to pull in and no table to keep in the source.
static GEAR: [u64; 256] = gear_table();


/// A content address: the SHA-256 digest of a chunk's bytes.
///
/// Two byte-identical chunks share one address, which is what makes the store
/// deduplicating. The address is verifiable: a store re-hashes a submitted
/// chunk and rejects it unless the bytes produce the claimed address, so a
/// client cannot mislabel a chunk.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentId([u8; ADDR_LEN]);

impl ContentId {
	/// Computes the content address of a byte slice.
	pub fn of(bytes: &[u8]) -> Self {
		Self(sha256::digest(bytes))
	}

	/// Constructs an address from a raw 32-byte digest.
	pub const fn from_bytes(bytes: [u8; ADDR_LEN]) -> Self {
		Self(bytes)
	}

	/// Constructs an address from a byte slice, which must be exactly
	/// [`ADDR_LEN`] bytes.
	pub fn from_slice(bytes: &[u8]) -> Outcome<Self> {
		if bytes.len() != ADDR_LEN {
			return Err(err!(
				"A ContentId requires exactly {} bytes, got {}.",
				ADDR_LEN, bytes.len();
			Invalid, Input, Size));
		}
		let mut arr = [0u8; ADDR_LEN];
		arr.copy_from_slice(bytes);
		Ok(Self(arr))
	}

	/// Returns the address as a byte slice.
	pub fn as_bytes(&self) -> &[u8; ADDR_LEN] {
		&self.0
	}

	/// Reports whether `bytes` hash to this address. Used by a store to reject
	/// a chunk whose claimed address does not match its content.
	pub fn verifies(&self, bytes: &[u8]) -> bool {
		self.0 == sha256::digest(bytes)
	}

	/// Lowercase-hex rendering of the address, for logs and keys.
	pub fn to_hex(&self) -> String {
		let mut s = String::with_capacity(ADDR_LEN * 2);
		for b in &self.0 {
			s.push(hex_char(b >> 4));
			s.push(hex_char(b & 0x0f));
		}
		s
	}

	/// Parses a lowercase- or uppercase-hex address of exactly `2 * ADDR_LEN`
	/// characters.
	pub fn from_hex(s: &str) -> Outcome<Self> {
		let bytes = s.as_bytes();
		if bytes.len() != ADDR_LEN * 2 {
			return Err(err!(
				"A hex ContentId requires {} characters, got {}.",
				ADDR_LEN * 2, bytes.len();
			Invalid, Input, Size));
		}
		let mut arr = [0u8; ADDR_LEN];
		for i in 0..ADDR_LEN {
			let hi = res!(nibble(bytes[i * 2]));
			let lo = res!(nibble(bytes[i * 2 + 1]));
			arr[i] = (hi << 4) | lo;
		}
		Ok(Self(arr))
	}
}

impl std::fmt::Display for ContentId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.to_hex())
	}
}


/// A single content-addressed chunk: its address and its bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunk {
	/// The chunk's content address.
	pub id:		ContentId,
	/// The chunk's bytes.
	pub bytes:	Vec<u8>,
}

impl Chunk {
	/// Constructs a chunk, computing its address from its bytes.
	pub fn new(bytes: Vec<u8>) -> Self {
		let id = ContentId::of(&bytes);
		Self { id, bytes }
	}
}


/// A reference to one chunk within a [`Manifest`]: its address and byte length.
///
/// The length lets a reader validate a fetched chunk and lets a planner size a
/// download without fetching, so it costs one small integer per chunk to make
/// the manifest self-checking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkRef {
	/// The chunk's content address.
	pub id:		ContentId,
	/// The chunk's length in bytes.
	pub len:	usize,
}


/// The ordered list of chunk addresses that reconstruct a payload, with the
/// payload's total length for validation.
///
/// A manifest is small -- one address plus a length per chunk -- and is itself
/// an opaque value the caller may store or encrypt. It is the only thing a
/// caller must keep to recover a payload from a content-addressed store.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Manifest {
	/// The reconstructed payload's total length, the sum of the chunk lengths.
	pub total_len:	usize,
	/// The chunk references, in payload order.
	pub chunks:		Vec<ChunkRef>,
}

impl Manifest {
	/// Reports whether the manifest describes an empty payload.
	pub fn is_empty(&self) -> bool {
		self.chunks.is_empty()
	}

	/// Returns the number of chunks.
	pub fn len(&self) -> usize {
		self.chunks.len()
	}

	/// Iterates the chunk addresses in payload order.
	pub fn addrs(&self) -> impl Iterator<Item = &ContentId> {
		self.chunks.iter().map(|c| &c.id)
	}

	/// Reconstructs the payload, fetching each chunk through `fetch` and
	/// verifying it against the manifest.
	///
	/// Each fetched chunk is checked for the expected length and re-hashed to
	/// confirm it matches the address the manifest names, so a corrupted or
	/// substituted chunk is rejected rather than returned. The final length is
	/// checked against `total_len`.
	pub fn reassemble<F>(&self, mut fetch: F)
		-> Outcome<Vec<u8>>
	where
		F: FnMut(&ContentId) -> Outcome<Vec<u8>>,
	{
		let mut out = Vec::with_capacity(self.total_len);
		for (i, cref) in self.chunks.iter().enumerate() {
			let bytes = res!(fetch(&cref.id));
			if bytes.len() != cref.len {
				return Err(err!(
					"Chunk {} ({}) has length {}, manifest expects {}.",
					i, cref.id, bytes.len(), cref.len;
				Invalid, Input, Size, Mismatch));
			}
			if !cref.id.verifies(&bytes) {
				return Err(err!(
					"Chunk {} does not hash to its manifest address {}.",
					i, cref.id;
				Invalid, Input, Mismatch));
			}
			out.extend_from_slice(&bytes);
		}
		if out.len() != self.total_len {
			return Err(err!(
				"Reassembled {} bytes, manifest declares {}.",
				out.len(), self.total_len;
			Invalid, Input, Size, Mismatch));
		}
		Ok(out)
	}

	/// Serialises the manifest to a [`Dat`] for storage or transport. The shape
	/// is `[total_len, [[addr, len], ...]]`.
	pub fn to_dat(&self) -> Dat {
		let mut list = Vec::with_capacity(self.chunks.len());
		for cref in &self.chunks {
			list.push(Dat::List(vec![
				Dat::BU8(cref.id.as_bytes().to_vec()),
				Dat::U64(cref.len as u64),
			]));
		}
		Dat::List(vec![
			Dat::U64(self.total_len as u64),
			Dat::List(list),
		])
	}

	/// Reconstructs a manifest from a [`Dat`] produced by [`Manifest::to_dat`].
	pub fn from_dat(dat: &Dat) -> Outcome<Self> {
		let top = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"Manifest expects a 2-element Dat::List, got {:?}.", dat;
				Decode, Input, Mismatch)),
		};
		let total_len = match &top[0] {
			Dat::U64(n) => *n as usize,
			other => return Err(err!(
				"Manifest total_len expects Dat::U64, got {:?}.", other;
				Decode, Input, Mismatch)),
		};
		let entries = match &top[1] {
			Dat::List(v) => v,
			other => return Err(err!(
				"Manifest chunks expect Dat::List, got {:?}.", other;
				Decode, Input, Mismatch)),
		};
		let mut chunks = Vec::with_capacity(entries.len());
		for entry in entries {
			let pair = match entry {
				Dat::List(v) if v.len() == 2 => v,
				_ => return Err(err!(
					"Manifest chunk entry expects a 2-element list, got {:?}.",
					entry;
					Decode, Input, Mismatch)),
			};
			let id = match &pair[0] {
				Dat::BU8(b) => res!(ContentId::from_slice(b)),
				other => return Err(err!(
					"Manifest chunk address expects Dat::BU8, got {:?}.", other;
					Decode, Input, Mismatch)),
			};
			let len = match &pair[1] {
				Dat::U64(n) => *n as usize,
				other => return Err(err!(
					"Manifest chunk length expects Dat::U64, got {:?}.", other;
					Decode, Input, Mismatch)),
			};
			chunks.push(ChunkRef { id, len });
		}
		Ok(Self { total_len, chunks })
	}
}


/// Splits a payload into fixed-size, content-addressed chunks.
#[derive(Clone, Copy, Debug)]
pub struct Chunker {
	/// Target chunk size in bytes; the final chunk may be shorter.
	chunk_size:	usize,
}

impl Default for Chunker {
	fn default() -> Self {
		Self { chunk_size: DEFAULT_CHUNK_SIZE }
	}
}

impl Chunker {
	/// Constructs a chunker with the given chunk size, which must be non-zero.
	pub fn new(chunk_size: usize) -> Outcome<Self> {
		if chunk_size == 0 {
			return Err(err!(
				"Chunk size must be non-zero.";
			Invalid, Input, Range));
		}
		Ok(Self { chunk_size })
	}

	/// Returns the configured chunk size.
	pub fn chunk_size(&self) -> usize {
		self.chunk_size
	}

	/// Splits `payload` into chunks, returning the ordered [`Manifest`] and the
	/// chunk bytes.
	///
	/// A payload shorter than one chunk yields a single chunk; an empty payload
	/// yields an empty manifest and no chunks. Byte-identical chunks share an
	/// address, so the returned `Vec<Chunk>` may contain duplicates that a
	/// deduplicating store collapses on write.
	pub fn split(&self, payload: &[u8])
		-> (Manifest, Vec<Chunk>)
	{
		let mut refs = Vec::new();
		let mut chunks = Vec::new();
		for part in payload.chunks(self.chunk_size) {
			let chunk = Chunk::new(part.to_vec());
			refs.push(ChunkRef { id: chunk.id, len: chunk.bytes.len() });
			chunks.push(chunk);
		}
		(Manifest { total_len: payload.len(), chunks: refs }, chunks)
	}
}


/// Splits a payload into content-defined, content-addressed chunks, cutting
/// where a FastCDC-style gear rolling hash says the content changes.
///
/// The hash reads the payload one byte at a time, keeping a value that depends
/// only on the last few dozen bytes. Where that value hits a mask, a boundary is
/// declared. Because the boundary follows the bytes and not the offset, an
/// insertion or deletion perturbs only the chunks around it: every chunk beyond
/// the disturbance re-synchronises on the same content and keeps the address it
/// had, so a store already holding it needs nothing sent.
///
/// Boundaries are constrained to `[min, max]` and steered towards `avg` by
/// normalised chunking: below the average the mask is a couple of bits
/// stricter, above it a couple of bits looser. The bytes before `min` are not
/// hashed at all -- no boundary could be accepted there -- which is the
/// cut-point skipping that makes the scan cheap.
///
/// The manifest it produces is the same [`Manifest`] the fixed-size [`Chunker`]
/// produces, and reassembles the same way.
#[derive(Clone, Copy, Debug)]
pub struct CdcChunker {
	/// Smallest permitted chunk, except for a payload shorter than this.
	min:	usize,
	/// Target average chunk size, the point at which the mask loosens.
	avg:	usize,
	/// Largest permitted chunk; a boundary is forced here.
	max:	usize,
	/// Strict mask, applied below `avg`.
	mask_s:	u64,
	/// Loose mask, applied at and above `avg`.
	mask_l:	u64,
}

impl Default for CdcChunker {
	fn default() -> Self {
		Self::sizes(
			DEFAULT_MIN_CHUNK_SIZE,
			DEFAULT_CHUNK_SIZE,
			DEFAULT_MAX_CHUNK_SIZE,
		)
	}
}

impl CdcChunker {
	/// Constructs a chunker with the given minimum, average and maximum chunk
	/// sizes, which must satisfy `0 < min <= avg <= max`.
	pub fn new(min: usize, avg: usize, max: usize)
		-> Outcome<Self>
	{
		if min == 0 {
			return Err(err!(
				"Minimum chunk size must be non-zero.";
			Invalid, Input, Range));
		}
		if min > avg || avg > max {
			return Err(err!(
				"Chunk sizes must satisfy min <= avg <= max, got {}, {}, {}.",
				min, avg, max;
			Invalid, Input, Range));
		}
		Ok(Self::sizes(min, avg, max))
	}

	/// Builds a chunker from already-valid sizes, deriving the two masks.
	fn sizes(min: usize, avg: usize, max: usize) -> Self {
		let bits = log2_floor(avg);				// Mask width for the average.
		let strict = (bits + NORM_LEVEL).min(63);
		let loose = bits.saturating_sub(NORM_LEVEL).max(1);
		Self {
			min,
			avg,
			max,
			mask_s:	high_mask(strict),
			mask_l:	high_mask(loose),
		}
	}

	/// Returns the minimum chunk size.
	pub fn min_size(&self) -> usize {
		self.min
	}

	/// Returns the target average chunk size.
	pub fn avg_size(&self) -> usize {
		self.avg
	}

	/// Returns the maximum chunk size.
	pub fn max_size(&self) -> usize {
		self.max
	}

	/// Splits `payload` into chunks, returning the ordered [`Manifest`] and the
	/// chunk bytes.
	///
	/// The contract matches [`Chunker::split`]: an empty payload yields an empty
	/// manifest and no chunks, a payload shorter than the minimum chunk size
	/// yields a single chunk, and byte-identical chunks share an address, so the
	/// returned `Vec<Chunk>` may contain duplicates that a deduplicating store
	/// collapses on write.
	pub fn split(&self, payload: &[u8])
		-> (Manifest, Vec<Chunk>)
	{
		let mut refs = Vec::new();
		let mut chunks = Vec::new();
		let mut pos = 0;
		while pos < payload.len() {
			let len = self.cut(&payload[pos..]);
			let chunk = Chunk::new(payload[pos..pos + len].to_vec());
			refs.push(ChunkRef { id: chunk.id, len: chunk.bytes.len() });
			chunks.push(chunk);
			pos += len;
		}
		(Manifest { total_len: payload.len(), chunks: refs }, chunks)
	}

	/// Returns the length of the first chunk of `data`, always at least one byte
	/// so that [`CdcChunker::split`] terminates.
	fn cut(&self, data: &[u8]) -> usize {
		let n = data.len();
		if n <= self.min {
			return n;	// Too short to cut: the whole remainder is one chunk.
		}
		let end = self.max.min(n);			// Forced boundary.
		let mid = self.avg.min(end);		// Where the mask loosens.
		let mut fp = 0u64;					// The rolling gear hash.
		let mut i = self.min;				// Skip: no boundary may land below.
		while i < mid {
			fp = (fp << 1).wrapping_add(GEAR[data[i] as usize]);
			if fp & self.mask_s == 0 {
				return i + 1;
			}
			i += 1;
		}
		while i < end {
			fp = (fp << 1).wrapping_add(GEAR[data[i] as usize]);
			if fp & self.mask_l == 0 {
				return i + 1;
			}
			i += 1;
		}
		end
	}
}


/// A store of chunks keyed by content address.
///
/// Implementations must be internally thread-safe. The store is deliberately
/// dumb: it holds opaque bytes addressed by their hash and enforces only that a
/// chunk's bytes match its address. Which chunks are live -- reachable from a
/// current manifest -- is the caller's knowledge, supplied to [`Cas::sweep`]
/// for garbage collection.
pub trait Cas {
	/// Stores a chunk, rejecting it if its bytes do not hash to its address.
	/// Storing an address already present is a no-op (the bytes are identical
	/// by definition), so writes are idempotent.
	fn put(&self, chunk: &Chunk) -> Outcome<()>;

	/// Fetches a chunk's bytes by address, or `None` if absent.
	fn get(&self, id: &ContentId) -> Outcome<Option<Vec<u8>>>;

	/// Reports whether the store holds a chunk at this address.
	fn has(&self, id: &ContentId) -> Outcome<bool>;

	/// Removes a chunk by address, returning `true` if one was present.
	fn delete(&self, id: &ContentId) -> Outcome<bool>;

	/// Enumerates every address the store holds.
	fn ids(&self) -> Outcome<Vec<ContentId>>;

	/// Convenience: chunks `bytes` and stores it, returning its address.
	fn put_bytes(&self, bytes: Vec<u8>)
		-> Outcome<ContentId>
	{
		let chunk = Chunk::new(bytes);
		let id = chunk.id;
		res!(self.put(&chunk));
		Ok(id)
	}

	/// Deletes every chunk not in `live`, returning the number removed.
	///
	/// This is mark-and-sweep garbage collection: the caller assembles the set
	/// of addresses reachable from every manifest it still holds and hands it
	/// in; everything else is unreferenced and freed. Deleting only the
	/// unreferenced set is what lets a lapse evict overflow without disturbing
	/// chunks a live manifest still needs.
	fn sweep(&self, live: &HashSet<ContentId>)
		-> Outcome<usize>
	{
		let mut removed = 0;
		for id in res!(self.ids()) {
			if !live.contains(&id) {
				if res!(self.delete(&id)) {
					removed += 1;
				}
			}
		}
		Ok(removed)
	}
}


/// An in-memory [`Cas`] backed by a `HashMap`, for tests and loopback demos.
pub struct MemoryCas {
	inner:	Mutex<HashMap<ContentId, Vec<u8>>>,
}

impl MemoryCas {
	/// Constructs an empty in-memory store.
	pub fn new() -> Self {
		Self { inner: Mutex::new(HashMap::new()) }
	}

	/// Returns the number of distinct chunks held.
	pub fn len(&self) -> Outcome<usize> {
		let guard = lock_mutex!(self.inner);
		Ok(guard.len())
	}

	/// Reports whether the store is empty.
	pub fn is_empty(&self) -> Outcome<bool> {
		Ok(res!(self.len()) == 0)
	}
}

impl Default for MemoryCas {
	fn default() -> Self {
		Self::new()
	}
}

impl Cas for MemoryCas {
	fn put(&self, chunk: &Chunk) -> Outcome<()> {
		if !chunk.id.verifies(&chunk.bytes) {
			return Err(err!(
				"Refusing chunk whose bytes do not hash to its address {}.",
				chunk.id;
			Invalid, Input, Mismatch));
		}
		let mut guard = lock_mutex!(self.inner);
		guard.entry(chunk.id).or_insert_with(|| chunk.bytes.clone());
		Ok(())
	}

	fn get(&self, id: &ContentId) -> Outcome<Option<Vec<u8>>> {
		let guard = lock_mutex!(self.inner);
		Ok(guard.get(id).cloned())
	}

	fn has(&self, id: &ContentId) -> Outcome<bool> {
		let guard = lock_mutex!(self.inner);
		Ok(guard.contains_key(id))
	}

	fn delete(&self, id: &ContentId) -> Outcome<bool> {
		let mut guard = lock_mutex!(self.inner);
		Ok(guard.remove(id).is_some())
	}

	fn ids(&self) -> Outcome<Vec<ContentId>> {
		let guard = lock_mutex!(self.inner);
		Ok(guard.keys().copied().collect())
	}
}


/// Generates the gear table by running splitmix64 from [`GEAR_SEED`].
///
/// Splitmix64 is a handful of multiplies and shifts, which is why it can run in
/// a `const` context and save the crate a dependency and a 2 KiB literal.
const fn gear_table() -> [u64; 256] {
	let mut table = [0u64; 256];
	let mut state = GEAR_SEED;
	let mut i = 0;
	while i < 256 {
		state = state.wrapping_add(GEAR_SEED);
		let mut z = state;
		z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		table[i] = z ^ (z >> 31);
		i += 1;
	}
	table
}

/// Builds a mask over the top `bits` bits of a `u64`, for `1 <= bits <= 63`.
///
/// The top bits are the ones to test. A gear hash shifts left by one per byte,
/// so bit `k` of the hash depends on the last `k + 1` bytes; testing the high
/// bits therefore tests a window dozens of bytes wide, while testing the low
/// bits would decide a boundary on almost nothing.
const fn high_mask(bits: u32) -> u64 {
	((1u64 << bits) - 1) << (64 - bits)
}

/// Floor of the base-2 logarithm of a non-zero value.
fn log2_floor(n: usize) -> u32 {
	(usize::BITS - 1) - n.leading_zeros()
}

/// Converts a 4-bit nibble to its lowercase hex character.
fn hex_char(nib: u8) -> char {
	match nib {
		0..=9	=> (b'0' + nib) as char,
		10..=15	=> (b'a' + nib - 10) as char,
		_		=> '?',	// Unreachable: callers mask to 0..=15.
	}
}

/// Converts a hex character byte to its 4-bit nibble.
fn nibble(b: u8)
	-> Outcome<u8>
{
	match b {
		b'0'..=b'9'	=> Ok(b - b'0'),
		b'a'..=b'f'	=> Ok(b - b'a' + 10),
		b'A'..=b'F'	=> Ok(b - b'A' + 10),
		_			=> Err(err!(
			"Invalid hex character: 0x{:02x}.", b;
			Invalid, Input)),
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	/// A content address is deterministic and distinguishes distinct inputs.
	#[test]
	fn content_id_deterministic_and_verifies() -> Outcome<()> {
		let a = ContentId::of(b"hello");
		let b = ContentId::of(b"hello");
		let c = ContentId::of(b"world");
		assert_eq!(a, b);
		assert_ne!(a, c);
		assert!(a.verifies(b"hello"));
		assert!(!a.verifies(b"world"));
		Ok(())
	}

	/// A hex address round-trips through parse and render.
	#[test]
	fn content_id_hex_round_trip() -> Outcome<()> {
		let id = ContentId::of(b"some bytes");
		let hex = id.to_hex();
		assert_eq!(hex.len(), ADDR_LEN * 2);
		let back = res!(ContentId::from_hex(&hex));
		assert_eq!(id, back);
		Ok(())
	}

	/// Chunking then reassembling recovers the payload, for an empty payload,
	/// one shorter than a chunk, an exact multiple, and one with a remainder.
	#[test]
	fn chunk_reassemble_round_trip() -> Outcome<()> {
		let chunker = res!(Chunker::new(4));
		let store = MemoryCas::new();
		for payload in [
			Vec::new(),
			b"ab".to_vec(),
			b"abcdefgh".to_vec(),		// Exact multiple of 4.
			b"abcdefghij".to_vec(),		// Remainder of 2.
		] {
			let (manifest, chunks) = chunker.split(&payload);
			for chunk in &chunks {
				res!(store.put(chunk));
			}
			assert_eq!(manifest.total_len, payload.len());
			let got = res!(manifest.reassemble(|id| {
				match res!(store.get(id)) {
					Some(b) => Ok(b),
					None => Err(err!("missing chunk {}", id; Test, Missing)),
				}
			}));
			assert_eq!(got, payload);
		}
		Ok(())
	}

	/// Identical chunks collapse to one stored entry.
	#[test]
	fn identical_chunks_deduplicate() -> Outcome<()> {
		let chunker = res!(Chunker::new(4));
		let store = MemoryCas::new();
		let payload = b"aaaaaaaa".to_vec();	// Two identical "aaaa" chunks.
		let (manifest, chunks) = chunker.split(&payload);
		assert_eq!(manifest.len(), 2);
		for chunk in &chunks {
			res!(store.put(chunk));
		}
		assert_eq!(res!(store.len()), 1);	// Deduplicated.
		Ok(())
	}

	/// A manifest survives a Dat round trip.
	#[test]
	fn manifest_dat_round_trip() -> Outcome<()> {
		let chunker = res!(Chunker::new(3));
		let (manifest, _) = chunker.split(b"the quick brown fox");
		let dat = manifest.to_dat();
		let back = res!(Manifest::from_dat(&dat));
		assert_eq!(manifest, back);
		Ok(())
	}

	/// Reassembly rejects a chunk whose bytes have been tampered with.
	#[test]
	fn reassemble_rejects_tampered_chunk() -> Outcome<()> {
		let chunker = res!(Chunker::new(4));
		let (manifest, _) = chunker.split(b"abcdefgh");
		// Fetch returns the wrong bytes for whatever is asked.
		let outcome = manifest.reassemble(|_id| Ok(b"XXXX".to_vec()));
		assert!(outcome.is_err());
		Ok(())
	}

	/// A store refuses a chunk whose address does not match its bytes.
	#[test]
	fn put_rejects_mislabelled_chunk() -> Outcome<()> {
		let store = MemoryCas::new();
		let bad = Chunk {
			id:		ContentId::of(b"claimed"),
			bytes:	b"actual".to_vec(),
		};
		assert!(store.put(&bad).is_err());
		Ok(())
	}

	/// The store round-trips a chunk and reports presence and deletion.
	#[test]
	fn memory_cas_put_get_has_delete() -> Outcome<()> {
		let store = MemoryCas::new();
		let id = res!(store.put_bytes(b"payload".to_vec()));
		assert!(res!(store.has(&id)));
		assert_eq!(res!(store.get(&id)), Some(b"payload".to_vec()));
		assert!(res!(store.delete(&id)));
		assert!(!res!(store.has(&id)));
		assert!(!res!(store.delete(&id)));	// Second delete is false.
		Ok(())
	}

	/// A sweep frees exactly the chunks no live manifest references.
	#[test]
	fn sweep_frees_unreferenced_chunks() -> Outcome<()> {
		let store = MemoryCas::new();
		let keep = res!(store.put_bytes(b"keep me".to_vec()));
		let _drop = res!(store.put_bytes(b"drop me".to_vec()));
		assert_eq!(res!(store.len()), 2);
		let mut live = HashSet::new();
		live.insert(keep);
		let removed = res!(store.sweep(&live));
		assert_eq!(removed, 1);
		assert_eq!(res!(store.len()), 1);
		assert!(res!(store.has(&keep)));
		Ok(())
	}

	/// A deterministic pseudorandom byte stream.
	///
	/// Splitmix64 again, seeded separately from the gear table, so a test needs
	/// no `rand` crate and produces the same payload on every machine and every
	/// run -- a shift-resistance figure is only worth quoting if it is stable.
	fn pseudorandom(len: usize, seed: u64) -> Vec<u8> {
		let mut out = Vec::with_capacity(len);
		let mut state = seed;
		while out.len() < len {
			state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
			let mut z = state;
			z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
			z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
			z ^= z >> 31;
			for b in z.to_le_bytes() {
				if out.len() == len {
					break;
				}
				out.push(b);
			}
		}
		out
	}

	/// The fraction of `edited`'s chunks whose addresses also occur in `orig`,
	/// counting multiplicity. This is the quantity that matters: it is the share
	/// of the edited payload a store already holds, and so need not be sent.
	fn shared_fraction(orig: &Manifest, edited: &Manifest) -> f64 {
		let mut have: HashMap<ContentId, usize> = HashMap::new();
		for cref in &orig.chunks {
			*have.entry(cref.id).or_insert(0) += 1;
		}
		let mut shared = 0usize;
		for cref in &edited.chunks {
			if let Some(n) = have.get_mut(&cref.id) {
				if *n > 0 {
					*n -= 1;
					shared += 1;
				}
			}
		}
		shared as f64 / edited.chunks.len() as f64
	}

	/// Inserts one byte at `at`, the smallest edit that shifts everything after
	/// it.
	fn insert_byte(payload: &[u8], at: usize) -> Vec<u8> {
		let mut out = payload.to_vec();
		out.insert(at, 0x5a);
		out
	}

	/// The size triple is validated, and the accessors report it back.
	#[test]
	fn cdc_validates_its_sizes() -> Outcome<()> {
		assert!(CdcChunker::new(0, 16, 64).is_err());		// Zero minimum.
		assert!(CdcChunker::new(32, 16, 64).is_err());		// min > avg.
		assert!(CdcChunker::new(16, 128, 64).is_err());		// avg > max.
		let cdc = res!(CdcChunker::new(16, 64, 256));
		assert_eq!(cdc.min_size(), 16);
		assert_eq!(cdc.avg_size(), 64);
		assert_eq!(cdc.max_size(), 256);
		let def = CdcChunker::default();
		assert_eq!(def.min_size(), DEFAULT_MIN_CHUNK_SIZE);
		assert_eq!(def.avg_size(), DEFAULT_CHUNK_SIZE);
		assert_eq!(def.max_size(), DEFAULT_MAX_CHUNK_SIZE);
		Ok(())
	}

	/// The same payload and the same configuration give the same manifest, and
	/// the degenerate payloads behave as the fixed chunker's do.
	#[test]
	fn cdc_split_is_deterministic() -> Outcome<()> {
		let cdc = res!(CdcChunker::new(64, 256, 1024));
		let payload = pseudorandom(300_000, 7);
		let (m1, c1) = cdc.split(&payload);
		let (m2, c2) = cdc.split(&payload);
		assert_eq!(m1, m2);
		assert_eq!(c1, c2);
		assert!(m1.len() > 1);					// It really did cut.

		let (empty, chunks) = cdc.split(&[]);	// Empty payload, empty manifest.
		assert!(empty.is_empty());
		assert_eq!(empty.total_len, 0);
		assert!(chunks.is_empty());

		let short = pseudorandom(30, 9);		// Below the minimum: one chunk.
		let (m, chunks) = cdc.split(&short);
		assert_eq!(m.len(), 1);
		assert_eq!(m.total_len, 30);
		assert_eq!(chunks[0].bytes, short);
		Ok(())
	}

	/// An insertion near the front of a payload disturbs only the chunks around
	/// it; the rest keep their addresses.
	///
	/// The fixed-size chunker is measured on the same payload for contrast. It
	/// cuts at offsets, so one inserted byte shifts every boundary after it and
	/// almost nothing survives, which is the whole reason for the
	/// content-defined chunker.
	#[test]
	fn cdc_survives_an_insertion_near_the_front() -> Outcome<()> {
		let payload = pseudorandom(4 * 1024 * 1024, 0x0da7_a5ee_d1);
		let edited = insert_byte(&payload, 1024);

		let cdc = res!(CdcChunker::new(16 * 1024, 64 * 1024, 256 * 1024));
		let (before, _) = cdc.split(&payload);
		let (after, _) = cdc.split(&edited);
		let kept = shared_fraction(&before, &after);
		assert!(
			kept > 0.90,
			"CDC kept only {:.3} of {} chunks across a front insertion.",
			kept, after.len(),
		);

		let fixed = res!(Chunker::new(64 * 1024));
		let (fbefore, _) = fixed.split(&payload);
		let (fafter, _) = fixed.split(&edited);
		let fkept = shared_fraction(&fbefore, &fafter);
		assert!(
			fkept < 0.10,
			"The fixed chunker was expected to lose nearly everything, kept {:.3}.",
			fkept,
		);
		Ok(())
	}

	/// An edit in the middle of a payload leaves both halves alone.
	#[test]
	fn cdc_survives_an_insertion_in_the_middle() -> Outcome<()> {
		let payload = pseudorandom(4 * 1024 * 1024, 0x0da7_a5ee_d2);
		let edited = insert_byte(&payload, 2 * 1024 * 1024);

		let cdc = res!(CdcChunker::new(16 * 1024, 64 * 1024, 256 * 1024));
		let (before, _) = cdc.split(&payload);
		let (after, _) = cdc.split(&edited);
		let kept = shared_fraction(&before, &after);
		assert!(
			kept > 0.90,
			"CDC kept only {:.3} of {} chunks across a middle insertion.",
			kept, after.len(),
		);
		Ok(())
	}

	/// Every chunk lands inside the configured bounds, and the mean sits near
	/// the requested average.
	#[test]
	fn cdc_chunk_sizes_stay_within_bounds() -> Outcome<()> {
		let cdc = CdcChunker::default();
		let payload = pseudorandom(4 * 1024 * 1024, 0x51ce_51ce);
		let (manifest, _) = cdc.split(&payload);
		assert!(manifest.len() > 4);
		for (i, cref) in manifest.chunks.iter().enumerate() {
			assert!(
				cref.len <= cdc.max_size(),
				"Chunk {} of {} bytes exceeds the maximum.", i, cref.len,
			);
			if i + 1 < manifest.len() {
				// Only the last chunk may fall below the minimum.
				assert!(
					cref.len >= cdc.min_size(),
					"Chunk {} of {} bytes falls below the minimum.", i, cref.len,
				);
			}
		}
		let mean = payload.len() as f64 / manifest.len() as f64;
		let avg = cdc.avg_size() as f64;
		assert!(
			mean > avg / 2.0 && mean < avg * 2.0,
			"Mean chunk size {:.0} strays from the {:.0} requested.", mean, avg,
		);
		Ok(())
	}

	/// A content-defined split reassembles byte for byte through a store.
	#[test]
	fn cdc_reassembles_byte_for_byte() -> Outcome<()> {
		let cdc = res!(CdcChunker::new(1024, 4096, 16384));
		let store = MemoryCas::new();
		for payload in [
			Vec::new(),
			pseudorandom(1, 1),
			pseudorandom(999, 2),				// Below the minimum.
			pseudorandom(200_000, 3),
			vec![0u8; 200_000],					// A long identical run.
		] {
			let (manifest, chunks) = cdc.split(&payload);
			for chunk in &chunks {
				res!(store.put(chunk));
			}
			assert_eq!(manifest.total_len, payload.len());
			let got = res!(manifest.reassemble(|id| {
				match res!(store.get(id)) {
					Some(b) => Ok(b),
					None => Err(err!("missing chunk {}", id; Test, Missing)),
				}
			}));
			assert_eq!(got, payload);
		}
		Ok(())
	}

	/// The gear table is fully populated and free of duplicates, which a broken
	/// generator would not manage.
	#[test]
	fn gear_table_is_distinct() -> Outcome<()> {
		let seen: HashSet<u64> = GEAR.iter().copied().collect();
		assert_eq!(seen.len(), 256);
		assert!(!GEAR.iter().any(|g| *g == 0));
		Ok(())
	}
}
