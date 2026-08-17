//! Reconciling two logs by their difference rather than by their size.
//!
//! An invertible Bloom lookup table is a fixed-size sketch of a set. Subtract
//! one peer's sketch from another's and what remains encodes the symmetric
//! difference; peel it, and the two halves fall out -- what only they hold, and
//! what only we hold. The cost is the size of the sketch, which is chosen from
//! the expected difference, and not the size of either history. Two large logs
//! that differ by a handful of operations reconcile in a few hundred bytes.
//!
//! # The key
//!
//! A table needs a fixed-length key, and an [`OpId`] does not have one: its
//! encoding is two varints, so a name spends between two and twenty bytes
//! depending on how far the replica and the counter have got. The sketch key is
//! therefore its own spelling: sixteen bytes, the replica in eight big-endian
//! bytes and the counter in eight more. Big-endian so that the byte order of the
//! keys is the order of the identifiers, which costs nothing and makes a dumped
//! table legible.
//!
//! # The size
//!
//! Per the sizing rule of [`oxedyne_fe2o3_data::iblt`]: about one and a half
//! cells per expected difference at three hashes. The estimate is the caller's,
//! because only the caller knows how long it has been since the last exchange.
//! Below [`MIN_CELLS`] the table is padded, because one and a half cells per
//! entry is an asymptotic statement and a handful of keys in a handful of cells
//! stalls far too often to be worth the round trip.
//!
//! # When the estimate was wrong
//!
//! The peeling decoder stalls, and that is reported as [`Diff::Undecodable`]
//! with the reason, not as an error. It is not a failure of anything: the
//! estimate was a guess, the guess was low, and the frontier walk is still there
//! to answer with. What must never happen is a decode that stalled being treated
//! as a decode that finished, since the partial difference it recovered is
//! exactly the arbitrary subset that must never be sent.
//!
//! # A sketch is compared under the sender's shape
//!
//! Two tables can only be subtracted if they agree on cells, hashes, key length
//! and seed. Rather than make the peers negotiate that, a receiver builds its own
//! table under the shape the arriving one declares. Both sides do it, so two
//! peers that estimated differently still reconcile -- each answering under the
//! other's shape -- and there is no configuration to get wrong.

use crate::id::{
	OpId,
	ReplicaId,
};
use crate::log::OpLog;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::iblt::{
	DecodeOutcome,
	Iblt,
	IbltConfig,
};


/// The length of the fixed-length key an operation name takes in a sketch.
pub const KEY_LEN: usize = 16;

/// The number of hashes each key is placed under, which the sizing rule below
/// is stated at.
pub const HASHES: usize = 3;

/// The fewest cells a sketch is built with, whatever the estimate.
pub const MIN_CELLS: usize = 16;

/// The most cells a sketch may declare before a receiver answers with the walk
/// instead.
///
/// A peer that genuinely expects a difference this large wants a bulk transfer,
/// which is what the walk is, so the cap costs nothing that was worth having.
pub const MAX_CELLS: usize = 1 << 20;

/// The seed a sketch is built under where the caller has no reason to choose
/// another.
///
/// Two peers must sketch under the same seed to subtract at all, and they do:
/// the seed travels in the table's own serialised form, and a receiver adopts
/// it.
pub const SEED: u64 = 0x4f52_4553_594e_4331;


/// Why a sketch could not be turned into a difference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fallback {
	/// The peeling decoder stalled: the difference was larger than the table was
	/// sized for.
	Incomplete {
		/// Cells still holding state when peeling stopped.
		remaining:	usize,
		/// Names recovered before it stopped, which are discarded: a part of a
		/// difference is not a difference.
		recovered:	usize,
	},
	/// The table declares more cells than [`MAX_CELLS`].
	Oversized {
		/// The cell count declared.
		cells: usize,
	},
}

