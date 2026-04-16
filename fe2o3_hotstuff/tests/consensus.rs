//! Integration tests for the HotStuff primitive.
//!
//! Tests run a tiny in-memory driver over a cohort of [`Replica`] instances,
//! routing every emitted [`Command`] to its targets until the simulation
//! settles. The driver has no notion of time or faults -- its job is to
//! prove the happy-path state machine reaches a unanimous decision.

use oxedyne_fe2o3_core::prelude::*;

use oxedyne_fe2o3_hotstuff::{
	replica::{
		Command,
		Config,
		Replica,
	},
	types::{
		BLOCK_HASH_LEN,
		BlockHash,
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
	Vote {
		to:		ReplicaId,
		vote:	Vote,
	},
}

/// The in-memory driver: owns `n` replicas and a FIFO of pending deliveries.
/// Routes each emitted command into the queue and runs until the queue is
/// empty or every replica has decided.
struct Driver {
	replicas:	Vec<Replica>,
	queue:		VecDeque<Delivery>,
	decided:	Vec<Option<Vec<u8>>>,
}

impl Driver {
	fn new(cohort_size: usize, f: usize, leader_id: ReplicaId) -> Outcome<Self> {
		let mut replicas = Vec::with_capacity(cohort_size);
		for id in 0..cohort_size {
			let cfg = Config {
				cohort_size,
				f,
				self_id: id as ReplicaId,
				leader_id,
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
				Command::Decide { block, .. } => {
					self.decided[from as usize] = Some(block);
				},
			}
		}
	}

	fn run(&mut self) -> Outcome<()> {
		let max_steps = 10_000;
		let mut steps = 0;
		while let Some(delivery) = self.queue.pop_front() {
			steps += 1;
			if steps > max_steps {
				return Err(err!(
					"Driver exceeded {} steps without terminating.", max_steps;
				Bug, Timeout));
			}
			match delivery {
				Delivery::Proposal(proposal) => {
					// Broadcast: deliver to every replica.
					let cohort = self.replicas.len();
					for id in 0..cohort {
						let cmds = res!(self.replicas[id].on_proposal(proposal.clone()));
						self.handle_commands(id as ReplicaId, cmds);
					}
				},
				Delivery::Vote { to, vote } => {
					let cmds = res!(self.replicas[to as usize].on_vote(vote));
					self.handle_commands(to, cmds);
				},
			}
			// Early exit once all replicas have decided.
			if self.decided.iter().all(|d| d.is_some()) {
				break;
			}
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


#[test]
fn config_rejects_degenerate() -> Outcome<()> {
	// cohort_size == 0.
	let cfg = Config { cohort_size: 0, f: 0, self_id: 0, leader_id: 0 };
	assert!(Replica::new(cfg).is_err());
	// cohort_size < 3f + 1.
	let cfg = Config { cohort_size: 3, f: 1, self_id: 0, leader_id: 0 };
	assert!(Replica::new(cfg).is_err());
	// self_id out of range.
	let cfg = Config { cohort_size: 4, f: 1, self_id: 4, leader_id: 0 };
	assert!(Replica::new(cfg).is_err());
	// leader_id out of range.
	let cfg = Config { cohort_size: 4, f: 1, self_id: 0, leader_id: 4 };
	assert!(Replica::new(cfg).is_err());
	Ok(())
}

#[test]
fn config_quorum_follows_lambda_minus_z() -> Outcome<()> {
	let cfg_5 = Config { cohort_size: 5, f: 1, self_id: 0, leader_id: 0 };
	let cfg_7 = Config { cohort_size: 7, f: 2, self_id: 0, leader_id: 0 };
	let cfg_9 = Config { cohort_size: 9, f: 2, self_id: 0, leader_id: 0 };
	assert_eq!(cfg_5.quorum(), 4);
	assert_eq!(cfg_7.quorum(), 5);
	assert_eq!(cfg_9.quorum(), 7);
	Ok(())
}

#[test]
fn propose_rejected_on_non_leader() -> Outcome<()> {
	let cfg = Config { cohort_size: 5, f: 1, self_id: 1, leader_id: 0 };
	let mut r = res!(Replica::new(cfg));
	let block = b"hello".to_vec();
	let h = fixed_block_hash(1);
	assert!(r.propose(block, h).is_err());
	Ok(())
}

#[test]
fn five_replica_cohort_reaches_decide() -> Outcome<()> {
	let mut drv = res!(Driver::new(5, 1, 0));
	let block = b"consensus input".to_vec();
	let h = fixed_block_hash(7);
	let cmds = res!(drv.replicas[0].propose(block.clone(), h));
	drv.handle_commands(0, cmds);
	res!(drv.run());
	// Every replica must have decided on the same block.
	for (i, d) in drv.decided.iter().enumerate() {
		let got = match d {
			Some(b) => b,
			None => return Err(err!(
				"replica {} did not decide", i;
			Bug, Fatal)),
		};
		assert_eq!(got, &block, "replica {} decided a wrong block", i);
	}
	for r in &drv.replicas {
		assert!(r.has_decided(), "replica has_decided() inconsistent");
	}
	Ok(())
}

#[test]
fn seven_replica_cohort_reaches_decide() -> Outcome<()> {
	let mut drv = res!(Driver::new(7, 2, 0));
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
	let mut drv = res!(Driver::new(9, 2, 3));
	let block = b"rotated leader".to_vec();
	let h = fixed_block_hash(12);
	let cmds = res!(drv.replicas[3].propose(block.clone(), h));
	drv.handle_commands(3, cmds);
	res!(drv.run());
	for d in &drv.decided {
		assert_eq!(d.as_deref(), Some(block.as_slice()));
	}
	Ok(())
}

#[test]
fn duplicate_votes_are_idempotent() -> Outcome<()> {
	let mut drv = res!(Driver::new(5, 1, 0));
	let block = b"test duplicates".to_vec();
	let h = fixed_block_hash(5);
	let cmds = res!(drv.replicas[0].propose(block.clone(), h));
	drv.handle_commands(0, cmds);
	// Process one step: the Prepare proposal gets broadcast and votes come
	// back. Instead of running to completion, intercept one vote and replay
	// it.
	// Handle the single Proposal delivery manually.
	let first = match drv.queue.pop_front() {
		Some(Delivery::Proposal(p)) => p,
		_ => return Err(err!("expected initial Prepare proposal"; Bug)),
	};
	let cohort = drv.replicas.len();
	let mut first_vote: Option<Vote> = None;
	for id in 0..cohort {
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
	// Replay the first vote into the leader twice; the leader's quorum
	// counting must ignore the replay.
	let v = match first_vote {
		Some(v) => v,
		None => return Err(err!("no vote captured"; Bug)),
	};
	let _ = res!(drv.replicas[0].on_vote(v.clone()));
	let _ = res!(drv.replicas[0].on_vote(v));
	// Then drain as normal.
	res!(drv.run());
	for d in &drv.decided {
		assert_eq!(d.as_deref(), Some(block.as_slice()));
	}
	Ok(())
}

#[test]
fn out_of_range_voter_rejected() -> Outcome<()> {
	let cfg = Config { cohort_size: 5, f: 1, self_id: 0, leader_id: 0 };
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
fn qc_validate_catches_duplicate_voter() -> Outcome<()> {
	let qc = Qc {
		view:		1,
		phase:		Phase::Prepare,
		block_hash:	fixed_block_hash(1),
		signatures:	vec![
			(0, vec![]),
			(2, vec![]),
			(2, vec![]),
			(3, vec![]),
		],
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
		signatures:	vec![
			(2, vec![]),
			(0, vec![]),
			(3, vec![]),
		],
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

#[test]
fn prepare_without_block_is_rejected() -> Outcome<()> {
	let cfg = Config { cohort_size: 5, f: 1, self_id: 1, leader_id: 0 };
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
	let cfg = Config { cohort_size: 5, f: 1, self_id: 1, leader_id: 0 };
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
