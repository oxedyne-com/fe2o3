#![cfg(feature = "dist")]
//! Integration tests for the distributed-Ozone wire codec.
//!
//! The layout tests state the wire form independently, as `Dat` trees and as bytes built by hand,
//! so a codec that agrees only with itself cannot pass them.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
	dist::{
		codec::{
			MAX_ENVELOPE_BYTES,
			MAX_ENVELOPE_DEPTH,
			MAX_ENVELOPE_VALUES,
			WIRE_VERSION,
		},
		hotstuff::types::{
			NewView,
			Phase,
			Proposal,
			Qc,
			Vote,
		},
		record::{
			Record,
			RecordId,
		},
		transport::{
			Envelope,
			MsgKind,
		},
	},
	kademlia::id::NodeId,
};

use std::collections::BTreeSet;


// Distinct, non-uniform bytes, so a swapped or shifted field cannot hide.
fn bytes32(n: u8) -> [u8; 32] {
	let mut b = [0u8; 32];
	for (i, x) in b.iter_mut().enumerate() {
		*x = n.wrapping_mul(31).wrapping_add(i as u8);
	}
	b
}

fn rid(n: u8) -> RecordId {
	RecordId::from_bytes(bytes32(n))
}

fn nid(n: u8) -> NodeId {
	NodeId::from_bytes(bytes32(n))
}

fn payload(n: u8, len: usize) -> Vec<u8> {
	(0..len).map(|i| (i as u8).wrapping_mul(7).wrapping_add(n)).collect()
}

fn rec(n: u8, table: &str, len: usize) -> Record {
	Record::new(rid(n), table, payload(n, len))
}

fn env(body: MsgKind) -> Envelope {
	Envelope::new(nid(1), nid(2), body)
}

fn qc(view: u64, phase: Phase, voters: &[u16]) -> Qc {
	Qc {
		view,
		phase,
		block_hash:	bytes32(9),
		signatures:	voters.iter().map(|v| (*v, payload(*v as u8, 64))).collect(),
	}
}

const PHASES: [Phase; 4] = [Phase::Prepare, Phase::PreCommit, Phase::Commit, Phase::Decide];

// The nine-deep shape: the greatest the encoder writes.
fn deepest() -> Envelope {
	env(MsgKind::CohortNewView {
		table:		"t".to_string(),
		id:			rid(3),
		new_view:	NewView { view: 2, sender: 1, prepare_qc: Some(qc(1, Phase::Prepare, &[0, 1])) },
	})
}