impl Fallback {
	/// Returns a short account of the reason, for a caller that logs one.
	pub fn why(&self) -> String {
		match self {
			Self::Incomplete { remaining, recovered } => fmt!(
				"the peeling decoder stalled with {} cell{} left, having recovered {} \
				name{}", remaining, if *remaining == 1 { "" } else { "s" },
				recovered, if *recovered == 1 { "" } else { "s" },
			),
			Self::Oversized { cells } => fmt!(
				"the sketch declares {} cells, and this reader will not allocate past \
				{}", cells, MAX_CELLS,
			),
		}
	}
}


/// What subtracting two sketches yielded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Diff {
	/// The difference, whole.
	Decoded {
		/// Operations only the remote peer holds, ascending.
		remote_only:	Vec<OpId>,
		/// Operations only this log holds, ascending, which is what is owed.
		local_only:		Vec<OpId>,
	},
	/// The difference could not be recovered, and why.
	Undecodable(Fallback),
}


/// Returns the sixteen-byte sketch key of an operation name.
pub fn key(id: &OpId) -> [u8; KEY_LEN] {
	let mut out = [0u8; KEY_LEN];
	out[..8].copy_from_slice(&id.replica.inner().to_be_bytes());
	out[8..].copy_from_slice(&id.counter.to_be_bytes());
	out
}

/// Reconstructs an operation name from a sketch key.
pub fn key_id(bytes: &[u8])
	-> Outcome<OpId>
{
	if bytes.len() != KEY_LEN {
		return Err(err!(
			"A sketch key is {} bytes, and {} arrived.", KEY_LEN, bytes.len();
		Decode, Input, Size, Mismatch));
	}
	let mut replica = [0u8; 8];
	replica.copy_from_slice(&bytes[..8]);
	let mut counter = [0u8; 8];
	counter.copy_from_slice(&bytes[8..]);
	Ok(OpId::new(
		ReplicaId::new(u64::from_be_bytes(replica)),
		u64::from_be_bytes(counter),
	))
}

/// Returns the number of cells a sketch is given for an expected difference of
/// `estimate` names: one and a half per name, and never fewer than
/// [`MIN_CELLS`].
pub fn cells_for(estimate: usize) -> usize {
	// Three halves, rounded up, without leaving the integers.
	let want = estimate.saturating_mul(3).saturating_add(1) / 2;
	want.max(MIN_CELLS).min(MAX_CELLS)
}

/// Returns the table shape for an expected difference of `estimate` names.
pub fn config(estimate: usize, seed: u64) -> IbltConfig {
	IbltConfig {
		num_cells:	cells_for(estimate),
		num_hashes:	HASHES,
		key_len:	KEY_LEN,
		value_len:	0,
		seed,
	}
}

/// Builds a sketch of every operation name the log holds.
pub fn sketch(log: &OpLog, cfg: IbltConfig)
	-> Outcome<Iblt>
{
	res!(check(&cfg));
	let mut table = res!(Iblt::new(cfg));
	for rec in log.iter() {
		res!(table.insert(&key(&rec.id()), &[]));
	}
	Ok(table)
}

/// Builds a sketch of every operation name the log holds, sized from an
/// estimate of the difference, and returns its bytes.
pub fn sketch_bytes(log: &OpLog, estimate: usize, seed: u64)
	-> Outcome<Vec<u8>>
{
	Ok(res!(sketch(log, config(estimate, seed))).to_bytes())
}

/// Subtracts this log's sketch from an arriving one and peels the result.
///
/// The arriving table's shape is adopted, so peers that estimated differently
/// still reconcile. A shape that is not a sketch of operation names at all is an
/// error; a shape that is one but too large to work with is a fallback, since
/// that is a judgement about effort rather than a malformed message.
pub fn reconcile(log: &OpLog, remote: &[u8])
	-> Outcome<Diff>
{
	let mut diff = res!(Iblt::from_bytes(remote));
	let cfg = diff.config();
	res!(check(&cfg));
	if cfg.num_cells > MAX_CELLS {
		return Ok(Diff::Undecodable(Fallback::Oversized { cells: cfg.num_cells }));
	}
	let mine = res!(sketch(log, cfg));
	res!(diff.subtract(&mine));
	match res!(diff.decode()) {
		DecodeOutcome::Complete { inserted, deleted } => {
			// Inserted is what the arriving table had and ours did not.
			let mut remote_only = res!(names(&inserted));
			let mut local_only = res!(names(&deleted));
			remote_only.sort();
			local_only.sort();
			// A name we do not hold cannot be owed by us, whatever the table said.
			local_only.retain(|id| log.contains(id));
			Ok(Diff::Decoded { remote_only, local_only })
		},
		DecodeOutcome::Incomplete { inserted, deleted, remaining_cells } =>
			Ok(Diff::Undecodable(Fallback::Incomplete {
				remaining:	remaining_cells,
				recovered:	inserted.len() + deleted.len(),
			})),
	}
}


