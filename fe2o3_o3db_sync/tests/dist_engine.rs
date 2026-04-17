#![cfg(feature = "dist")]
//! Integration tests for the distributed-Ozone engine.
//!
//! Tests cover the pure state-machine behaviour of the engine against the
//! in-memory [`MemoryStorage`] adapter: configuration validation, placement
//! decisions, write-path outbound construction, read-path local/remote
//! branching, inbound handling, response correlation, and peer-set / OAM
//! mutation.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_kademlia::id::NodeId;
use oxedyne_fe2o3_oam::{
	config::OamConfig,
	threshold::Threshold,
};
use oxedyne_fe2o3_o3db_sync::dist::{
	config::{
		Consistency,
		DistOzoneConfig,
		TableConfig,
	},
	engine::{
		DistOzone,
		GetOutcome,
		InboundOutcome,
		PollOutcome,
	},
	peer_set::PeerSet,
	placement::Placement,
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


/// Deterministic splitmix64 for reproducible tests.
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

	fn next_record_id(&mut self) -> RecordId {
		RecordId(*self.next_id().as_bytes())
	}
}


fn node_id_from_u8(b: u8) -> NodeId {
	let mut bytes = [0u8; 32];
	bytes[31] = b;
	NodeId::from_bytes(bytes)
}


// ---------------------------------------------------------------------------
// Config tests.
// ---------------------------------------------------------------------------

#[test]
fn config_rejects_empty_table_list() -> Outcome<()> {
	let oam = res!(OamConfig::new(20, 500));
	let err = DistOzoneConfig::new(
		node_id_from_u8(1),
		Vec::new(),
		oam,
		Vec::new(),
	);
	assert!(err.is_err());
	Ok(())
}

#[test]
fn config_rejects_duplicate_table_names() -> Outcome<()> {
	let oam = res!(OamConfig::new(20, 500));
	let tables = vec![
		res!(TableConfig::eventual("identity")),
		res!(TableConfig::eventual("identity")),
	];
	let err = DistOzoneConfig::new(node_id_from_u8(1), Vec::new(), oam, tables);
	assert!(err.is_err());
	Ok(())
}

#[test]
fn config_rejects_invalid_cohort_lambda() -> Outcome<()> {
	assert!(TableConfig::new(
		"treasury",
		Consistency::Cohort { lambda: 4 },
		TableConfig::DEFAULT_AE,
		TableConfig::DEFAULT_IBLT_CELLS,
	).is_err());
	assert!(TableConfig::new(
		"treasury",
		Consistency::Cohort { lambda: 6 },
		TableConfig::DEFAULT_AE,
		TableConfig::DEFAULT_IBLT_CELLS,
	).is_err());
	assert!(TableConfig::new(
		"treasury",
		Consistency::Cohort { lambda: 7 },
		TableConfig::DEFAULT_AE,
		TableConfig::DEFAULT_IBLT_CELLS,
	).is_ok());
	// Zero cells rejected.
	assert!(TableConfig::new(
		"identity",
		Consistency::Eventual,
		TableConfig::DEFAULT_AE,
		0,
	).is_err());
	Ok(())
}

#[test]
fn config_rejects_empty_table_name() -> Outcome<()> {
	assert!(TableConfig::eventual("").is_err());
	Ok(())
}


// ---------------------------------------------------------------------------
// Peer set tests.
// ---------------------------------------------------------------------------

#[test]
fn peer_set_filters_self_on_bootstrap() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let a = node_id_from_u8(2);
	let b = node_id_from_u8(3);
	let set = PeerSet::from_bootstrap(&me, vec![a, me, b, a]);
	// Self removed, duplicates removed, sorted.
	assert_eq!(set.len(), 2);
	assert!(!set.contains(&me));
	assert!(set.contains(&a));
	assert!(set.contains(&b));
	Ok(())
}

#[test]
fn peer_set_insert_is_idempotent() -> Outcome<()> {
	let mut set = PeerSet::new();
	let a = node_id_from_u8(7);
	assert!(set.insert(a));
	assert!(!set.insert(a));
	assert_eq!(set.len(), 1);
	Ok(())
}

#[test]
fn peer_set_is_sorted() -> Outcome<()> {
	let mut rng = Rng::new(0x5a5a);
	let mut set = PeerSet::new();
	for _ in 0..20 {
		set.insert(rng.next_id());
	}
	for pair in set.as_slice().windows(2) {
		assert!(pair[0].as_bytes() < pair[1].as_bytes(),
			"peer set not sorted");
	}
	Ok(())
}


