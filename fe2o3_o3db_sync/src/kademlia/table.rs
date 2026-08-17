//! The full Kademlia routing table.
//!
//! A [`RoutingTable`] owns 256 [`KMap`]s, one per bit of XOR distance from the
//! local node. Peer placement is deterministic -- the most-significant set bit
//! of the XOR distance between the local id and the remote id selects the
//! k-map. The table exposes insertion (with overflow handled by the caller
//! via LRU probe), removal, lookup and a `k_closest` query used by both
//! `FIND_NODE` and `FIND_CLOSEST` message-layer flows.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::{
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
	local_id:	NodeId,
	k:			usize,		// shared by every k-map
	maps:		Vec<KMap>,	// 256, indexed by XOR-distance bit position
}

impl RoutingTable {
	/// Every k-map gets capacity `k`.
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

	pub fn local_id(&self) -> &NodeId {
		&self.local_id
	}

	pub fn k(&self) -> usize {
		self.k
	}

	/// `None` where no caller follow-up is required: a new insertion, a refresh
	/// of an existing entry, or a contact that is the local node itself, which
	/// is never routed through. [`InsertOutcome::Full`] means the target k-map
	/// is full and the caller must probe the returned candidate, then call
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

	/// Call this after an external liveness probe on a candidate returned by
	/// [`InsertOutcome::Full`] succeeded. `now` becomes the refreshed
	/// `last_seen` tick.
	pub fn keep_lru(&mut self, probed: &NodeId, now: u64) -> Outcome<()> {
		let Some(idx) = self.local_id.bucket_index(probed) else {
			return Err(err!(
				"Cannot keep_lru for the local node itself.";
			Invalid, Input));
		};
		res!(self.map_mut(idx)).keep_lru(now);
		Ok(())
	}

	/// Call this after an external liveness probe on a candidate returned by
	/// [`InsertOutcome::Full`] failed. `new` is the contact that was pending.
	pub fn evict_and_insert(&mut self, new: Contact) -> Outcome<Option<Contact>> {
		let Some(idx) = self.local_id.bucket_index(&new.node_id) else {
			return Err(err!(
				"Cannot evict_and_insert for the local node itself.";
			Invalid, Input));
		};
		Ok(res!(self.map_mut(idx)).evict_and_insert(new))
	}

	pub fn remove(&mut self, id: &NodeId) -> Outcome<Option<Contact>> {
		let Some(idx) = self.local_id.bucket_index(id) else {
			return Ok(None);
		};
		Ok(res!(self.map_mut(idx)).remove(id))
	}

	pub fn get(&self, id: &NodeId) -> Outcome<Option<&Contact>> {
		let Some(idx) = self.local_id.bucket_index(id) else {
			return Ok(None);
		};
		Ok(res!(self.map(idx)).get(id))
	}

	/// Refreshes the `last_seen` of an existing contact and nothing else. False
	/// if the contact was not there.
	pub fn touch(&mut self, id: &NodeId, now: u64) -> Outcome<bool> {
		let Some(idx) = self.local_id.bucket_index(id) else {
			return Ok(false);
		};
		Ok(res!(self.map_mut(idx)).touch(id, now))
	}

	/// Across all k-maps.
	pub fn len(&self) -> usize {
		self.maps.iter().map(|m| m.len()).sum()
	}

	pub fn is_empty(&self) -> bool {
		self.maps.iter().all(|m| m.is_empty())
	}

	/// Up to `want` contacts, in ascending XOR distance from `target`.
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
		let mut tagged: Vec<(super::id::Distance, Contact)> =
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