/// Refuses a table that is not a sketch of operation names.
fn check(cfg: &IbltConfig)
	-> Outcome<()>
{
	if cfg.key_len != KEY_LEN {
		return Err(err!(
			"A sketch of operation names keys on {} bytes, and this table keys on {}.",
			KEY_LEN, cfg.key_len;
		Decode, Input, Size, Mismatch));
	}
	if cfg.value_len != 0 {
		return Err(err!(
			"A sketch of operation names carries no values, and this table carries {} \
			bytes of them per cell.", cfg.value_len;
		Decode, Input, Mismatch));
	}
	Ok(())
}

/// Turns recovered keys into operation names.
fn names(recovered: &[(Vec<u8>, Vec<u8>)])
	-> Outcome<Vec<OpId>>
{
	let mut out = Vec::with_capacity(recovered.len());
	for (k, _) in recovered {
		out.push(res!(key_id(k)));
	}
	Ok(out)
}


#[cfg(test)]
mod tests {
	use super::*;

	use crate::op::{
		Header,
		Op,
		Record,
	};

	/// An operation identifier.
	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// A log of `n` marks chained by one replica, starting at counter one.
	fn chain(replica: u64, n: u64)
		-> Outcome<OpLog>
	{
		let mut log = OpLog::new();
		let r = ReplicaId::new(replica);
		for i in 0..n {
			res!(log.author(r, Op::Mark { name: fmt!("m{}", i), body: None, time: None }));
		}
		Ok(log)
	}

	/// The key is the replica and the counter, big-endian, and round trips.
	#[test]
	fn the_key_is_sixteen_fixed_bytes() -> Outcome<()> {
		let id = oid(1, 7);
		let k = key(&id);
		assert_eq!(k.len(), KEY_LEN);
		assert_eq!(&k[..8], &[0, 0, 0, 0, 0, 0, 0, 1]);
		assert_eq!(&k[8..], &[0, 0, 0, 0, 0, 0, 0, 7]);
		for id in [oid(0, 1), oid(1, 1), oid(u64::MAX, u64::MAX), oid(300, 70_000)] {
			assert_eq!(res!(key_id(&key(&id))), id);
		}
		// Byte order is identifier order, which a varint spelling would not give.
		assert!(key(&oid(1, 2)) < key(&oid(1, 3)));
		assert!(key(&oid(1, 9)) < key(&oid(2, 1)));
		// And a key of the wrong length is refused rather than padded.
		assert!(key_id(&[0u8; 8]).is_err());
		assert!(key_id(&[0u8; 17]).is_err());
		Ok(())
	}

	/// Sizing follows the rule, with a floor under it and a ceiling over it.
	#[test]
	fn sizing_is_three_halves_of_the_estimate() -> Outcome<()> {
		assert_eq!(cells_for(0), MIN_CELLS);
		assert_eq!(cells_for(1), MIN_CELLS);
		assert_eq!(cells_for(10), MIN_CELLS, "under the floor");
		assert_eq!(cells_for(100), 150);
		assert_eq!(cells_for(101), 152, "rounded up");
		assert_eq!(cells_for(usize::MAX), MAX_CELLS, "and it never overflows");
		let cfg = config(100, SEED);
		assert_eq!(cfg.num_cells, 150);
		assert_eq!(cfg.num_hashes, HASHES);
		assert_eq!(cfg.key_len, KEY_LEN);
		assert_eq!(cfg.value_len, 0);
		Ok(())
	}

