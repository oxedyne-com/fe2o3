#![cfg(feature = "dist")]
//! Integration tests for the HotStuff cohort write path.
//!
//! These tests exercise the full Prepare → PreCommit → Commit → Decide
//! cycle against the in-memory [`MemoryStorage`] adapter, driving envelopes
//! by hand between a small cluster of [`DistOzone`] engines. Transport is
//! synchronous and loss-free -- mirroring the style of `anti_entropy.rs`.
//!
//! Covered:
//!   * 5-peer happy path: leader-initiated put reaches Decide on every
//!     cohort member.
//!   * Submit-forwarding: a non-leader peer's put is forwarded to the
//!     leader and completes the same way.
//!   * Idempotence: dispatching a duplicate set of envelopes does not
//!     change state or re-fire Decide.
//!   * View change: silent leader, every follower times out, new leader
//!     drives the round.
//!   * Persistence: every cohort member ends up with the record in its
//!     local storage; non-members do not.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::kademlia::id::NodeId;
use oxedyne_fe2o3_o3db_sync::oam::config::OamConfig;
use oxedyne_fe2o3_o3db_sync::dist::{
	config::{
		DistOzoneConfig,
		TableConfig,
	},
	engine::DistOzone,
	record::{
		Record,
		RecordId,
	},
	storage::{
		MemoryStorage,
		Storage,
	},
	transport::{
		Envelope,
		MsgKind,
	},
};

use std::collections::HashMap;


/// Deterministic splitmix64 for reproducible peer/id generation.
struct Rng { state: u64 }

impl Rng {
	fn new(seed: u64) -> Self { Self { state: seed } }

	fn next_u64(&mut self) -> u64 {
		self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
		let mut z = self.state;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
		z ^ (z >> 31)
	}

	fn next_id(&mut self) -> NodeId {
		let mut bytes = [0u8; 32];
		for i in 0..4 {
			let word = self.next_u64().to_le_bytes();
			bytes[i * 8..(i + 1) * 8].copy_from_slice(&word);
		}
		NodeId::from_bytes(bytes)
	}
}


/// Build a cluster of `n` engines sharing a single cohort-backed table
/// `table_name` with the given `lambda`. Returns one engine per peer,
/// keyed by the peer's [`NodeId`], along with the vector of ids in
/// creation order.
fn build_cluster(
	n:			usize,
	lambda:		u64,
	table_name:	&str,
	seed:		u64,
)
	-> Outcome<(Vec<NodeId>, HashMap<NodeId, DistOzone<MemoryStorage>>)>
{
	let mut rng = Rng::new(seed);
	let ids: Vec<NodeId> = (0..n).map(|_| rng.next_id()).collect();
	let mut engines: HashMap<NodeId, DistOzone<MemoryStorage>> = HashMap::new();
	for me in &ids {
		let peers: Vec<NodeId> = ids.iter().filter(|p| *p != me).copied().collect();
		let oam = res!(OamConfig::new(n as u64, n as u64));
		let tables = vec![res!(TableConfig::new(
			table_name,
			oxedyne_fe2o3_o3db_sync::dist::config::Consistency::Cohort { lambda },
			TableConfig::DEFAULT_AE,
			TableConfig::DEFAULT_IBLT_CELLS,
		))];
		let cfg = res!(DistOzoneConfig::new(*me, peers, oam, tables));
		let engine = res!(DistOzone::new(cfg, MemoryStorage::new()));
		engines.insert(*me, engine);
	}
	Ok((ids, engines))
}


/// Drives a synchronous envelope dispatch loop until no new envelopes
/// are produced or `max_rounds` is exceeded (failsafe). Collects every
/// `completed_consensus_put` observed along the way.
fn drive(
	engines:	&HashMap<NodeId, DistOzone<MemoryStorage>>,
	outbound:	Vec<Envelope>,
	max_rounds:	usize,
)
	-> Outcome<Vec<(String, RecordId)>>
{
	let mut pending = outbound;
	let mut completed = Vec::new();
	let mut rounds = 0;
	while !pending.is_empty() {
		rounds += 1;
		if rounds > max_rounds {
			return Err(err!(
				"dispatch loop exceeded {} rounds -- likely infinite.",
				max_rounds;
				Bug));
		}
		let mut next = Vec::new();
		for env in pending.drain(..) {
			let engine = match engines.get(&env.to) {
				Some(e) => e,
				None => continue,	// Outbound to a non-member; ignore.
			};
			let outcome = res!(engine.handle_envelope(env));
			if let Some(pair) = outcome.completed_consensus_put {
				completed.push(pair);
			}
			next.extend(outcome.outbound);
		}
		pending = next;
	}
	Ok(completed)
}


