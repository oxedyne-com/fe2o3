//! The three-way merge of two keyed maps against their common base.
//!
//! Each side is compared with the base key by key, a key absent from a map
//! reading as deleted there. The answer names, for every key either side
//! touched, whether to take `theirs`, keep `ours`, or report a conflict; what to
//! do with a conflict is the caller's policy, not this module's.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use std::collections::{
	BTreeMap,
	BTreeSet,
};


/// The keys a three-way merge sorts into each outcome. A key in none of the
/// sets is unchanged on both sides.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Merge3<K: Ord> {
	pub take:		BTreeSet<K>,	// theirs changed, ours did not: take theirs (absent means delete)
	pub keep:		BTreeSet<K>,	// ours changed, theirs did not, or both made the same change
	pub conflict:	BTreeSet<K>,	// both changed, differently
}

impl<K: Ord> Merge3<K> {
	/// Did the merge finish without a conflict?
	pub fn is_clean(&self) -> bool {
		self.conflict.is_empty()
	}
}

/// Sorts every key `ours` or `theirs` changed from `base` into take, keep
/// or conflict.
pub fn merge3<K: Ord + Clone, V: PartialEq>(
	base:	&BTreeMap<K, V>,
	ours:	&BTreeMap<K, V>,
	theirs:	&BTreeMap<K, V>,
)
	-> Merge3<K>
{
	let mut out = Merge3 {
		take:		BTreeSet::new(),
		keep:		BTreeSet::new(),
		conflict:	BTreeSet::new(),
	};
	let keys: BTreeSet<&K> = base.keys()
		.chain(ours.keys())
		.chain(theirs.keys())
		.collect();
	for k in keys {
		let (b, o, t) = (base.get(k), ours.get(k), theirs.get(k));
		match (o != b, t != b) {
			(false, false)	=> {}
			(false, true)	=> { out.take.insert(k.clone()); }
			(true, false)	=> { out.keep.insert(k.clone()); }
			// Both deleting, or both writing the same value, agree.
			(true, true) if o == t	=> { out.keep.insert(k.clone()); }
			(true, true)	=> { out.conflict.insert(k.clone()); }
		}
	}
	out
}