// ---------------------------------------------------------------------------
// Placement service tests.
// ---------------------------------------------------------------------------

#[test]
fn placement_local_holder_branches_on_threshold() -> Outcome<()> {
	// Saturated threshold: every peer is a holder.
	let oam = res!(OamConfig::new(500, 500));
	let local = node_id_from_u8(1);
	let p = Placement::new(local, oam);
	assert!(matches!(p.threshold(), Threshold::All));
	assert!(p.i_am_holder(&RecordId::from_bytes([0; 32])));

	// Zero threshold: no peer is a holder.
	let oam = res!(OamConfig::new(0, 500));
	let p = Placement::new(local, oam);
	assert!(matches!(p.threshold(), Threshold::None));
	assert!(!p.i_am_holder(&RecordId::from_bytes([0; 32])));
	Ok(())
}

#[test]
fn placement_update_oam_refreshes_threshold() -> Outcome<()> {
	let local = node_id_from_u8(1);
	let mut p = Placement::new(local, res!(OamConfig::new(1, 1_000_000)));
	let first = *res!(p.threshold().as_bytes().ok_or_else(|| err!(
		"expected Bounded threshold"; Bug)));
	p.update_oam(res!(OamConfig::new(1, 2)));
	let second = *res!(p.threshold().as_bytes().ok_or_else(|| err!(
		"expected Bounded threshold"; Bug)));
	assert_ne!(first, second);
	Ok(())
}

#[test]
fn placement_holder_count_matches_decision() -> Outcome<()> {
	let oam = res!(OamConfig::new(100, 200));
	let me = node_id_from_u8(1);
	let p = Placement::new(me, oam);
	let mut set = PeerSet::new();
	let mut rng = Rng::new(0xdead);
	for _ in 0..50 {
		set.insert(rng.next_id());
	}
	let rid = rng.next_record_id();
	let decision = p.decide(&rid, &set);
	assert_eq!(
		decision.holder_count(),
		decision.remote_holders.len() + usize::from(decision.local_is_holder),
	);
	Ok(())
}


// ---------------------------------------------------------------------------
// DistOzone write-path tests.
// ---------------------------------------------------------------------------

fn build_engine(
	local:			NodeId,
	peers:			Vec<NodeId>,
	replication:	u64,
	network_size:	u64,
)
	-> Outcome<DistOzone<MemoryStorage>>
{
	let oam = res!(OamConfig::new(replication, network_size));
	let tables = vec![
		res!(TableConfig::eventual("identity")),
		res!(TableConfig::eventual("escrow")),
	];
	let cfg = res!(DistOzoneConfig::new(local, peers, oam, tables));
	DistOzone::new(cfg, MemoryStorage::new())
}


#[test]
fn put_rejects_unknown_table() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 20, 500));
	let rid = RecordId::from_bytes([0; 32]);
	let record = Record::new(rid, "missing", b"v".to_vec());
	assert!(engine.put(record).is_err());
	Ok(())
}

#[test]
fn put_on_cohort_table_enters_consensus() -> Outcome<()> {
	// On a cohort-backed table with no peers, the sole member is the
	// local node -- which is therefore the leader. `put` opens a HotStuff
	// round and reports consensus_pending. The record is *not* persisted
	// yet (that happens on Decide).
	let me = node_id_from_u8(1);
	let oam = res!(OamConfig::new(20, 500));
	let tables = vec![res!(TableConfig::cohort_default("treasury"))];
	let cfg = res!(DistOzoneConfig::new(me, Vec::new(), oam, tables));
	let engine = res!(DistOzone::new(cfg, MemoryStorage::new()));
	let rid = RecordId::from_bytes([0; 32]);
	let record = Record::new(rid, "treasury", b"v".to_vec());
	let outcome = res!(engine.put(record.clone()));
	assert!(!outcome.local_persisted);
	assert_eq!(outcome.consensus_pending, Some((record.table.clone(), rid)));
	Ok(())
}

