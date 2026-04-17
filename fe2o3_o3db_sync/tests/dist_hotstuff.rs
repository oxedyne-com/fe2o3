#![cfg(feature = "dist")]
//! Integration tests for the HotStuff primitive.

use oxedyne_fe2o3_core::prelude::*;

use oxedyne_fe2o3_o3db_sync::dist::hotstuff::{
	replica::{
		Command,
		Config,
		Replica,
	},
	types::{
		BLOCK_HASH_LEN,
		BlockHash,
		NewView,
		Phase,
		Proposal,
		Qc,
		ReplicaId,
		Vote,
	},
};

use std::collections::VecDeque;


/// A pending delivery in the in-memory simulation.
#[derive(Clone, Debug)]
enum Delivery {
	Proposal(Proposal),
	Vote		{ to: ReplicaId, vote:		Vote },
	NewView		{ to: ReplicaId, new_view:	NewView },
}

/// In-memory driver. Routes each emitted command into a FIFO and runs until
/// the queue drains or every replica has decided.
struct Driver {
	replicas:	Vec<Replica>,
	queue:		VecDeque<Delivery>,
	decided:	Vec<Option<Vec<u8>>>,
}

impl Driver {
	fn new(cohort_size: usize, f: usize) -> Outcome<Self> {
		let mut replicas = Vec::with_capacity(cohort_size);
		for id in 0..cohort_size {
			let cfg = Config {
				cohort_size,
				f,
				self_id: id as ReplicaId,
			};
			replicas.push(res!(Replica::new(cfg)));
		}
		Ok(Self {
			replicas,
			queue:		VecDeque::new(),
			decided:	vec![None; cohort_size],
		})
	}

	fn handle_commands(&mut self, from: ReplicaId, cmds: Vec<Command>) {
		for cmd in cmds {
			match cmd {
				Command::BroadcastProposal(p) => {
					self.queue.push_back(Delivery::Proposal(p));
				},
				Command::SendVote { to, vote } => {
					self.queue.push_back(Delivery::Vote { to, vote });
				},
				Command::SendNewView { to, new_view } => {
					self.queue.push_back(Delivery::NewView { to, new_view });
				},
				Command::Decide { block, .. } => {
					self.decided[from as usize] = Some(block);
				},
			}
		}
	}

	fn run(&mut self) -> Outcome<()> {
		let max_steps = 10_000;
		let mut steps = 0;
		while let Some(d) = self.queue.pop_front() {
			steps += 1;
			if steps > max_steps {
				return Err(err!(
					"Driver exceeded {} steps without terminating.", max_steps;
				Bug, Timeout));
			}
			self.deliver(d)?;
			if self.decided.iter().all(|d| d.is_some()) {
				break;
			}
		}
		Ok(())
	}

	fn deliver(&mut self, d: Delivery) -> Outcome<()> {
		match d {
			Delivery::Proposal(p) => {
				let cohort = self.replicas.len();
				for id in 0..cohort {
					let cmds = res!(self.replicas[id].on_proposal(p.clone()));
					self.handle_commands(id as ReplicaId, cmds);
				}
			},
			Delivery::Vote { to, vote } => {
				let cmds = res!(self.replicas[to as usize].on_vote(vote));
				self.handle_commands(to, cmds);
			},
			Delivery::NewView { to, new_view } => {
				let cmds = res!(self.replicas[to as usize].on_new_view(new_view));
				self.handle_commands(to, cmds);
			},
		}
		Ok(())
	}

	/// Fires a timeout on every replica that has not yet decided, routing
	/// the resulting NewView commands into the queue. Simulates a cohort-
	/// wide timeout event.
	fn timeout_all(&mut self) -> Outcome<()> {
		let cohort = self.replicas.len();
		for id in 0..cohort {
			if self.decided[id].is_some() {
				continue;
			}
			let cmds = res!(self.replicas[id].on_timeout());
			self.handle_commands(id as ReplicaId, cmds);
		}
		Ok(())
	}
}

