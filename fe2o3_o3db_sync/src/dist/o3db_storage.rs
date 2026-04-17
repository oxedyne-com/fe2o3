//! [`Storage`] adapter backed by a local [`O3db`](crate::O3db) instance.
//!
//! Plumbs the distributed-Ozone [`Storage`] trait onto the existing local
//! Ozone engine. Every record is persisted as one key/value pair:
//!
//! - Key: `Dat::Str("{table}:{hex_id}")`. The `{table}:` prefix lets a
//!   `scan` with [`ScanOpts::with_str_prefix`] enumerate every record in
//!   a single table, which the anti-entropy loop needs for its digest
//!   round. `{hex_id}` is the lowercase-hex encoding of the 32-byte
//!   [`RecordId`].
//! - Value: `Dat::BU8(record.value)`. The record's table and id are
//!   recoverable from the key, so the stored value payload carries only
//!   the application-opaque bytes.
//!
//! Reads block on the responder channel through the synchronous
//! [`OzoneApi::get_wait`] wrapper. Writes and deletes block on the
//! responder for the `Chunks` and `KeyExists` messages the store path
//! emits, surfacing any error along the way as an [`Outcome`] error.
//!
//! # Performance notes
//!
//! - `digests` performs a prefix scan followed by a per-record fetch.
//!   Scan v1 in `fe2o3_o3db_sync` does not return values, so the
//!   content hash has to come from a second round trip per record.
//!   This is O(n) fetches per anti-entropy round; acceptable for small
//!   cohort-backed tables (identity, peer_set, revocation) but a
//!   bottleneck for large ones. A scan v2 that returns values is the
//!   planned optimisation; the [`Storage`] trait does not need to
//!   change to adopt it.
//! - The content hash is the splitmix64-based 32-byte hash used by
//!   [`MemoryStorage`](super::storage::MemoryStorage) so that a mixed
//!   cluster (in-memory peer + O3db-backed peer) converges. A
//!   cryptographic replacement can slot in without touching the trait.

use super::record::{
	Record,
	RecordDigest,
	RecordId,
};
use super::storage::Storage;

use crate::O3db;
use crate::base::{
	constant,
	id::usr_kind_id_deleted,
};
use crate::comm::msg::OzoneMsg;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::ScanOpts;
use oxedyne_fe2o3_iop_hash::{
	api::Hasher,
	csum::Checksummer,
};
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};

use std::sync::Arc;


/// Storage adapter persisting records in a local [`O3db`] instance.
///
/// The adapter is generic over the six O3db type parameters so a caller
/// can plug its chosen crypto and hash schemes in directly. In practice
/// most callers instantiate it once at start-up with concrete types and
/// then hand it to [`DistOzone::new`](super::engine::DistOzone::new).
pub struct O3dbStorage<
	const UIDL: usize,
	UID:	NumIdDat<UIDL> + 'static,
	ENC:	Encrypter + 'static,
	KH:		Hasher + 'static,
	PR:		Hasher + 'static,
	CS:		Checksummer + 'static,
> {
	db:		Arc<O3db<UIDL, UID, ENC, KH, PR, CS>>,
	user:	UID,
}

impl<
	const UIDL: usize,
	UID:	NumIdDat<UIDL> + 'static,
	ENC:	Encrypter + 'static,
	KH:		Hasher + 'static,
	PR:		Hasher + 'static,
	CS:		Checksummer + 'static,
