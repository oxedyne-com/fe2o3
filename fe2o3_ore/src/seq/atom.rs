//! The content layer: the bytes each splice brought into existence.
//!
//! An atom is immutable and is never divided. Only views of it divide, and a
//! view is a [`crate::seq::slot::Slot`]. Byte `k` of the atom created by
//! operation `a` is named `a+k` for as long as the history lasts, in every file,
//! before and after any number of moves, which is the property the whole
//! structure is built on.
//!
//! # Origin anchors
//!
//! A file's creation mints an atom too: one byte, [`ORIGIN`], born dead. It never
//! renders, because [`crate::seq::claim::Dead`] buries it the moment it is made,
//! and no frontend can name it, because nothing dead appears in a render. What it
//! is for is that an empty file then has a byte in it, so a splice into an empty
//! file anchors after a byte like every other splice, and every operation without
//! exception is placed by the content it names rather than by a file it asserts.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::{
	ContentRange,
	OpId,
};
use crate::op::Op;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;


// The byte a file's origin anchor holds.  It is born dead and never renders, so
// its value is arbitrary and nothing reads it; it is here so that the atom has a
// length of one and the offset zero names something.
pub const ORIGIN: u8 = 0;


/// Every atom an operation set creates, keyed by the operation that created it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Atoms {
	map: BTreeMap<OpId, Vec<u8>>, // inserted bytes, by creating operation
}

impl Atoms {

	pub fn new() -> Self {
		Self { map: BTreeMap::new() }
	}

	/// Collects the atoms an operation set creates.
	///
	/// A splice inserting nothing creates no atom, so no identifier is spent on
	/// a pure deletion. A file's creation always creates one, of a single
	/// [`ORIGIN`] byte, which is that file's origin anchor.
	pub fn build(ops: &[(OpId, &Op)])
		-> Outcome<Self>
	{
		let mut map: BTreeMap<OpId, Vec<u8>> = BTreeMap::new();
		for (id, op) in ops {
			let made = match op {
				Op::FileCreate { .. }				=> vec![ORIGIN],
				Op::Splice { insert, .. } if !insert.is_empty()	=> insert.clone(),
				_						=> continue,
			};
			if map.insert(*id, made).is_some() {
				return Err(err!(
					"Two operations were given the identity {}; an operation \
					identity names exactly one atom.", id;
				Invalid, Input, Duplicate));
			}
		}
		Ok(Self { map })
	}

	pub fn get(&self, id: &OpId)
		-> Option<&[u8]>
	{
		self.map.get(id).map(|v| v.as_slice())
	}

	/// An operation that inserted nothing creates no atom, so its run is zero
	/// rather than absent.
	pub fn run_len(&self, id: &OpId) -> u64 {
		self.map.get(id).map(|v| v.len() as u64).unwrap_or(0)
	}

	pub fn count(&self) -> usize {
		self.map.len()
	}

	/// Bytes held, across every atom.
	pub fn total(&self) -> u64 {
		self.map.values().map(|v| v.len() as u64).sum()
	}

	/// In ascending order of creating operation.
	pub fn iter(&self)
		-> impl Iterator<Item = (&OpId, &[u8])>
	{
		self.map.iter().map(|(id, v)| (id, v.as_slice()))
	}

	/// Fails when the range names an atom the set does not hold, or reaches past
	/// the end of one it does; either means the set is not causally complete.
	pub fn slice(&self, range: &ContentRange)
		-> Outcome<&[u8]>
	{
		let atom = match self.map.get(&range.op()) {
			Some(a) => a,
			None => return Err(err!(
				"The content range {} names an atom that no operation in the set \
				created.", range;
			Invalid, Input, Missing)),
		};
		if range.to() > atom.len() as u64 {
			return Err(err!(
				"The content range {} reaches past the {} bytes its atom holds.",
				range, atom.len();
			Invalid, Input, Range));
		}
		Ok(&atom[range.from() as usize..range.to() as usize])
	}
}
