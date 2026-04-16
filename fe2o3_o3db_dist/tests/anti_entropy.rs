//! Integration tests for the IBLT anti-entropy reconciliation cycle.
//!
//! Tests cover:
//!
//! - Digest envelope construction and rejection on cohort / unknown tables.
//! - Digest handling when the peers are already in sync (zero diff).
//! - Digest handling with a small symmetric difference (both directions).
//! - Reply -> Push follow-up, exchanging records in the direction the
//!   recipient of the reply lacks them.
//! - Bulk-reply fallback when the sketch is overloaded.
//! - End-to-end convergence across a three-message round.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_kademlia::id::NodeId;
use oxedyne_fe2o3_oam::config::OamConfig;
use oxedyne_fe2o3_o3db_dist::{
	config::{
		Consistency,
		DistOzoneConfig,
		TableConfig,
	},
	dist::DistOzone,
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

use std::time::Duration;


fn node_id_from_u8(b: u8) -> NodeId {
	let mut bytes = [0u8; 32];
	bytes[31] = b;
	NodeId::from_bytes(bytes)
}

fn record_id_from_u8(b: u8) -> RecordId {
	let mut bytes = [0u8; 32];
	bytes[0] = b;
	RecordId::from_bytes(bytes)
}

/// Builds two engines, each considering itself the sole holder of
/// everything in its tables. Replication factor equals network size so
/// placement re-checks never drop a record.
fn build_two_engines(
	a: NodeId,
	b: NodeId,
	table_cells: usize,
)
	-> Outcome<(DistOzone<MemoryStorage>, DistOzone<MemoryStorage>)>
{
	let oam = res!(OamConfig::new(2, 2));
	let table = res!(TableConfig::new(
		"identity",
		Consistency::Eventual,
		Duration::from_secs(30),
		table_cells,
	));
	let cfg_a = res!(DistOzoneConfig::new(
		a, vec![b], oam, vec![table.clone()],
	));
	let cfg_b = res!(DistOzoneConfig::new(
		b, vec![a], oam, vec![table],
	));
	Ok((
		res!(DistOzone::new(cfg_a, MemoryStorage::new())),
		res!(DistOzone::new(cfg_b, MemoryStorage::new())),
	))
}


#[test]
fn build_anti_entropy_request_rejects_unknown_table() -> Outcome<()> {
	let (engine_a, _) = res!(build_two_engines(
		node_id_from_u8(1), node_id_from_u8(2), 64,
	));
	assert!(engine_a.build_anti_entropy_request(
		"missing", node_id_from_u8(2),
	).is_err());
	Ok(())
}

#[test]
fn build_anti_entropy_request_rejects_cohort_table() -> Outcome<()> {
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let oam = res!(OamConfig::new(2, 2));
	let eventual = res!(TableConfig::eventual("identity"));
	let cohort = res!(TableConfig::cohort_default("treasury"));
	let cfg = res!(DistOzoneConfig::new(
		a, vec![b], oam, vec![eventual, cohort],
	));
	let engine = res!(DistOzone::new(cfg, MemoryStorage::new()));
	assert!(engine.build_anti_entropy_request("treasury", b).is_err());
	Ok(())
}

#[test]
fn anti_entropy_with_identical_tables_decodes_empty() -> Outcome<()> {
	// Both engines hold the same record; digest exchange must produce an
	// empty reply.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (engine_a, engine_b) = res!(build_two_engines(a, b, 64));

	let rid = record_id_from_u8(7);
	let record = Record::new(rid, "identity", b"same".to_vec());
	res!(engine_a.storage().put(&record));
	res!(engine_b.storage().put(&record));

	let digest = res!(engine_a.build_anti_entropy_request("identity", b));
	let out = res!(engine_b.handle_envelope(digest));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::AntiEntropyReply { records, requested_ids, bulk, .. } => {
			assert!(records.is_empty(), "expected no records in reply");
			assert!(requested_ids.is_empty(), "expected no requested ids");
			assert!(!bulk);
		},
		other => panic!("expected AntiEntropyReply, got {:?}", other),
	}
	Ok(())
}

#[test]
fn anti_entropy_sender_has_extra_record_receiver_requests_it() -> Outcome<()> {
	// A has record X, B does not. A's digest is received by B; B's reply
	// requests X.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (engine_a, engine_b) = res!(build_two_engines(a, b, 64));

	let rid = record_id_from_u8(3);
	let record = Record::new(rid, "identity", b"a-only".to_vec());
	res!(engine_a.storage().put(&record));

	let digest = res!(engine_a.build_anti_entropy_request("identity", b));
	let out = res!(engine_b.handle_envelope(digest));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::AntiEntropyReply { records, requested_ids, bulk, .. } => {
			assert!(records.is_empty(),
				"B shouldn't have anything to send to A");
			assert_eq!(requested_ids.len(), 1);
			assert_eq!(requested_ids[0], rid);
			assert!(!bulk);
		},
		other => panic!("expected AntiEntropyReply, got {:?}", other),
	}
	Ok(())
}

