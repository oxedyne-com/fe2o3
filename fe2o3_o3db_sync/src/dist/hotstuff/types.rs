//! Shared value types for the HotStuff primitive.
//!
//! The types here are deliberately minimal and cryptography-free. A
//! [`Vote`] carries an opaque `signature` byte vector that the caller is
//! expected to produce and verify outside of this crate; the state machine
//! trusts that any vote it receives has already been checked by its caller
//! before being handed in.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;


/// A replica identifier within a cohort. `0..cohort_size` are the valid
/// values; an id at or above `cohort_size` is an error when encountered.
pub type ReplicaId = u16;

/// A view identifier, starting at 1. Basic HotStuff advances a view per failed
/// leader; [`Replica::on_timeout`](super::replica::Replica::on_timeout) is what
/// advances it here.
pub type ViewId = u64;

/// The fixed hash length used by this primitive. 32 bytes accommodates
/// SHA3-256, BLAKE3 and other standard choices. The primitive does not
/// compute block hashes itself -- the caller supplies them.
pub const BLOCK_HASH_LEN: usize = 32;

pub type BlockHash = [u8; BLOCK_HASH_LEN];


/// The three substantive phases of basic HotStuff, plus a terminal `Decide`
/// marker. Each is visited in order and does not revisit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Phase {
	Prepare,	// a quorum have seen the leader's proposal
	PreCommit,	// a quorum know a prepare QC exists
	Commit,		// a quorum know a pre-commit QC exists
	Decide,		// terminal, and observed through Command::Decide
}

impl Phase {
	pub fn next(self) -> Option<Self> {
		match self {
			Self::Prepare	=> Some(Self::PreCommit),
			Self::PreCommit	=> Some(Self::Commit),
			Self::Commit	=> Some(Self::Decide),
			Self::Decide	=> None,
		}
	}
}


/// A proposal broadcast by the leader to every replica at a particular phase
/// of a particular view. The first proposal of a view (phase `Prepare`)
/// carries the full block payload; subsequent proposals carry only the
/// justifying QC since the block was pinned by `Prepare`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
	pub view:		ViewId,
	pub phase:		Phase,				// a Vote replies with a matching phase
	pub block_hash:	BlockHash,
	pub block:		Option<Vec<u8>>,	// Some only on the Prepare proposal
	// None only for the opening Prepare proposal. Otherwise its phase must be
	// the one immediately preceding this proposal's phase.
	pub justify:	Option<Qc>,
}


/// A replica's vote for a specific phase of a specific view over a specific
/// block hash. The signature is opaque to this primitive -- callers produce
/// and verify it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Vote {
	pub view:		ViewId,
	pub phase:		Phase,
	pub block_hash:	BlockHash,
	pub voter:		ReplicaId,	// must be less than cohort_size
	pub signature:	Vec<u8>,	// opaque, aggregated into the Qc uninspected
}


/// A quorum certificate: at least `cohort_size - f` distinct votes at the
/// same `(view, phase, block_hash)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Qc {
	pub view:		ViewId,
	pub phase:		Phase,
	pub block_hash:	BlockHash,
	// In ascending voter order with no duplicates. The primitive does not
	// verify the signatures -- that is the caller's job.
	pub signatures:	Vec<(ReplicaId, Vec<u8>)>,
}

/// A view-change message sent by a replica to the leader of the next view.
///
/// The leader of the new view aggregates a quorum of `NewView` messages and
/// picks the highest `prepare_qc` among them as the basis of the next
/// `Prepare` proposal. A replica that has never seen a prepare QC sends
/// `prepare_qc = None`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewView {
	pub view:		ViewId,		// the view the sender is entering
	pub sender:		ReplicaId,
	pub prepare_qc:	Option<Qc>,
}


impl Qc {
	pub fn voter_count(&self) -> usize {
		self.signatures.len()
	}

	/// The QC must endorse the expected `(view, phase, block_hash)` triple and
	/// carry at least `quorum` distinct voters, each in `0..cohort_size`.
	pub fn validate(
		&self,
		view:			ViewId,
		phase:			Phase,
		block_hash:		&BlockHash,
		quorum:			usize,
		cohort_size:	usize,
	)
		-> Outcome<()>
	{
		if self.view != view {
			return Err(err!(
				"QC view mismatch: expected {}, got {}.", view, self.view;
			Invalid, Input, Mismatch));
		}
		if self.phase != phase {
			return Err(err!(
				"QC phase mismatch: expected {:?}, got {:?}.", phase, self.phase;
			Invalid, Input, Mismatch));
		}
		if &self.block_hash != block_hash {
			return Err(err!(
				"QC block hash mismatch.";
			Invalid, Input, Mismatch));
		}
		if self.signatures.len() < quorum {
			return Err(err!(
				"QC has {} signatures, need at least {}.",
				self.signatures.len(), quorum;
			Invalid, Input, Size));
		}
		// Check ascending-unique voter order and in-range ids.
		let mut last: Option<ReplicaId> = None;
		for (voter, _) in &self.signatures {
			if (*voter as usize) >= cohort_size {
				return Err(err!(
					"QC voter id {} out of range (cohort_size = {}).",
					voter, cohort_size;
				Invalid, Input));
			}
			if let Some(prev) = last {
				if *voter <= prev {
					return Err(err!(
						"QC voter ids not strictly ascending or unique.";
					Invalid, Input));
				}
			}
			last = Some(*voter);
		}
		Ok(())
	}
}