// One of every kind, with the variations that change the bytes.
fn samples() -> Vec<Envelope> {
	let mut v = Vec::new();
	// ReplicatePut: a value past the 255 bytes a Dat::BU8 can count, a short one, an empty one.
	v.push(env(MsgKind::ReplicatePut { record: rec(1, "ledger", 300) }));
	v.push(env(MsgKind::ReplicatePut { record: rec(2, "", 3) }));
	v.push(env(MsgKind::ReplicatePut { record: rec(3, "ledger", 0) }));
	v.push(env(MsgKind::GetRequest { request_id: u64::MAX, table: "ledger".to_string(), id: rid(4) }));
	v.push(env(MsgKind::GetResponse { request_id: 0, record: None }));
	v.push(env(MsgKind::GetResponse { request_id: 7, record: Some(rec(5, "ledger", 40)) }));
	v.push(env(MsgKind::AntiEntropyDigest { table: "ledger".to_string(), sketch: payload(6, 5000), after: None }));
	v.push(env(MsgKind::AntiEntropyDigest { table: "ledger".to_string(), sketch: Vec::new(), after: Some(rid(14)) }));
	// AntiEntropyReply: both bulk settings, empty and full vectors, and every cursor shape.
	for bulk in [false, true] {
		for (after, next) in [(None, None), (Some(rid(15)), None), (None, Some(rid(16))), (Some(rid(15)), Some(rid(16)))] {
			v.push(env(MsgKind::AntiEntropyReply {
				table:			"ledger".to_string(),
				records:		Vec::new(),
				requested_ids:	Vec::new(),
				bulk,
				after,
				next,
			}));
			v.push(env(MsgKind::AntiEntropyReply {
				table:			"ledger".to_string(),
				records:		vec![rec(7, "ledger", 10), rec(8, "ledger", 0)],
				requested_ids:	vec![rid(9), rid(10), rid(11)],
				bulk,
				after,
				next,
			}));
		}
	}
	v.push(env(MsgKind::AntiEntropyPush { table: "ledger".to_string(), records: Vec::new() }));
	v.push(env(MsgKind::AntiEntropyPush { table: "ledger".to_string(), records: vec![rec(12, "ledger", 300)] }));
	v.push(env(MsgKind::CohortSubmit { record: rec(13, "names", 21) }));
	// CohortPropose: every phase, with and without the block and the justifying QC.
	for phase in PHASES {
		for block in [None, Some(payload(2, 300))] {
			for justify in [None, Some(qc(5, Phase::Prepare, &[]))] {
				v.push(env(MsgKind::CohortPropose {
					table:		"names".to_string(),
					id:			rid(14),
					proposal:	Proposal {
						view:		u64::MAX,
						phase,
						block_hash:	bytes32(15),
						block:		block.clone(),
						justify,
					},
				}));
			}
		}
	}
	v.push(env(MsgKind::CohortPropose {
		table:		"names".to_string(),
		id:			rid(14),
		proposal:	Proposal {
			view:		3,
			phase:		Phase::Commit,
			block_hash:	bytes32(15),
			block:		Some(Vec::new()),
			justify:	Some(qc(3, Phase::PreCommit, &[0, 2, 3, u16::MAX])),
		},
	}));
	for phase in PHASES {
		v.push(env(MsgKind::CohortVote {
			table:	"names".to_string(),
			id:		rid(16),
			vote:	Vote { view: 4, phase, block_hash: bytes32(17), voter: u16::MAX, signature: payload(1, 64) },
		}));
	}
	v.push(env(MsgKind::CohortNewView {
		table:		"names".to_string(),
		id:			rid(18),
		new_view:	NewView { view: 5, sender: 0, prepare_qc: None },
	}));
	v.push(deepest());
	v
}

fn tags_of(e: &Error<ErrTag>) -> Vec<ErrTag> {
	e.tags()
}

// A refusal of malformed input names Decode and Input.
fn assert_refused(what: &str, bytes: &[u8]) {
	match Envelope::decode(bytes) {
		Ok(e) => panic!("{}: decode accepted {} bytes as {:?}.", what, bytes.len(), e.body.label()),
		Err(e) => {
			let tags = tags_of(&e);
			assert!(
				tags.contains(&ErrTag::Decode) && tags.contains(&ErrTag::Input),
				"{}: the refusal carries {:?}, not Decode and Input: {}", what, tags, e,
			);
		},
	}
}

fn bdat(dat: &Dat) -> Vec<u8> {
	match dat.to_bytes(Vec::new()) {
		Ok(b) => b,
		Err(e) => panic!("The hand-built Dat did not encode: {}", e),
	}
}

fn b32(n: u8) -> Dat {
	Dat::B32(B32(bytes32(n)))
}

fn s(x: &str) -> Dat {
	Dat::Str(x.to_string())
}

fn list(v: Vec<Dat>) -> Dat {
	Dat::List(v)
}

fn opt(d: Option<Dat>) -> Dat {
	Dat::Opt(Box::new(d))
}

// The wire form of a record, built by hand: [B32 id, Str table, BU64 value].
fn record_dat(n: u8, table: &str, len: usize) -> Dat {
	list(vec![b32(n), s(table), Dat::BU64(payload(n, len))])
}

// A ReplicatePut envelope, built by hand from the layout and not from the codec.
fn put_dat(version: u8, kind: u8) -> Dat {
	list(vec![
		Dat::U8(version),
		b32(1),
		b32(2),
		list(vec![Dat::U8(kind), record_dat(5, "ledger", 6)]),
	])
}