#[test]
fn anti_entropy_receiver_has_extra_record_replies_with_it() -> Outcome<()> {
	// B has record Y, A does not. A's digest contains nothing new; B's
	// reply contains Y as records-for-sender.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (engine_a, engine_b) = res!(build_two_engines(a, b, 64));

	let rid = record_id_from_u8(5);
	let record = Record::new(rid, "identity", b"b-only".to_vec());
	res!(engine_b.storage().put(&record));

	let digest = res!(engine_a.build_anti_entropy_request("identity", b));
	let out = res!(engine_b.handle_envelope(digest));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::AntiEntropyReply { records, requested_ids, bulk, .. } => {
			assert_eq!(records.len(), 1);
			assert_eq!(records[0], record);
			assert!(requested_ids.is_empty());
			assert!(!bulk);
		},
		other => panic!("expected AntiEntropyReply, got {:?}", other),
	}
	Ok(())
}

#[test]
fn anti_entropy_reply_persists_received_records() -> Outcome<()> {
	// A receives a reply from B containing a record A is missing. A
	// persists it through handle_envelope.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (engine_a, _) = res!(build_two_engines(a, b, 64));

	let rid = record_id_from_u8(11);
	let record = Record::new(rid, "identity", b"payload".to_vec());

	let reply = Envelope::new(b, a, MsgKind::AntiEntropyReply {
		table:			"identity".to_string(),
		records:		vec![record.clone()],
		requested_ids:	Vec::new(),
		bulk:			false,
	});
	let out = res!(engine_a.handle_envelope(reply));
	assert!(out.outbound.is_empty(),
		"no requested ids => no push follow-up");
	let stored = res!(engine_a.storage().get("identity", &rid));
	assert_eq!(stored, Some(record));
	Ok(())
}

#[test]
fn anti_entropy_reply_triggers_push_for_requested_ids() -> Outcome<()> {
	// A holds a record that B's reply requests; A's handle_envelope on
	// that reply emits an AntiEntropyPush carrying the record.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (engine_a, _) = res!(build_two_engines(a, b, 64));

	let rid = record_id_from_u8(13);
	let record = Record::new(rid, "identity", b"x".to_vec());
	res!(engine_a.storage().put(&record));

	let reply = Envelope::new(b, a, MsgKind::AntiEntropyReply {
		table:			"identity".to_string(),
		records:		Vec::new(),
		requested_ids:	vec![rid],
		bulk:			false,
	});
	let out = res!(engine_a.handle_envelope(reply));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::AntiEntropyPush { table, records } => {
			assert_eq!(table, "identity");
			assert_eq!(records.len(), 1);
			assert_eq!(records[0], record);
		},
		other => panic!("expected AntiEntropyPush, got {:?}", other),
	}
	Ok(())
}

#[test]
fn anti_entropy_push_persists_records() -> Outcome<()> {
	// B receives a Push from A carrying a record B is missing.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (_, engine_b) = res!(build_two_engines(a, b, 64));

	let rid = record_id_from_u8(17);
	let record = Record::new(rid, "identity", b"pushed".to_vec());
	let push = Envelope::new(a, b, MsgKind::AntiEntropyPush {
		table:		"identity".to_string(),
		records:	vec![record.clone()],
	});
	let out = res!(engine_b.handle_envelope(push));
	assert!(out.outbound.is_empty());
	assert_eq!(
		res!(engine_b.storage().get("identity", &rid)),
		Some(record),
	);
	Ok(())
}

#[test]
fn anti_entropy_overload_triggers_bulk_reply() -> Outcome<()> {
	// With a tiny sketch (num_cells = 6) and many records the IBLT decode
	// is overloaded. B must fall back to a bulk reply, including every
	// record it holds for the table.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (engine_a, engine_b) = res!(build_two_engines(a, b, 6));

	// Populate B with 50 records that A has none of.
	for i in 0..50u8 {
		let rid = record_id_from_u8(i);
		let record = Record::new(
			rid, "identity", vec![i, i ^ 0xa5],
		);
		res!(engine_b.storage().put(&record));
	}

	let digest = res!(engine_a.build_anti_entropy_request("identity", b));
	let out = res!(engine_b.handle_envelope(digest));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::AntiEntropyReply { records, requested_ids, bulk, .. } => {
			assert!(*bulk, "expected bulk fallback on overloaded sketch");
			assert_eq!(records.len(), 50);
			assert!(requested_ids.is_empty());
		},
		other => panic!("expected AntiEntropyReply, got {:?}", other),
	}
	Ok(())
}

