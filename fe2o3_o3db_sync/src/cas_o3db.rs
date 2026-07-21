//! [`Cas`] adapter backed by a local [`O3db`](crate::O3db) instance.
//!
//! Plumbs the content-addressed [`Cas`] trait onto the local Ozone engine, so
//! a caller that already runs an O3db (a gateway, say) gains a chunk store
//! without a second storage system. Every chunk is one key/value pair:
//!
//! - Key: `Dat::Str("chunk:{hex_addr}")`. The `chunk:` prefix lets a `scan`
//!   with [`ScanOpts::with_str_prefix`] enumerate every chunk for
//!   [`Cas::ids`], which garbage collection needs. `{hex_addr}` is the
//!   lowercase-hex [`ContentId`].
//! - Value: `Dat::BU8(chunk.bytes)`. The address is recoverable from the key,
//!   so the value carries only the opaque chunk bytes -- ciphertext, when the
//!   caller has encrypted before storing.
//!
//! Deletes are tombstones flowing through the ordinary store path, matching
//! [`O3dbStorage`](super::dist::o3db_storage) so a subsequent `get` sees the
//! tombstone on its first read rather than racing a direct-to-disk delete. The
//! log-structured engine reclaims the space on compaction.

use crate::cas::{
	Cas,
	Chunk,
	ContentId,
};

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


/// The key prefix under which every chunk is stored. A `scan` on this prefix
/// enumerates the store.
const CHUNK_PREFIX: &str = "chunk:";