#[test]
fn every_kind_round_trips() -> Outcome<()> {
	let all = samples();
	let labels: BTreeSet<&str> = all.iter().map(|e| e.body.label()).collect();
	assert_eq!(labels.len(), 10, "The samples cover only {:?}.", labels);
	for e in &all {
		let bytes = res!(e.encode());
		let back = res!(Envelope::decode(&bytes));
		assert_eq!(&back, e, "{} did not survive its own encoding.", e.body.label());
		assert_eq!(res!(back.encode()), bytes, "{} re-encodes to other bytes.", e.body.label());
	}
	Ok(())
}

#[test]
fn wire_layout_is_pinned() -> Outcome<()> {
	let qc_dat = |view: u64, phase: u8, sigs: Vec<(u16, Vec<u8>)>| list(vec![
		Dat::U64(view),
		Dat::U8(phase),
		b32(9),
		list(sigs.into_iter().map(|(v, g)| list(vec![Dat::U16(v), Dat::BU64(g)])).collect()),
	]);
	let cases: Vec<(Envelope, Dat)> = vec![
		(
			env(MsgKind::ReplicatePut { record: rec(5, "ledger", 6) }),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![Dat::U8(1), record_dat(5, "ledger", 6)])]),
		),
		(
			env(MsgKind::GetRequest { request_id: 77, table: "t".to_string(), id: rid(4) }),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(2),
				list(vec![Dat::U64(77), s("t"), b32(4)]),
			])]),
		),
		(
			env(MsgKind::GetResponse { request_id: 8, record: Some(rec(5, "ledger", 2)) }),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(3),
				list(vec![Dat::U64(8), opt(Some(record_dat(5, "ledger", 2)))]),
			])]),
		),
		(
			env(MsgKind::GetResponse { request_id: 8, record: None }),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(3),
				list(vec![Dat::U64(8), opt(None)]),
			])]),
		),
		(
			env(MsgKind::AntiEntropyDigest { table: "t".to_string(), sketch: payload(1, 4), after: Some(rid(8)) }),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(4),
				list(vec![s("t"), Dat::BU64(payload(1, 4)), opt(Some(b32(8)))]),
			])]),
		),
		(
			env(MsgKind::AntiEntropyReply {
				table:			"t".to_string(),
				records:		vec![rec(5, "t", 1)],
				requested_ids:	vec![rid(6), rid(7)],
				bulk:			true,
				after:			None,
				next:			Some(rid(9)),
			}),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(5),
				list(vec![
					s("t"), list(vec![record_dat(5, "t", 1)]), list(vec![b32(6), b32(7)]), Dat::Bool(true),
					opt(None), opt(Some(b32(9))),
				]),
			])]),
		),
		(
			env(MsgKind::AntiEntropyPush { table: "t".to_string(), records: vec![rec(5, "t", 1)] }),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(6),
				list(vec![s("t"), list(vec![record_dat(5, "t", 1)])]),
			])]),
		),
		(
			env(MsgKind::CohortSubmit { record: rec(5, "ledger", 6) }),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![Dat::U8(7), record_dat(5, "ledger", 6)])]),
		),
		(
			env(MsgKind::CohortPropose {
				table:		"t".to_string(),
				id:			rid(3),
				proposal:	Proposal {
					view:		9,
					phase:		Phase::PreCommit,
					block_hash:	bytes32(9),
					block:		Some(payload(2, 3)),
					justify:	Some(qc(8, Phase::Prepare, &[4])),
				},
			}),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(8),
				list(vec![s("t"), b32(3), list(vec![
					Dat::U64(9),
					Dat::U8(1),
					b32(9),
					opt(Some(Dat::BU64(payload(2, 3)))),
					opt(Some(qc_dat(8, 0, vec![(4, payload(4, 64))]))),
				])]),
			])]),
		),
		(
			env(MsgKind::CohortVote {
				table:	"t".to_string(),
				id:		rid(3),
				vote:	Vote { view: 6, phase: Phase::Commit, block_hash: bytes32(9), voter: 3, signature: payload(1, 5) },
			}),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(9),
				list(vec![s("t"), b32(3), list(vec![
					Dat::U64(6),
					Dat::U8(2),
					b32(9),
					Dat::U16(3),
					Dat::BU64(payload(1, 5)),
				])]),
			])]),
		),
		(
			env(MsgKind::CohortNewView {
				table:		"t".to_string(),
				id:			rid(3),
				new_view:	NewView { view: 7, sender: 2, prepare_qc: Some(qc(6, Phase::Decide, &[0, 2])) },
			}),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(10),
				list(vec![s("t"), b32(3), list(vec![
					Dat::U64(7),
					Dat::U16(2),
					opt(Some(qc_dat(6, 3, vec![(0, payload(0, 64)), (2, payload(2, 64))]))),
				])]),
			])]),
		),
		(
			env(MsgKind::CohortNewView {
				table:		"t".to_string(),
				id:			rid(3),
				new_view:	NewView { view: 7, sender: 2, prepare_qc: None },
			}),
			list(vec![Dat::U8(1), b32(1), b32(2), list(vec![
				Dat::U8(10),
				list(vec![s("t"), b32(3), list(vec![Dat::U64(7), Dat::U16(2), opt(None)])]),
			])]),
		),
	];
	assert_eq!(WIRE_VERSION, 1);
	let mut kinds = BTreeSet::new();
	for (e, want) in &cases {
		let got = res!(e.to_dat());
		assert_eq!(&got, want, "The {} layout moved.", e.body.label());
		assert_eq!(res!(e.encode()), bdat(want), "The {} bytes moved.", e.body.label());
		kinds.insert(e.body.label());
	}
	assert_eq!(kinds.len(), 10, "The pinned layouts cover only {:?}.", kinds);
	Ok(())
}

