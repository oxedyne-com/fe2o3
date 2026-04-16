//! The per-replica HotStuff state machine.
//!
//! [`Replica`] is a pure, deterministic state machine. It consumes a leader-
//! side [`Replica::propose`], [`Replica::on_proposal`], [`Replica::on_vote`],
//! [`Replica::on_new_view`] and [`Replica::on_timeout`] and emits a list of
//! [`Command`]s the caller turns into wire sends or a terminal decide.
//!
//! The implementation here is Basic HotStuff with:
//!
//! - Three-phase happy path: `Prepare -> PreCommit -> Commit -> Decide`.
//! - Round-robin leader rotation: `leader_for(view) = (view - 1) mod cohort_size`.
//! - View change via [`Replica::on_timeout`] + [`Replica::on_new_view`].
//! - The classical safety predicate `safeBlock`: on a Prepare proposal in
//!   view `v > 1` the replica accepts iff the proposal's justify QC either
//!   endorses the replica's locked block or is from a view strictly newer
//!   than the locked QC's view.
//! - Opaque ride-along signatures (not verified by this primitive).
//!
//! Not yet in scope: checkpointing of multiple decisions, Byzantine-fault
//! simulation tests, signature aggregation (we pass individual signatures
//! through the QC).

use crate::types::{
	BLOCK_HASH_LEN,
	BlockHash,
	NewView,
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
	/// Maximum tolerated Byzantine members, i.e. `z = floor((λ - 1) / 3)`
	/// in the spec.
	pub f:				usize,
	/// This replica's identifier.
	pub self_id:		ReplicaId,
}

impl Config {
	/// The quorum threshold `λ - z`.
	pub fn quorum(&self) -> usize {
		self.cohort_size - self.f
	}

	/// The leader of `view`, under round-robin rotation. View 1's leader is
	/// replica 0, view 2's is replica 1, and so on.
	pub fn leader_for(&self, view: ViewId) -> ReplicaId {
		let idx = (view.saturating_sub(1) as usize) % self.cohort_size;
		idx as ReplicaId
	}

	/// Returns `true` if this replica is the leader for `view`.
	pub fn is_leader_for(&self, view: ViewId) -> bool {
		self.self_id == self.leader_for(view)
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
		Ok(())
	}
}


/// A command the state machine asks the caller to perform.
#[derive(Clone, Debug)]
pub enum Command {
	/// Broadcast a proposal to every replica in the cohort (including the
	/// leader itself, so that the leader's own [`Replica::on_proposal`]
	/// records its vote consistently).
	BroadcastProposal(Proposal),
	/// Send a vote to a single recipient, normally the current leader.
	SendVote {
		/// Recipient replica id.
		to:		ReplicaId,
		/// The vote to deliver.
		vote:	Vote,
	},
	/// Send a view-change message to a single recipient, normally the
	/// incoming leader.
	SendNewView {
		/// Recipient replica id.
		to:			ReplicaId,
		/// The `NewView` payload.
		new_view:	NewView,
	},
	/// The replica has committed to a decision. The caller hands the block
	/// back to the application; the state machine becomes inert.
	Decide {
		/// View in which the decision was reached.
		view:	ViewId,
		/// The decided block payload.
		block:	Vec<u8>,
	},
}


/// A pure HotStuff replica state machine (three-phase, with view change).
pub struct Replica {
	cfg:		Config,
	/// Current view. Starts at 1; advances on [`Replica::on_timeout`].
	view:		ViewId,
	/// Blocks observed by this replica, keyed by hash. Populated when a
	/// Prepare proposal arrives and consulted when emitting a Decide.
	blocks:		HashMap<BlockHash, Vec<u8>>,
	/// Leader-side vote aggregation for the *current* view, keyed by
	/// `(phase, block_hash)`. `BTreeMap<ReplicaId, _>` keeps voters in
	/// ascending-unique order for the QC.
	gathered:	HashMap<(Phase, BlockHash), BTreeMap<ReplicaId, Vote>>,
	/// Leader-side NewView aggregation for the *current* view.
	new_views:	BTreeMap<ReplicaId, NewView>,
	/// Most recent phase this replica has voted in for the current view.
	last_voted:	Option<Phase>,
	/// Highest prepare QC this replica has observed. Carried forward across
	/// views and included in outgoing NewView messages.
	prepare_qc:	Option<Qc>,
	/// Highest locked QC (pre-commit QC) this replica has observed. Gates
	/// the safety predicate on Prepare proposals and is carried forward.
	locked_qc:	Option<Qc>,
	/// `true` after a Decide has been emitted. The state machine is inert
	/// thereafter.
	decided:	bool,
	/// `true` once this leader has opened a Prepare proposal in the current
	/// view (so a later NewView quorum doesn't re-open one).
	prepared_this_view:	bool,
}

