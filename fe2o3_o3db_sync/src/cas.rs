//! Content-addressed storage (CAS): fixed-size chunking and SHA-256 content
//! addressing for opaque byte payloads.
//!
//! This module underpins large, syncable payloads that must not be shipped
//! whole. A payload is split into fixed-size chunks; each chunk is addressed by
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
//! within one caller's keyspace, never across callers. Chunking is deliberately
//! fixed-size for a first cut: simple, and enough to break the whole-payload
//! ceiling. A content-defined (rolling-hash) chunker that localises the churn an
//! edit causes is a later refinement, and can be added without changing
//! [`Manifest`] or [`Cas`].

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
}