#[test]
fn decode_refuses_malformed() -> Outcome<()> {
	// The baseline is good, so a refusal below is the fault planted and not a broken fixture.
	let good = bdat(&put_dat(1, 1));
	let want = env(MsgKind::ReplicatePut { record: rec(5, "ledger", 6) });
	assert_eq!(res!(Envelope::decode(&good)), want);

	assert_refused("version 0", &bdat(&put_dat(0, 1)));
	assert_refused("version 2", &bdat(&put_dat(2, 1)));
	assert_refused("kind 0", &bdat(&put_dat(1, 0)));
	assert_refused("kind 11", &bdat(&put_dat(1, 11)));
	assert_refused("kind 255", &bdat(&put_dat(1, 255)));

	// Trailing bytes, one and a whole second envelope.
	let mut one = good.clone();
	one.push(0);
	assert_refused("one trailing byte", &one);
	let mut two = good.clone();
	two.extend_from_slice(&good);
	assert_refused("two envelopes", &two);

	// Nothing, and every proper prefix of a valid envelope.
	assert_refused("empty", &[]);
	let rich = res!(deepest().encode());
	for n in 0..rich.len() {
		assert_refused(&fmt!("a prefix of {} bytes", n), &rich[..n]);
	}

	// A 31-byte id in each place an id sits, and ids in other widths.
	let short = |d: Dat| bdat(&list(vec![
		Dat::U8(1),
		b32(1),
		b32(2),
		list(vec![Dat::U8(1), list(vec![d, s("ledger"), Dat::BU64(payload(5, 6))])]),
	]));
	assert_refused("a 31-byte record id as BU8", &short(Dat::BU8(bytes32(5)[..31].to_vec())));
	assert_refused("a 32-byte record id as BU8", &short(Dat::BU8(bytes32(5).to_vec())));
	assert_refused("a 16-byte record id", &short(Dat::B16([5u8; 16])));
	let short_from = bdat(&list(vec![
		Dat::U8(1),
		Dat::BU8(bytes32(1)[..31].to_vec()),
		b32(2),
		list(vec![Dat::U8(1), record_dat(5, "ledger", 6)]),
	]));
	assert_refused("a 31-byte sender", &short_from);

	// A field missing, and a field too many, in the envelope, the body and the record.
	let body = list(vec![Dat::U8(1), record_dat(5, "ledger", 6)]);
	assert_refused("an envelope without a body", &bdat(&list(vec![Dat::U8(1), b32(1), b32(2)])));
	assert_refused("an envelope with a fifth field", &bdat(&list(vec![
		Dat::U8(1), b32(1), b32(2), body.clone(), Dat::U8(0),
	])));
	assert_refused("a body without its fields", &bdat(&list(vec![
		Dat::U8(1), b32(1), b32(2), list(vec![Dat::U8(1)]),
	])));
	assert_refused("a record without a value", &bdat(&list(vec![
		Dat::U8(1), b32(1), b32(2), list(vec![Dat::U8(1), list(vec![b32(5), s("ledger")])]),
	])));
	assert_refused("a record with a fourth field", &bdat(&list(vec![
		Dat::U8(1), b32(1), b32(2),
		list(vec![Dat::U8(1), list(vec![b32(5), s("ledger"), Dat::BU64(Vec::new()), Dat::U8(0)])]),
	])));

	// Shapes the encoder never writes: a narrower integer, a short byte string, a flag as a number.
	assert_refused("a version as U16", &bdat(&list(vec![Dat::U16(1), b32(1), b32(2), body.clone()])));
	assert_refused("a value as BU8", &bdat(&list(vec![
		Dat::U8(1), b32(1), b32(2),
		list(vec![Dat::U8(1), list(vec![b32(5), s("ledger"), Dat::BU8(vec![1, 2, 3])])]),
	])));
	assert_refused("an envelope that is no list", &bdat(&Dat::U8(1)));
	let vote = |phase: Dat, voter: Dat| bdat(&list(vec![
		Dat::U8(1), b32(1), b32(2),
		list(vec![Dat::U8(9), list(vec![s("t"), b32(3), list(vec![
			Dat::U64(6), phase, b32(9), voter, Dat::BU64(Vec::new()),
		])])]),
	]));
	let ok = vote(Dat::U8(2), Dat::U16(3));
	assert!(Envelope::decode(&ok).is_ok(), "The baseline vote must decode.");
	assert_refused("phase 4", &vote(Dat::U8(4), Dat::U16(3)));
	assert_refused("phase 255", &vote(Dat::U8(255), Dat::U16(3)));
	assert_refused("a phase as U16", &vote(Dat::U16(2), Dat::U16(3)));
	assert_refused("a voter as U8", &vote(Dat::U8(2), Dat::U8(3)));
	let reply_with = |bulk: Dat, after: Dat, next: Dat| bdat(&list(vec![
		Dat::U8(1), b32(1), b32(2),
		list(vec![Dat::U8(5), list(vec![s("t"), list(vec![]), list(vec![]), bulk, after, next])]),
	]));
	let reply = |bulk: Dat| reply_with(bulk, opt(None), opt(Some(b32(4))));
	assert!(Envelope::decode(&reply(Dat::Bool(true))).is_ok(), "The baseline reply must decode.");
	assert_refused("bulk as a number", &reply(Dat::U8(1)));
	assert_refused("a cursor without its option", &reply_with(Dat::Bool(true), b32(4), opt(None)));
	assert_refused("a cursor option holding a number", &reply_with(Dat::Bool(true), opt(None), opt(Some(Dat::U8(4)))));
	assert_refused("a cursor of the wrong length", &reply_with(Dat::Bool(true), opt(Some(Dat::BU64(vec![1, 2]))), opt(None)));
	let get = |rec: Dat| bdat(&list(vec![
		Dat::U8(1), b32(1), b32(2),
		list(vec![Dat::U8(3), list(vec![Dat::U64(1), rec])]),
	]));
	assert!(Envelope::decode(&get(opt(None))).is_ok(), "The baseline response must decode.");
	assert_refused("a record without its option", &get(record_dat(5, "t", 1)));
	assert_refused("a record option holding a number", &get(opt(Some(Dat::U8(1)))));
	Ok(())
}

