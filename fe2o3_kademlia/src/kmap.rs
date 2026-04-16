//! A single Kademlia k-map -- one bucket of the routing table.
//!
//! Each [`KMap`] stores up to `k` contacts ordered from most- to
//! least-recently-seen. On touch a contact moves to the front; on overflow the
//! LRU at the tail becomes the eviction candidate. Replacement is
//! LRU-biased: a live LRU is retained (the incoming contact is discarded) and
//! only a confirmed-dead LRU is evicted. The bias reduces churn and raises
//! the cost of eclipse attacks.

use crate::{
	contact::Contact,
	id::NodeId,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::VecDeque;


/// The outcome of attempting to insert a [`Contact`] into a [`KMap`].
#[derive(Clone, Debug)]
pub enum InsertOutcome {
	/// The contact is new and was placed at the front of the k-map.
	Inserted,
	/// The contact was already present; its existing entry was moved to the
	/// front and its mutable metadata (`last_seen`, `rtt`, `capabilities`,
	/// `addresses`) was overwritten from the incoming copy.
	Refreshed,
	/// The k-map is full. The contained [`Contact`] is the current LRU and
	/// should be probed by the caller. On a live response call
	/// [`KMap::keep_lru`]; on a confirmed-dead response call
	/// [`KMap::evict_and_insert`] with the new contact.
	Full {
		/// The LRU that stands in the way of the new contact.
		candidate:	Contact,
		/// The new contact that prompted the overflow, to be re-supplied to
		/// [`KMap::evict_and_insert`] once the LRU is confirmed dead.
		pending:	Contact,
	},
}


/// A single Kademlia k-map holding up to `k` contacts.
///
/// The front of the internal deque is the most-recently-seen contact; the
/// back is the least-recently-seen (the eviction candidate). Iteration order
/// is MRU first.
#[derive(Clone, Debug)]
pub struct KMap {
	/// The bucket's capacity.
	k:			usize,
	/// The bucket's entries, MRU at the front, LRU at the back.
	entries:	VecDeque<Contact>,
}

impl KMap {
	/// Builds an empty k-map with capacity `k`.
	pub fn new(k: usize) -> Outcome<Self> {
		if k == 0 {
			return Err(err!(
				"KMap capacity k must be greater than zero.";
			Invalid, Input));
		}
		Ok(Self {
			k,
			entries: VecDeque::with_capacity(k),
		})
	}

	/// The bucket's capacity `k`.
	pub fn capacity(&self) -> usize {
		self.k
	}

	/// The number of contacts currently held.
	pub fn len(&self) -> usize {
		self.entries.len()
	}

	/// Returns `true` if the bucket holds no contacts.
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	/// Returns `true` if the bucket is at capacity.
	pub fn is_full(&self) -> bool {
		self.entries.len() >= self.k
	}

	/// Iterates contacts in MRU-first order.
	pub fn iter(&self) -> impl Iterator<Item = &Contact> {
		self.entries.iter()
	}

	/// Attempts to insert a contact.
	///
	/// Behaviour by case:
	///
	/// - Not present, bucket has room: inserted at the front, returns
	///   [`InsertOutcome::Inserted`].
	/// - Already present: existing entry refreshed and moved to the front,
	///   returns [`InsertOutcome::Refreshed`].
	/// - Not present, bucket full: returns
	///   [`InsertOutcome::Full`] with the current LRU as the eviction
	///   candidate and the incoming contact to re-apply once liveness of the
	///   LRU is known.
	pub fn insert(&mut self, contact: Contact) -> InsertOutcome {
		if let Some(pos) = self.position(&contact.node_id) {
			// Refresh: overwrite metadata and move to front.
			if let Some(mut existing) = self.entries.remove(pos) {
				existing.addresses		= contact.addresses;
				existing.last_seen		= contact.last_seen;
				existing.rtt			= contact.rtt;
				existing.capabilities	= contact.capabilities;
				self.entries.push_front(existing);
			}
			return InsertOutcome::Refreshed;
		}
		if self.is_full() {
			// Copy the LRU out as the eviction candidate without mutating.
			let candidate = match self.entries.back() {
				Some(c) => c.clone(),
				None => {
					// Unreachable: is_full implies non-empty.
					self.entries.push_front(contact);
					return InsertOutcome::Inserted;
				},
			};
			return InsertOutcome::Full { candidate, pending: contact };
		}
		self.entries.push_front(contact);
		InsertOutcome::Inserted
	}

	/// Confirms the LRU is still live after an external probe.
	///
	/// Moves the contact at the tail to the front (so it becomes the
	/// most-recently-seen), updates its `last_seen` and discards any pending
	/// contact the caller was holding. No-op on an empty bucket.
	pub fn keep_lru(&mut self, now: u64) {
		if let Some(mut lru) = self.entries.pop_back() {
			lru.touch(now);
			self.entries.push_front(lru);
		}
	}

	/// Replaces the LRU with `new` after an external probe confirmed the LRU
	/// is dead.
	///
	/// The dead contact is dropped, `new` is inserted at the front. Returns
	/// the evicted contact.
	pub fn evict_and_insert(&mut self, new: Contact) -> Option<Contact> {
		let evicted = self.entries.pop_back();
		self.entries.push_front(new);
		evicted
	}

	/// Removes a contact by id, if present, and returns it.
	pub fn remove(&mut self, id: &NodeId) -> Option<Contact> {
		let pos = self.position(id)?;
		self.entries.remove(pos)
	}

	/// Returns a reference to the contact with the given id, if present.
	pub fn get(&self, id: &NodeId) -> Option<&Contact> {
		self.entries.iter().find(|c| c.node_id == *id)
	}

	/// Records a liveness observation by moving an existing contact to the
	/// front. Returns `true` if the contact was present and refreshed.
	pub fn touch(&mut self, id: &NodeId, now: u64) -> bool {
		let Some(pos) = self.position(id) else { return false; };
		if let Some(mut c) = self.entries.remove(pos) {
			c.touch(now);
			self.entries.push_front(c);
			return true;
		}
		false
	}

	fn position(&self, id: &NodeId) -> Option<usize> {
		self.entries.iter().position(|c| c.node_id == *id)
	}
}
