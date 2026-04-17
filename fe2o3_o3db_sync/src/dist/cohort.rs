//! Deterministic HotStuff cohort selection for cohort-backed tables.
//!
//! Cohort-backed tables ([`Consistency::Cohort`][c]) serialise writes through
//! a HotStuff consensus cohort of size `lambda`. Membership is decided
//! deterministically: every peer computes the same cohort for the same
//! `(table_name, record_id)` pair without exchanging any selection messages.
//!
//! The selection rule uses the existing XOR-distance primitive: mix the table
//! name into the record id to produce a `NodeId`-shaped seed, and take the
//! `lambda` closest peers from the peer set (including the local peer) by
//! XOR distance to the seed. This keeps cohorts tightly clustered in the
//! identifier space -- a property the spec calls out as desirable for
//! locality of future reads -- while still spreading membership uniformly
//! across the network under a well-mixed hash.
//!
//! # Leader rotation
//!
//! Within a cohort, the leader for a given HotStuff round is chosen by
//! round-robin indexing over the cohort's sorted member list. The round
//! counter is owned by the HotStuff instance itself (see
//! [`fe2o3_hotstuff`][hs]); this module provides only the *membership*
//! decision and the *initial* leader (round zero) for convenience.
//!
//! [c]: crate::config::Consistency::Cohort
//! [hs]: https://github.com/oxedyne-io/fe2o3/tree/main/fe2o3_hotstuff

use super::peer_set::PeerSet;
use super::record::RecordId;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_kademlia::id::NodeId;


/// A cohort's membership for a specific record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cohort {
	/// The cohort members in ascending XOR-distance order from the seed.
	/// First member is the closest to the seed (and the initial leader).
	/// Length equals `min(lambda, peers.len() + 1)`.
	pub members:			Vec<NodeId>,
	/// `true` if the local peer is one of the cohort members.
	pub local_is_member:	bool,
	/// `true` if the local peer is the initial (round-zero) leader.
	pub local_is_leader:	bool,
	/// The initial (round-zero) leader.
	pub leader:				NodeId,
}

impl Cohort {
	/// The number of members, clamped to the available peer count.
	pub fn size(&self) -> usize {
		self.members.len()
	}
}


/// Selects the HotStuff cohort for a given `(table_name, record_id)` pair.
///
/// The cohort is the `lambda` peers closest in XOR distance to the seed
/// `H(table_name) XOR record_id`, where `H(table_name)` is the deterministic
/// 32-byte splitmix64-derived hash of the table name used by
/// [`TableConfig::iblt_seed`][ts] (broadened to 32 bytes). The local peer is
/// considered a candidate; ties break by [`NodeId`] byte ordering.
///
/// `lambda` is the cohort size -- `{5, 7, 9}` in the spec. This function
/// does not validate the range (that sits on
/// [`TableConfig::new`][tc]); callers pass whatever `lambda` their table
/// config declared.
///
/// Returns an empty cohort if `lambda == 0`, which corresponds to the
/// degenerate "no consensus" case.
///
/// [ts]: crate::config::TableConfig::iblt_seed
/// [tc]: crate::config::TableConfig::new
pub fn select(
	table_name:	&str,
	record_id:	&RecordId,
	peer_set:	&PeerSet,
	local_id:	&NodeId,
	lambda:		u64,
)
	-> Outcome<Cohort>
{
	if lambda == 0 {
		let leader = *local_id;
		return Ok(Cohort {
			members:			Vec::new(),
			local_is_member:	false,
			local_is_leader:	false,
			leader,
		});
	}
	let seed = seed_for(table_name, record_id);

	// Build the candidate list: local peer plus the peer set. Compute XOR
	// distance to the seed for each and sort ascending.
	let mut candidates: Vec<(NodeId, NodeId)> = Vec::with_capacity(
		peer_set.len() + 1,
	);
	candidates.push((*local_id, local_id.distance(&seed).0.into_node_id()));
	for p in peer_set.as_slice() {
		candidates.push((*p, p.distance(&seed).0.into_node_id()));
	}
	// Sort by distance, tie-break on NodeId byte order for determinism.
	candidates.sort_by(|a, b| {
		a.1.as_bytes().cmp(b.1.as_bytes())
			.then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
	});

	let take = (lambda as usize).min(candidates.len());
	let members: Vec<NodeId> = candidates.into_iter()
		.take(take)
		.map(|(n, _)| n)
		.collect();

	let leader = res!(members.first().copied().ok_or_else(|| err!(
		"cohort selection produced no members despite lambda > 0"; Bug)));
	let local_is_member = members.iter().any(|n| n == local_id);
	let local_is_leader = leader == *local_id;
	Ok(Cohort {
		members,
		local_is_member,
		local_is_leader,
		leader,
	})
}


/// Deterministic 32-byte seed from the table name XOR the record id.
///
/// The table-name contribution uses the same splitmix64-based mixing that
/// [`TableConfig::iblt_seed`][ts] does, broadened from 64 bits to a full
/// 256 bits by successive mixing so the seed lives in the same identifier
/// space as the record id.
///
/// [ts]: crate::config::TableConfig::iblt_seed
fn seed_for(table_name: &str, record_id: &RecordId) -> NodeId {
	let mut state: u64 = 0x9E3779B97F4A7C15;
	for byte in table_name.as_bytes() {
		state = state.wrapping_add(*byte as u64);
		state = (state ^ (state >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
		state = (state ^ (state >> 27)).wrapping_mul(0x94D049BB133111EB);
		state ^= state >> 31;
	}
	let mut table_hash = [0u8; 32];
	for i in 0..4 {
		state = state.wrapping_mul(0x9E3779B97F4A7C15 ^ (i as u64 + 1));
		table_hash[i * 8..(i + 1) * 8].copy_from_slice(&state.to_le_bytes());
	}
	let mut seed = [0u8; 32];
	for i in 0..32 {
		seed[i] = table_hash[i] ^ record_id.as_bytes()[i];
	}
	NodeId::from_bytes(seed)
}


/// Sealed extension trait so we can convert a [`Distance`][d] into a
/// [`NodeId`] for comparison chaining. [`Distance`] already has `Ord`, but
/// the cohort-selection sort wants [`NodeId`] byte order as a tiebreaker
/// and re-wrapping is cleaner than duplicating the comparator.
///
/// [d]: oxedyne_fe2o3_kademlia::id::Distance
trait IntoNodeId {
	fn into_node_id(self) -> NodeId;
}

impl IntoNodeId for [u8; 32] {
	fn into_node_id(self) -> NodeId {
		NodeId::from_bytes(self)
	}
}