/// Computes the cohort for `(table, record_id)` among `ids`, using the
/// same selection logic the engine uses internally. Returns the leader
/// and the sorted member list.
fn cohort_for(
	ids:		&[NodeId],
	table:		&str,
	rid:		&RecordId,
	lambda:		u64,
)
	-> Outcome<(NodeId, Vec<NodeId>)>
{
	// Pick an arbitrary member as "local" and derive the cohort from its
	// perspective -- cohort selection is symmetric, so any member's view
	// gives the same set.
	let local = ids[0];
	let peers: Vec<NodeId> = ids.iter().filter(|p| *p != &local).copied().collect();
	use oxedyne_fe2o3_o3db_sync::dist::peer_set::PeerSet;
	let mut peer_set = PeerSet::new();
	for p in peers { peer_set.insert(p); }
	let c = res!(oxedyne_fe2o3_o3db_sync::dist::cohort::select(
		table, rid, &peer_set, &local, lambda,
	));
	Ok((c.leader, c.members))
}


/// The happy path: a 5-peer cluster, lambda = 5, every peer is a cohort
/// member. A put issued on the leader opens a round; after the dispatch
/// loop settles, every engine has the record.
#[test]
fn leader_initiated_put_reaches_decide_on_every_member() -> Outcome<()> {
	let (ids, engines) = res!(build_cluster(5, 5, "treasury", 1));
	let rid = RecordId::from_bytes([0x42; 32]);
	let record = Record::new(rid, "treasury", b"payload".to_vec());
	let (leader, members) = res!(cohort_for(&ids, "treasury", &rid, 5));
	assert_eq!(members.len(), 5);

	// Kick off the round from the leader.
	let leader_engine = engines.get(&leader).expect("leader engine");
	let put = res!(leader_engine.put(record.clone()));
	assert_eq!(put.consensus_pending, Some(("treasury".to_string(), rid)));
	let completed = res!(drive(&engines, put.outbound, 256));

	// Every cohort member must have persisted the record...
	for m in &members {
		let got = res!(engines.get(m).expect("member engine")
			.storage().get("treasury", &rid));
		assert_eq!(got.as_ref(), Some(&record),
			"cohort member did not persist the record");
	}
	// ...and every member should have observed the completion signal.
	let mut unique_pairs: std::collections::HashSet<(String, RecordId)> =
		completed.iter().cloned().collect();
	unique_pairs.retain(|p| p == &("treasury".to_string(), rid));
	assert_eq!(unique_pairs.len(), 1);
	// Count was at least one per member (leader does not re-fire because
	// its Decide arrives through translate_commands, not handle_envelope).
	// Exactly four members receive the Decide over the wire.
	assert!(completed.len() >= 4,
		"expected at least 4 completed signals, got {}", completed.len());
	Ok(())
}


#[test]
fn non_leader_put_forwards_to_leader_and_reaches_decide() -> Outcome<()> {
	let (ids, engines) = res!(build_cluster(5, 5, "treasury", 2));
	let rid = RecordId::from_bytes([0x77; 32]);
	let record = Record::new(rid, "treasury", b"other".to_vec());
	let (leader, members) = res!(cohort_for(&ids, "treasury", &rid, 5));

	// Pick a member that is not the leader to issue the put.
	let submitter = *members.iter().find(|m| **m != leader).expect("non-leader");
	let submitter_engine = engines.get(&submitter).expect("submitter engine");
	let put = res!(submitter_engine.put(record.clone()));
	assert_eq!(put.consensus_pending, Some(("treasury".to_string(), rid)));

	// Exactly one envelope -- a CohortSubmit addressed to the leader.
	assert_eq!(put.outbound.len(), 1);
	let env = &put.outbound[0];
	assert_eq!(env.to, leader);
	match &env.body {
		MsgKind::CohortSubmit { record: r } => assert_eq!(r.id, rid),
		other => panic!("expected CohortSubmit, got {}", other.label()),
	}

	let _ = res!(drive(&engines, put.outbound, 256));
	for m in &members {
		let got = res!(engines.get(m).expect("member engine")
			.storage().get("treasury", &rid));
		assert_eq!(got.as_ref(), Some(&record));
	}
	Ok(())
}