fn fixed_block_hash(seed: u8) -> BlockHash {
	let mut h = [0u8; BLOCK_HASH_LEN];
	for (i, cell) in h.iter_mut().enumerate() {
		*cell = seed.wrapping_add(i as u8);
	}
	h
}


// --- configuration ---------------------------------------------------------

#[test]
fn config_rejects_degenerate() -> Outcome<()> {
	assert!(Replica::new(Config { cohort_size: 0, f: 0, self_id: 0 }).is_err());
	assert!(Replica::new(Config { cohort_size: 3, f: 1, self_id: 0 }).is_err());
	assert!(Replica::new(Config { cohort_size: 4, f: 1, self_id: 4 }).is_err());
	Ok(())
}

#[test]
fn config_quorum_follows_lambda_minus_z() -> Outcome<()> {
	let cfg5 = Config { cohort_size: 5, f: 1, self_id: 0 };
	let cfg7 = Config { cohort_size: 7, f: 2, self_id: 0 };
	let cfg9 = Config { cohort_size: 9, f: 2, self_id: 0 };
	assert_eq!(cfg5.quorum(), 4);
	assert_eq!(cfg7.quorum(), 5);
	assert_eq!(cfg9.quorum(), 7);
	Ok(())
}

#[test]
fn leader_rotates_round_robin() -> Outcome<()> {
	let cfg = Config { cohort_size: 5, f: 1, self_id: 0 };
	assert_eq!(cfg.leader_for(1), 0);
	assert_eq!(cfg.leader_for(2), 1);
	assert_eq!(cfg.leader_for(3), 2);
	assert_eq!(cfg.leader_for(6), 0);  // wraps
	assert!(cfg.is_leader_for(1));
	assert!(!cfg.is_leader_for(2));
	Ok(())
}


// --- happy path ------------------------------------------------------------

#[test]
fn propose_rejected_on_non_leader() -> Outcome<()> {
	let cfg = Config { cohort_size: 5, f: 1, self_id: 1 };
	let mut r = res!(Replica::new(cfg));
	assert!(r.propose(b"x".to_vec(), fixed_block_hash(1)).is_err());
	Ok(())
}

#[test]
fn five_replica_cohort_reaches_decide() -> Outcome<()> {
	let mut drv = res!(Driver::new(5, 1));
	let block = b"consensus input".to_vec();
	let h = fixed_block_hash(7);
	let cmds = res!(drv.replicas[0].propose(block.clone(), h));
	drv.handle_commands(0, cmds);
	res!(drv.run());
	for (i, d) in drv.decided.iter().enumerate() {
		let got = match d {
			Some(b) => b,
			None => return Err(err!("replica {} did not decide", i; Bug, Fatal)),
		};
		assert_eq!(got, &block);
	}
	Ok(())
}

#[test]
fn seven_replica_cohort_reaches_decide() -> Outcome<()> {
	let mut drv = res!(Driver::new(7, 2));
	let block = b"more replicas".to_vec();
	let h = fixed_block_hash(3);
	let cmds = res!(drv.replicas[0].propose(block.clone(), h));
	drv.handle_commands(0, cmds);
	res!(drv.run());
	for d in &drv.decided {
		assert_eq!(d.as_deref(), Some(block.as_slice()));
	}
	Ok(())
}

#[test]
fn nine_replica_cohort_reaches_decide() -> Outcome<()> {
	let mut drv = res!(Driver::new(9, 2));
	let block = b"nine nodes".to_vec();
	let h = fixed_block_hash(12);
	let cmds = res!(drv.replicas[0].propose(block.clone(), h));
	drv.handle_commands(0, cmds);
	res!(drv.run());
	for d in &drv.decided {
		assert_eq!(d.as_deref(), Some(block.as_slice()));
	}
	Ok(())
}

