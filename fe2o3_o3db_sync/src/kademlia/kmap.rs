//! A single Kademlia k-map -- one bucket of the routing table.
//!
//! Each [`KMap`] stores up to `k` contacts ordered from most- to
//! least-recently-seen. On touch a contact moves to the front; on overflow the
//! LRU at the tail becomes the eviction candidate. Replacement is
//! LRU-biased: a live LRU is retained (the incoming contact is discarded) and
//! only a confirmed-dead LRU is evicted. The bias reduces churn and raises
//! the cost of eclipse attacks.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::{
	contact::Contact,
	id::NodeId,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::VecDeque;


#[derive(Clone, Debug)]
pub enum InsertOutcome {
	Inserted,	// placed at the front
	// The existing entry moved to the front, and its last_seen, rtt,
	// capabilities and addresses were overwritten from the incoming copy.
	Refreshed,
	// The k-map is full, and candidate is the current LRU standing in the way.
	// Probe it: call KMap::keep_lru if it answers, KMap::evict_and_insert with
	// pending if it does not.
	Full {
		candidate:	Contact,
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
	k:			usize,
	entries:	VecDeque<Contact>,	// MRU at the front, LRU at the back
}

impl KMap {
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

	pub fn capacity(&self) -> usize {
		self.k
	}

	pub fn len(&self) -> usize {
		self.entries.len()
	}

	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	pub fn is_full(&self) -> bool {
		self.entries.len() >= self.k
	}

	/// MRU-first order.
	pub fn iter(&self) -> impl Iterator<Item = &Contact> {
		self.entries.iter()
	}

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

	/// Call this once an external probe has confirmed the LRU is still live.
	/// The tail contact moves to the front and its `last_seen` is updated; any
	/// pending contact the caller was holding is discarded. No-op on an empty
	/// bucket.
	pub fn keep_lru(&mut self, now: u64) {
		if let Some(mut lru) = self.entries.pop_back() {
			lru.touch(now);
			self.entries.push_front(lru);
		}
	}

	/// Call this once an external probe has confirmed the LRU is dead. The
	/// dead contact is dropped and returned; `new` goes to the front.
	pub fn evict_and_insert(&mut self, new: Contact) -> Option<Contact> {
		let evicted = self.entries.pop_back();
		self.entries.push_front(new);
		evicted
	}

	pub fn remove(&mut self, id: &NodeId) -> Option<Contact> {
		let pos = ok!(self.position(id));
		self.entries.remove(pos)
	}

	pub fn get(&self, id: &NodeId) -> Option<&Contact> {
		self.entries.iter().find(|c| c.node_id == *id)
	}

	/// Records a liveness observation by moving an existing contact to the
	/// front. False if the contact was not there.
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