// Every one-byte change to a valid envelope is either refused or read as an envelope the encoder
// would write back as those same bytes, so decode accepts no shape encode cannot produce.
#[test]
fn decode_accepts_only_what_encode_writes() -> Outcome<()> {
	let mut seeds = vec![
		env(MsgKind::GetResponse { request_id: 300, record: Some(rec(5, "ledger", 4)) }),
		env(MsgKind::AntiEntropyReply {
			table:			"t".to_string(),
			records:		vec![rec(5, "t", 2)],
			requested_ids:	vec![rid(6)],
			bulk:			true,
			after:			Some(rid(2)),
			next:			None,
		}),
	];
	seeds.push(env(MsgKind::CohortNewView {
		table:		"t".to_string(),
		id:			rid(3),
		new_view:	NewView { view: 7, sender: 2, prepare_qc: Some(qc(6, Phase::Decide, &[0])) },
	}));
	let mut accepted = 0usize;
	for seed in &seeds {
		let bytes = res!(seed.encode());
		for i in 0..bytes.len() {
			for x in 0..=255u8 {
				if x == bytes[i] {
					continue;
				}
				let mut b = bytes.clone();
				b[i] = x;
				if let Ok(e) = Envelope::decode(&b) {
					accepted += 1;
					let again = res!(e.encode());
					let at = again.iter().zip(b.iter()).position(|(p, q)| p != q);
					assert_eq!(
						again, b,
						"Byte {} set to {} was read as {} but encodes to other bytes, first \
						differing at {:?} ({:?} against {:?}).",
						i, x, e.body.label(), at,
						at.map(|k| again[k]), at.map(|k| b[k]),
						
					);
				}
			}
		}
	}
	// Some changes are legitimate: a different id, value or number is another valid envelope.
	assert!(accepted > 0, "No one-byte change was accepted, so the test checks nothing.");

	// Arbitrary bytes never panic.
	let mut state: u64 = 0x2545_F491_4F6C_DD1D;
	for round in 0..3000usize {
		let len = round % 97;
		let mut b = Vec::with_capacity(len);
		for _ in 0..len {
			state ^= state << 13;
			state ^= state >> 7;
			state ^= state << 17;
			b.push((state >> 32) as u8);
		}
		let _ = Envelope::decode(&b);
	}
	Ok(())
}