	/// Two logs that differ by a little reconcile to exactly that difference,
	/// both halves of it, whichever side is asking.
	#[test]
	fn a_small_difference_decodes_whole() -> Outcome<()> {
		// A shared prefix, then each side writes its own.
		let mut a = res!(chain(1, 20));
		let mut b = a.clone();
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		let mut only_a = Vec::new();
		let mut only_b = Vec::new();
		for i in 0..3 {
			only_a.push(res!(a.author(r1, Op::Mark { name: fmt!("a{}", i), body: None, time: None })).id());
			only_b.push(res!(b.author(r2, Op::Mark { name: fmt!("b{}", i), body: None, time: None })).id());
		}
		let from_b = res!(sketch_bytes(&b, 8, SEED));
		match res!(reconcile(&a, &from_b)) {
			Diff::Decoded { remote_only, local_only } => {
				assert_eq!(remote_only, { let mut v = only_b.clone(); v.sort(); v });
				assert_eq!(local_only, { let mut v = only_a.clone(); v.sort(); v });
			},
			Diff::Undecodable(f) => return Err(err!(
				"A difference of six decoded incompletely: {}.", f.why(); Test)),
		}
		// And the other way round, which is the same computation mirrored.
		let from_a = res!(sketch_bytes(&a, 8, SEED));
		match res!(reconcile(&b, &from_a)) {
			Diff::Decoded { remote_only, local_only } => {
				assert_eq!(remote_only, { let mut v = only_a.clone(); v.sort(); v });
				assert_eq!(local_only, { let mut v = only_b; v.sort(); v });
			},
			Diff::Undecodable(f) => return Err(err!(
				"A difference of six decoded incompletely: {}.", f.why(); Test)),
		}
		Ok(())
	}

	/// Identical logs decode to no difference at all, in either direction.
	#[test]
	fn no_difference_decodes_to_nothing() -> Outcome<()> {
		let a = res!(chain(1, 30));
		let b = a.clone();
		match res!(reconcile(&a, &res!(sketch_bytes(&b, 4, SEED)))) {
			Diff::Decoded { remote_only, local_only } => {
				assert!(remote_only.is_empty());
				assert!(local_only.is_empty());
			},
			Diff::Undecodable(f) => return Err(err!(
				"Two identical logs failed to decode: {}.", f.why(); Test)),
		}
		Ok(())
	}

	/// A table sized for far less than the difference stalls, and says so rather
	/// than handing back the part of the difference it got to.
	#[test]
	fn an_undersized_sketch_stalls_and_says_so() -> Outcome<()> {
		let a = res!(chain(1, 200));
		let b = res!(chain(2, 200));
		// Disjoint histories: four hundred names of difference, sixteen cells.
		match res!(reconcile(&a, &res!(sketch_bytes(&b, 0, SEED)))) {
			Diff::Decoded { remote_only, local_only } => return Err(err!(
				"A sketch of {} cells decoded a difference of 400, as {} and {}.",
				MIN_CELLS, remote_only.len(), local_only.len(); Test)),
			Diff::Undecodable(Fallback::Incomplete { remaining, .. }) => {
				assert!(remaining > 0);
			},
			Diff::Undecodable(other) => return Err(err!(
				"Expected a stalled decode, got {}.", other.why(); Test)),
		}
		Ok(())
	}