#[test]
fn duplicate_votes_are_idempotent() -> Outcome<()> {
	let mut drv = res!(Driver::new(5, 1));
	let block = b"test duplicates".to_vec();
	let h = fixed_block_hash(5);
	let cmds = res!(drv.replicas[0].propose(block.clone(), h));
	drv.handle_commands(0, cmds);
	let first = match drv.queue.pop_front() {
		Some(Delivery::Proposal(p)) => p,
		_ => return Err(err!("expected initial Prepare proposal"; Bug)),
	};
	let mut first_vote: Option<Vote> = None;
	for id in 0..drv.replicas.len() {
		let out = res!(drv.replicas[id].on_proposal(first.clone()));
		for cmd in out {
			if let Command::SendVote { vote, .. } = &cmd {
				if first_vote.is_none() {
					first_vote = Some(vote.clone());
				}
			}
			drv.handle_commands(id as ReplicaId, vec![cmd]);
		}
	}
	let v = match first_vote {
		Some(v) => v,
		None => return Err(err!("no vote captured"; Bug)),
	};
	let _ = res!(drv.replicas[0].on_vote(v.clone()));
	let _ = res!(drv.replicas[0].on_vote(v));
	res!(drv.run());
	for d in &drv.decided {
		assert_eq!(d.as_deref(), Some(block.as_slice()));
	}
	Ok(())
}


// --- structural rejection --------------------------------------------------

#[test]
fn out_of_range_voter_rejected() -> Outcome<()> {
	let cfg = Config { cohort_size: 5, f: 1, self_id: 0 };
	let mut leader = res!(Replica::new(cfg));
	let bogus = Vote {
		view:		1,
		phase:		Phase::Prepare,
		block_hash:	fixed_block_hash(9),
		voter:		99,
		signature:	Vec::new(),
	};
	assert!(leader.on_vote(bogus).is_err());
	Ok(())
}

#[test]
fn prepare_without_block_is_rejected() -> Outcome<()> {
	let cfg = Config { cohort_size: 5, f: 1, self_id: 1 };
	let mut r = res!(Replica::new(cfg));
	let p = Proposal {
		view:		1,
		phase:		Phase::Prepare,
		block_hash:	fixed_block_hash(1),
		block:		None,
		justify:	None,
	};
	assert!(r.on_proposal(p).is_err());
	Ok(())
}

#[test]
fn precommit_without_justify_is_rejected() -> Outcome<()> {
	let cfg = Config { cohort_size: 5, f: 1, self_id: 1 };
	let mut r = res!(Replica::new(cfg));
	let p = Proposal {
		view:		1,
		phase:		Phase::PreCommit,
		block_hash:	fixed_block_hash(1),
		block:		None,
		justify:	None,
	};
	assert!(r.on_proposal(p).is_err());
	Ok(())
}


// --- QC validation ---------------------------------------------------------

#[test]
fn qc_validate_catches_duplicate_voter() -> Outcome<()> {
	let qc = Qc {
		view:		1,
		phase:		Phase::Prepare,
		block_hash:	fixed_block_hash(1),
		signatures:	vec![(0, vec![]), (2, vec![]), (2, vec![]), (3, vec![])],
	};
	assert!(qc.validate(1, Phase::Prepare, &fixed_block_hash(1), 3, 5).is_err());
	Ok(())
}

#[test]
fn qc_validate_catches_out_of_order_voter() -> Outcome<()> {
	let qc = Qc {
		view:		1,
		phase:		Phase::Prepare,
		block_hash:	fixed_block_hash(1),
		signatures:	vec![(2, vec![]), (0, vec![]), (3, vec![])],
	};
	assert!(qc.validate(1, Phase::Prepare, &fixed_block_hash(1), 3, 5).is_err());
	Ok(())
}

#[test]
fn qc_validate_catches_insufficient_quorum() -> Outcome<()> {
	let qc = Qc {
		view:		1,
		phase:		Phase::Prepare,
		block_hash:	fixed_block_hash(1),
		signatures:	vec![(0, vec![]), (1, vec![])],
	};
	assert!(qc.validate(1, Phase::Prepare, &fixed_block_hash(1), 4, 5).is_err());
	Ok(())
}


// --- view change -----------------------------------------------------------