// A list of lists `levels` deep, built as bytes: a recursive `Dat` of that depth would overflow
// the stack of this test before it reached the decoder.
fn nested_list_bytes(levels: usize) -> Vec<u8> {
	fn c64(n: usize) -> Vec<u8> {
		let be = (n as u64).to_be_bytes();
		let skip = be.iter().take_while(|b| **b == 0).count();
		let mut v = vec![Dat::C64_CODE_START + (8 - skip) as u8];
		v.extend_from_slice(&be[skip..]);
		v
	}
	let mut inner = vec![Dat::LIST_CODE];
	inner.extend(c64(0));
	for _ in 1..levels {
		let mut outer = vec![Dat::LIST_CODE];
		outer.extend(c64(inner.len()));
		outer.extend(inner);
		inner = outer;
	}
	inner
}

#[test]
fn decode_is_bounded() -> Outcome<()> {
	assert_eq!(MAX_ENVELOPE_BYTES, 16 * 1024 * 1024);
	assert!(MAX_ENVELOPE_DEPTH >= 9, "The limit must admit the deepest shape written.");
	// The deepest shape written is admitted.
	let deep = deepest();
	assert_eq!(res!(Envelope::decode(&res!(deep.encode()))), deep);

	// A list 10,000 deep is refused as too deep, before the decoder recurses to it.
	let lists = nested_list_bytes(10_000);
	match Envelope::decode(&lists) {
		Ok(_) => panic!("A list 10,000 deep decoded as an envelope."),
		Err(e) => assert!(
			tags_of(&e).contains(&ErrTag::Excessive),
			"A list 10,000 deep was not refused as Excessive: {}", e,
		),
	}
	// A depth the decoder could reach, but the limit forbids: the limit and not the shape check
	// is what refuses it.
	let limit = MAX_ENVELOPE_DEPTH + 1;
	match Envelope::decode(&nested_list_bytes(limit)) {
		Ok(_) => panic!("A list {} deep decoded as an envelope.", limit),
		Err(e) => assert!(
			tags_of(&e).contains(&ErrTag::Excessive),
			"A list {} deep, one past the limit, was not refused as Excessive: {}", limit, e,
		),
	}
	match Envelope::decode(&nested_list_bytes(MAX_ENVELOPE_DEPTH)) {
		Ok(_) => panic!("A list of lists decoded as an envelope."),
		Err(e) => assert!(
			!tags_of(&e).contains(&ErrTag::Excessive),
			"A list exactly at the depth limit was refused as Excessive: {}", e,
		),
	}

	// A buffer one byte over the size limit is refused for its size, one at the limit is not.
	let over = vec![0u8; MAX_ENVELOPE_BYTES + 1];
	match Envelope::decode(&over) {
		Ok(_) => panic!("A buffer over the size limit decoded."),
		Err(e) => assert!(
			tags_of(&e).contains(&ErrTag::Excessive),
			"A buffer over the size limit was not refused as Excessive: {}", e,
		),
	}
	let at = vec![0u8; MAX_ENVELOPE_BYTES];
	match Envelope::decode(&at) {
		Ok(_) => panic!("A buffer of zeros decoded."),
		Err(e) => assert!(
			!tags_of(&e).contains(&ErrTag::Excessive),
			"A buffer exactly at the size limit was refused as Excessive: {}", e,
		),
	}

	// The encoder refuses what no peer would read, so an envelope that encodes also decodes.
	let huge = env(MsgKind::ReplicatePut { record: Record::new(rid(1), "t", vec![7u8; MAX_ENVELOPE_BYTES]) });
	match huge.encode() {
		Ok(b) => panic!("An envelope of {} bytes encoded past the limit.", b.len()),
		Err(e) => assert!(
			tags_of(&e).contains(&ErrTag::Excessive),
			"An oversize envelope was not refused as Excessive: {}", e,
		),
	}
	Ok(())
}