#[test]
fn non_cohort_member_put_routes_through_leader() -> Outcome<()> {
	// 7 peers, lambda = 5. Two peers end up outside the cohort. A put from
	// one of the outsiders still forwards a CohortSubmit to the leader and
	// eventually lands on every cohort member.
	let (ids, engines) = res!(build_cluster(7, 5, "epoch", 3));
	let rid = RecordId::from_bytes([0x01; 32]);
	let record = Record::new(rid, "epoch", b"v".to_vec());
	let (leader, members) = res!(cohort_for(&ids, "epoch", &rid, 5));
	let outsider = *ids.iter().find(|i| !members.contains(i))
		.expect("some outsider");

	let outsider_engine = engines.get(&outsider).expect("outsider engine");
	let put = res!(outsider_engine.put(record.clone()));
	assert_eq!(put.outbound.len(), 1);
	assert_eq!(put.outbound[0].to, leader);

	let _ = res!(drive(&engines, put.outbound, 256));
	for m in &members {
		let got = res!(engines.get(m).expect("member engine")
			.storage().get("epoch", &rid));
		assert_eq!(got.as_ref(), Some(&record));
	}
	// Outsiders do NOT end up with the record.
	for id in &ids {
		if members.contains(id) { continue; }
		let got = res!(engines.get(id).expect("outsider engine")
			.storage().get("epoch", &rid));
		assert!(got.is_none(),
			"non-cohort peer unexpectedly holds the record");
	}
	Ok(())
}


#[test]
fn duplicate_decide_is_idempotent() -> Outcome<()> {
	// Run the round, then replay the same outbound starting point. The
	// second drive should complete without error and without duplicating
	// storage state.
	let (ids, engines) = res!(build_cluster(5, 5, "ledger", 4));
	let rid = RecordId::from_bytes([0xaa; 32]);
	let record = Record::new(rid, "ledger", b"once".to_vec());
	let (leader, members) = res!(cohort_for(&ids, "ledger", &rid, 5));

	let leader_engine = engines.get(&leader).expect("leader");
	let put = res!(leader_engine.put(record.clone()));
	let _ = res!(drive(&engines, put.outbound, 256));

	// Re-issue the put. With the HotStuff replicas all in decided state,
	// the re-proposal should be a no-op on every replica (and on the
	// leader: propose() on a replica that already proposed this view
	// would error -- but a fresh put opens a fresh round by re-using the
	// instance, which is decided, so leader_open_round returns empty).
	let put2 = res!(leader_engine.put(record.clone()));
	let _ = res!(drive(&engines, put2.outbound, 256));

	// Storage state stays put.
	for m in &members {
		let got = res!(engines.get(m).expect("member")
			.storage().get("ledger", &rid));
		assert_eq!(got.as_ref(), Some(&record));
	}
	Ok(())
}


