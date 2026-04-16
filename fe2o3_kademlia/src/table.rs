//! The full Kademlia routing table.
//!
//! A [`RoutingTable`] owns 256 [`KMap`]s, one per bit of XOR distance from the
//! local node. Peer placement is deterministic -- the most-significant set bit
//! of the XOR distance between the local id and the remote id selects the
//! k-map. The table exposes insertion (with overflow handled by the caller
//! via LRU probe), removal, lookup and a `k_closest` query used by both
//! `FIND_NODE` and `FIND_CLOSEST` message-layer flows.

use crate::{
	contact::Contact,
	id::{
		ID_BITS,
		NodeId,
	},
	kmap::{
		InsertOutcome,
		KMap,
	},
};

use oxedyne_fe2o3_core::prelude::*;


/// A Kademlia routing table for a single local node.
#[derive(Clone, Debug)]
pub struct RoutingTable {
	/// The local node's identifier.
	local_id:	NodeId,
	/// Per-bucket capacity shared by every k-map.
	k:			usize,
	/// The 256 k-maps, indexed by XOR-distance bit position.
	maps:		Vec<KMap>,
}

impl RoutingTable {
	/// Constructs an empty routing table for `local_id` with every k-map
	/// having capacity `k`.
	pub fn new(local_id: NodeId, k: usize) -> Outcome<Self> {
		if k == 0 {
			return Err(err!(
				"Routing table k must be greater than zero.";
			Invalid, Input));
		}
		let mut maps = Vec::with_capacity(ID_BITS);
		for _ in 0..ID_BITS {
			maps.push(res!(KMap::new(k)));
		}
		Ok(Self { local_id, k, maps })
	}

	/// The local node's identifier.
	pub fn local_id(&self) -> &NodeId {
		&self.local_id
	}

	/// The per-bucket capacity.
	pub fn k(&self) -> usize {
		self.k
	}

	/// Attempts to insert a contact.
	///
	/// Returns `Ok(None)` if the contact is the local node itself -- which is
	/// never routed through -- or in any outcome where no caller follow-up is
	/// required (new insertion or refresh of an existing entry). Returns
	/// `Ok(Some(InsertOutcome::Full { .. }))` when the target k-map is full;
	/// the caller must probe the returned candidate and then call
	/// [`RoutingTable::keep_lru`] or [`RoutingTable::evict_and_insert`].
	pub fn insert(&mut self, contact: Contact) -> Outcome<Option<InsertOutcome>> {
		let Some(idx) = self.local_id.bucket_index(&contact.node_id) else {
			// Distance zero -- the contact is the local node. Silently
			// refuse; this is a caller-side guarantee the table protects.
			return Ok(None);
		};
		let map = res!(self.map_mut(idx));
		Ok(match map.insert(contact) {
			InsertOutcome::Inserted | InsertOutcome::Refreshed => None,
			full @ InsertOutcome::Full { .. } => Some(full),
		})
	}

	/// Confirms the LRU of the k-map responsible for `probed` is still live.
	///
	/// Called by the caller after an external liveness probe on a candidate
	/// returned by [`InsertOutcome::Full`] succeeded. `now` is supplied as
	/// the refreshed `last_seen` tick.
	pub fn keep_lru(&mut self, probed: &NodeId, now: u64) -> Outcome<()> {
		let Some(idx) = self.local_id.bucket_index(probed) else {
			return Err(err!(
				"Cannot keep_lru for the local node itself.";
			Invalid, Input));
		};
		res!(self.map_mut(idx)).keep_lru(now);
		Ok(())
	}

	/// Replaces a confirmed-dead LRU with a new contact.
	///
	/// Called by the caller after an external liveness probe on a candidate
	/// returned by [`InsertOutcome::Full`] failed. The `new` contact is the
	/// one that was originally pending.
	pub fn evict_and_insert(&mut self, new: Contact) -> Outcome<Option<Contact>> {
		let Some(idx) = self.local_id.bucket_index(&new.node_id) else {
			return Err(err!(
				"Cannot evict_and_insert for the local node itself.";
			Invalid, Input));
		};
		Ok(res!(self.map_mut(idx)).evict_and_insert(new))
	}

	/// Removes a peer from the table, returning the old record if present.
	pub fn remove(&mut self, id: &NodeId) -> Outcome<Option<Contact>> {
		let Some(idx) = self.local_id.bucket_index(id) else {
			return Ok(None);
		};
		Ok(res!(self.map_mut(idx)).remove(id))
	}

	/// Looks up a peer by id.
	pub fn get(&self, id: &NodeId) -> Outcome<Option<&Contact>> {
		let Some(idx) = self.local_id.bucket_index(id) else {
			return Ok(None);
		};
		Ok(res!(self.map(idx)).get(id))
	}

	/// Refreshes the `last_seen` of an existing contact without mutating
	/// anything else. Returns `true` if the contact was present.
	pub fn touch(&mut self, id: &NodeId, now: u64) -> Outcome<bool> {
		let Some(idx) = self.local_id.bucket_index(id) else {
			return Ok(false);
		};
		Ok(res!(self.map_mut(idx)).touch(id, now))
	}

	/// The total number of contacts across all k-maps.
	pub fn len(&self) -> usize {
		self.maps.iter().map(|m| m.len()).sum()
	}

	/// Returns `true` if the routing table holds no contacts.
	pub fn is_empty(&self) -> bool {
		self.maps.iter().all(|m| m.is_empty())
	}

	/// Returns the `want` contacts closest to `target` by XOR distance, in
	/// ascending distance order.
	///
	/// Serves both `FIND_NODE(target)` and `FIND_CLOSEST(region)` at the
	/// message layer. The underlying algorithm is the same -- only the caller
	/// context differs. Ties on distance break by MRU: contacts in the same
	/// bucket appear in MRU-first order, which is the natural iteration order
	/// of a [`KMap`].
	pub fn k_closest(&self, target: &NodeId, want: usize) -> Vec<Contact> {
		let mut out: Vec<Contact> = Vec::with_capacity(want.min(self.k));
		if want == 0 {
			return out;
		}
		// Gather every contact, tagged with its distance to the target.
		let mut tagged: Vec<(crate::id::Distance, Contact)> =
			Vec::with_capacity(self.len());
		for map in &self.maps {
			for c in map.iter() {
				let d = c.node_id.distance(target);
				tagged.push((d, c.clone()));
			}
		}
		// Sort by distance ascending; stable to preserve MRU tiebreak.
		tagged.sort_by(|a, b| a.0.cmp(&b.0));
		for (_, c) in tagged.into_iter().take(want) {
			out.push(c);
		}
		out
	}

	fn map(&self, idx: usize) -> Outcome<&KMap> {
		self.maps.get(idx).ok_or_else(|| err!(
			"Bucket index {} out of range (0..{}).", idx, ID_BITS;
		Invalid, Input, Bug))
	}

	fn map_mut(&mut self, idx: usize) -> Outcome<&mut KMap> {
		if idx >= self.maps.len() {
			return Err(err!(
				"Bucket index {} out of range (0..{}).", idx, ID_BITS;
			Invalid, Input, Bug));
		}
		Ok(&mut self.maps[idx])
	}
}