#[test]
fn silent_leader_recovers_via_view_change() -> Outcome<()> {
	// Leader 0 stays silent; everyone times out; leader 1 takes over with
	// a fresh block.
	let mut drv = res!(Driver::new(5, 1));
	res!(drv.timeout_all());
	res!(drv.run());
	// After timeout, queue has 5 NewView deliveries to leader 1. Leader 1
	// collects quorum, has no prepare_qc from anyone, is awaiting a fresh
	// block.
	assert!(drv.replicas[1].awaiting_fresh_block(),
		"leader 1 should be awaiting a fresh block after timeout_all");
	assert_eq!(drv.replicas[1].view(), 2);

	// Leader 1 now proposes a fresh block.
	let block = b"second view input".to_vec();
	let h = fixed_block_hash(22);
	let cmds = res!(drv.replicas[1].propose(block.clone(), h));
	drv.handle_commands(1, cmds);
	res!(drv.run());
	for d in &drv.decided {
		assert_eq!(d.as_deref(), Some(block.as_slice()));
	}
	Ok(())
}

#[test]
fn view_change_preserves_pinned_block_when_prepare_qc_exists() -> Outcome<()> {
	// Leader 0 proposes and everyone reaches PreCommit stage (i.e. prepare
	// QC is formed and distributed). Then they time out. Leader 1 takes
	// over; because a prepare QC existed, it MUST re-propose the same block.
	let mut drv = res!(Driver::new(5, 1));
	let block = b"pinned block".to_vec();
	let h = fixed_block_hash(44);
	let cmds = res!(drv.replicas[0].propose(block.clone(), h));
	drv.handle_commands(0, cmds);
	// Drive until a PreCommit proposal has been broadcast (but not further).
	// The PreCommit proposal distributes the Prepare QC, populating each
	// replica's prepare_qc.
	let mut saw_precommit = false;
	while let Some(d) = drv.queue.pop_front() {
		let is_precommit = matches!(&d,
			Delivery::Proposal(p) if p.phase == Phase::PreCommit);
		res!(drv.deliver(d));
		if is_precommit {
			saw_precommit = true;
			break;
		}
	}
	assert!(saw_precommit, "expected a PreCommit proposal in the queue");
	// Drop the remainder of the queue (simulating the leader going silent
	// mid-PreCommit before any Commit happened), and time out.
	drv.queue.clear();
	assert!(drv.replicas.iter().all(|r| r.prepare_qc().is_some()),
		"every replica should hold a prepare_qc after PreCommit proposal");
	assert!(drv.replicas.iter().all(|r| r.locked_qc().is_none()),
		"no replica should be locked yet (no Commit proposal seen)");
	res!(drv.timeout_all());
	res!(drv.run());
	// Leader 1 collected NewViews with prepare_qc pinning block `h`, so it
	// must have broadcast a Prepare for `block` without a fresh propose().
	for d in &drv.decided {
		assert_eq!(d.as_deref(), Some(block.as_slice()),
			"view-changed cohort must decide the pinned block");
	}
	Ok(())
}

