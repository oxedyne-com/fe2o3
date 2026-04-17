//! Per-record HotStuff consensus state for cohort-backed tables.
//!
//! Cohort-backed tables ([`Consistency::Cohort`][c]) serialise writes through
//! a HotStuff consensus cohort. This module owns the per-`(table, record_id)`
//! state that bridges the pure [`fe2o3_hotstuff::replica::Replica`] state
//! machine to the wire: it maps between [`NodeId`] (the peer-space
//! identifier) and [`ReplicaId`] (HotStuff's cohort-local index), it encodes
//! and decodes the block payload so HotStuff can carry a full [`Record`] as
//! opaque bytes, and it hashes the block deterministically so every peer
//! agrees on the identity of what is being decided.
//!
//! The module is deliberately pure -- it holds no transport, no timers, and
//! no storage handle. The engine ([`crate::dist::DistOzone`]) drives it by
//! feeding inbound HotStuff messages in and translating the emitted
//! [`fe2o3_hotstuff::replica::Command`] list back out into
//! [`crate::transport::Envelope`]s.
//!
//! [c]: crate::config::Consistency::Cohort

use super::cohort::Cohort;
use super::hotstuff::{
	replica::{
		Config as HsConfig,
		Replica,
	},
	types::{
		BLOCK_HASH_LEN,
		BlockHash,
		ReplicaId,
	},
};
use super::record::{
	Record,
	RecordId,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_kademlia::id::NodeId;


/// The number of Byzantine members a cohort of the given size can tolerate.
///
/// `f = floor((lambda - 1) / 3)`, matching the Hematite spec and the
/// requirement that HotStuff sees `cohort_size >= 3f + 1`.
pub fn faults_tolerated(lambda: u64) -> usize {
	((lambda.saturating_sub(1)) / 3) as usize
}


/// Per-`(table, record_id)` HotStuff state bundled with the cohort membership.
///
/// One instance is created per record that goes through consensus. The
/// [`Replica`] inside drives the three-phase protocol; the cohort membership
/// is a snapshot of the deterministic selection at creation time. Members are
/// indexed `0..cohort_size` in the order produced by
/// [`cohort::select`](crate::cohort::select), so every peer's instance uses
/// the same mapping without exchanging it on the wire.
pub struct CohortInstance {
	/// The underlying HotStuff state machine.
	pub replica:	Replica,
	/// The cohort members, indexed by [`ReplicaId`]. Deterministically
	/// ordered by XOR distance to the cohort seed, matching
	/// [`Cohort::members`].
	pub members:	Vec<NodeId>,
	/// The block hash that consensus has committed (`Some` only after a
	/// successful Decide). Kept so a duplicate Decide observation is a no-op.
	pub decided_hash:	Option<BlockHash>,
}

impl CohortInstance {
	/// Constructs a new instance for a local peer that is a cohort member.
	///
	/// Rejects cohorts the local peer does not appear in -- such a peer has
	/// no role in the consensus and should never hold an instance.
	pub fn new(cohort: Cohort, local_id: &NodeId, lambda: u64) -> Outcome<Self> {
		if !cohort.local_is_member {
			return Err(err!(
				"CohortInstance requires the local peer to be a cohort member.";
				Invalid, Input, Missing));
		}
		let members = cohort.members;
		let self_id = match members.iter().position(|m| m == local_id) {
			Some(i) => i as ReplicaId,
			None => return Err(err!(
				"Local peer missing from cohort members despite \
				local_is_member = true.";
				Bug, Invalid)),
		};
		let cfg = HsConfig {
			cohort_size:	members.len(),
			f:				faults_tolerated(lambda),
			self_id,
		};
		let replica = res!(Replica::new(cfg));
		Ok(Self {
			replica,
			members,
			decided_hash:	None,
		})
	}

	/// Returns the cohort-local [`ReplicaId`] for a peer, or `None` if the
	/// peer is not a cohort member.
	pub fn replica_id(&self, node: &NodeId) -> Option<ReplicaId> {
		self.members.iter()
			.position(|m| m == node)
			.map(|i| i as ReplicaId)
	}

	/// Returns the [`NodeId`] for the given cohort-local [`ReplicaId`], or
	/// `None` if the id is out of range.
	pub fn node_id(&self, rid: ReplicaId) -> Option<NodeId> {
		self.members.get(rid as usize).copied()
	}

	/// Returns the [`NodeId`] of the leader of the current HotStuff view.
	pub fn current_leader(&self) -> Outcome<NodeId> {
		let cfg = self.replica.config();
		let leader_rid = cfg.leader_for(self.replica.view());
		match self.node_id(leader_rid) {
			Some(n) => Ok(n),
			None => Err(err!(
				"HotStuff leader id {} out of cohort range (size = {}).",
				leader_rid, self.members.len();
				Bug, Invalid)),
		}
	}

	/// Marks the instance as decided on the given block hash. Idempotent: a
	/// duplicate Decide on the same hash is accepted silently; a Decide on a
	/// different hash is a bug and is rejected.
	pub fn mark_decided(&mut self, hash: BlockHash) -> Outcome<()> {
		match self.decided_hash {
			Some(prev) if prev == hash => Ok(()),
			Some(_) => Err(err!(
				"CohortInstance decided twice on different blocks -- safety \
				violation in upstream HotStuff.";
				Bug, Invalid)),
			None => {
				self.decided_hash = Some(hash);
				Ok(())
			}
		}
	}

	/// Returns `true` if this instance has already reached Decide. New wire
	/// messages for a decided instance are dropped by the engine.
	pub fn has_decided(&self) -> bool {
		self.decided_hash.is_some()
	}
}


/// Serialises a [`Record`] into the block payload carried by HotStuff.
///
/// Wire format (little-endian throughout):
/// ```text
/// [32 bytes record id][4 bytes table_len][table_bytes][4 bytes value_len][value_bytes]
/// ```
///
/// Length fields cap at `u32::MAX`; this is enforced at decode time rather
/// than at encode time (callers produce `String` and `Vec<u8>` both of which
/// can exceed that in principle, but in practice are tiny).
pub fn encode_record(record: &Record) -> Vec<u8> {
	let table_bytes = record.table.as_bytes();
	let mut out = Vec::with_capacity(
		32 + 4 + table_bytes.len() + 4 + record.value.len(),
	);
	out.extend_from_slice(record.id.as_bytes());
	out.extend_from_slice(&(table_bytes.len() as u32).to_le_bytes());
	out.extend_from_slice(table_bytes);
	out.extend_from_slice(&(record.value.len() as u32).to_le_bytes());
	out.extend_from_slice(&record.value);
	out
}


/// Decodes a block payload produced by [`encode_record`] back into a
/// [`Record`]. Validates the declared lengths fit in the buffer.
pub fn decode_record(bytes: &[u8]) -> Outcome<Record> {
	if bytes.len() < 32 + 4 {
		return Err(err!(
			"Record block too short: {} bytes, need at least 36.",
			bytes.len();
			Invalid, Input, Size));
	}
	let mut id_bytes = [0u8; 32];
	id_bytes.copy_from_slice(&bytes[..32]);
	let id = RecordId::from_bytes(id_bytes);

	let mut cursor = 32;
	let table_len = {
		let mut buf = [0u8; 4];
		buf.copy_from_slice(&bytes[cursor..cursor + 4]);
		cursor += 4;
		u32::from_le_bytes(buf) as usize
	};
	if bytes.len() < cursor + table_len + 4 {
		return Err(err!(
			"Record block truncated: table_len {} exceeds remaining bytes.",
			table_len;
			Invalid, Input, Size));
	}
	let table = match std::str::from_utf8(&bytes[cursor..cursor + table_len]) {
		Ok(s) => s.to_string(),
		Err(_) => return Err(err!(
			"Record block table name is not valid UTF-8.";
			Invalid, Input)),
	};
	cursor += table_len;

	let value_len = {
		let mut buf = [0u8; 4];
		buf.copy_from_slice(&bytes[cursor..cursor + 4]);
		cursor += 4;
		u32::from_le_bytes(buf) as usize
	};
	if bytes.len() != cursor + value_len {
		return Err(err!(
			"Record block length mismatch: expected {} bytes after header, \
			got {}.",
			value_len, bytes.len() - cursor;
			Invalid, Input, Size));
	}
	let value = bytes[cursor..].to_vec();
	Ok(Record { id, table, value })
}


/// Deterministic 32-byte hash of a block payload.
///
/// Implementation: splitmix64-style mixing, widened to 32 bytes by mixing the
/// accumulated state through four successive multiplicative rounds. Matches
/// the hashing style used elsewhere in this crate (cohort seed derivation,
/// IBLT seed) so that the whole dist-Ozone layer remains dependency-minimal
/// and self-contained.
///
/// This hash is *deterministic* and *collision-robust against unstructured
/// input*, but is not a cryptographic hash. The HotStuff protocol itself
/// does not require cryptographic-strength block hashes -- only that every
/// honest peer computes the same hash for the same block, which splitmix64
/// satisfies. A deployment that wants collision-resistance against
/// adversarial leaders can swap this function for SHA3-256 or BLAKE3 without
/// touching the rest of the crate; the [`BlockHash`] alias is already
/// 32 bytes for both.
pub fn block_hash(block: &[u8]) -> BlockHash {
	let mut state: u64 = 0x9E3779B97F4A7C15;
	for chunk in block.chunks(8) {
		let mut buf = [0u8; 8];
		buf[..chunk.len()].copy_from_slice(chunk);
		let word = u64::from_le_bytes(buf);
		state = state.wrapping_add(word);
		state = (state ^ (state >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
		state = (state ^ (state >> 27)).wrapping_mul(0x94D049BB133111EB);
		state ^= state >> 31;
	}
	// Also mix in the length so blocks of different lengths hash differently
	// even if the trailing bytes align.
	state = state.wrapping_add(block.len() as u64);
	state = (state ^ (state >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
	state = (state ^ (state >> 27)).wrapping_mul(0x94D049BB133111EB);
	state ^= state >> 31;
	let mut out = [0u8; BLOCK_HASH_LEN];
	for i in 0..4 {
		let limb = state.wrapping_mul(0x9E3779B97F4A7C15 ^ ((i as u64) + 1));
		out[i * 8..(i + 1) * 8].copy_from_slice(&limb.to_le_bytes());
	}
	out
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn record_round_trip_through_encode_decode() -> Outcome<()> {
		let id = RecordId::from_bytes([0x42; 32]);
		let rec = Record::new(id, "identity", b"hello world".to_vec());
		let bytes = encode_record(&rec);
		let back = res!(decode_record(&bytes));
		assert_eq!(back, rec);
		Ok(())
	}

	#[test]
	fn record_with_empty_value_round_trips() -> Outcome<()> {
		let id = RecordId::from_bytes([1; 32]);
		let rec = Record::new(id, "epoch", Vec::new());
		let bytes = encode_record(&rec);
		let back = res!(decode_record(&bytes));
		assert_eq!(back, rec);
		Ok(())
	}

	#[test]
	fn decode_rejects_truncated_block() {
		assert!(decode_record(&[0u8; 10]).is_err());
	}

	#[test]
	fn decode_rejects_wrong_trailing_length() {
		let id = RecordId::from_bytes([2; 32]);
		let rec = Record::new(id, "identity", b"v".to_vec());
		let mut bytes = encode_record(&rec);
		bytes.pop();
		assert!(decode_record(&bytes).is_err());
	}

	#[test]
	fn block_hash_is_deterministic() {
		let a = block_hash(b"some block bytes");
		let b = block_hash(b"some block bytes");
		assert_eq!(a, b);
	}

	#[test]
	fn block_hash_differs_for_distinct_payloads() {
		let a = block_hash(b"some block bytes");
		let b = block_hash(b"some block byteS");
		assert_ne!(a, b);
	}

	#[test]
	fn faults_tolerated_matches_spec() {
		assert_eq!(faults_tolerated(5), 1);
		assert_eq!(faults_tolerated(7), 2);
		assert_eq!(faults_tolerated(9), 2);
	}
}