#[test]
fn put_with_saturated_threshold_reaches_everyone() -> Outcome<()> {
	// Replication == network size => everyone holds everything.
	let me = node_id_from_u8(1);
	let peers: Vec<NodeId> = (2..=5).map(node_id_from_u8).collect();
	let engine = res!(build_engine(me, peers.clone(), 5, 5));
	let rid = RecordId::from_bytes([7; 32]);
	let record = Record::new(rid, "identity", b"hello".to_vec());
	let outcome = res!(engine.put(record.clone()));
	assert!(outcome.local_persisted);
	assert_eq!(outcome.outbound.len(), peers.len());
	// Every outbound goes to a distinct remote peer.
	let mut seen: Vec<NodeId> = outcome.outbound.iter().map(|e| e.to).collect();
	seen.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
	seen.dedup();
	assert_eq!(seen.len(), peers.len());
	// Every outbound is a ReplicatePut with the right record.
	for env in &outcome.outbound {
		assert_eq!(env.from, me);
		match &env.body {
			MsgKind::ReplicatePut { record: r } => assert_eq!(r, &record),
			other => panic!("unexpected outbound: {:?}", other),
		}
	}
	// Local store got the record.
	let stored = res!(engine.storage().get("identity", &rid));
	assert_eq!(stored, Some(record));
	Ok(())
}

#[test]
fn put_with_empty_threshold_writes_nowhere() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let peers: Vec<NodeId> = (2..=10).map(node_id_from_u8).collect();
	let engine = res!(build_engine(me, peers, 0, 10)); // n=0: no holders.
	let record = Record::new(
		RecordId::from_bytes([9; 32]), "identity", b"x".to_vec(),
	);
	let outcome = res!(engine.put(record));
	assert!(!outcome.local_persisted);
	assert!(outcome.outbound.is_empty());
	Ok(())
}


// ---------------------------------------------------------------------------
// DistOzone read-path tests.
// ---------------------------------------------------------------------------

#[test]
fn get_local_returns_record_when_holder() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 100, 100));
	let rid = RecordId::from_bytes([4; 32]);
	let record = Record::new(rid, "identity", b"v".to_vec());
	let _ = res!(engine.put(record.clone()));
	let outcome = res!(engine.get("identity", &rid));
	match outcome {
		GetOutcome::Local(r) => assert_eq!(r, record),
		other => panic!("expected Local, got {:?}", other),
	}
	Ok(())
}

#[test]
fn get_local_miss_when_holder_but_no_record() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 100, 100));
	let outcome = res!(engine.get("identity", &RecordId::from_bytes([9; 32])));
	assert!(matches!(outcome, GetOutcome::LocalMiss));
	Ok(())
}

#[test]
fn get_with_zero_threshold_and_peers_routes_remote() -> Outcome<()> {
	// n = 0 means the local peer is not a holder. With peers known, a remote
	// read is scheduled.
	let me = node_id_from_u8(1);
	let peers: Vec<NodeId> = (2..=10).map(node_id_from_u8).collect();
	let engine = res!(build_engine(me, peers.clone(), 0, 10));
	let rid = RecordId::from_bytes([5; 32]);
	let outcome = res!(engine.get("identity", &rid));
	match outcome {
		GetOutcome::Remote { request_id, outbound } => {
			assert!(request_id > 0);
			assert!(!outbound.is_empty());
			assert!(outbound.len() <= peers.len());
			for env in &outbound {
				assert_eq!(env.from, me);
				match &env.body {
					MsgKind::GetRequest { request_id: rid_msg, table, id } => {
						assert_eq!(*rid_msg, request_id);
						assert_eq!(table, "identity");
						assert_eq!(id, &rid);
					},
					other => panic!("unexpected outbound: {:?}", other),
				}
			}
		},
		other => panic!("expected Remote, got {:?}", other),
	}
	Ok(())
}

#[test]
fn get_no_peers_and_not_holder_returns_notargets() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 0, 10));
	let rid = RecordId::from_bytes([0; 32]);
	let outcome = res!(engine.get("identity", &rid));
	assert!(matches!(outcome, GetOutcome::NoTargets));
	Ok(())
}


// ---------------------------------------------------------------------------
// Inbound handling tests.
// ---------------------------------------------------------------------------

#[test]
fn handle_replicate_put_persists_when_holder() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 100, 100));
	let sender = node_id_from_u8(2);
	let record = Record::new(
		RecordId::from_bytes([3; 32]),
		"identity",
		b"payload".to_vec(),
	);
	let env = Envelope::new(sender, me, MsgKind::ReplicatePut {
		record: record.clone(),
	});
	let out = res!(engine.handle_envelope(env));
	assert!(out.outbound.is_empty());
	let stored = res!(engine.storage().get("identity", &record.id));
	assert_eq!(stored, Some(record));
	Ok(())
}

#[test]
fn handle_replicate_put_drops_when_not_holder() -> Outcome<()> {
	// n = 0: not a holder of anything. Incoming put must not be persisted.
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 0, 10));
	let sender = node_id_from_u8(2);
	let record = Record::new(
		RecordId::from_bytes([3; 32]),
		"identity",
		b"payload".to_vec(),
	);
	let env = Envelope::new(sender, me, MsgKind::ReplicatePut {
		record: record.clone(),
	});
	let out = res!(engine.handle_envelope(env));
	assert!(out.outbound.is_empty());
	assert_eq!(res!(engine.storage().len()), 0);
	Ok(())
}