#[test]
fn view_change_rejects_unsafe_proposal() -> Outcome<()> {
	// A replica that is LOCKED on block B (saw a Commit proposal for B in
	// view 1) must reject a Prepare proposal in view 2 for a different
	// block whose justify is from view 1.
	let cfg = Config { cohort_size: 5, f: 1, self_id: 1 };
	let mut victim = res!(Replica::new(cfg));

	// Construct a synthetic PreCommit QC on block B, view 1, which is what
	// a Commit proposal's justify would carry. Feeding this through the
	// Commit code path installs locked_qc on the victim.
	let block_b = b"locked block".to_vec();
	let h_b = fixed_block_hash(100);
	// First: feed a Prepare for B so that blocks[h_b] is populated and
	// last_voted tracks Prepare.
	let p1 = Proposal {
		view:		1, phase: Phase::Prepare,
		block_hash:	h_b, block: Some(block_b.clone()), justify: None,
	};
	let _ = res!(victim.on_proposal(p1));
	// Synthesise a Prepare QC for (view=1, phase=Prepare, hash=h_b) with
	// quorum=4 votes.
	let prepare_qc = Qc {
		view: 1, phase: Phase::Prepare, block_hash: h_b,
		signatures: (0..4).map(|i| (i as ReplicaId, vec![i as u8])).collect(),
	};
	let p2 = Proposal {
		view:		1, phase: Phase::PreCommit,
		block_hash:	h_b, block: None, justify: Some(prepare_qc),
	};
	let _ = res!(victim.on_proposal(p2));
	let precommit_qc = Qc {
		view: 1, phase: Phase::PreCommit, block_hash: h_b,
		signatures: (0..4).map(|i| (i as ReplicaId, vec![i as u8])).collect(),
	};
	let p3 = Proposal {
		view:		1, phase: Phase::Commit,
		block_hash:	h_b, block: None, justify: Some(precommit_qc),
	};
	let _ = res!(victim.on_proposal(p3));
	assert!(victim.locked_qc().is_some(), "victim should be locked on B");
	assert_eq!(victim.locked_qc().unwrap().block_hash, h_b);

	// Now simulate view change to view 2.
	res!(victim.on_timeout());
	assert_eq!(victim.view(), 2);

	// Byzantine leader proposes a DIFFERENT block C in view 2, with a
	// fabricated justify from view 1 (same view as the victim's lock).
	// safeBlock says: justify.block_hash != locked.block_hash AND
	// justify.view == locked.view, so REJECT.
	let block_c = b"attacker block".to_vec();
	let h_c = fixed_block_hash(77);
	let evil_justify = Qc {
		view: 1, phase: Phase::Prepare, block_hash: h_c,
		signatures: (0..4).map(|i| (i as ReplicaId, vec![i as u8])).collect(),
	};
	let attack = Proposal {
		view:		2, phase: Phase::Prepare,
		block_hash:	h_c, block: Some(block_c), justify: Some(evil_justify),
	};
	let outcome = victim.on_proposal(attack);
	assert!(outcome.is_err(),
		"locked victim must reject an unsafe view-change proposal");
	Ok(())
}

#[test]
fn view_change_accepts_later_view_proposal() -> Outcome<()> {
	// A replica that was ONLY prepared (not locked) in view 1 may accept a
	// different block in view 2 if its justify.view > locked_qc.view --
	// but here locked_qc is None, so anything with a valid prepare QC
	// passes safeBlock trivially.
	let cfg = Config { cohort_size: 5, f: 1, self_id: 1 };
	let mut r = res!(Replica::new(cfg));
	let block_a = b"view-1 block".to_vec();
	let h_a = fixed_block_hash(50);
	let _ = res!(r.on_proposal(Proposal {
		view: 1, phase: Phase::Prepare,
		block_hash: h_a, block: Some(block_a), justify: None,
	}));
	let prepare_qc_a = Qc {
		view: 1, phase: Phase::Prepare, block_hash: h_a,
		signatures: (0..4).map(|i| (i as ReplicaId, vec![i as u8])).collect(),
	};
	let _ = res!(r.on_proposal(Proposal {
		view: 1, phase: Phase::PreCommit,
		block_hash: h_a, block: None, justify: Some(prepare_qc_a),
	}));
	assert!(r.prepare_qc().is_some());
	assert!(r.locked_qc().is_none());

	// Time out to view 2, then feed a Prepare for a different block whose
	// justify is from view 1 -- legal because locked_qc is None.
	res!(r.on_timeout());
	let block_b = b"view-2 block".to_vec();
	let h_b = fixed_block_hash(60);
	let qc_b = Qc {
		view: 1, phase: Phase::Prepare, block_hash: h_b,
		signatures: (0..4).map(|i| (i as ReplicaId, vec![i as u8])).collect(),
	};
	let proposal = Proposal {
		view: 2, phase: Phase::Prepare,
		block_hash: h_b, block: Some(block_b), justify: Some(qc_b),
	};
	let out = res!(r.on_proposal(proposal));
	assert!(out.iter().any(|c| matches!(c, Command::SendVote { .. })),
		"replica without a lock should vote on a valid view-2 Prepare");
	Ok(())
}
