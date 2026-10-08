#![cfg(feature = "dist")]
//! Symmetric anti-entropy (A2 R3, A1 QA 2-F3).
//!
//! The sketch is keyed on a record's id followed by its content hash, with an empty value, so a
//! record held at other bytes is a different key and decodes as one entry on each side. One
//! exchange then offers every differing record to both peers, whether the sketch decodes or
//! overloads, and a record held at the same bytes on both sides is never sent.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_data::iblt::{
	Iblt,
	IbltConfig,
};
use oxedyne_fe2o3_o3db_sync::{
	dist::{
		config::{
			Consistency,
			DistOzoneConfig,
			TableConfig,
		},
		engine::DistOzone,
		record::{
			Record,
			RecordDigest,
			RecordId,
		},
		resolve::{
			LastVersionWins,
			ReadView,
			ResolveCtx,
			Resolver,
			Verdict,
		},
		storage::{
			MemoryStorage,
			Storage,
		},
		transport::{
			Envelope,
			MsgKind,
		},
	},
	kademlia::id::NodeId,
	oam::config::OamConfig,
};

use std::{
	collections::BTreeSet,
	sync::{
		Mutex,
		atomic::{
			AtomicUsize,
			Ordering,
		},
	},
	time::Duration,
};


fn node(b: u8) -> NodeId {
	let mut bytes = [0u8; 32];
	bytes[31] = b;
	NodeId::from_bytes(bytes)
}

fn rid(b: u8) -> RecordId {
	RecordId::from_bytes([b; 32])
}

fn rec(id: RecordId, value: &[u8]) -> Record {
	Record::new(id, "identity", value.to_vec())
}

/// A value the stock resolver orders by its version.
fn ver(version: u64, text: &[u8]) -> Vec<u8> {
	LastVersionWins::value(version, text)
}

fn ids(records: &[Record]) -> BTreeSet<RecordId> {
	records.iter().map(|r| r.id).collect()
}

fn cfg(local: u8, remote: u8, cells: usize) -> Outcome<DistOzoneConfig> {
	let oam = res!(OamConfig::new(2, 2));
	let table = res!(TableConfig::new("identity", Consistency::Eventual, Duration::from_secs(30), cells));
	DistOzoneConfig::new(node(local), vec![node(remote)], oam, vec![table])
}

/// Peer 1 and peer 2, each holding everything, on a table whose sketch has `cells` cells.
fn pair<S: Storage>(cells: usize, s1: S, s2: S) -> Outcome<(DistOzone<S>, DistOzone<S>)> {
	Ok((
		res!(DistOzone::new(res!(cfg(1, 2, cells)), s1)),
		res!(DistOzone::new(res!(cfg(2, 1, cells)), s2)),
	))
}

fn put<S: Storage, R: Resolver>(e: &DistOzone<S, R>, id: RecordId, value: &[u8]) -> Outcome<()> {
	e.storage().put(&rec(id, value))
}

fn stored<S: Storage, R: Resolver>(e: &DistOzone<S, R>, id: RecordId) -> Outcome<Option<Vec<u8>>> {
	Ok(res!(e.storage().get("identity", &id)).map(|r| r.value))
}

fn table_ids<S: Storage, R: Resolver>(e: &DistOzone<S, R>) -> Outcome<BTreeSet<RecordId>> {
	Ok(res!(e.storage().digests("identity")).into_iter().map(|d| d.id).collect())
}

/// A storage that logs the id of every write.
struct Counting {
	inner:	MemoryStorage,
	puts:	Mutex<Vec<RecordId>>,
}

impl Counting {
	fn new() -> Self {
		Self { inner: MemoryStorage::new(), puts: Mutex::new(Vec::new()) }
	}

	/// The ids written since the last call.
	fn take_puts(&self) -> Outcome<BTreeSet<RecordId>> {
		let mut log = lock_mutex!(self.puts);
		Ok(std::mem::take(&mut *log).into_iter().collect())
	}
}

impl Storage for Counting {
	fn put(&self, record: &Record) -> Outcome<()> {
		lock_mutex!(self.puts).push(record.id);
		self.inner.put(record)
	}

	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Record>> {
		self.inner.get(table, id)
	}

	fn delete(&self, table: &str, id: &RecordId) -> Outcome<bool> {
		self.inner.delete(table, id)
	}