#[test]
fn handle_replicate_put_rejects_unknown_table() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 100, 100));
	let sender = node_id_from_u8(2);
	let record = Record::new(
		RecordId::from_bytes([3; 32]),
		"missing",
		b"payload".to_vec(),
	);
	let env = Envelope::new(sender, me, MsgKind::ReplicatePut { record });
	assert!(engine.handle_envelope(env).is_err());
	Ok(())
}

#[test]
fn handle_get_request_responds_with_record() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 100, 100));
	let record = Record::new(
		RecordId::from_bytes([5; 32]),
		"identity",
		b"aaa".to_vec(),
	);
	let _ = res!(engine.put(record.clone()));
	let requester = node_id_from_u8(2);
	let env = Envelope::new(requester, me, MsgKind::GetRequest {
		request_id:	42,
		table:		"identity".to_string(),
		id:			record.id,
	});
	let out = res!(engine.handle_envelope(env));
	assert_eq!(out.outbound.len(), 1);
	let reply = &out.outbound[0];
	assert_eq!(reply.to, requester);
	match &reply.body {
		MsgKind::GetResponse { request_id, record: r } => {
			assert_eq!(*request_id, 42);
			assert_eq!(r.as_ref(), Some(&record));
		},
		other => panic!("expected GetResponse, got {:?}", other),
	}
	Ok(())
}

#[test]
fn handle_get_response_completes_pending_read() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let peers: Vec<NodeId> = (2..=5).map(node_id_from_u8).collect();
	let mut engine = res!(build_engine(me, peers, 0, 10));
	engine.set_read_fanout(1);

	let rid = RecordId::from_bytes([7; 32]);
	let outcome = res!(engine.get("identity", &rid));
	let (request_id, outbound) = match outcome {
		GetOutcome::Remote { request_id, outbound } => (request_id, outbound),
		other => panic!("expected Remote, got {:?}", other),
	};
	assert_eq!(outbound.len(), 1);

	// Still pending.
	assert!(matches!(res!(engine.poll_get(request_id)), PollOutcome::Pending));

	// Target replies with the record.
	let target = outbound[0].to;
	let record = Record::new(rid, "identity", b"found".to_vec());
	let response = Envelope::new(target, me, MsgKind::GetResponse {
		request_id,
		record:	Some(record.clone()),
	});
	let in_out = res!(engine.handle_envelope(response));
	assert_eq!(in_out.completed_get, Some(request_id));

	match res!(engine.poll_get(request_id)) {
		PollOutcome::Record(r) => assert_eq!(r, record),
		other => panic!("expected Record, got {:?}", other),
	}
	Ok(())
}

#[test]
fn handle_get_response_resolves_notfound_after_all_miss() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let peers: Vec<NodeId> = (2..=4).map(node_id_from_u8).collect();
	let mut engine = res!(build_engine(me, peers, 0, 10));
	engine.set_read_fanout(3);

	let rid = RecordId::from_bytes([2; 32]);
	let (request_id, outbound) = match res!(engine.get("identity", &rid)) {
		GetOutcome::Remote { request_id, outbound } => (request_id, outbound),
		other => panic!("expected Remote, got {:?}", other),
	};
	assert_eq!(outbound.len(), 3);

	// All three targets reply empty.
	let mut last: Option<InboundOutcome> = None;
	for env in outbound {
		let resp = Envelope::new(env.to, me, MsgKind::GetResponse {
			request_id,
			record:	None,
		});
		last = Some(res!(engine.handle_envelope(resp)));
	}
	let last = res!(last.ok_or_else(|| err!("no outbound"; Bug)));
	assert_eq!(last.completed_get, Some(request_id));
	assert!(matches!(res!(engine.poll_get(request_id)), PollOutcome::NotFound));
	Ok(())
}

#[test]
fn handle_get_response_unknown_request_id_is_ignored() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 100, 100));
	let sender = node_id_from_u8(2);
	let env = Envelope::new(sender, me, MsgKind::GetResponse {
		request_id:	9999,
		record:	None,
	});
	let out = res!(engine.handle_envelope(env));
	assert!(out.outbound.is_empty());
	assert_eq!(out.completed_get, None);
	Ok(())
}

