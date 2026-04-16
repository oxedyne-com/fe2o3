//! The rolling view of known peers.
//!
//! A [`PeerSet`] is held by every distributed-Ozone engine and consulted on
//! every placement decision. The set is updated:
//!
//! - at start-up, from the configuration's bootstrap list;
//! - at runtime, as Kademlia DHT lookups surface new contacts;
//! - at runtime, as peers are evicted after sustained unresponsiveness.
//!
//! Internally the set is a sorted vector keyed by [`NodeId`]. Sorted order
//! gives deterministic iteration -- two peers with the same membership will
//! iterate in the same order, which keeps placement decisions byte-identical
//! across peers for debugging. The insert / remove cost is `O(log n)` for the
//! search and `O(n)` for the shift, which is appropriate for the expected
//! peer counts (tens to low thousands, updated infrequently).

use oxedyne_fe2o3_kademlia::id::NodeId;


/// A rolling, sorted, deduplicated view of known peers.
///
/// The local peer is always excluded from the set -- distributed Ozone never
/// asks itself to be a holder via [`placement::holders`][crate::placement].
#[derive(Clone, Debug, Default)]
pub struct PeerSet {
	peers:	Vec<NodeId>,
}

impl PeerSet {
	/// Constructs an empty peer set.
	pub fn new() -> Self {
		Self { peers: Vec::new() }
	}

	/// Constructs a peer set from an unordered collection, excluding the
	/// local peer and deduplicating. Runs in `O(n log n)`.
	pub fn from_bootstrap(
		local_peer_id:	&NodeId,
		candidates:		impl IntoIterator<Item = NodeId>,
	)
		-> Self
	{
		let mut peers: Vec<NodeId> = candidates.into_iter()
			.filter(|p| p != local_peer_id)
			.collect();
		peers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
		peers.dedup();
		Self { peers }
	}

	/// Inserts a peer, maintaining sorted-deduplicated order. Returns `true`
	/// if the peer was added, `false` if it was already present.
	pub fn insert(&mut self, peer: NodeId) -> bool {
		match self.peers.binary_search_by(|p| p.as_bytes().cmp(peer.as_bytes())) {
			Ok(_) => false,
			Err(idx) => {
				self.peers.insert(idx, peer);
				true
			}
		}
	}

	/// Removes a peer. Returns `true` if the peer was present, `false` if
	/// it was not.
	pub fn remove(&mut self, peer: &NodeId) -> bool {
		match self.peers.binary_search_by(|p| p.as_bytes().cmp(peer.as_bytes())) {
			Ok(idx) => {
				self.peers.remove(idx);
				true
			}
			Err(_) => false,
		}
	}

	/// Returns `true` if the set contains the given peer.
	pub fn contains(&self, peer: &NodeId) -> bool {
		self.peers.binary_search_by(|p| p.as_bytes().cmp(peer.as_bytes())).is_ok()
	}

	/// Returns the peers in sorted order.
	pub fn as_slice(&self) -> &[NodeId] {
		&self.peers
	}

	/// Returns the number of peers.
	pub fn len(&self) -> usize {
		self.peers.len()
	}

	/// Returns `true` if the set is empty.
	pub fn is_empty(&self) -> bool {
		self.peers.is_empty()
	}
}