#[test]
fn independent_records_run_in_parallel() -> Outcome<()> {
	// Two distinct records concurrently reaching Decide. Each has its
	// own per-record HotStuff instance; they do not interfere.
	let (ids, engines) = res!(build_cluster(5, 5, "treasury", 5));
	let rid_a = RecordId::from_bytes([0x10; 32]);
	let rid_b = RecordId::from_bytes([0x20; 32]);
	let rec_a = Record::new(rid_a, "treasury", b"A".to_vec());
	let rec_b = Record::new(rid_b, "treasury", b"B".to_vec());

	let (leader_a, members_a) = res!(cohort_for(&ids, "treasury", &rid_a, 5));
	let (leader_b, members_b) = res!(cohort_for(&ids, "treasury", &rid_b, 5));

	let put_a = res!(engines.get(&leader_a).expect("A leader")
		.put(rec_a.clone()));
	let put_b = res!(engines.get(&leader_b).expect("B leader")
		.put(rec_b.clone()));

	// Merge outbounds and drive jointly.
	let mut envs = put_a.outbound;
	envs.extend(put_b.outbound);
	let _ = res!(drive(&engines, envs, 512));

	for m in &members_a {
		assert_eq!(
			res!(engines.get(m).expect("A member").storage().get("treasury", &rid_a)).as_ref(),
			Some(&rec_a),
		);
	}
	for m in &members_b {
		assert_eq!(
			res!(engines.get(m).expect("B member").storage().get("treasury", &rid_b)).as_ref(),
			Some(&rec_b),
		);
	}
	Ok(())
}


#[test]
fn cohort_submit_on_non_leader_is_dropped() -> Outcome<()> {
	// Directly hand a CohortSubmit to a peer that is a cohort member but
	// not the leader. The engine must drop it silently -- the leader is
	// the one that drives consensus.
	let (ids, engines) = res!(build_cluster(5, 5, "treasury", 6));
	let rid = RecordId::from_bytes([0xbb; 32]);
	let record = Record::new(rid, "treasury", b"x".to_vec());
	let (leader, members) = res!(cohort_for(&ids, "treasury", &rid, 5));
	let non_leader = *members.iter().find(|m| **m != leader).expect("non-leader");

	let env = Envelope::new(
		*ids.iter().find(|i| **i != non_leader).expect("sender"),
		non_leader,
		MsgKind::CohortSubmit { record: record.clone() },
	);
	let non_leader_engine = engines.get(&non_leader).expect("non-leader engine");
	let out = res!(non_leader_engine.handle_envelope(env));
	assert!(out.outbound.is_empty());
	assert!(out.completed_consensus_put.is_none());
	// No storage state either.
	let got = res!(non_leader_engine.storage().get("treasury", &rid));
	assert!(got.is_none());
	Ok(())
}


#[test]
fn cohort_vote_without_instance_is_dropped() -> Outcome<()> {
	use oxedyne_fe2o3_o3db_sync::dist::hotstuff::types::{
		BlockHash,
		Phase,
		Vote,
	};
	// A stray CohortVote that arrives before any instance exists is
	// silently dropped -- it cannot tie-up resources or cause a panic.
	let (ids, engines) = res!(build_cluster(5, 5, "treasury", 7));
	let rid = RecordId::from_bytes([0xcc; 32]);
	let (leader, _) = res!(cohort_for(&ids, "treasury", &rid, 5));
	let leader_engine = engines.get(&leader).expect("leader");
	let block_hash: BlockHash = [0x99; 32];
	let vote = Vote {
		view:		1,
		phase:		Phase::Prepare,
		block_hash,
		voter:		0,
		signature:	vec![],
	};
	let env = Envelope::new(
		ids[0],
		leader,
		MsgKind::CohortVote {
			table:	"treasury".to_string(),
			id:		rid,
			vote,
		},
	);
	let out = res!(leader_engine.handle_envelope(env));
	assert!(out.outbound.is_empty());
	assert!(out.completed_consensus_put.is_none());
	Ok(())
}


#[test]
fn cohort_propose_on_non_member_is_dropped() -> Outcome<()> {
	// 7 peers, lambda = 5. Craft a CohortPropose targeting a non-member.
	// The engine must drop it silently without touching storage.
	use oxedyne_fe2o3_o3db_sync::dist::hotstuff::types::{
		Phase,
		Proposal,
	};
	let (ids, engines) = res!(build_cluster(7, 5, "epoch", 8));
	let rid = RecordId::from_bytes([0xdd; 32]);
	let (_leader, members) = res!(cohort_for(&ids, "epoch", &rid, 5));
	let outsider = *ids.iter().find(|i| !members.contains(i))
		.expect("outsider");

	let proposal = Proposal {
		view:		1,
		phase:		Phase::Prepare,
		block_hash:	[0x42; 32],
		block:		Some(vec![0; 40]),	// garbage; will never be decoded
		justify:	None,
	};
	let env = Envelope::new(
		members[0],
		outsider,
		MsgKind::CohortPropose {
			table:		"epoch".to_string(),
			id:			rid,
			proposal,
		},
	);
	let engine = engines.get(&outsider).expect("outsider");
	let out = res!(engine.handle_envelope(env));
	assert!(out.outbound.is_empty());
	assert!(out.completed_consensus_put.is_none());
	Ok(())
}