// The value cap. A buffer of one-byte values must not become a gibibyte of `Dat`, and no envelope
// the encoder writes may be refused by the cap.

#[test]
fn a_16_mib_page_of_small_records_encodes_and_decodes_under_the_value_cap() -> Outcome<()> {
	let small = |n: u32| {
		let mut id = [0u8; 32];
		id[..4].copy_from_slice(&n.to_be_bytes());
		Record { id: RecordId::from_bytes(id), table: String::new(), value: Vec::new() }
	};
	let push = |records: Vec<Record>| env(MsgKind::AntiEntropyPush { table: String::new(), records });
	// The size of one record, from the difference between two envelopes.
	let one = res!(push(vec![small(0)]).encode()).len();
	let two = res!(push(vec![small(0), small(1)]).encode()).len();
	let each = two - one;
	let n = (MAX_ENVELOPE_BYTES - one) / each + 1;
	let records: Vec<Record> = (0..n as u32).map(small).collect();
	let bytes = res!(push(records).encode());
	assert!(bytes.len() > MAX_ENVELOPE_BYTES - each, "The page is {} of {} bytes.", bytes.len(), MAX_ENVELOPE_BYTES);
	// Four values a record: its list, id, table and value.
	let values = 4 * n;
	assert!(values < MAX_ENVELOPE_VALUES, "{} values in a full page, over the cap of {}.", values, MAX_ENVELOPE_VALUES);
	match res!(Envelope::decode(&bytes)).body {
		MsgKind::AntiEntropyPush { records, .. }	=> assert_eq!(records.len(), n),
		other										=> return Err(err!("Decoded {:?}.", other; Test, Mismatch)),
	}
	Ok(())
}

#[test]
fn a_16_mib_list_of_one_byte_values_is_refused_at_the_value_cap() -> Outcome<()> {
	let n = MAX_ENVELOPE_BYTES - 8;
	// Under 2^24, so the minimal c64 is three bytes.
	let mut b = vec![Dat::LIST_CODE, Dat::C64_CODE_START + 3];
	b.extend_from_slice(&(n as u32).to_be_bytes()[1..]);
	b.resize(b.len() + n, Dat::EMPTY_CODE);
	assert!(b.len() <= MAX_ENVELOPE_BYTES);
	let msg = match Envelope::decode(&b) {
		Ok(e)	=> return Err(err!("A buffer of one-byte values decoded to {:?}.", e.body.label(); Test, Invalid)),
		Err(e)	=> fmt!("{}", e),
	};
	assert!(msg.contains(&fmt!("maximum of {} values", MAX_ENVELOPE_VALUES)), "The refusal does not name the cap: {}", msg);
	assert!(msg.contains(&fmt!("value number {}", MAX_ENVELOPE_VALUES + 1)), "The refusal did not stop at the cap: {}", msg);
	Ok(())
}