	/// Peers that estimated differently still reconcile, because a receiver
	/// answers under the shape it was sent.
	#[test]
	fn a_receiver_adopts_the_arriving_shape() -> Outcome<()> {
		let mut a = res!(chain(1, 40));
		let b = a.clone();
		res!(a.author(ReplicaId::new(1), Op::Mark { name: fmt!("extra"), body: None, time: None }));
		// B sketches generously; A would have sketched tightly.
		let from_b = res!(sketch_bytes(&b, 500, SEED));
		assert!(res!(Iblt::from_bytes(&from_b)).config().num_cells == 750);
		match res!(reconcile(&a, &from_b)) {
			Diff::Decoded { remote_only, local_only } => {
				assert!(remote_only.is_empty());
				assert_eq!(local_only.len(), 1);
			},
			Diff::Undecodable(f) => return Err(err!(
				"A generous sketch failed to decode: {}.", f.why(); Test)),
		}
		// A seed of the sender's choosing is adopted along with the shape.
		let odd = res!(sketch_bytes(&b, 8, 0x1234));
		assert!(matches!(res!(reconcile(&a, &odd)), Diff::Decoded { .. }));
		Ok(())
	}

	/// A table that is not a sketch of operation names is an error, and one that
	/// is but is too big to work with is a fallback.
	#[test]
	fn a_table_of_the_wrong_shape_is_refused() -> Outcome<()> {
		let log = res!(chain(1, 5));
		let wrong = IbltConfig {
			num_cells:	MIN_CELLS,
			num_hashes:	HASHES,
			key_len:	8,
			value_len:	0,
			seed:		SEED,
		};
		let table = res!(Iblt::new(wrong));
		assert!(reconcile(&log, &table.to_bytes()).is_err(), "keyed on eight bytes");
		let valued = IbltConfig { key_len: KEY_LEN, value_len: 4, ..wrong };
		let table = res!(Iblt::new(valued));
		assert!(reconcile(&log, &table.to_bytes()).is_err(), "carrying values");
		assert!(sketch(&log, valued).is_err());
		// Rubbish where a table should be.
		assert!(reconcile(&log, b"not a sketch").is_err());
		Ok(())
	}

	/// The bytes a sketch costs are set by the estimate and not by the log, which
	/// is the whole reason to send one.
	#[test]
	fn the_cost_follows_the_estimate_not_the_log() -> Outcome<()> {
		let small = res!(chain(1, 10));
		let large = res!(chain(1, 1000));
		let a = res!(sketch_bytes(&small, 8, SEED)).len();
		let b = res!(sketch_bytes(&large, 8, SEED)).len();
		assert_eq!(a, b, "a hundredfold more history, the same sketch");
		// And the per-cell cost is the key, the fingerprint and the count.
		assert_eq!(a, 40 + MIN_CELLS * (KEY_LEN + 8 + 4));
		Ok(())
	}

	/// The difference a full decode yields is causally closed against what the
	/// peer holds, because it is the complement of what both hold.
	#[test]
	fn a_decoded_difference_is_already_closed() -> Outcome<()> {
		use crate::sync::walk::close;
		use std::collections::BTreeSet;

		let mut a = OpLog::new();
		let r1 = ReplicaId::new(1);
		let r2 = ReplicaId::new(2);
		for i in 0..6 {
			res!(a.author(r1, Op::Mark { name: fmt!("shared{}", i), body: None, time: None }));
		}
		let mut b = a.clone();
		// A merge on each side, so the divergence is not a straight line.
		for i in 0..3 {
			res!(a.author(r1, Op::Mark { name: fmt!("a{}", i), body: None, time: None }));
			res!(b.author(r2, Op::Mark { name: fmt!("b{}", i), body: None, time: None }));
		}
		res!(a.append(Record::new(
			res!(Header::new(oid(1, 20), a.frontier())),
			Op::Mark { name: fmt!("merge"), body: None, time: None },
		)));
		let local_only = match res!(reconcile(&a, &res!(sketch_bytes(&b, 16, SEED)))) {
			Diff::Decoded { local_only, .. } => local_only,
			Diff::Undecodable(f) => return Err(err!(
				"The difference failed to decode: {}.", f.why(); Test)),
		};
		let held: BTreeSet<OpId> = a.iter()
			.map(|rec| rec.id())
			.filter(|id| !local_only.contains(id))
			.collect();
		let closed = close(&a, &local_only, &held);
		assert_eq!(
			closed.into_iter().collect::<Vec<_>>(), local_only,
			"closing a decoded difference added something",
		);
		Ok(())
	}
}
