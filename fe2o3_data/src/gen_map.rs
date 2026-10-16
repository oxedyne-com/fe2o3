//! A map swept by generation under a byte ceiling, the shape of an incremental compiler's cache.
//!
//! A compile calls [`GenMap::begin`], reads and inserts, then calls [`GenMap::sweep`]. A read marks an
//! entry as touched in the generation, and the sweep drops an entry that neither of the last two
//! generations touched, so the map holds at most the working sets of the last two compiles.
//!
//! An insert that would take the held bytes over the ceiling is refused, and nothing is evicted to make
//! room: an entry the running compile may still read is never dropped under it. The caller states each
//! entry's size, since the map cannot measure a value; a map whose entries are not worth counting states
//! zero and is given no ceiling ([`GenMap::unbounded`]).

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;


#[derive(Clone, Debug)]
struct Slot<V> {
	val:	V,
	size:	usize,
	last:	u64,	// The generation that last touched the entry.
}

#[derive(Clone, Debug)]
pub struct GenMap<K, V> {
	map:		HashMap<K, Slot<V>>,
	epoch:		u64,
	bytes:		usize,		// The sizes of the entries held.
	ceiling:	usize,
	refused:	u64,		// Inserts refused at the ceiling since the map was made.
}

// An unbounded map is the default, for a holder that derives its own.
impl<K, V> Default for GenMap<K, V> {
	fn default() -> Self { Self::unbounded() }
}

impl<K, V> GenMap<K, V> {
	/// A map that holds at most `ceiling` bytes of entries.
	pub fn new(ceiling: usize) -> Self {
		Self {
			map:		HashMap::new(),
			epoch:		0,
			bytes:		0,
			ceiling,
			refused:	0,
		}
	}

	/// A map with no ceiling, swept by generation alone.
	pub fn unbounded() -> Self { Self::new(usize::MAX) }

	/// Opens a generation. Entries read or inserted from now on are touched in it.
	pub fn begin(&mut self) { self.epoch = self.epoch.wrapping_add(1); }

	/// Drops every entry that neither this generation nor the one before touched, and answers how many
	/// went. Called at the end of a compile, it leaves the entries of the last two compiles.
	pub fn sweep(&mut self) -> usize {
		let epoch		= self.epoch;
		let before		= self.map.len();
		let mut freed	= 0usize;
		self.map.retain(|_, s| {
			let keep = epoch.wrapping_sub(s.last) < 2;
			if !keep {
				freed += s.size;
			}
			keep
		});
		self.bytes -= freed;
		before - self.map.len()
	}

	/// Drops every entry. The generation, the ceiling and the count of refusals stand.
	pub fn clear(&mut self) {
		self.map.clear();
		self.bytes = 0;
	}

	pub fn len(&self) -> usize { self.map.len() }

	pub fn is_empty(&self) -> bool { self.map.is_empty() }

	pub fn bytes(&self) -> usize { self.bytes }

	pub fn ceiling(&self) -> usize { self.ceiling }

	/// Sets the ceiling. A ceiling below the bytes held evicts nothing: inserts are refused until sweeps
	/// have brought the map under it.
	pub fn set_ceiling(&mut self, ceiling: usize) { self.ceiling = ceiling; }

	pub fn refused(&self) -> u64 { self.refused }

	pub fn epoch(&self) -> u64 { self.epoch }
}

impl<K: Eq + Hash, V> GenMap<K, V> {
	/// Reads an entry and marks it touched in this generation.
	pub fn get<Q>(&mut self, key: &Q) -> Option<&V>
	where
		K: Borrow<Q>,
		Q: Eq + Hash + ?Sized,
	{
		let epoch = self.epoch;
		match self.map.get_mut(key) {
			Some(s)	=> {
				s.last = epoch;
				Some(&s.val)
			},
			None	=> None,
		}
	}

	/// Reads an entry without marking it, so that a look does not keep it from the sweep.
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

	/// Stores an entry of `size` bytes, touched in this generation, and answers whether it was stored. An
	/// entry that would take the map over its ceiling is refused, and one that replaces another is refused
	/// when the replacement would. A refused replacement leaves the old entry in place and touched.
	pub fn insert(&mut self, key: K, val: V, size: usize) -> bool {
		let epoch	= self.epoch;
		let held	= self.map.get(&key).map_or(0, |s| s.size);
		let next	= (self.bytes - held).checked_add(size).filter(|n| *n <= self.ceiling);
		match next {
			Some(n)	=> {
				self.map.insert(key, Slot { val, size, last: epoch });
				self.bytes = n;
				true
			},
			None	=> {
				if let Some(s) = self.map.get_mut(&key) {
					s.last = epoch;
				}
				self.refused += 1;
				false
			},
		}
	}
}