>
	O3dbStorage<UIDL, UID, ENC, KH, PR, CS>
{
	/// Constructs a new adapter over a shared [`O3db`] handle.
	///
	/// `user` is the caller identity under which every distributed-mode
	/// write is stamped -- distributed Ozone does not model per-write
	/// authorship beyond this. Applications that need finer-grained
	/// multi-user bookkeeping should layer that above the adapter.
	pub fn new(db: Arc<O3db<UIDL, UID, ENC, KH, PR, CS>>, user: UID) -> Self {
		Self { db, user }
	}

	/// Composite key format: `"{table}:{hex_id}"`.
	fn encode_key(table: &str, id: &RecordId) -> Dat {
		let mut s = String::with_capacity(table.len() + 1 + 64);
		s.push_str(table);
		s.push(':');
		for b in id.as_bytes() {
			s.push(hex_char(b >> 4));
			s.push(hex_char(b & 0x0f));
		}
		Dat::Str(s)
	}

	/// Parses a key produced by [`encode_key`] back to a [`RecordId`],
	/// asserting the expected table prefix along the way.
	fn parse_key(dat: &Dat, table: &str) -> Outcome<RecordId> {
		let s = match dat {
			Dat::Str(s) => s,
			_ => return Err(err!(
				"O3dbStorage expected Dat::Str key, got {:?}.", dat;
				Invalid, Input, Mismatch)),
		};
		let expected_prefix_len = table.len() + 1;
		if s.len() != expected_prefix_len + 64 {
			return Err(err!(
				"O3dbStorage key '{}' has unexpected length (table='{}').",
				s, table;
				Invalid, Input, Size));
		}
		if !s.starts_with(table) || s.as_bytes()[table.len()] != b':' {
			return Err(err!(
				"O3dbStorage key '{}' lacks expected '{}:' prefix.",
				s, table;
				Invalid, Input, Mismatch));
		}
		let hex = &s.as_bytes()[expected_prefix_len..];
		let mut arr = [0u8; 32];
		for i in 0..32 {
			let hi = res!(nibble(hex[i * 2]));
			let lo = res!(nibble(hex[i * 2 + 1]));
			arr[i] = (hi << 4) | lo;
		}
		Ok(RecordId::from_bytes(arr))
	}

	/// Drains the two-message acknowledgement a successful store emits.
	/// Returns `Ok(())` on `Chunks(_)` + `KeyExists(_)`; surfaces any
	/// `Error` variant as an [`Outcome`] error.
	fn drain_store_ack(
		resp:	&crate::comm::response::Responder<UIDL, UID, ENC, KH>,
	)
		-> Outcome<()>
	{
		match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
			OzoneMsg::Chunks(_) => {},
			OzoneMsg::Error(e) => return Err(err!(e,
				"O3dbStorage: store ack failed at Chunks phase.";
				IO, Write)),
			other => return Err(err!(
				"O3dbStorage: unexpected message at Chunks phase: {:?}.", other;
				Unexpected, Channel)),
		}
		match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
			OzoneMsg::KeyExists(_) => Ok(()),
			OzoneMsg::Error(e) => Err(err!(e,
				"O3dbStorage: store ack failed at KeyExists phase.";
				IO, Write)),
			other => Err(err!(
				"O3dbStorage: unexpected message at KeyExists phase: {:?}.",
				other;
				Unexpected, Channel)),
		}
	}

	/// Extracts the record value bytes from a fetched `Dat`, handling
	/// the unsigned-bytes family (the store path always writes
	/// `Dat::BU8`; the other widths show up if a caller had pre-existing
	/// data under a different width).
	fn extract_value(dat: &Dat) -> Outcome<Vec<u8>> {
		match dat {
			Dat::BU8(b)	 | Dat::BU16(b) | Dat::BU32(b) | Dat::BU64(b) =>
				Ok(b.clone()),
			other => Err(err!(
				"O3dbStorage expected a byte-vector Dat value, got {:?}.",
				other;
				Decode, Unexpected)),
		}
	}
}

impl<
	const UIDL: usize,
	UID:	NumIdDat<UIDL> + 'static,
	ENC:	Encrypter + 'static,
	KH:		Hasher + 'static,
	PR:		Hasher + 'static,
	CS:		Checksummer + 'static,
