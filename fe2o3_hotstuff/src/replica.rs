//! The per-replica state machine.
//!
//! [`Replica`] is a pure, deterministic state machine. It consumes three
//! kinds of input -- a leader-side [`Replica::propose`] call, an incoming
//! [`Proposal`], and an incoming [`Vote`] -- and emits a list of
//! [`Command`]s that the caller turns into wire sends or a terminal decide.
//! The primitive has no notion of time, network, or crypto; it does carry
//! opaque signature bytes on votes and into quorum certificates so that the
//! wire format is self-contained.
//!
//! The implementation here is the happy-path three-phase skeleton with a
//! fixed leader and a single view. View change, checkpointing and
//! locked/prepared-safety rules are deliberately deferred -- see the crate
//! doc for the roadmap note.

use crate::types::{
	BlockHash,
	Phase,
	Proposal,
	Qc,
	ReplicaId,
	ViewId,
	Vote,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{
	BTreeMap,
	HashMap,
};


/// Per-replica configuration.
#[derive(Clone, Copy, Debug)]
pub struct Config {
	/// Number of replicas in the cohort, i.e. `λ` in the spec.
	pub cohort_size:	usize,
	/// Maximum tolerated Byzantine members, i.e. `z = floor((λ - 1) / 3)` in
	/// the spec.
	pub f:				usize,
	/// This replica's identifier.
	pub self_id:		ReplicaId,
	/// The leader's identifier. Fixed for the lifetime of the current
	/// happy-path implementation; will become dynamic once view change
	/// lands.
	pub leader_id:		ReplicaId,
}

impl Config {
	/// The quorum threshold `λ - z`.
	pub fn quorum(&self) -> usize {
		self.cohort_size - self.f
	}

	/// Validates the configuration is internally consistent.
	pub fn validate(&self) -> Outcome<()> {
		if self.cohort_size == 0 {
			return Err(err!(
				"HotStuff cohort_size must be > 0.";
			Invalid, Input));
		}
		if self.cohort_size < 3 * self.f + 1 {
			return Err(err!(
				"HotStuff cohort_size ({}) must be at least 3f+1 = {}.",
				self.cohort_size, 3 * self.f + 1;
			Invalid, Input));
		}
		if (self.self_id as usize) >= self.cohort_size {
			return Err(err!(
				"HotStuff self_id {} out of range (cohort_size = {}).",
				self.self_id, self.cohort_size;
			Invalid, Input));
		}
		if (self.leader_id as usize) >= self.cohort_size {
			return Err(err!(
				"HotStuff leader_id {} out of range (cohort_size = {}).",
				self.leader_id, self.cohort_size;
			Invalid, Input));
		}
		Ok(())
	}
}


/// A command the state machine asks the caller to perform.
#[derive(Clone, Debug)]
pub enum Command {
	/// Broadcast a proposal to every replica in the cohort. Includes the
	/// leader itself -- the caller may short-circuit a self-send but must
	/// still deliver the proposal to the leader's own [`Replica::on_proposal`]
	/// so the leader records its own vote.
	BroadcastProposal(Proposal),
	/// Send a vote to a single recipient, normally the current leader.
	SendVote {
		/// Recipient replica id.
		to:		ReplicaId,
		/// The vote to deliver.
		vote:	Vote,
	},
	/// The replica has committed to a decision. The caller hands the block
	/// back to the application; the state machine becomes inert for this
	/// view.
	Decide {
		/// View in which the decision was reached.
		view:	ViewId,
		/// The decided block payload.
		block:	Vec<u8>,
	},
}


/// A pure HotStuff replica state machine (happy-path, three-phase).
pub struct Replica {
	cfg:		Config,
	/// Current view. The happy-path implementation only ever operates on
	/// view 1.
	view:		ViewId,
	/// Blocks seen by this replica, keyed by hash. Populated on
	/// `Phase::Prepare` proposals and consulted when the replica outputs a
	/// `Decide` command.
	blocks:		HashMap<BlockHash, Vec<u8>>,
	/// Leader-side vote aggregation, keyed by `(phase, block_hash)` within
	/// the current view. Votes are kept in a `BTreeMap<ReplicaId, Vote>` to
	/// guarantee ascending-unique voter order when the QC is materialised.
	gathered:	HashMap<(Phase, BlockHash), BTreeMap<ReplicaId, Vote>>,
	/// Most recent phase this replica has voted in for the current view.
	/// Prevents replay or double-voting within the happy-path.
	last_voted:	Option<Phase>,
	/// `true` after a `Decide` has been emitted for the current view.
	decided:	bool,
}

impl Replica {
	/// Constructs a replica. Validates the configuration.
	pub fn new(cfg: Config) -> Outcome<Self> {
		res!(cfg.validate());
		Ok(Self {
			cfg,
			view:		1,
			blocks:		HashMap::new(),
			gathered:	HashMap::new(),
			last_voted:	None,
			decided:	false,
		})
	}

	/// Returns the configuration.
	pub fn config(&self) -> Config {
		self.cfg
	}

	/// Returns the current view.
	pub fn view(&self) -> ViewId {
		self.view
	}

	/// Returns `true` if the replica has decided in the current view.
	pub fn has_decided(&self) -> bool {
		self.decided
	}

	/// Called by the leader to propose an initial block for the current view.
	///
	/// Must be called exactly once by the leader at the start of the view.
	/// The `block_hash` is the caller's pre-computed hash of the block --
	/// keeping the hash function external is consistent with the rest of
	/// Hematite's primitives.
	///
	/// Returns a single [`Command::BroadcastProposal`] with a `Prepare`
	/// proposal carrying the block payload.
	pub fn propose(&mut self, block: Vec<u8>, block_hash: BlockHash) -> Outcome<Vec<Command>> {
		if self.cfg.self_id != self.cfg.leader_id {
			return Err(err!(
				"Only the leader may call propose (self_id = {}, leader_id = {}).",
				self.cfg.self_id, self.cfg.leader_id;
			Invalid, Order));
		}
		if self.decided {
			return Err(err!(
				"propose called after Decide; the view is inert.";
			Invalid, Order));
		}
		// Cache the block under its hash so later phases can produce Decide
		// with the full payload.
		self.blocks.insert(block_hash, block.clone());
		let proposal = Proposal {
			view:		self.view,
			phase:		Phase::Prepare,
			block_hash,
			block:		Some(block),
			justify:	None,
		};
		Ok(vec![Command::BroadcastProposal(proposal)])
	}

	/// Consumes an incoming proposal from the leader.
	///
	/// Returns the replica's response: a `SendVote` if the proposal is
	/// accepted, or nothing on a rejected proposal. The function never
	/// returns an error for a rejected proposal -- the skeleton simply drops
	/// it. Protocol-structural errors (malformed proposal, wrong view,
	/// missing justify) are surfaced as `Outcome` errors so the caller can
	/// log the offending peer.
	pub fn on_proposal(&mut self, proposal: Proposal) -> Outcome<Vec<Command>> {
		if self.decided {
			return Ok(Vec::new());
		}
		if proposal.view != self.view {
			return Err(err!(
				"Proposal view {} does not match current view {}.",
				proposal.view, self.view;
			Invalid, Input, Mismatch));
		}

		match proposal.phase {
			Phase::Prepare => {
				let block = match proposal.block {
					Some(b) => b,
					None => return Err(err!(
						"Prepare proposal must carry a block payload.";
					Invalid, Input, Missing)),
				};
				if proposal.justify.is_some() {
					return Err(err!(
						"Prepare proposal must not carry a justify QC.";
					Invalid, Input));
				}
				self.blocks.insert(proposal.block_hash, block);
			},
			Phase::PreCommit | Phase::Commit => {
				let justify = match &proposal.justify {
					Some(q) => q,
					None => return Err(err!(
						"{:?} proposal requires a justify QC.", proposal.phase;
					Invalid, Input, Missing)),
				};
				let expected_prev = match proposal.phase {
					Phase::PreCommit	=> Phase::Prepare,
					Phase::Commit		=> Phase::PreCommit,
					_					=> unreachable!(),
				};
				res!(justify.validate(
					self.view,
					expected_prev,
					&proposal.block_hash,
					self.cfg.quorum(),
					self.cfg.cohort_size,
				));
				if proposal.block.is_some() {
					return Err(err!(
						"{:?} proposal must not re-send the block payload.",
						proposal.phase;
					Invalid, Input));
				}
			},
			Phase::Decide => {
				let justify = match &proposal.justify {
					Some(q) => q,
					None => return Err(err!(
						"Decide proposal requires a Commit QC.";
					Invalid, Input, Missing)),
				};
				res!(justify.validate(
					self.view,
					Phase::Commit,
					&proposal.block_hash,
					self.cfg.quorum(),
					self.cfg.cohort_size,
				));
				// Output the decision.
				let block = match self.blocks.get(&proposal.block_hash) {
					Some(b) => b.clone(),
					None => return Err(err!(
						"Decide proposal references unknown block hash.";
					Invalid, Input, Missing)),
				};
				self.decided = true;
				return Ok(vec![Command::Decide { view: self.view, block }]);
			},
		}

		// Refuse to double-vote within the same phase.
		if let Some(p) = self.last_voted {
			if p == proposal.phase || self.phase_rank(p) >= self.phase_rank(proposal.phase) {
				// Already voted for this or a later phase -- ignore the replay.
				return Ok(Vec::new());
			}
		}
		self.last_voted = Some(proposal.phase);
		let vote = Vote {
			view:		self.view,
			phase:		proposal.phase,
			block_hash:	proposal.block_hash,
			voter:		self.cfg.self_id,
			signature:	self.self_signature(proposal.phase, &proposal.block_hash),
		};
		Ok(vec![Command::SendVote { to: self.cfg.leader_id, vote }])
	}

	/// Consumes an incoming vote. Only meaningful on the leader.
	///
	/// Aggregates votes by `(phase, block_hash)` until a quorum is reached,
	/// then emits the next phase's proposal -- `PreCommit` after a `Prepare`
	/// QC, `Commit` after a `PreCommit` QC, and `Decide` after a `Commit`
	/// QC.
	pub fn on_vote(&mut self, vote: Vote) -> Outcome<Vec<Command>> {
		if self.cfg.self_id != self.cfg.leader_id {
			// Non-leaders ignore votes.
			return Ok(Vec::new());
		}
		if self.decided {
			return Ok(Vec::new());
		}
		if vote.view != self.view {
			return Err(err!(
				"Vote view {} does not match current view {}.",
				vote.view, self.view;
			Invalid, Input, Mismatch));
		}
		if (vote.voter as usize) >= self.cfg.cohort_size {
			return Err(err!(
				"Vote voter id {} out of range (cohort_size = {}).",
				vote.voter, self.cfg.cohort_size;
			Invalid, Input));
		}
		// Decide-phase votes are not collected; Decide is a broadcast, not a
		// vote round.
		if vote.phase == Phase::Decide {
			return Err(err!(
				"Decide votes are not part of the protocol.";
			Invalid, Input));
		}
		let key = (vote.phase, vote.block_hash);
		let entry = self.gathered.entry(key).or_insert_with(BTreeMap::new);
		// Idempotent on re-delivery.
		entry.insert(vote.voter, vote);
		if entry.len() < self.cfg.quorum() {
			return Ok(Vec::new());
		}

		// Quorum reached for this phase. Build the QC and open the next
		// phase.
		let (phase, block_hash) = key;
		let sigs: Vec<(ReplicaId, Vec<u8>)> = entry.iter()
			.map(|(id, v)| (*id, v.signature.clone()))
			.collect();
		let qc = Qc {
			view: self.view,
			phase,
			block_hash,
			signatures: sigs,
		};
		// Clear the bucket so a second quorum isn't double-counted if late
		// votes dribble in.
		self.gathered.remove(&key);

		let next_phase = match phase.next() {
			Some(p) => p,
			None => return Err(err!(
				"Quorum reached on terminal phase Decide -- invalid state.";
			Invalid, Order, Bug)),
		};

		// Build the next proposal.
		let proposal = Proposal {
			view:		self.view,
			phase:		next_phase,
			block_hash,
			block:		None,
			justify:	Some(qc),
		};
		Ok(vec![Command::BroadcastProposal(proposal)])
	}

	fn phase_rank(&self, p: Phase) -> u8 {
		match p {
			Phase::Prepare		=> 1,
			Phase::PreCommit	=> 2,
			Phase::Commit		=> 3,
			Phase::Decide		=> 4,
		}
	}

	/// Deterministic placeholder signature derived from the voter id and
	/// the signed content. Real deployments swap this for a proper signing
	/// key at the caller layer; the primitive never inspects the bytes.
	fn self_signature(&self, phase: Phase, block_hash: &BlockHash) -> Vec<u8> {
		let mut out = Vec::with_capacity(2 + 1 + BLOCK_HASH_LEN);
		out.extend_from_slice(&self.cfg.self_id.to_le_bytes());
		out.push(phase_tag(phase));
		out.extend_from_slice(block_hash);
		out
	}
}

/// Local import for the signature helper; keeping this close to the call
/// site avoids pulling the constant into the public types module.
use crate::types::BLOCK_HASH_LEN;

fn phase_tag(p: Phase) -> u8 {
	match p {
		Phase::Prepare		=> 1,
		Phase::PreCommit	=> 2,
		Phase::Commit		=> 3,
		Phase::Decide		=> 4,
	}
}