	fn digests(&self, table: &str) -> Outcome<Vec<RecordDigest>> {
		self.inner.digests(table)
	}
}

/// Takes whatever arrives, and counts the calls.
struct CountCalls {
	calls: AtomicUsize,
}

impl Resolver for CountCalls {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		_held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		self.calls.fetch_add(1, Ordering::SeqCst);
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

/// What each leg of one exchange carried.
struct Legs {
	records:	Vec<Record>,	// the listener's reply
	requested:	Vec<RecordId>,	// the ids the reply asked for
	bulk:		bool,
	pushed:		Vec<Record>,	// the dialler's push, empty when there was none
	pushes:		usize,			// how many push envelopes the dialler sent
}

/// One whole exchange on "identity", the dialler's digest to the listener, the listener's reply
/// and the dialler's push, every leg delivered and recorded.
fn exchange<S1: Storage, S2: Storage>(
	dialler:		&DistOzone<S1>,
	listener:		&DistOzone<S2>,
	listener_id:	u8,
)
	-> Outcome<Legs>
{
	let req = res!(dialler.build_anti_entropy_request("identity", node(listener_id)));
	let mut got = res!(listener.handle_envelope_at(req, 1_000));
	assert_eq!(got.outbound.len(), 1);
	let reply = got.outbound.remove(0);
	let (records, requested, bulk) = match &reply.body {
		MsgKind::AntiEntropyReply { records, requested_ids, bulk, .. } =>
			(records.clone(), requested_ids.clone(), *bulk),
		other => return Err(err!("Expected an AntiEntropyReply, got {:?}.", other; Test, Mismatch)),
	};
	let mut mine = res!(dialler.handle_envelope_at(reply, 1_000));
	assert!(mine.failed.is_empty(), "the dialler failed a record: {:?}", mine.failed);
	let mut pushed = Vec::new();
	let mut pushes = 0;
	for p in std::mem::take(&mut mine.outbound) {
		match &p.body {
			MsgKind::AntiEntropyPush { records, .. } => {
				pushes += 1;
				pushed.extend(records.clone());
			},
			other => return Err(err!("Expected an AntiEntropyPush, got {:?}.", other; Test, Mismatch)),
		}
		let landed = res!(listener.handle_envelope_at(p, 1_000));
		assert!(landed.failed.is_empty(), "the listener failed a record: {:?}", landed.failed);
	}
	Ok(Legs { records, requested, bulk, pushed, pushes })
}

/// The sketch the engine is expected to send: the id and the content hash as the key, no value.
fn sketch_cfg(cells: usize) -> Outcome<IbltConfig> {
	let table = res!(TableConfig::new("identity", Consistency::Eventual, Duration::from_secs(30), cells));
	Ok(IbltConfig {
		num_cells:	cells,
		num_hashes:	TableConfig::IBLT_NUM_HASHES,
		key_len:	64,
		value_len:	0,
		seed:		table.iblt_seed(),
	})
}

fn digest_envelope(from: u8, to: u8, sketch: Vec<u8>) -> Envelope {
	Envelope::new(node(from), node(to), MsgKind::AntiEntropyDigest {
		table:	"identity".to_string(),
		sketch,
	})
}


// One id at two values among twenty shared records: the sketch decodes, the reply is not bulk,
// and it carries that one record and asks for that one id. The dialler resolves, so the push
// carries the newer value whichever side held it.
#[test]
fn value_divergence_decodes_without_bulk() -> Outcome<()> {
	let v1 = ver(1, b"old");
	let v2 = ver(2, b"new");
	for dialler_newer in [true, false] {
		let (d, l) = res!(pair(256, MemoryStorage::new(), MemoryStorage::new()));
		for i in 1..=20u8 {
			let v = ver(1, &[i]);
			res!(put(&d, rid(i), &v));
			res!(put(&l, rid(i), &v));
		}
		let (dv, lv) = if dialler_newer { (&v2, &v1) } else { (&v1, &v2) };
		res!(put(&d, rid(100), dv));
		res!(put(&l, rid(100), lv));

		let legs = res!(exchange(&d, &l, 2));
		let who = if dialler_newer { "dialler newer" } else { "listener newer" };
		assert!(!legs.bulk, "{}: a value difference made the exchange bulk", who);
		assert_eq!(legs.records, vec![rec(rid(100), lv)], "{}: the reply", who);
		assert_eq!(legs.requested, vec![rid(100)], "{}: the ids asked for", who);
		assert_eq!(legs.pushes, 1, "{}: the pushes", who);
		assert_eq!(legs.pushed, vec![rec(rid(100), &v2)], "{}: the push", who);
		assert_eq!(res!(stored(&d, rid(100))), Some(v2.clone()), "{}: the dialler", who);
		assert_eq!(res!(stored(&l, rid(100))), Some(v2.clone()), "{}: the listener", who);
		assert_eq!(res!(table_ids(&d)).len(), 21);
		assert_eq!(res!(table_ids(&l)).len(), 21);
	}
	Ok(())
}

// A sketch of six cells cannot hold the difference, so the reply is bulk. The dialler takes what
// it lacks and pushes back everything the listener lacks or holds at other bytes, and nothing
// else: after one exchange both hold the same forty records with the newer X.
#[test]
fn bulk_exchange_repairs_both_sides() -> Outcome<()> {
	let v1 = ver(1, b"old");
	let v2 = ver(2, b"new");
	for dialler_newer in [true, false] {
		let (d, l) = res!(pair(6, MemoryStorage::new(), MemoryStorage::new()));
		let dialler_only:	BTreeSet<RecordId> = (1..=30u8).map(rid).collect();
		let listener_only:	BTreeSet<RecordId> = (31..=35u8).map(rid).collect();
		let shared:			BTreeSet<RecordId> = (36..=39u8).map(rid).collect();
		for i in 1..=39u8 {
			let v = ver(1, &[i]);
			if i <= 30 || i >= 36 {
				res!(put(&d, rid(i), &v));
			}
			if i >= 31 {
				res!(put(&l, rid(i), &v));
			}
		}
		let (dv, lv) = if dialler_newer { (&v2, &v1) } else { (&v1, &v2) };
		res!(put(&d, rid(100), dv));
		res!(put(&l, rid(100), lv));

		let legs = res!(exchange(&d, &l, 2));
		let who = if dialler_newer { "dialler newer" } else { "listener newer" };
		assert!(legs.bulk, "{}: six cells decoded a difference of thirty-seven", who);
		assert!(legs.requested.is_empty(), "{}: a bulk reply asked for {:?}", who, legs.requested);
		let mut reply: BTreeSet<RecordId> = listener_only.union(&shared).copied().collect();
		reply.insert(rid(100));
		assert_eq!(ids(&legs.records), reply, "{}: the bulk reply is the listener's whole table", who);

		// The push is the dialler's thirty, and X only while it differs from the listener's copy.
		let mut push = dialler_only.clone();
		if dialler_newer {
			push.insert(rid(100));
		}
		assert_eq!(legs.pushes, 1, "{}: the pushes", who);
		assert_eq!(legs.pushed.len(), push.len(), "{}: the push repeats a record", who);
		assert_eq!(ids(&legs.pushed), push, "{}: the push", who);

		let mut all: BTreeSet<RecordId> = dialler_only.union(&listener_only).copied().collect();
		all.extend(shared.iter().copied());
		all.insert(rid(100));
		assert_eq!(all.len(), 40);
		assert_eq!(res!(table_ids(&d)), all, "{}: the dialler's table", who);
		assert_eq!(res!(table_ids(&l)), all, "{}: the listener's table", who);
		assert_eq!(res!(stored(&d, rid(100))), Some(v2.clone()), "{}: the dialler's X", who);
		assert_eq!(res!(stored(&l, rid(100))), Some(v2.clone()), "{}: the listener's X", who);
		for id in &all {
			assert_eq!(res!(stored(&d, *id)), res!(stored(&l, *id)), "{}: the peers differ at {:?}", who, id);
		}
	}
	Ok(())
}

// A record held at the same bytes on both sides crosses no leg and is written nowhere, whether
// the sketch decodes an empty difference, a real one, or an empty one again.
#[test]
fn same_bytes_never_sent() -> Outcome<()> {
	let shared: BTreeSet<RecordId> = (1..=20u8).map(rid).collect();
	let (d, l) = res!(pair(256, Counting::new(), Counting::new()));
	for i in 1..=20u8 {
		let v = ver(1, &[i]);
		res!(put(&d, rid(i), &v));
		res!(put(&l, rid(i), &v));
	}
	res!(d.storage().take_puts());
	res!(l.storage().take_puts());

	// Identical tables: an empty reply, no push, no write.
	let legs = res!(exchange(&d, &l, 2));
	assert!(legs.records.is_empty() && legs.requested.is_empty() && !legs.bulk, "identical tables: a reply");
	assert_eq!(legs.pushes, 0, "identical tables: a push");
	assert!(res!(d.storage().take_puts()).is_empty() && res!(l.storage().take_puts()).is_empty());

	// Three only the dialler holds, two only the listener holds, and X at two values.
	for i in 50..=52u8 {
		res!(put(&d, rid(i), &ver(1, &[i])));
	}
	for i in 60..=61u8 {
		res!(put(&l, rid(i), &ver(1, &[i])));
	}
	res!(put(&d, rid(100), &ver(2, b"new")));
	res!(put(&l, rid(100), &ver(1, b"old")));
	res!(d.storage().take_puts());
	res!(l.storage().take_puts());

	let legs = res!(exchange(&d, &l, 2));
	assert!(!legs.bulk);
	let sent: BTreeSet<RecordId> = ids(&legs.records).union(&ids(&legs.pushed)).copied().collect();
	assert!(sent.is_disjoint(&shared), "a shared record crossed a leg: {:?}", sent);
	let want_l: BTreeSet<RecordId> = [50u8, 51, 52, 100].into_iter().map(rid).collect();
	let want_d: BTreeSet<RecordId> = [60u8, 61].into_iter().map(rid).collect();
	assert_eq!(res!(l.storage().take_puts()), want_l, "the listener's writes");
	assert_eq!(res!(d.storage().take_puts()), want_d, "the dialler's writes");

	// Now equal: nothing crosses and nothing is written, in either direction.
	for (a, b, to) in [(&d, &l, 2u8), (&l, &d, 1u8)] {
		let legs = res!(exchange(a, b, to));
		assert!(legs.records.is_empty() && legs.requested.is_empty() && !legs.bulk, "second exchange: a reply");
		assert_eq!(legs.pushes, 0, "second exchange: a push");
	}
	assert!(res!(d.storage().take_puts()).is_empty() && res!(l.storage().take_puts()).is_empty());
	Ok(())
}

// A reply that repeats a requested id and a record is acted on once for each: the push holds each
// requested record once, and the resolver sees each distinct record once.
#[test]
fn duplicate_requests_and_records_applied_once() -> Outcome<()> {
	let oam = res!(OamConfig::new(2, 2));
	let table = res!(TableConfig::eventual("identity"));
	let c = res!(DistOzoneConfig::new(node(1), vec![node(2)], oam, vec![table]));
	let e = res!(DistOzone::with_resolver(c, MemoryStorage::new(), CountCalls { calls: AtomicUsize::new(0) }));
	res!(put(&e, rid(7), b"y"));
	res!(put(&e, rid(8), b"z"));

	let r = rec(rid(9), b"r");
	let q = rec(rid(10), b"q");
	let env = Envelope::new(node(2), node(1), MsgKind::AntiEntropyReply {
		table:			"identity".to_string(),
		records:		vec![r.clone(), r.clone(), q.clone(), r.clone()],
		requested_ids:	vec![rid(7), rid(7), rid(8), rid(7), rid(8)],
		bulk:			false,
	});
	let out = res!(e.handle_envelope_at(env, 1_000));
	assert_eq!(e.resolver().calls.load(Ordering::SeqCst), 2, "the resolver saw a repeated record");
	assert_eq!(out.persisted.len(), 2, "persisted: {:?}", out.persisted);
	assert_eq!(res!(stored(&e, rid(9))), Some(b"r".to_vec()));
	assert_eq!(res!(stored(&e, rid(10))), Some(b"q".to_vec()));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::AntiEntropyPush { records, .. } => {
			assert_eq!(records.len(), 2, "the push repeats a record: {:?}", records);
			assert_eq!(ids(records), [rid(7), rid(8)].into_iter().collect::<BTreeSet<_>>());
		},
		other => panic!("expected an AntiEntropyPush, got {:?}", other),
	}
	Ok(())
}