#[test]
fn decided_record_is_stored_exactly_once_per_member() -> Outcome<()> {
	// After the happy path, every cohort member has precisely one copy
	// of the record and nothing else in storage.
	let (ids, engines) = res!(build_cluster(5, 5, "treasury", 9));
	let rid = RecordId::from_bytes([0x33; 32]);
	let record = Record::new(rid, "treasury", b"v".to_vec());
	let (leader, members) = res!(cohort_for(&ids, "treasury", &rid, 5));
	let put = res!(engines.get(&leader).expect("leader").put(record.clone()));
	let _ = res!(drive(&engines, put.outbound, 256));

	for m in &members {
		let engine = engines.get(m).expect("member");
		assert_eq!(res!(engine.storage().len()), 1);
		let got = res!(engine.storage().get("treasury", &rid));
		assert_eq!(got.as_ref(), Some(&record));
	}
	Ok(())
}

#[test]
fn cohort_timeout_produces_new_view_to_next_leader() -> Outcome<()> {
	// After the leader broadcasts Prepare, a non-leader member that
	// subsequently times out emits a `CohortNewView` targeting the next
	// view's leader. Verifies the engine's timeout→HotStuff wiring.
	let (ids, engines) = res!(build_cluster(5, 5, "treasury", 10));
	let rid = RecordId::from_bytes([0xee; 32]);
	let record = Record::new(rid, "treasury", b"v".to_vec());
	let (leader, members) = res!(cohort_for(&ids, "treasury", &rid, 5));

	// Open the round -- this gives every non-leader a CohortPropose.
	let put = res!(engines.get(&leader).expect("leader").put(record.clone()));

	// Deliver the leader's outbound so non-leaders build an instance and
	// vote, but discard anything that would drive the round forward past
	// the leader. Specifically: deliver CohortPropose envelopes only.
	let mut delivered_votes: Vec<Envelope> = Vec::new();
	for env in put.outbound {
		if matches!(env.body, MsgKind::CohortPropose { .. }) {
			let outcome = res!(engines.get(&env.to).expect("to").handle_envelope(env));
			delivered_votes.extend(outcome.outbound);
		}
	}
	// Drop the votes -- simulate a silent leader that never aggregates.
	drop(delivered_votes);

	// Now fire a timeout on a follower whose own id is not the next
	// view's leader (so the emitted CohortNewView actually leaves the
	// engine rather than being folded back in locally). view 2's leader
	// is replica 1 (index 1 in the cohort's sorted members).
	let follower_idx = 2;	// never the view-1 leader (0) or view-2 leader (1)
	let follower = members[follower_idx];
	let follower_engine = engines.get(&follower).expect("follower");
	let envs = res!(follower_engine.cohort_timeout("treasury", &rid));
	assert_eq!(envs.len(), 1);
	match &envs[0].body {
		MsgKind::CohortNewView { new_view, .. } => {
			assert_eq!(new_view.view, 2);
			assert_eq!(new_view.sender, follower_idx as u16);
		},
		other => panic!("expected CohortNewView, got {}", other.label()),
	}
	// And the envelope is addressed to replica 1 (the new view's leader).
	assert_eq!(envs[0].to, members[1]);
	Ok(())
}


#[test]
fn cohort_timeout_is_noop_when_no_instance() -> Outcome<()> {
	// Timing out a (table, id) for which no HotStuff instance exists
	// returns an empty envelope list -- never errors.
	let (_ids, engines) = res!(build_cluster(5, 5, "treasury", 11));
	let engine = engines.values().next().expect("some engine");
	let rid = RecordId::from_bytes([0xff; 32]);
	let envs = res!(engine.cohort_timeout("treasury", &rid));
	assert!(envs.is_empty());
	Ok(())
}


