//! A least-recently-used map under a byte ceiling.
//!
//! Where [`GenMap`](crate::gen_map::GenMap) refuses an insert at its ceiling, so that a running
//! compile never loses an entry it may still read, this map makes room: an insert evicts the entries
//! read or written longest ago until the new one fits. It suits a read cache of decoded values in
//! front of a store, where any entry can be decoded again at a cost and the bound is on memory.
//!
//! The caller states each entry's size, since the map cannot measure a value. An entry larger than
//! the whole ceiling is not stored, and evicts nothing.

use std::borrow::Borrow;
use std::collections::{
	BTreeMap,
	HashMap,
};
use std::hash::Hash;


#[derive(Clone, Debug)]
struct Slot<V> {
	val:	V,
	size:	usize,
	tick:	u64,	// when it was last read or written
}

#[derive(Clone, Debug)]
pub struct SizedLru<K, V> {
	map:		HashMap<K, Slot<V>>,
	order:		BTreeMap<u64, K>,	// tick to key, oldest first
	tick:		u64,
	bytes:		usize,			// the sizes of the entries held
	ceiling:	usize,
	evicted:	u64,			// entries evicted since the map was made
}

impl<K, V> SizedLru<K, V> {
	/// A map that holds at most `ceiling` bytes of entries.
	pub fn new(ceiling: usize) -> Self {
		Self {
			map:		HashMap::new(),
			order:		BTreeMap::new(),
			tick:		0,
			bytes:		0,
			ceiling,
			evicted:	0,
		}
	}

	pub fn len(&self) -> usize { self.map.len() }

	pub fn is_empty(&self) -> bool { self.map.is_empty() }

	pub fn bytes(&self) -> usize { self.bytes }

	pub fn ceiling(&self) -> usize { self.ceiling }

	pub fn evicted(&self) -> u64 { self.evicted }

	/// Drops every entry. The ceiling and the count of evictions stand.
	pub fn clear(&mut self) {
		self.map.clear();
		self.order.clear();
		self.bytes = 0;
	}
}

impl<K: Eq + Hash + Clone, V> SizedLru<K, V> {
	/// Reads an entry and makes it the most recently used.
	pub fn get<Q>(&mut self, key: &Q) -> Option<&V>
	where
		K: Borrow<Q>,
		Q: Eq + Hash + ?Sized,
	{
		self.tick = self.tick.wrapping_add(1);
		let tick = self.tick;
		match self.map.get_mut(key) {
			Some(s)	=> {
				if let Some(k) = self.order.remove(&s.tick) {
					self.order.insert(tick, k);
				}
				s.tick = tick;
				Some(&s.val)
			},
			None	=> None,
		}
	}

	/// Reads an entry without making it recent.
	pub fn peek<Q>(&self, key: &Q) -> Option<&V>
	where
		K: Borrow<Q>,
		Q: Eq + Hash + ?Sized,
	{
		self.map.get(key).map(|s| &s.val)
	}

	pub fn contains<Q>(&self, key: &Q) -> bool
	where
		K: Borrow<Q>,
		Q: Eq + Hash + ?Sized,
	{
		self.map.contains_key(key)
	}

	pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
	where
		K: Borrow<Q>,
		Q: Eq + Hash + ?Sized,
	{
		match self.map.remove(key) {
			Some(s)	=> {
				self.order.remove(&s.tick);
				self.bytes -= s.size;
				Some(s.val)
			},
			None	=> None,
		}
	}

	/// Stores an entry of `size` bytes as the most recently used, evicting the least recently used
	/// until it fits, and answers whether it was stored. One larger than the ceiling is not stored
	/// and evicts nothing, though it still replaces, by removal, an entry under the same key.
	pub fn insert(&mut self, key: K, val: V, size: usize) -> bool {
		self.remove(&key);
		if size > self.ceiling {
			return false;
		}
		while self.bytes.saturating_add(size) > self.ceiling {
			let oldest = match self.order.pop_first() {
				Some((_, k))	=> k,
				None		=> break,	// unreachable while bytes count only held entries
			};
			if let Some(s) = self.map.remove(&oldest) {
				self.bytes -= s.size;
				self.evicted += 1;
			}
		}
		self.tick = self.tick.wrapping_add(1);
		self.order.insert(self.tick, key.clone());
		self.map.insert(key, Slot { val, size, tick: self.tick });
		self.bytes += size;
		true
	}
}
