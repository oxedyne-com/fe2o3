//! Shared value types for the HotStuff primitive.
//!
//! The types here are deliberately minimal and cryptography-free. A
//! [`Vote`] carries an opaque `signature` byte vector that the caller is
//! expected to produce and verify outside of this crate; the state machine
//! trusts that any vote it receives has already been checked by its caller
//! before being handed in.

use oxedyne_fe2o3_core::prelude::*;


/// A replica identifier within a cohort. `0..cohort_size` are the valid
/// values; an id at or above `cohort_size` is an error when encountered.
pub type ReplicaId = u16;

/// A view identifier. Basic HotStuff advances a view per failed leader; the
/// happy-path skeleton in this crate only ever uses a single view (1), but
/// the field is carried end-to-end so the deferred view-change work can slot
/// in without a breaking schema change.
pub type ViewId = u64;

/// The fixed hash length used by this primitive. 32 bytes accommodates
/// SHA3-256, BLAKE3 and other standard choices. The primitive does not
/// compute block hashes itself -- the caller supplies them.
pub const BLOCK_HASH_LEN: usize = 32;

/// A block hash, sized for standard 256-bit digests.
pub type BlockHash = [u8; BLOCK_HASH_LEN];


/// The three substantive phases of basic HotStuff, plus a terminal `Decide`
/// marker. Each is visited in order and does not revisit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Phase {
	/// The first round of voting; establishes that a quorum have seen the
	/// leader's proposal.
	Prepare,
	/// The second round; establishes that a quorum know a prepare QC exists.
	PreCommit,
	/// The third round; establishes that a quorum know a pre-commit QC
	/// exists. After the commit QC is broadcast as a `Decide` message every
	/// honest replica outputs the block.
	Commit,
	/// Terminal marker -- the block has been decided. Never associated with a
	/// vote or a proposal directly; callers observe it through
	/// [`crate::replica::Command::Decide`].
	Decide,
}

impl Phase {
	/// Returns the phase that follows this one, or `None` after [`Phase::Decide`].
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
#[derive(Clone, Debug)]
pub struct Proposal {
	/// The view this proposal belongs to.
	pub view:		ViewId,
	/// The phase this proposal is intended to open. Replicas reply with a
	/// [`Vote`] whose `phase` matches.
	pub phase:		Phase,
	/// The hash of the block under consideration.
	pub block_hash:	BlockHash,
	/// The block payload. `Some` on the `Prepare` proposal to seed the block
	/// into every replica; `None` on subsequent proposals within the same
	/// view (replicas already have the block cached by hash).
	pub block:		Option<Vec<u8>>,
	/// The quorum certificate justifying this proposal. `None` only for the
	/// opening `Prepare` proposal; required for `PreCommit`, `Commit` and
	/// `Decide`. Its `phase` must be the phase that immediately precedes this
	/// proposal's phase -- i.e. `Prepare` for a `PreCommit` proposal, and so
	/// on.
	pub justify:	Option<Qc>,
}


/// A replica's vote for a specific phase of a specific view over a specific
/// block hash. The signature is opaque to this primitive -- callers produce
/// and verify it.
#[derive(Clone, Debug)]
pub struct Vote {
	/// View the vote belongs to.
	pub view:		ViewId,
	/// Phase the vote is for.
	pub phase:		Phase,
	/// Block hash the vote endorses.
	pub block_hash:	BlockHash,
	/// Voting replica identifier. Must be less than `cohort_size`.
	pub voter:		ReplicaId,
	/// Opaque signature bytes produced by the caller. Ride-along data,
	/// aggregated into the resulting [`Qc`] without inspection.
	pub signature:	Vec<u8>,
}


/// A quorum certificate: at least `cohort_size - f` distinct votes at the
/// same `(view, phase, block_hash)`.
#[derive(Clone, Debug)]
pub struct Qc {
	/// View the QC belongs to.
	pub view:		ViewId,
	/// Phase the QC endorses.
	pub phase:		Phase,
	/// Block hash the QC endorses.
	pub block_hash:	BlockHash,
	/// Per-voter signatures, in ascending `voter` order. The primitive
	/// guarantees there are no duplicate voters; it does not verify the
	/// signatures -- that is the caller's job before the QC is used.
	pub signatures:	Vec<(ReplicaId, Vec<u8>)>,
}

/// A view-change message sent by a replica to the leader of the next view.
///
/// The leader of the new view aggregates a quorum of `NewView` messages and
/// picks the highest `prepare_qc` among them as the basis of the next
/// `Prepare` proposal. A replica that has never seen a prepare QC sends
/// `prepare_qc = None`.
#[derive(Clone, Debug)]
pub struct NewView {
	/// The view the sender is entering. The new leader is
	/// `leader_for(view)`; it accumulates messages whose `view` equals its
	/// own current view.
	pub view:		ViewId,
	/// Replica identifier of the sender.
	pub sender:		ReplicaId,
	/// The sender's highest prepare QC, if any.
	pub prepare_qc:	Option<Qc>,
}


impl Qc {
	/// Returns the number of distinct voters in the QC.
	pub fn voter_count(&self) -> usize {
		self.signatures.len()
	}

	/// Checks that the QC endorses the expected `(view, phase, block_hash)`
	/// triple and contains at least `quorum` distinct voters, each in the
	/// replica-id range `0..cohort_size`.
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
