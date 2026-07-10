//! The OAM placement decisions that consume the threshold.
//!
//! The three functions here answer the three placement questions a peer asks:
//!
//! - Am I, specifically, a holder of this record? ([`is_holder`])
//! - Which peers, from a known set, are holders of this record? ([`holders`])
//! - Which peers are closest to this record, regardless of the threshold, so
//!   that a read request can be issued even when my view of the threshold
//!   disagrees with theirs? ([`closest_holders`])
//!
//! All three reduce to XOR distance comparisons between 256-bit identifiers.
//! None of them take locks, issue I/O, or spawn tasks.

use super::threshold::Threshold;

use crate::kademlia::id::NodeId;


/// Returns `true` if the peer holds the record under the given threshold.
///
/// Evaluation is a single XOR-distance computation followed by a bytewise
/// comparison against the stored 256-bit threshold.
pub fn is_holder(
	peer_id:		&NodeId,
	record_hash:	&NodeId,
	threshold:		&Threshold,
)
	-> bool
{
	let d = peer_id.distance(record_hash);
	threshold.contains(&d)
}

/// Returns references to those peers, from the given slice, that hold the
/// record under the given threshold.
///
/// The return order mirrors the input order. Duplicates in the input produce
/// duplicates in the output: callers that keep a canonical peer set should
/// deduplicate before calling in.
pub fn holders<'a>(
	record_hash:	&NodeId,
	peers:			&'a [NodeId],
	threshold:		&Threshold,
)
	-> Vec<&'a NodeId>
{
	match threshold {
		Threshold::None => Vec::new(),
		_ => peers.iter()
			.filter(|p| is_holder(p, record_hash, threshold))
			.collect(),
	}
}

/// Returns references to the `count` peers closest to the record hash by
/// XOR distance, regardless of the OAM threshold.
///
/// Useful when the local peer is not itself a holder but needs to read the
/// record: the closest peers are, under a well-mixed hash, the ones most
/// likely to consider themselves holders -- even when the local view of `N`
/// disagrees by a small margin with theirs.
///
/// If `peers.len()` is less than `count`, every peer is returned, still
/// sorted from closest to furthest.
pub fn closest_holders<'a>(
	record_hash:	&NodeId,
	peers:			&'a [NodeId],
	count:			usize,
)
	-> Vec<&'a NodeId>
{
	if count == 0 || peers.is_empty() {
		return Vec::new();
	}
	let mut indexed: Vec<(_, &NodeId)> = peers.iter()
		.map(|p| (p.distance(record_hash), p))
		.collect();
	indexed.sort_by(|a, b| a.0.cmp(&b.0));
	indexed.into_iter()
		.take(count)
		.map(|(_, p)| p)
		.collect()
}