>
	Storage for O3dbStorage<UIDL, UID, ENC, KH, PR, CS>
{
	fn put(&self, record: &Record) -> Outcome<()> {
		let key = Self::encode_key(&record.table, &record.id);
		let value = Dat::BU8(record.value.clone());
		let resp = res!(self.db.api().store(key, value, self.user));
		Self::drain_store_ack(&resp)
	}

	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Record>> {
		let key = Self::encode_key(table, id);
		match res!(self.db.api().get_wait(&key, None)) {
			None => Ok(None),
			Some((dat, _meta)) => {
				// A deletion tombstone is stored as
				// `Dat::Usr(usr_kind_id_deleted(), _)`; `get_wait`
				// returns it verbatim, so we have to recognise it
				// here and surface "not present" to the caller.
				if let Dat::Usr(kind, _) = &dat {
					if *kind == usr_kind_id_deleted() {
						return Ok(None);
					}
				}
				let value = res!(Self::extract_value(&dat));
				Ok(Some(Record {
					id:		*id,
					table:	table.to_string(),
					value,
				}))
			}
		}
	}

	fn delete(&self, table: &str, id: &RecordId) -> Outcome<bool> {
		// Fetch first to determine whether a record was present; the
		// overwrite below cannot distinguish "overwrote an existing
		// record" from "wrote a fresh tombstone", and [`Storage::delete`]
		// promises the former answer.
		let existed = res!(self.get(table, id)).is_some();
		if !existed {
			return Ok(false);
		}
		// Overwrite the key with a tombstone via the ordinary `store`
		// path. Going through `store` rather than
		// `delete_using_responder` means the tombstone flows through
		// the same encryption and cache pipeline as a regular write,
		// so subsequent `get` calls see the tombstone on the first
		// read rather than racing the deletion path's unencrypted
		// direct-to-disk write.
		let key = Self::encode_key(table, id);
		let tombstone = Dat::Usr(
			usr_kind_id_deleted(),
			Some(Box::new(Dat::Empty)),
		);
		let resp = res!(self.db.api().store(key, tombstone, self.user));
		res!(Self::drain_store_ack(&resp));
		Ok(true)
	}

	fn digests(&self, table: &str) -> Outcome<Vec<RecordDigest>> {
		let prefix = fmt!("{}:", table);
		let opts = ScanOpts::with_str_prefix(prefix);
		let entries = res!(self.db.api().scan(&opts, None));
		let mut out = Vec::with_capacity(entries.len());
		for (k, _v, _meta) in &entries {
			let id = res!(Self::parse_key(k, table));
			// Fetch each record so we can hash its value bytes. Scan v1
			// does not return values, hence the per-record round trip.
			let record = match res!(self.get(table, &id)) {
				Some(r) => r,
				None => continue,	// Deleted between scan and fetch.
			};
			let content = content_hash(&record.value);
			out.push(RecordDigest { id, content });
		}
		out.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));
		Ok(out)
	}
}


/// Deterministic splitmix64-widened 32-byte content hash of a byte
/// slice. Matches the hash used by
/// [`MemoryStorage`](super::storage::MemoryStorage) so that a mixed
/// cluster (in-memory peer and O3db-backed peer) reconciles via
/// anti-entropy without the two adapters disagreeing on digests.
///
/// Not cryptographic; see the module doc comment.
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


/// Converts a 4-bit nibble to its lowercase hex character.
fn hex_char(nib: u8) -> char {
	match nib {
		0..=9	=> (b'0' + nib) as char,
		10..=15	=> (b'a' + nib - 10) as char,
		_		=> '?',	// Unreachable: caller masks to 0..=15.
	}
}

/// Converts a hex character byte to its 4-bit nibble.
fn nibble(b: u8) -> Outcome<u8> {
	match b {
		b'0'..=b'9' => Ok(b - b'0'),
		b'a'..=b'f' => Ok(b - b'a' + 10),
		b'A'..=b'F' => Ok(b - b'A' + 10),
		_ => Err(err!(
			"Invalid hex character: 0x{:02x}.", b;
			Invalid, Input)),
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn key_round_trips() -> Outcome<()> {
		let id = RecordId::from_bytes([0xab; 32]);
		let dat = O3dbStorage::<16, oxedyne_fe2o3_jdat::id::IdDat<16, u128>,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::csum::ChecksumScheme>::encode_key(
			"identity", &id,
		);
		let back = res!(O3dbStorage::<16, oxedyne_fe2o3_jdat::id::IdDat<16, u128>,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::csum::ChecksumScheme>::parse_key(
			&dat, "identity",
		));
		assert_eq!(back, id);
		Ok(())
	}

	#[test]
	fn content_hash_is_deterministic() {
		assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
		assert_ne!(content_hash(b"hello"), content_hash(b"world"));
	}

	#[test]
	fn hex_roundtrip() -> Outcome<()> {
		for byte in 0u8..=255 {
			let hi = hex_char(byte >> 4);
			let lo = hex_char(byte & 0x0f);
			let back = (res!(nibble(hi as u8)) << 4) | res!(nibble(lo as u8));
			assert_eq!(back, byte);
		}
		Ok(())
	}

	#[test]
	fn parse_key_rejects_wrong_prefix() {
		let id = RecordId::from_bytes([0xcc; 32]);
		let dat = O3dbStorage::<16, oxedyne_fe2o3_jdat::id::IdDat<16, u128>,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::csum::ChecksumScheme>::encode_key(
			"identity", &id,
		);
		assert!(O3dbStorage::<16, oxedyne_fe2o3_jdat::id::IdDat<16, u128>,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::csum::ChecksumScheme>::parse_key(
			&dat, "escrow",
		).is_err());
	}
}