// In a bulk reply an id the sender also asked for is pushed once, not once for the request and
// once for the difference.
#[test]
fn bulk_push_names_each_record_once() -> Outcome<()> {
	let (e, _) = res!(pair(256, MemoryStorage::new(), MemoryStorage::new()));
	res!(put(&e, rid(1), &ver(1, b"a")));	// the sender lacks it, and asks for it
	res!(put(&e, rid(2), &ver(1, b"c")));	// the sender lacks it
	res!(put(&e, rid(3), &ver(2, b"b")));	// the sender holds it at an older value
	res!(put(&e, rid(4), &ver(1, b"d")));	// the sender holds it at the same bytes
	let env = Envelope::new(node(2), node(1), MsgKind::AntiEntropyReply {
		table:			"identity".to_string(),
		records:		vec![rec(rid(3), &ver(1, b"b")), rec(rid(4), &ver(1, b"d"))],
		requested_ids:	vec![rid(1), rid(1)],
		bulk:			true,
	});
	let out = res!(e.handle_envelope_at(env, 1_000));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::AntiEntropyPush { records, .. } => {
			assert_eq!(records.len(), 3, "the push repeats or omits a record: {:?}", records);
			assert_eq!(ids(records), [rid(1), rid(2), rid(3)].into_iter().collect::<BTreeSet<_>>());
			let kept = records.iter().find(|r| r.id == rid(3)).map(|r| r.value.clone());
			assert_eq!(kept, Some(ver(2, b"b")), "the push offers the sender's older value");
		},
		other => panic!("expected an AntiEntropyPush, got {:?}", other),
	}
	Ok(())
}