impl Replica {
	/// Constructs a replica. Validates the configuration.
	pub fn new(cfg: Config) -> Outcome<Self> {
		res!(cfg.validate());
		Ok(Self {
			cfg,
			view:				1,
			blocks:				HashMap::new(),
			gathered:			HashMap::new(),
			new_views:			BTreeMap::new(),
			last_voted:			None,
			prepare_qc:			None,
			locked_qc:			None,
			decided:			false,
			prepared_this_view:	false,
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

	/// Returns `true` if the replica has decided.
	pub fn has_decided(&self) -> bool {
		self.decided
	}

	/// Returns a reference to the locked QC, if any.
	pub fn locked_qc(&self) -> Option<&Qc> {
		self.locked_qc.as_ref()
	}

	/// Returns a reference to the prepare QC, if any.
	pub fn prepare_qc(&self) -> Option<&Qc> {
		self.prepare_qc.as_ref()
	}

	/// Leader-only: propose an initial block for the current view.
	///
	/// Valid in view 1 (no previous state) or after a NewView quorum in
	/// view > 1 where no replica reported a prepare QC (otherwise the
	/// leader must propose the block already pinned by the highest
	/// prepare QC). Caller supplies the block bytes and hash.
	pub fn propose(&mut self, block: Vec<u8>, block_hash: BlockHash) -> Outcome<Vec<Command>> {
		if !self.cfg.is_leader_for(self.view) {
			return Err(err!(
				"Only the leader of view {} may call propose (self_id = {}).",
				self.view, self.cfg.self_id;
			Invalid, Order));
		}
		if self.decided {
			return Err(err!(
				"propose called after Decide.";
			Invalid, Order));
		}
		if self.prepared_this_view {
			return Err(err!(
				"propose called twice in view {}.", self.view;
			Invalid, Order));
		}
		self.blocks.insert(block_hash, block.clone());
		self.prepared_this_view = true;
		let proposal = Proposal {
			view:		self.view,
			phase:		Phase::Prepare,
			block_hash,
			block:		Some(block),
			justify:	None,
		};
		Ok(vec![Command::BroadcastProposal(proposal)])
	}

	/// Consumes an incoming proposal from the leader of its view.
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
				// View 1: no justify is legal.
				// View > 1: a justify is required if this replica holds a
				// locked QC (otherwise the safeBlock rule cannot be checked
				// and a locked block could be overridden by a fresh proposal).
				// A justify-less Prepare in view > 1 is legal iff no lock is
				// held -- the new leader learned via NewView that no replica
				// was locked and is starting fresh.
				if self.view == 1 {
					if proposal.justify.is_some() {
						return Err(err!(
							"Prepare proposal in view 1 must not carry a justify QC.";
						Invalid, Input));
					}
				} else {
					match &proposal.justify {
						Some(justify) => {
							if justify.phase != Phase::Prepare {
								return Err(err!(
									"Prepare proposal's justify must be a Prepare QC, got {:?}.",
									justify.phase;
								Invalid, Input));
							}
							res!(justify.validate(
								justify.view,
								Phase::Prepare,
								&justify.block_hash,
								self.cfg.quorum(),
								self.cfg.cohort_size,
							));
							if !self.safe_block(&proposal.block_hash, justify) {
								return Err(err!(
									"Prepare proposal fails safeBlock: justify view {} vs locked view {:?}.",
									justify.view,
									self.locked_qc.as_ref().map(|q| q.view);
								Security, Invalid));
							}
						},
						None => {
							if self.locked_qc.is_some() {
								return Err(err!(
									"Prepare proposal in view {} lacks a justify QC \
									but this replica is locked on view {:?} -- cannot \
									accept a fresh proposal that would abandon the lock.",
									self.view,
									self.locked_qc.as_ref().map(|q| q.view);
								Security, Invalid, Missing));
							}
						},
					}
				}
				self.blocks.insert(proposal.block_hash, block);
			},
			Phase::PreCommit => {
				let justify = match &proposal.justify {
					Some(q) => q,
					None => return Err(err!(
						"PreCommit proposal requires a Prepare QC.";
					Invalid, Input, Missing)),
				};
				res!(justify.validate(
					self.view,
					Phase::Prepare,
					&proposal.block_hash,
					self.cfg.quorum(),
					self.cfg.cohort_size,
				));
				// Record as our prepare_qc if newer.
				if self.prepare_qc.as_ref().map(|q| q.view).unwrap_or(0) < justify.view {
					self.prepare_qc = Some(justify.clone());
				}
				if proposal.block.is_some() {
					return Err(err!(
						"PreCommit proposal must not re-send the block payload.";
					Invalid, Input));
				}
			},
			Phase::Commit => {
				let justify = match &proposal.justify {
					Some(q) => q,
					None => return Err(err!(
						"Commit proposal requires a PreCommit QC.";
					Invalid, Input, Missing)),
				};
				res!(justify.validate(
					self.view,
					Phase::PreCommit,
					&proposal.block_hash,
					self.cfg.quorum(),
					self.cfg.cohort_size,
				));
				// Locking: record the pre-commit QC as locked_qc.
				if self.locked_qc.as_ref().map(|q| q.view).unwrap_or(0) < justify.view {
					self.locked_qc = Some(justify.clone());
				}
				if proposal.block.is_some() {
					return Err(err!(
						"Commit proposal must not re-send the block payload.";
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

		// Cast vote unless we've already voted in this or a later phase.
		if let Some(p) = self.last_voted {
			if self.phase_rank(p) >= self.phase_rank(proposal.phase) {
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
		let leader = self.cfg.leader_for(self.view);
		Ok(vec![Command::SendVote { to: leader, vote }])
	}

	/// Leader-only: consumes an incoming vote. Non-leaders ignore votes.
	pub fn on_vote(&mut self, vote: Vote) -> Outcome<Vec<Command>> {
		if !self.cfg.is_leader_for(self.view) {
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
		if vote.phase == Phase::Decide {
			return Err(err!(
				"Decide votes are not part of the protocol.";
			Invalid, Input));
		}
		let key = (vote.phase, vote.block_hash);
		let entry = self.gathered.entry(key).or_insert_with(BTreeMap::new);
		entry.insert(vote.voter, vote);
		if entry.len() < self.cfg.quorum() {
			return Ok(Vec::new());
		}
		let (phase, block_hash) = key;
		let sigs: Vec<(ReplicaId, Vec<u8>)> = entry.iter()
			.map(|(id, v)| (*id, v.signature.clone()))
			.collect();
		let qc = Qc { view: self.view, phase, block_hash, signatures: sigs };
		self.gathered.remove(&key);
		// Update leader's own prepare_qc / locked_qc bookkeeping as it forms
		// each QC, so subsequent views behave consistently if this replica
		// remains in the cohort after a leader handover.
		match phase {
			Phase::Prepare => {
				if self.prepare_qc.as_ref().map(|q| q.view).unwrap_or(0) < qc.view {
					self.prepare_qc = Some(qc.clone());
				}
			},
			Phase::PreCommit => {
				if self.locked_qc.as_ref().map(|q| q.view).unwrap_or(0) < qc.view {
					self.locked_qc = Some(qc.clone());
				}
			},
			_ => {},
		}
		let next_phase = match phase.next() {
			Some(p) => p,
			None => return Err(err!(
				"Quorum reached on terminal phase Decide -- invalid state.";
			Invalid, Order, Bug)),
		};
		let proposal = Proposal {
			view:		self.view,
			phase:		next_phase,
			block_hash,
			block:		None,
			justify:	Some(qc),
		};
		Ok(vec![Command::BroadcastProposal(proposal)])
	}

	/// Advances the replica to the next view and emits a [`Command::SendNewView`]
	/// targeting the new leader.
	///
	/// The caller is expected to invoke this when its local timer fires
	/// without observing progress in the current view. The state machine
	/// has no notion of time -- timeout policy is the caller's to set.
	pub fn on_timeout(&mut self) -> Outcome<Vec<Command>> {
		if self.decided {
			return Ok(Vec::new());
		}
		self.view = self.view.wrapping_add(1);
		self.gathered.clear();
		self.new_views.clear();
		self.last_voted = None;
		self.prepared_this_view = false;
		let new_view = NewView {
			view:		self.view,
			sender:		self.cfg.self_id,
			prepare_qc:	self.prepare_qc.clone(),
		};
		let to = self.cfg.leader_for(self.view);
		Ok(vec![Command::SendNewView { to, new_view }])
	}

	/// Leader-only: consumes an incoming [`NewView`] and, on accumulating a
	/// quorum, opens the next view's Prepare proposal if a prior prepare QC
	/// pinned a block.
	///
	/// If no participating replica reported a prepare QC the primitive does
	/// *not* speculate a block -- the leader must then call
	/// [`Replica::propose`] with a fresh block of its choice.
	pub fn on_new_view(&mut self, nv: NewView) -> Outcome<Vec<Command>> {
		if !self.cfg.is_leader_for(self.view) {
			return Ok(Vec::new());
		}
		if self.decided {
			return Ok(Vec::new());
		}
		if nv.view != self.view {
			return Err(err!(
				"NewView view {} does not match current view {}.",
				nv.view, self.view;
			Invalid, Input, Mismatch));
		}
		if (nv.sender as usize) >= self.cfg.cohort_size {
			return Err(err!(
				"NewView sender {} out of range (cohort_size = {}).",
				nv.sender, self.cfg.cohort_size;
			Invalid, Input));
		}
		// Validate the carried prepare_qc, if any.
		if let Some(qc) = &nv.prepare_qc {
			res!(qc.validate(
				qc.view,
				Phase::Prepare,
				&qc.block_hash,
				self.cfg.quorum(),
				self.cfg.cohort_size,
			));
			// Update the leader's own prepare_qc if this one is higher --
			// helps leaders that were offline for earlier views catch up.
			if self.prepare_qc.as_ref().map(|q| q.view).unwrap_or(0) < qc.view {
				self.prepare_qc = Some(qc.clone());
			}
		}
		self.new_views.insert(nv.sender, nv);
		if self.new_views.len() < self.cfg.quorum() {
			return Ok(Vec::new());
		}
		if self.prepared_this_view {
			// Already opened the view; further NewViews are dropped.
			return Ok(Vec::new());
		}
		// Pick the highest prepare_qc among gathered NewViews.
		let highest: Option<Qc> = self.new_views.values()
			.filter_map(|nv| nv.prepare_qc.clone())
			.max_by_key(|qc| qc.view);
		match highest {
			Some(qc) => {
				// The block hash is pinned by qc; we need the block bytes.
				let block_bytes = match self.blocks.get(&qc.block_hash) {
					Some(b) => b.clone(),
					None => return Err(err!(
						"Leader for view {} lacks block payload for pinned hash. \
						The caller must arrange payload retrieval (e.g. out-of-band \
						fetch) before the new view's Prepare can be opened.",
						self.view;
					Missing, Data)),
				};
				self.prepared_this_view = true;
				let proposal = Proposal {
					view:		self.view,
					phase:		Phase::Prepare,
					block_hash:	qc.block_hash,
					block:		Some(block_bytes),
					justify:	Some(qc),
				};
				Ok(vec![Command::BroadcastProposal(proposal)])
			},
			None => {
				// No replica had a prepare QC. Leader must call propose()
				// with a fresh block of its choice. Nothing to emit here.
				Ok(Vec::new())
			},
		}
	}

	/// Returns `true` if this replica is currently expecting the leader
	/// to supply a fresh block via [`Replica::propose`] -- i.e. it is the
	/// leader, a NewView quorum has been accumulated, and no prior
	/// prepare QC pinned a block.
	pub fn awaiting_fresh_block(&self) -> bool {
		self.cfg.is_leader_for(self.view)
			&& !self.decided
			&& !self.prepared_this_view
			&& self.new_views.len() >= self.cfg.quorum()
			&& self.new_views.values().all(|nv| nv.prepare_qc.is_none())
	}

	fn safe_block(&self, _block_hash: &BlockHash, justify: &Qc) -> bool {
		match &self.locked_qc {
			None => true,
			Some(lqc) => {
				// Liveness: a later view overrides.
				if justify.view > lqc.view {
					return true;
				}
				// Safety: the justified block must match our locked block.
				justify.block_hash == lqc.block_hash
			},
		}
	}

	fn phase_rank(&self, p: Phase) -> u8 {
		match p {
			Phase::Prepare		=> 1,
			Phase::PreCommit	=> 2,
			Phase::Commit		=> 3,
			Phase::Decide		=> 4,
		}
	}

	fn self_signature(&self, phase: Phase, block_hash: &BlockHash) -> Vec<u8> {
		let mut out = Vec::with_capacity(2 + 1 + BLOCK_HASH_LEN);
		out.extend_from_slice(&self.cfg.self_id.to_le_bytes());
		out.push(phase_tag(phase));
		out.extend_from_slice(block_hash);
		out
	}
}

fn phase_tag(p: Phase) -> u8 {
	match p {
		Phase::Prepare		=> 1,
		Phase::PreCommit	=> 2,
		Phase::Commit		=> 3,
		Phase::Decide		=> 4,
	}
}