#[test]
fn anti_entropy_full_round_converges_two_peers() -> Outcome<()> {
	// End-to-end: A has records {1, 2, 3}; B has records {2, 3, 4}. One
	// anti-entropy round leaves both peers holding {1, 2, 3, 4}.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (engine_a, engine_b) = res!(build_two_engines(a, b, 64));

	let rec = |i: u8| -> Record {
		Record::new(
			record_id_from_u8(i),
			"identity",
			vec![i],
		)
	};
	for i in [1u8, 2, 3] {
		res!(engine_a.storage().put(&rec(i)));
	}
	for i in [2u8, 3, 4] {
		res!(engine_b.storage().put(&rec(i)));
	}

	// Round 1: A -> B digest. B replies.
	let digest = res!(engine_a.build_anti_entropy_request("identity", b));
	let reply_out = res!(engine_b.handle_envelope(digest));
	assert_eq!(reply_out.outbound.len(), 1);

	// A handles reply; should persist rec(4) and push rec(1).
	let reply_env = reply_out.outbound[0].clone();
	let push_out = res!(engine_a.handle_envelope(reply_env));
	assert_eq!(push_out.outbound.len(), 1, "expected a push for rec(1)");

	// B handles the push; should persist rec(1).
	let push_env = push_out.outbound[0].clone();
	let tail_out = res!(engine_b.handle_envelope(push_env));
	assert!(tail_out.outbound.is_empty());

	// Final state: both hold {1, 2, 3, 4}.
	for i in 1u8..=4 {
		let rid = record_id_from_u8(i);
		assert_eq!(
			res!(engine_a.storage().get("identity", &rid)),
			Some(rec(i)),
			"engine_a missing record {}", i,
		);
		assert_eq!(
			res!(engine_b.storage().get("identity", &rid)),
			Some(rec(i)),
			"engine_b missing record {}", i,
		);
	}
	Ok(())
}

#[test]
fn anti_entropy_digest_rejects_cohort_table_on_receive() -> Outcome<()> {
	// Malicious or misconfigured sender directs a digest at a cohort-
	// backed table; handler rejects.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let oam = res!(OamConfig::new(2, 2));
	let eventual = res!(TableConfig::eventual("identity"));
	let cohort = res!(TableConfig::cohort_default("treasury"));
	let cfg_b = res!(DistOzoneConfig::new(
		b, vec![a], oam, vec![eventual, cohort],
	));
	let engine_b = res!(DistOzone::new(cfg_b, MemoryStorage::new()));

	// Hand-built envelope with an empty sketch -- we won't get that far.
	let fake_digest = Envelope::new(a, b, MsgKind::AntiEntropyDigest {
		table:	"treasury".to_string(),
		sketch:	vec![0u8; 100],
	});
	assert!(engine_b.handle_envelope(fake_digest).is_err());
	Ok(())
}

#[test]
fn anti_entropy_digest_rejects_unknown_table() -> Outcome<()> {
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let (_, engine_b) = res!(build_two_engines(a, b, 64));
	let fake = Envelope::new(a, b, MsgKind::AntiEntropyDigest {
		table:	"missing".to_string(),
		sketch:	vec![0u8; 100],
	});
	assert!(engine_b.handle_envelope(fake).is_err());
	Ok(())
}

#[test]
fn anti_entropy_digest_rejects_mismatched_sketch_shape() -> Outcome<()> {
	// Wire up two engines with differing iblt_cells for the same table.
	// A builds a digest; B's handler detects the config mismatch and
	// errors rather than silently decoding nonsense.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let oam = res!(OamConfig::new(2, 2));

	let table_a = res!(TableConfig::new(
		"identity",
		Consistency::Eventual,
		Duration::from_secs(30),
		64,
	));
	let table_b = res!(TableConfig::new(
		"identity",
		Consistency::Eventual,
		Duration::from_secs(30),
		128,
	));
	let cfg_a = res!(DistOzoneConfig::new(
		a, vec![b], oam, vec![table_a],
	));
	let cfg_b = res!(DistOzoneConfig::new(
		b, vec![a], oam, vec![table_b],
	));
	let engine_a = res!(DistOzone::new(cfg_a, MemoryStorage::new()));
	let engine_b = res!(DistOzone::new(cfg_b, MemoryStorage::new()));

	let digest = res!(engine_a.build_anti_entropy_request("identity", b));
	assert!(engine_b.handle_envelope(digest).is_err());
	Ok(())
}

#[test]
fn anti_entropy_reply_placement_rechecks_incoming() -> Outcome<()> {
	// Receiver-side placement re-check: a reply delivering a record to a
	// peer that does not consider itself a holder must drop the record
	// rather than persisting it.
	let a = node_id_from_u8(1);
	let b = node_id_from_u8(2);
	let oam = res!(OamConfig::new(0, 2)); // n=0: no peer is ever a holder.
	let table = res!(TableConfig::eventual("identity"));
	let cfg_b = res!(DistOzoneConfig::new(
		b, vec![a], oam, vec![table],
	));
	let engine_b = res!(DistOzone::new(cfg_b, MemoryStorage::new()));

	let record = Record::new(
		record_id_from_u8(99),
		"identity",
		b"nope".to_vec(),
	);
	let reply = Envelope::new(a, b, MsgKind::AntiEntropyReply {
		table:			"identity".to_string(),
		records:		vec![record.clone()],
		requested_ids:	Vec::new(),
		bulk:			false,
	});
	let _ = res!(engine_b.handle_envelope(reply));
	assert_eq!(res!(engine_b.storage().len()), 0);
	Ok(())
}