// A hand-made sketch that holds one id under two contents asks for that id once.
#[test]
fn hostile_sketch_naming_one_id_twice_requests_it_once() -> Outcome<()> {
	let (_, l) = res!(pair(256, MemoryStorage::new(), MemoryStorage::new()));
	let mut theirs = res!(Iblt::new(res!(sketch_cfg(256))));
	for content in [[1u8; 32], [2u8; 32]] {
		let mut key = rid(5).as_bytes().to_vec();
		key.extend_from_slice(&content);
		res!(theirs.insert(&key, &[]));
	}
	let mut got = res!(l.handle_envelope_at(digest_envelope(1, 2, theirs.to_bytes()), 1_000));
	assert_eq!(got.outbound.len(), 1);
	match got.outbound.remove(0).body {
		MsgKind::AntiEntropyReply { records, requested_ids, bulk, .. } => {
			assert!(records.is_empty() && !bulk);
			assert_eq!(requested_ids, vec![rid(5)], "the same id was asked for twice");
		},
		other => panic!("expected an AntiEntropyReply, got {:?}", other),
	}
	Ok(())
}

// The sketch on the wire is the key `id ‖ content` with no value, and a sketch of the old shape
// (the id as the key, the content as the value) is refused with Mismatch and writes nothing.
#[test]
fn old_shape_sketch_is_refused() -> Outcome<()> {
	let (d, l) = res!(pair(256, MemoryStorage::new(), MemoryStorage::new()));
	res!(put(&d, rid(1), &ver(1, b"a")));
	let req = res!(d.build_anti_entropy_request("identity", node(2)));
	match &req.body {
		MsgKind::AntiEntropyDigest { sketch, .. } => {
			let wire = res!(Iblt::from_bytes(sketch)).config();
			assert_eq!(wire, res!(sketch_cfg(256)), "the sketch on the wire");
		},
		other => panic!("expected an AntiEntropyDigest, got {:?}", other),
	}

	let old_cfg = IbltConfig { key_len: 32, value_len: 32, ..res!(sketch_cfg(256)) };
	let mut old = res!(Iblt::new(old_cfg));
	res!(old.insert(rid(2).as_bytes(), &[7u8; 32]));
	match l.handle_envelope_at(digest_envelope(1, 2, old.to_bytes()), 1_000) {
		Ok(out) => panic!("an old-shape sketch was accepted: {:?}", out.outbound),
		Err(e) => assert!(e.tags().contains(&ErrTag::Mismatch), "the refusal does not say Mismatch: {:?}", e),
	}
	assert!(res!(table_ids(&l)).is_empty());
	Ok(())
}