/// Content-addressed chunk store persisting chunks in a local [`O3db`].
///
/// Generic over the six O3db type parameters so a caller plugs its chosen
/// crypto, hash and checksum schemes in directly, exactly as
/// [`O3dbStorage`](super::dist::o3db_storage::O3dbStorage) does. Most callers
/// construct one at start-up with concrete types.
pub struct O3dbCas<
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
	O3dbCas<UIDL, UID, ENC, KH, PR, CS>
{
	/// Constructs a new chunk store over a shared [`O3db`] handle.
	///
	/// `user` is the caller identity every write is stamped with; the chunk
	/// store does not model per-chunk authorship beyond this.
	pub fn new(db: Arc<O3db<UIDL, UID, ENC, KH, PR, CS>>, user: UID) -> Self {
		Self { db, user }
	}

	/// Composite key format: `"chunk:{hex_addr}"`.
	fn encode_key(id: &ContentId) -> Dat {
		let mut s = String::with_capacity(CHUNK_PREFIX.len() + 64);
		s.push_str(CHUNK_PREFIX);
		s.push_str(&id.to_hex());
		Dat::Str(s)
	}

	/// Parses a key produced by [`encode_key`] back to a [`ContentId`].
	fn parse_key(dat: &Dat) -> Outcome<ContentId> {
		let s = match dat {
			Dat::Str(s) => s,
			_ => return Err(err!(
				"O3dbCas expected a Dat::Str key, got {:?}.", dat;
				Invalid, Input, Mismatch)),
		};
		if !s.starts_with(CHUNK_PREFIX) {
			return Err(err!(
				"O3dbCas key '{}' lacks the '{}' prefix.", s, CHUNK_PREFIX;
				Invalid, Input, Mismatch));
		}
		ContentId::from_hex(&s[CHUNK_PREFIX.len()..])
	}

	/// Drains the two-message acknowledgement a successful store emits.
	fn drain_store_ack(
		resp:	&crate::comm::response::Responder<UIDL, UID, ENC, KH>,
	)
		-> Outcome<()>
	{
		match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
			OzoneMsg::Chunks(_) => {},
			OzoneMsg::Error(e) => return Err(err!(e,
				"O3dbCas: store ack failed at Chunks phase.";
				IO, Write)),
			other => return Err(err!(
				"O3dbCas: unexpected message at Chunks phase: {:?}.", other;
				Unexpected, Channel)),
		}
		match res!(resp.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
			OzoneMsg::KeyExists(_) => Ok(()),
			OzoneMsg::Error(e) => Err(err!(e,
				"O3dbCas: store ack failed at KeyExists phase.";
				IO, Write)),
			other => Err(err!(
				"O3dbCas: unexpected message at KeyExists phase: {:?}.", other;
				Unexpected, Channel)),
		}
	}

	/// Extracts chunk bytes from a fetched `Dat`, handling the unsigned-bytes
	/// family (the store path always writes `Dat::BU8`).
	fn extract_bytes(dat: &Dat) -> Outcome<Vec<u8>> {
		match dat {
			Dat::BU8(b) | Dat::BU16(b) | Dat::BU32(b) | Dat::BU64(b) =>
				Ok(b.clone()),
			other => Err(err!(
				"O3dbCas expected a byte-vector Dat value, got {:?}.", other;
				Decode, Unexpected)),
		}
	}

	/// Reads a chunk's stored bytes, returning `None` for an absent key or a
	/// deletion tombstone.
	fn read(&self, id: &ContentId)
		-> Outcome<Option<Vec<u8>>>
	{
		let key = Self::encode_key(id);
		match res!(self.db.api().get_wait(&key, None)) {
			None => Ok(None),
			Some((dat, _meta)) => {
				if let Dat::Usr(kind, _) = &dat {
					if *kind == usr_kind_id_deleted() {
						return Ok(None);
					}
				}
				Ok(Some(res!(Self::extract_bytes(&dat))))
			}
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
	Cas for O3dbCas<UIDL, UID, ENC, KH, PR, CS>
{
	fn put(&self, chunk: &Chunk) -> Outcome<()> {
		if !chunk.id.verifies(&chunk.bytes) {
			return Err(err!(
				"Refusing chunk whose bytes do not hash to its address {}.",
				chunk.id;
			Invalid, Input, Mismatch));
		}
		let key = Self::encode_key(&chunk.id);
		let value = Dat::BU8(chunk.bytes.clone());
		let resp = res!(self.db.api().store(key, value, self.user));
		Self::drain_store_ack(&resp)
	}

	fn get(&self, id: &ContentId) -> Outcome<Option<Vec<u8>>> {
		self.read(id)
	}

	fn has(&self, id: &ContentId) -> Outcome<bool> {
		Ok(res!(self.read(id)).is_some())
	}

	fn delete(&self, id: &ContentId) -> Outcome<bool> {
		let existed = res!(self.read(id)).is_some();
		if !existed {
			return Ok(false);
		}
		let key = Self::encode_key(id);
		let tombstone = Dat::Usr(
			usr_kind_id_deleted(),
			Some(Box::new(Dat::Empty)),
		);
		let resp = res!(self.db.api().store(key, tombstone, self.user));
		res!(Self::drain_store_ack(&resp));
		Ok(true)
	}

	fn ids(&self) -> Outcome<Vec<ContentId>> {
		let opts = ScanOpts::with_str_prefix(CHUNK_PREFIX.to_string());
		let entries = res!(self.db.api().scan(&opts, None));
		let mut out = Vec::with_capacity(entries.len());
		for (k, _v, _meta) in &entries {
			let id = res!(Self::parse_key(k));
			// Skip a key whose value is a tombstone: scan enumerates the key
			// space, and a deleted chunk lingers until compaction.
			if res!(self.read(&id)).is_some() {
				out.push(id);
			}
		}
		Ok(out)
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	/// The chunk key round-trips through encode and parse.
	#[test]
	fn key_round_trips() -> Outcome<()> {
		let id = ContentId::of(b"a chunk of bytes");
		let dat = O3dbCas::<16, oxedyne_fe2o3_jdat::id::IdDat<16, u128>,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::csum::ChecksumScheme>::encode_key(&id);
		let back = res!(O3dbCas::<16, oxedyne_fe2o3_jdat::id::IdDat<16, u128>,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::csum::ChecksumScheme>::parse_key(&dat));
		assert_eq!(back, id);
		Ok(())
	}

	/// A key without the chunk prefix is refused.
	#[test]
	fn parse_key_rejects_wrong_prefix() {
		let dat = Dat::Str("other:deadbeef".to_string());
		assert!(O3dbCas::<16, oxedyne_fe2o3_jdat::id::IdDat<16, u128>,
			oxedyne_fe2o3_crypto::enc::EncryptionScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::hash::HashScheme,
			oxedyne_fe2o3_hash::csum::ChecksumScheme>::parse_key(&dat).is_err());
	}
}
