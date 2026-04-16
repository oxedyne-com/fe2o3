//! The storage abstraction that distributed Ozone sits on top of.
//!
//! [`Storage`] is the trait every concrete backend implements. The canonical
//! production backend is `fe2o3_o3db_sync` driving a per-peer local Ozone,
//! but the distributed-Ozone engine is agnostic -- any type that persists
//! `(table, id) -> Record` with digest enumeration for anti-entropy will do.
//!
//! This keeps the test suite honest: the tests in `tests/` use
//! [`MemoryStorage`], an in-memory `HashMap`-backed adapter, and exercise the
//! full engine without touching disk. A later commit wires the
//! `fe2o3_o3db_sync` adapter in an integration crate.

use crate::record::{
	Record,
	RecordDigest,
	RecordId,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::sync::Mutex;


/// Distributed Ozone's storage contract.
///
/// Implementations must be internally thread-safe: distributed Ozone holds
/// one `Storage` handle per engine and may access it from multiple threads
/// (the write path, the anti-entropy loop, the consensus cohort driver, and
/// the inbound envelope dispatcher).
pub trait Storage {
	/// Persist or overwrite a record in the given table.
	fn put(&self, record: &Record) -> Outcome<()>;

	/// Read a record by identifier.
	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Record>>;

	/// Remove a record by identifier. Returns `true` if a record was
	/// removed, `false` if no such record existed.
	fn delete(&self, table: &str, id: &RecordId) -> Outcome<bool>;

	/// Enumerate every record in a table as a digest.
	///
	/// Used by the IBLT anti-entropy layer to build a symmetric-difference
	/// sketch against a peer's view of the same table. The `content` hash
	/// on each digest must be deterministic: two peers that hold the same
	/// record bytes must produce the same `content`.
	fn digests(&self, table: &str) -> Outcome<Vec<RecordDigest>>;
}


/// A simple in-memory [`Storage`] backed by a `HashMap`. Intended for tests,
/// two-peer loopback demos, and documentation examples -- not for
/// production.
///
/// Thread safety is via a single `Mutex` around the inner state; contention
/// is irrelevant at test scale.
pub struct MemoryStorage {
	inner:	Mutex<HashMap<(String, [u8; 32]), Record>>,
}

impl MemoryStorage {
	/// Constructs an empty memory-backed storage adapter.
	pub fn new() -> Self {
		Self { inner: Mutex::new(HashMap::new()) }
	}

	/// Returns the total number of records across all tables.
	pub fn len(&self) -> Outcome<usize> {
		let guard = lock_mutex!(self.inner);
		Ok(guard.len())
	}
}

impl Default for MemoryStorage {
	fn default() -> Self {
		Self::new()
	}
}

impl Storage for MemoryStorage {
	fn put(&self, record: &Record) -> Outcome<()> {
		let mut guard = lock_mutex!(self.inner);
		let key = (record.table.clone(), *record.id.as_bytes());
		guard.insert(key, record.clone());
		Ok(())
	}

	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Record>> {
		let guard = lock_mutex!(self.inner);
		let key = (table.to_string(), *id.as_bytes());
		Ok(guard.get(&key).cloned())
	}

	fn delete(&self, table: &str, id: &RecordId) -> Outcome<bool> {
		let mut guard = lock_mutex!(self.inner);
		let key = (table.to_string(), *id.as_bytes());
		Ok(guard.remove(&key).is_some())
	}

	fn digests(&self, table: &str) -> Outcome<Vec<RecordDigest>> {
		let guard = lock_mutex!(self.inner);
		let mut out = Vec::new();
		for ((t, _), rec) in guard.iter() {
			if t == table {
				let content = content_hash(&rec.value);
				out.push(RecordDigest { id: rec.id, content });
			}
		}
		// Sort for deterministic iteration in tests. The digests call site
		// already feeds an IBLT, which is order-independent, so sorting here
		// is purely for test reproducibility.
		out.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));
		Ok(out)
	}
}


/// Deterministic 256-bit content hash of a byte slice, used by the in-memory
/// storage adapter to produce [`RecordDigest::content`] values.
///
/// This is *not* a cryptographic hash -- it is splitmix64-based and intended
/// only for test adapters. Production storage backends should use a proper
/// hash (SHA-3, BLAKE3) that is resistant to adversarial collisions.
fn content_hash(bytes: &[u8]) -> [u8; 32] {
	let mut state: u64 = 0x9E3779B97F4A7C15;
	for chunk in bytes.chunks(8) {
		let mut buf = [0u8; 8];
		buf[..chunk.len()].copy_from_slice(chunk);
		let word = u64::from_le_bytes(buf);
		state = state.wrapping_add(word);
		state = (state ^ (state >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
		state = (state ^ (state >> 27)).wrapping_mul(0x94D049BB133111EB);
		state ^= state >> 31;
	}
	let mut out = [0u8; 32];
	for i in 0..4 {
		let limb = state.wrapping_mul(0x9E3779B97F4A7C15 ^ (i as u64));
		out[i * 8..(i + 1) * 8].copy_from_slice(&limb.to_le_bytes());
	}
	out
}