#[test]
fn cancel_get_drops_pending_state() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let peers: Vec<NodeId> = (2..=5).map(node_id_from_u8).collect();
	let engine = res!(build_engine(me, peers, 0, 10));
	let rid = RecordId::from_bytes([0xaa; 32]);
	let (request_id, _) = match res!(engine.get("identity", &rid)) {
		GetOutcome::Remote { request_id, outbound } => (request_id, outbound),
		other => panic!("expected Remote, got {:?}", other),
	};
	res!(engine.cancel_get(request_id));
	assert!(matches!(res!(engine.poll_get(request_id)), PollOutcome::Unknown));
	Ok(())
}

#[test]
fn handle_envelope_rejects_misaddressed() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let engine = res!(build_engine(me, Vec::new(), 100, 100));
	let sender = node_id_from_u8(2);
	let other = node_id_from_u8(3);
	let env = Envelope::new(sender, other, MsgKind::ReplicatePut {
		record: Record::new(
			RecordId::from_bytes([0; 32]), "identity", b"v".to_vec(),
		),
	});
	let out = res!(engine.handle_envelope(env));
	assert!(out.outbound.is_empty());
	assert_eq!(out.completed_get, None);
	// Nothing got stored.
	assert_eq!(res!(engine.storage().len()), 0);
	Ok(())
}


// ---------------------------------------------------------------------------
// Peer-set / OAM mutation.
// ---------------------------------------------------------------------------

#[test]
fn insert_peer_rejects_self() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let mut engine = res!(build_engine(me, Vec::new(), 20, 500));
	assert!(!engine.insert_peer(me));
	assert_eq!(engine.peer_set().len(), 0);
	Ok(())
}

#[test]
fn insert_and_remove_peer_round_trip() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let a = node_id_from_u8(2);
	let mut engine = res!(build_engine(me, Vec::new(), 20, 500));
	assert!(engine.insert_peer(a));
	assert!(!engine.insert_peer(a));
	assert_eq!(engine.peer_set().len(), 1);
	assert!(engine.remove_peer(&a));
	assert!(!engine.remove_peer(&a));
	assert_eq!(engine.peer_set().len(), 0);
	Ok(())
}

#[test]
fn update_network_size_refreshes_threshold() -> Outcome<()> {
	let me = node_id_from_u8(1);
	let mut engine = res!(build_engine(me, Vec::new(), 1, 1_000_000));
	let first = *res!(engine.placement().threshold().as_bytes()
		.ok_or_else(|| err!("expected Bounded threshold"; Bug)));
	res!(engine.update_network_size(2));
	let second = *res!(engine.placement().threshold().as_bytes()
		.ok_or_else(|| err!("expected Bounded threshold"; Bug)));
	assert_ne!(first, second);
	// n=1, N=2: top bit set.
	assert_eq!(second[0], 0x80);
	Ok(())
}


// ---------------------------------------------------------------------------
// End-to-end simulation across two engines.
// ---------------------------------------------------------------------------

#[test]
fn two_peer_replicate_and_read_back() -> Outcome<()> {
	// Peer A (me) writes a record; peer B's engine receives the replicate and
	// persists it. Then B serves a get request from a third peer (simulated
	// by us sending a GetRequest to B).
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);

	let engine_a = res!(build_engine(a, vec![b], 5, 5));
	let engine_b = res!(build_engine(b, vec![a], 5, 5));

	let rid = RecordId::from_bytes([0x1a; 32]);
	let record = Record::new(rid, "identity", b"hello".to_vec());

	// A puts.
	let put_outcome = res!(engine_a.put(record.clone()));
	assert!(put_outcome.local_persisted);
	assert_eq!(put_outcome.outbound.len(), 1);
	let env_to_b = &put_outcome.outbound[0];
	assert_eq!(env_to_b.to, b);

	// B handles A's replicate put.
	let in_out = res!(engine_b.handle_envelope(env_to_b.clone()));
	assert!(in_out.outbound.is_empty());
	let stored = res!(engine_b.storage().get("identity", &rid));
	assert_eq!(stored, Some(record.clone()));

	// Simulate a third peer asking B for the record.
	let requester = node_id_from_u8(99);
	let ask = Envelope::new(requester, b, MsgKind::GetRequest {
		request_id:	7,
		table:		"identity".to_string(),
		id:			rid,
	});
	let out = res!(engine_b.handle_envelope(ask));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::GetResponse { request_id, record: r } => {
			assert_eq!(*request_id, 7);
			assert_eq!(r.as_ref(), Some(&record));
		},
		other => panic!("expected GetResponse, got {:?}", other),
	}
	Ok(())
}
