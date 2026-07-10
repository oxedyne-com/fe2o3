//! The placement service.
//!
//! [`Placement`] is the glue between [`OamConfig`] and [`PeerSet`]: it
//! answers the three placement questions of distributed Ozone without
//! involving transport or storage.
//!
//! The service holds a cached [`Threshold`] computed from the current
//! `OamConfig`. Recomputation happens only on explicit configuration change
//! (e.g. when the HyperLogLog estimate revises `N`). Every placement decision
//! therefore costs one XOR-distance computation plus a bytewise comparison.
//!
//! # Read-through routing
//!
//! When the local peer is *not* a holder of a record it wants to read, it
//! picks from the peers nearest the record hash regardless of the local
//! threshold -- those are, under a well-mixed hash, the ones most likely to
//! consider themselves holders even if the local view of `N` differs slightly
//! from theirs. [`Placement::read_targets`] returns this list.
//!
//! [`OamConfig`]: crate::oam::config::OamConfig
//! [`Threshold`]: crate::oam::threshold::Threshold

use super::peer_set::PeerSet;
use super::record::RecordId;

use oxedyne_fe2o3_core::prelude::*;
use crate::kademlia::id::NodeId;
use crate::oam::{
	config::OamConfig,
	placement as oam,
	threshold::Threshold,
};


/// The placement service consulted on every write and every non-local read.
#[derive(Clone, Debug)]
pub struct Placement {
	local_peer_id:	NodeId,
	oam:			OamConfig,
	threshold:		Threshold,
}

impl Placement {
	/// Constructs a placement service for the given local peer and OAM
	/// configuration, precomputing the 256-bit threshold.
	pub fn new(local_peer_id: NodeId, oam: OamConfig) -> Self {
		let threshold = oam.threshold();
		Self { local_peer_id, oam, threshold }
	}

	/// Returns the local peer identifier.
	pub fn local_peer_id(&self) -> &NodeId {
		&self.local_peer_id
	}

	/// Returns the current OAM configuration.
	pub fn oam(&self) -> &OamConfig {
		&self.oam
	}

	/// Returns the cached 256-bit placement threshold.
	pub fn threshold(&self) -> &Threshold {
		&self.threshold
	}

	/// Updates the OAM configuration (typically after a HyperLogLog estimate
	/// refresh revises `N`) and recomputes the cached threshold.
	pub fn update_oam(&mut self, oam: OamConfig) {
		self.threshold = oam.threshold();
		self.oam = oam;
	}

	/// Returns `true` if the local peer is a holder of the given record.
	pub fn i_am_holder(&self, record: &RecordId) -> bool {
		oam::is_holder(
			&self.local_peer_id,
			&record.as_node_id(),
			&self.threshold,
		)
	}

	/// Returns the peers (other than the local peer) that hold the record
	/// under the cached threshold. The local peer is never included in the
	/// result -- distributed Ozone handles the local replica separately.
	pub fn remote_holders<'a>(
		&self,
		record:	&RecordId,
		peers:	&'a PeerSet,
	)
		-> Vec<&'a NodeId>
	{
		oam::holders(&record.as_node_id(), peers.as_slice(), &self.threshold)
	}

	/// Returns up to `count` read targets -- the peers nearest the record
	/// hash by XOR distance, regardless of the cached threshold. Used when
	/// the local peer is not a holder and must fetch the record from the
	/// network.
	pub fn read_targets<'a>(
		&self,
		record:	&RecordId,
		peers:	&'a PeerSet,
		count:	usize,
	)
		-> Vec<&'a NodeId>
	{
		oam::closest_holders(&record.as_node_id(), peers.as_slice(), count)
	}

	/// Summarises the placement decision for a record: is the local peer a
	/// holder, which remote peers hold it, and how many targets are there
	/// in total. Convenience for call sites that need all three.
	pub fn decide<'a>(
		&self,
		record:	&RecordId,
		peers:	&'a PeerSet,
	)
		-> PlacementDecision<'a>
	{
		let local = self.i_am_holder(record);
		let remote = self.remote_holders(record, peers);
		PlacementDecision {
			local_is_holder:	local,
			remote_holders:		remote,
		}
	}
}


/// The result of a placement decision for one record.
#[derive(Clone, Debug)]
pub struct PlacementDecision<'a> {
	/// `true` if the local peer is a holder of the record.
	pub local_is_holder:	bool,
	/// The remote peers that hold the record.
	pub remote_holders:		Vec<&'a NodeId>,
}

impl<'a> PlacementDecision<'a> {
	/// The total number of holders (local + remote).
	pub fn holder_count(&self) -> usize {
		self.remote_holders.len() + usize::from(self.local_is_holder)
	}
}
