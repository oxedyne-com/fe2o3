#![cfg(feature = "dist")]
//! Integration tests for the resolver seam of the distributed-Ozone engine.
//!
//! Every record that reaches an eventual table is offered to a [`Resolver`],
//! and the engine stores only what the verdict says. The tests drive the four
//! ways a record arrives (a local put, `ReplicatePut`, an anti-entropy reply
//! and an anti-entropy push) with small resolvers that refuse, keep, defer,
//! merge, fail or record, and check what reaches storage, what is sent, and
//! which of `persisted`, `deferred`, `refused` and `failed` lists each record.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::kademlia::id::NodeId;
use oxedyne_fe2o3_o3db_sync::oam::config::OamConfig;
use oxedyne_fe2o3_o3db_sync::dist::{
	cohort,
	config::{
		Consistency,
		DistOzoneConfig,
		TableConfig,
	},
	engine::{
		DistOzone,
		InboundOutcome,
	},
	peer_set::PeerSet,
	record::{
		Record,
		RecordDigest,
		RecordId,
	},
	resolve::{
		Convergence,
		LastVersionWins,
		ReadView,
		ResolveCtx,
		Resolver,
		Verdict,
		check_convergence,
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

use std::{
	collections::HashMap,
	sync::{
		Arc,
		Mutex,
		atomic::{
			AtomicBool,
			AtomicU64,
			AtomicUsize,
			Ordering,
		},
		mpsc,
	},
	thread,
	time::Duration,
};


fn node(b: u8) -> NodeId {
	let mut bytes = [0u8; 32];
	bytes[31] = b;
	NodeId::from_bytes(bytes)
}

fn rid(b: u8) -> RecordId {
	let mut bytes = [0u8; 32];
	bytes[0] = b;
	RecordId::from_bytes(bytes)
}

fn s(text: &str) -> String {
	text.to_string()
}

fn rec(table: &str, id: RecordId, value: &[u8]) -> Record {
	Record::new(id, table, value.to_vec())
}

/// Two peers that both hold everything (replication equals network size), or
/// one peer that holds nothing (replication 0).
fn engine<S: Storage, R: Resolver>(
	local:			u8,
	remote:			u8,
	replication:	u64,
	network_size:	u64,
	storage:		S,
	resolver:		R,
)
	-> Outcome<DistOzone<S, R>>
{
	let oam = res!(OamConfig::new(replication, network_size));
	let tables = vec![
		res!(TableConfig::eventual("identity")),
		res!(TableConfig::eventual("escrow")),
	];
	let cfg = res!(DistOzoneConfig::new(node(local), vec![node(remote)], oam, tables));
	DistOzone::with_resolver(cfg, storage, resolver)
}

fn holder<S: Storage, R: Resolver>(
	local:		u8,
	remote:		u8,
	storage:	S,
	resolver:	R,
)
	-> Outcome<DistOzone<S, R>>
{
	engine(local, remote, 2, 2, storage, resolver)
}

fn stored<S: Storage, R: Resolver>(
	e:		&DistOzone<S, R>,
	table:	&str,
	id:		RecordId,
)
	-> Outcome<Option<Vec<u8>>>
{
	Ok(res!(e.storage().get(table, &id)).map(|r| r.value))
}

fn replicate(from: u8, to: u8, record: Record) -> Envelope {
	Envelope::new(node(from), node(to), MsgKind::ReplicatePut { record })
}

fn reply(from: u8, to: u8, table: &str, records: Vec<Record>) -> Envelope {
	Envelope::new(node(from), node(to), MsgKind::AntiEntropyReply {
		table:			s(table),
		records,
		requested_ids:	Vec::new(),
		bulk:			false,
	})
}

fn push(from: u8, to: u8, table: &str, records: Vec<Record>) -> Envelope {
	Envelope::new(node(from), node(to), MsgKind::AntiEntropyPush {
		table: s(table),
		records,
	})
}

/// All four lists empty.
fn nothing_listed(out: &InboundOutcome) -> bool {
	out.persisted.is_empty()
		&& out.deferred.is_empty()
		&& out.refused.is_empty()
		&& out.failed.is_empty()
}

/// One full anti-entropy round on one table: the dialler sends its digest, the
/// listener replies, and the dialler applies the reply and answers any request
/// with a push. Returns what the dialler's engine made of the reply.
fn round<S1: Storage, R1: Resolver, S2: Storage, R2: Resolver>(
	dialler:		&DistOzone<S1, R1>,
	listener:		&DistOzone<S2, R2>,
	listener_id:	u8,
	table:			&str,
	now_ms:			u64,
)
	-> Outcome<InboundOutcome>
{
	let req = res!(dialler.build_anti_entropy_request(table, node(listener_id)));
	let mut got = res!(listener.handle_envelope_at(req, now_ms));
	assert_eq!(got.outbound.len(), 1);
	let answer = got.outbound.remove(0);
	let mut mine = res!(dialler.handle_envelope_at(answer, now_ms));
	for p in std::mem::take(&mut mine.outbound) {
		res!(listener.handle_envelope_at(p, now_ms));
	}
	Ok(mine)
}


// ---------------------------------------------------------------------------
// Test storage and resolvers.
// ---------------------------------------------------------------------------

/// Counts writes and fails a write or a read of a chosen id on demand.
struct TestStorage {
	inner:		MemoryStorage,
	puts:		AtomicUsize,
	fail_put:	Mutex<Option<RecordId>>,
	fail_get:	Mutex<Option<RecordId>>,
}

impl TestStorage {
	fn new() -> Self {
		Self {
			inner:		MemoryStorage::new(),
			puts:		AtomicUsize::new(0),
			fail_put:	Mutex::new(None),
			fail_get:	Mutex::new(None),
		}
	}

	fn puts(&self) -> usize {
		self.puts.load(Ordering::SeqCst)
	}

	fn fail_put_of(&self, id: Option<RecordId>) -> Outcome<()> {
		*lock_mutex!(self.fail_put) = id;
		Ok(())
	}

	fn fail_get_of(&self, id: Option<RecordId>) -> Outcome<()> {
		*lock_mutex!(self.fail_get) = id;
		Ok(())
	}
}

impl Storage for TestStorage {
	fn put(&self, record: &Record) -> Outcome<()> {
		if *lock_mutex!(self.fail_put) == Some(record.id) {
			return Err(err!("Injected write fault."; IO, Write));
		}
		self.puts.fetch_add(1, Ordering::SeqCst);
		self.inner.put(record)
	}

	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Record>> {
		if *lock_mutex!(self.fail_get) == Some(*id) {
			return Err(err!("Injected read fault."; IO, Read));
		}
		self.inner.get(table, id)
	}

	fn delete(&self, table: &str, id: &RecordId) -> Outcome<bool> {
		self.inner.delete(table, id)
	}

	fn digests(&self, table: &str) -> Outcome<Vec<RecordDigest>> {
		self.inner.digests(table)
	}
}

/// Refuses a value that begins with "BAD", and takes any other.
struct RefuseBad;

impl Resolver for RefuseBad {
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
		if incoming.starts_with(b"BAD") {
			return Ok(Verdict::Refuse(s("marked bad")));
		}
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

/// Answers the same verdict whatever it is offered.
struct Fixed(Verdict);

impl Resolver for Fixed {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		_held:		Option<&[u8]>,
		_incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		Ok(self.0.clone())
	}
}

/// Always takes the incoming bytes, even where they equal the held bytes.
struct TakeIncoming;

impl Resolver for TakeIncoming {
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
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

/// A merge: the stored value is held, a plus sign, then the incoming value.
struct Concat;

impl Resolver for Concat {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		match held {
			Some(h) => {
				let mut merged = h.to_vec();
				merged.push(b'+');
				merged.extend_from_slice(incoming);
				Ok(Verdict::Take(merged))
			},
			None => Ok(Verdict::Take(incoming.to_vec())),
		}
	}
}

/// An escrow record names its base (the id's first byte is its own first byte)
/// and defers until that base is in the identity table.
struct NeedsBase;

impl Resolver for NeedsBase {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		view:		&V,
		table:		&str,
		_id:		&RecordId,
		_held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		if table == "escrow" {
			let base = match incoming.first() {
				Some(b)	=> rid(*b),
				None	=> return Ok(Verdict::Refuse(s("empty escrow"))),
			};
			if res!(view.get("identity", &base)).is_none() {
				return Ok(Verdict::Defer);
			}
		}
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

/// Fails on the value "ERR" while armed, and takes anything else.
struct Faulty(Arc<AtomicBool>);

impl Resolver for Faulty {
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
		if incoming == b"ERR" && self.0.load(Ordering::SeqCst) {
			return Err(err!("Injected resolver fault."; Invalid, Input));
		}
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

/// The first byte of the value picks the verdict: T takes, K keeps, D defers
/// and R refuses.
struct ByTag;

impl Resolver for ByTag {
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
		match incoming.first() {
			Some(b'T')	=> Ok(Verdict::Take(incoming.to_vec())),
			Some(b'K')	=> Ok(Verdict::Keep),
			Some(b'D')	=> Ok(Verdict::Defer),
			_			=> Ok(Verdict::Refuse(s("r"))),
		}
	}
}

#[derive(Debug)]
struct Seen {
	table:		String,
	id:			RecordId,
	held:		Option<Vec<u8>>,
	incoming:	Vec<u8>,
	now_ms:		u64,
	other:		Option<Vec<u8>>,	// identity record 200, read through the view
}

/// Records every call, reads one other record through the view, and takes.
struct Recorder {
	seen: Mutex<Vec<Seen>>,
}

impl Recorder {
	fn new() -> Self {
		Self { seen: Mutex::new(Vec::new()) }
	}

	fn calls(&self) -> Outcome<usize> {
		Ok(lock_mutex!(self.seen).len())
	}
}

impl Resolver for Recorder {
	fn resolve<V: ReadView>(
		&self,
		ctx:		&ResolveCtx,
		view:		&V,
		table:		&str,
		id:			&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		let other = res!(view.get("identity", &rid(200)));
		lock_mutex!(self.seen).push(Seen {
			table:		table.to_string(),
			id:			*id,
			held:		held.map(|h| h.to_vec()),
			incoming:	incoming.to_vec(),
			now_ms:		ctx.now_ms,
			other,
		});
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

/// A view with nothing in it, for calling a resolver directly.
struct EmptyView;

impl ReadView for EmptyView {
	fn get(&self, _table: &str, _id: &RecordId) -> Outcome<Option<Vec<u8>>> {
		Ok(None)
	}
}


// ---------------------------------------------------------------------------
// Verdicts.
// ---------------------------------------------------------------------------

#[test]
fn refuse_never_persists() -> Outcome<()> {
	let id = rid(7);
	let held = rec("identity", id, b"good");
	let bad = rec("identity", id, b"BAD value");
	let refusal = vec![(s("identity"), id, s("marked bad"))];

	// Every inbound path: the record is refused with its reason, the held
	// bytes stand, and nothing is listed as stored.
	let paths = [
		replicate(2, 1, bad.clone()),
		reply(2, 1, "identity", vec![bad.clone()]),
		push(2, 1, "identity", vec![bad.clone()]),
	];
	for env in paths {
		let e = res!(holder(1, 2, MemoryStorage::new(), RefuseBad));
		res!(e.storage().put(&held));
		let out = res!(e.handle_envelope_at(env, 1));
		assert_eq!(out.refused, refusal);
		assert!(out.persisted.is_empty() && out.deferred.is_empty() && out.failed.is_empty());
		assert!(out.outbound.is_empty());
		assert_eq!(res!(stored(&e, "identity", id)), Some(b"good".to_vec()));
	}

	// A local put: the verdict comes back, nothing is stored or sent.
	let e = res!(holder(1, 2, MemoryStorage::new(), RefuseBad));
	res!(e.storage().put(&held));
	let out = res!(e.put_at(bad, 1));
	assert_eq!(out.verdict, Some(Verdict::Refuse(s("marked bad"))));
	assert!(!out.local_persisted);
	assert!(out.outbound.is_empty());
	assert_eq!(res!(stored(&e, "identity", id)), Some(b"good".to_vec()));
	Ok(())
}

#[test]
fn keep_leaves_held_bytes() -> Outcome<()> {
	let id = rid(7);
	let held = rec("identity", id, b"X");
	let other = rec("identity", id, b"Y");

	let e = res!(holder(1, 2, MemoryStorage::new(), Fixed(Verdict::Keep)));
	res!(e.storage().put(&held));
	let out = res!(e.handle_envelope_at(replicate(2, 1, other.clone()), 1));
	assert!(nothing_listed(&out));
	assert!(out.outbound.is_empty());
	assert_eq!(res!(stored(&e, "identity", id)), Some(b"X".to_vec()));

	// A local put sends nothing, though a remote holder exists.
	let out = res!(e.put_at(other, 1));
	assert_eq!(out.verdict, Some(Verdict::Keep));
	assert!(!out.local_persisted);
	assert!(out.outbound.is_empty());
	assert_eq!(res!(stored(&e, "identity", id)), Some(b"X".to_vec()));
	Ok(())
}

#[test]
fn take_stores_resolver_bytes() -> Outcome<()> {
	let id = rid(7);
	let held = rec("identity", id, b"A");
	let incoming = rec("identity", id, b"B");

	// Inbound: the merge is stored, not the incoming bytes.
	let e = res!(holder(1, 2, MemoryStorage::new(), Concat));
	res!(e.storage().put(&held));
	let out = res!(e.handle_envelope_at(replicate(2, 1, incoming.clone()), 1));
	assert_eq!(out.persisted, vec![(s("identity"), id)]);
	assert_eq!(res!(stored(&e, "identity", id)), Some(b"A+B".to_vec()));

	// Local: the merge is stored, and the remote holder is sent the merge.
	let e = res!(holder(1, 2, MemoryStorage::new(), Concat));
	res!(e.storage().put(&held));
	let out = res!(e.put_at(incoming, 1));
	assert_eq!(out.verdict, Some(Verdict::Take(b"A+B".to_vec())));
	assert!(out.local_persisted);
	assert_eq!(res!(stored(&e, "identity", id)), Some(b"A+B".to_vec()));
	assert_eq!(out.outbound.len(), 1);
	match &out.outbound[0].body {
		MsgKind::ReplicatePut { record } => {
			assert_eq!(record.value, b"A+B".to_vec());
			assert_eq!(record.id, id);
			assert_eq!(record.table, "identity");
		},
		other => panic!("Expected a ReplicatePut, got {:?}.", other),
	}
	assert_eq!(out.outbound[0].to, node(2));
	Ok(())
}

#[test]
fn take_of_held_is_keep() -> Outcome<()> {
	let id = rid(7);
	let held = rec("identity", id, b"X");

	let e = res!(holder(1, 2, TestStorage::new(), TakeIncoming));
	res!(e.storage().put(&held));
	let base = e.storage().puts();

	// The same bytes again: no write, nothing listed, nothing sent.
	let out = res!(e.handle_envelope_at(replicate(2, 1, held.clone()), 1));
	assert!(nothing_listed(&out));
	assert!(out.outbound.is_empty());
	assert_eq!(e.storage().puts(), base);
	let out = res!(e.put_at(held, 1));
	assert_eq!(out.verdict, Some(Verdict::Keep));
	assert!(!out.local_persisted);
	assert!(out.outbound.is_empty());
	assert_eq!(e.storage().puts(), base);

	// Control: other bytes are a real Take, so the counter does move.
	let out = res!(e.handle_envelope_at(replicate(2, 1, rec("identity", id, b"Y")), 1));
	assert_eq!(out.persisted, vec![(s("identity"), id)]);
	assert_eq!(e.storage().puts(), base + 1);
	Ok(())
}

#[test]
fn defer_retried_by_next_round() -> Outcome<()> {
	let base_id = rid(1);
	let dep_id = rid(9);
	let a = res!(holder(1, 2, MemoryStorage::new(), NeedsBase));
	let b = res!(holder(2, 1, MemoryStorage::new(), NeedsBase));
	res!(a.storage().put(&rec("identity", base_id, b"base")));
	res!(a.storage().put(&rec("escrow", dep_id, &[1, b'd'])));

	// The dependency is missing, so the record is deferred and not stored, and
	// the next round offers it again with the same result.
	for _ in 0..2 {
		let out = res!(round(&b, &a, 1, "escrow", 1));
		assert_eq!(out.deferred, vec![(s("escrow"), dep_id)]);
		assert!(out.persisted.is_empty() && out.refused.is_empty() && out.failed.is_empty());
		assert_eq!(res!(stored(&b, "escrow", dep_id)), None);
	}

	// The base arrives; the next escrow round stores the record.
	let out = res!(round(&b, &a, 1, "identity", 2));
	assert_eq!(out.persisted, vec![(s("identity"), base_id)]);
	let out = res!(round(&b, &a, 1, "escrow", 3));
	assert_eq!(out.persisted, vec![(s("escrow"), dep_id)]);
	assert!(out.deferred.is_empty());
	assert_eq!(res!(stored(&b, "escrow", dep_id)), Some(vec![1, b'd']));
	Ok(())
}

#[test]
fn resolve_err_is_failed_not_refused() -> Outcome<()> {
	let (id1, id2, id3) = (rid(1), rid(2), rid(3));
	let armed = Arc::new(AtomicBool::new(true));
	let b = res!(holder(2, 1, TestStorage::new(), Faulty(armed.clone())));
	let a = res!(holder(1, 2, MemoryStorage::new(), Faulty(Arc::new(AtomicBool::new(false)))));
	for (id, v) in [(id1, &b"one"[..]), (id2, b"ERR"), (id3, b"three")] {
		res!(a.storage().put(&rec("identity", id, v)));
	}

	// One record of three fails in resolve. It is failed, not refused, and not
	// stored; the other two are applied.
	let out = res!(b.handle_envelope_at(push(1, 2, "identity", vec![
		rec("identity", id1, b"one"),
		rec("identity", id2, b"ERR"),
		rec("identity", id3, b"three"),
	]), 1));
	assert_eq!(out.failed.len(), 1);
	assert_eq!((out.failed[0].0.as_str(), out.failed[0].1), ("identity", id2));
	assert!(out.refused.is_empty() && out.deferred.is_empty());
	assert_eq!(out.persisted, vec![(s("identity"), id1), (s("identity"), id3)]);
	assert_eq!(res!(stored(&b, "identity", id2)), None);
	assert_eq!(res!(stored(&b, "identity", id3)), Some(b"three".to_vec()));

	// A local put returns the Err.
	assert!(b.put_at(rec("identity", id2, b"ERR"), 1).is_err());

	// A failed read of the held value is also a per-record failure.
	let id4 = rid(4);
	res!(b.storage().fail_get_of(Some(id4)));
	let out = res!(b.handle_envelope_at(push(1, 2, "identity", vec![
		rec("identity", id4, b"four"),
		rec("identity", rid(5), b"five"),
	]), 1));
	assert_eq!(out.failed.len(), 1);
	assert_eq!(out.failed[0].1, id4);
	assert_eq!(out.persisted, vec![(s("identity"), rid(5))]);
	res!(b.storage().fail_get_of(None));

	// With the fault cleared the record is offered again and stored.
	armed.store(false, Ordering::SeqCst);
	let out = res!(round(&b, &a, 1, "identity", 2));
	assert_eq!(out.persisted, vec![(s("identity"), id2)]);
	assert!(out.failed.is_empty());
	assert_eq!(res!(stored(&b, "identity", id2)), Some(b"ERR".to_vec()));
	Ok(())
}

#[test]
fn persisted_lists_exactly_stored() -> Outcome<()> {
	let (t1, t2, k, d, r, t3) = (rid(1), rid(2), rid(3), rid(4), rid(5), rid(6));
	let records = vec![
		rec("identity", t1, b"T1"),
		rec("identity", t2, b"T2"),
		rec("identity", k, b"K-new"),
		rec("identity", d, b"D"),
		rec("identity", r, b"R"),
		rec("identity", t3, b"T3"),
	];
	for kind in 0..2 {
		let e = res!(holder(1, 2, TestStorage::new(), ByTag));
		res!(e.storage().put(&rec("identity", k, b"K-old")));
		// t3 is a Take whose write fails: it is failed, never persisted.
		res!(e.storage().fail_put_of(Some(t3)));
		let env = if kind == 0 {
			reply(2, 1, "identity", records.clone())
		} else {
			push(2, 1, "identity", records.clone())
		};
		let out = res!(e.handle_envelope_at(env, 1));

		assert_eq!(out.persisted, vec![(s("identity"), t1), (s("identity"), t2)]);
		assert_eq!(out.deferred, vec![(s("identity"), d)]);
		assert_eq!(out.refused, vec![(s("identity"), r, s("r"))]);
		assert_eq!(out.failed.len(), 1);
		assert_eq!(out.failed[0].1, t3);

		// Each of the five listed records is in one list only, and the Keep is in none.
		let mut listed: Vec<RecordId> = out.persisted.iter().map(|x| x.1).collect();
		listed.extend(out.deferred.iter().map(|x| x.1));
		listed.extend(out.refused.iter().map(|x| x.1));
		listed.extend(out.failed.iter().map(|x| x.1));
		listed.sort();
		let before = listed.len();
		listed.dedup();
		assert_eq!(before, 5);
		assert_eq!(listed.len(), 5);
		assert!(!listed.contains(&k));

		// Storage agrees with the lists.
		assert_eq!(res!(stored(&e, "identity", t1)), Some(b"T1".to_vec()));
		assert_eq!(res!(stored(&e, "identity", t2)), Some(b"T2".to_vec()));
		assert_eq!(res!(stored(&e, "identity", k)), Some(b"K-old".to_vec()));
		for id in [d, r, t3] {
			assert_eq!(res!(stored(&e, "identity", id)), None);
		}
	}
	Ok(())
}

#[test]
fn resolver_sees_held_and_view() -> Outcome<()> {
	let e = res!(holder(1, 2, MemoryStorage::new(), Recorder::new()));
	res!(e.storage().put(&rec("identity", rid(200), b"BASE")));
	res!(e.storage().put(&rec("escrow", rid(5), b"H1")));

	// A local put at a given time: the stored held value, the time and the view.
	res!(e.put_at(rec("escrow", rid(5), b"N1"), 1234));
	// A first arrival is validated too, with no held value.
	res!(e.handle_envelope_at(replicate(2, 1, rec("escrow", rid(6), b"N2")), 99));
	// A reply sees the value the put left.
	res!(e.handle_envelope_at(reply(2, 1, "escrow", vec![rec("escrow", rid(5), b"N3")]), 100));

	let seen = lock_mutex!(e.resolver().seen);
	assert_eq!(seen.len(), 3);
	assert_eq!(seen[0].table, "escrow");
	assert_eq!(seen[0].id, rid(5));
	assert_eq!(seen[0].held, Some(b"H1".to_vec()));
	assert_eq!(seen[0].incoming, b"N1".to_vec());
	assert_eq!(seen[0].now_ms, 1234);
	assert_eq!(seen[0].other, Some(b"BASE".to_vec()));
	assert_eq!(seen[1].held, None);
	assert_eq!(seen[1].now_ms, 99);
	assert_eq!(seen[1].other, Some(b"BASE".to_vec()));
	assert_eq!(seen[2].held, Some(b"N1".to_vec()));
	assert_eq!(seen[2].now_ms, 100);
	Ok(())
}

#[test]
fn not_holder_skips_resolver() -> Outcome<()> {
	let r = rec("identity", rid(3), b"v");

	// Replication 0: this peer holds nothing, so no path may call the resolver.
	let e = res!(engine(1, 2, 0, 10, MemoryStorage::new(), Recorder::new()));
	let out = res!(e.handle_envelope_at(replicate(2, 1, r.clone()), 1));
	assert!(nothing_listed(&out));
	let out = res!(e.handle_envelope_at(reply(2, 1, "identity", vec![r.clone()]), 1));
	assert!(nothing_listed(&out));
	let out = res!(e.handle_envelope_at(push(2, 1, "identity", vec![r.clone()]), 1));
	assert!(nothing_listed(&out));
	let out = res!(e.put_at(r.clone(), 1));
	assert_eq!(out.verdict, None);
	assert!(!out.local_persisted);
	assert_eq!(res!(e.resolver().calls()), 0);
	assert_eq!(res!(e.storage().len()), 0);

	// Control: a holder calls it on each of the four paths.
	let e = res!(holder(1, 2, MemoryStorage::new(), Recorder::new()));
	res!(e.handle_envelope_at(replicate(2, 1, r.clone()), 1));
	res!(e.handle_envelope_at(reply(2, 1, "identity", vec![r.clone()]), 1));
	res!(e.handle_envelope_at(push(2, 1, "identity", vec![r.clone()]), 1));
	res!(e.put_at(r, 1));
	assert_eq!(res!(e.resolver().calls()), 4);
	Ok(())
}


// ---------------------------------------------------------------------------
// The default resolver.
// ---------------------------------------------------------------------------

#[test]
fn last_version_wins_lattice() -> Outcome<()> {
	// The value layout is a big-endian u64 version, then the payload.
	let v = LastVersionWins::value(0x0102_0304_0506_0708, b"pay");
	assert_eq!(&v[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
	assert_eq!(&v[8..], b"pay");
	assert_eq!(LastVersionWins::version(&v), Some(0x0102_0304_0506_0708));
	assert_eq!(LastVersionWins::version(&LastVersionWins::value(0, b"")), Some(0));
	assert_eq!(LastVersionWins::version(&v[..7]), None);
	assert_eq!(LastVersionWins::version(&[]), None);
	// A later version beats an earlier one whatever the payloads are.
	assert!(LastVersionWins::value(2, b"a") > LastVersionWins::value(1, b"zzz"));

	// Held and incoming, straight from the resolver.
	let ctx = ResolveCtx::at(0);
	let lo = LastVersionWins::value(1, b"a");
	let hi = LastVersionWins::value(2, b"a");
	let id = rid(1);
	assert_eq!(
		res!(LastVersionWins.resolve(&ctx, &EmptyView, "identity", &id, None, &lo)),
		Verdict::Take(lo.clone()),
	);
	assert_eq!(
		res!(LastVersionWins.resolve(&ctx, &EmptyView, "identity", &id, Some(&lo), &hi)),
		Verdict::Take(hi.clone()),
	);
	assert_eq!(
		res!(LastVersionWins.resolve(&ctx, &EmptyView, "identity", &id, Some(&hi), &lo)),
		Verdict::Keep,
	);
	assert_eq!(
		res!(LastVersionWins.resolve(&ctx, &EmptyView, "identity", &id, Some(&hi), &hi)),
		Verdict::Keep,
	);

	// Through the engine, over all ordered pairs of six values, including the
	// empty value and one too short to carry a version: the stored result is
	// the greater of the two whatever the order, and a repeat changes nothing.
	let values: Vec<Vec<u8>> = vec![
		LastVersionWins::value(1, b"a"),
		LastVersionWins::value(1, b"b"),
		LastVersionWins::value(2, b""),
		LastVersionWins::value(2, b"a"),
		Vec::new(),
		b"short".to_vec(),
	];
	let settle = |first: &[u8], second: &[u8]| -> Outcome<Vec<u8>> {
		let e = res!(DistOzone::new(
			res!(DistOzoneConfig::new(
				node(1),
				Vec::new(),
				res!(OamConfig::new(1, 1)),
				vec![res!(TableConfig::eventual("identity"))],
			)),
			MemoryStorage::new(),
		));
		res!(e.put_at(rec("identity", rid(1), first), 0));
		res!(e.put_at(rec("identity", rid(1), second), 0));
		res!(e.put_at(rec("identity", rid(1), first), 0));
		match res!(stored(&e, "identity", rid(1))) {
			Some(v)	=> Ok(v),
			None	=> Err(err!("Nothing stored for the pair."; Missing)),
		}
	};
	for x in &values {
		for y in &values {
			let xy = res!(settle(x, y));
			let yx = res!(settle(y, x));
			assert_eq!(xy, yx);
			assert_eq!(&xy, std::cmp::max(x, y));
		}
	}
	Ok(())
}


// ---------------------------------------------------------------------------
// R2: atomicity, and the convergence helper.
// ---------------------------------------------------------------------------

/// A set of bytes, kept sorted and without repeats. A commutative merge.
fn union(held: Option<&[u8]>, incoming: &[u8]) -> Vec<u8> {
	let mut set = held.unwrap_or(&[]).to_vec();
	set.extend_from_slice(incoming);
	set.sort();
	set.dedup();
	set
}

/// The set union, optionally giving other threads the processor between the
/// engine's read of the held value and its write of the merge.
struct Union {
	pause:	bool,
}

impl Resolver for Union {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		let merged = union(held, incoming);
		if self.pause {
			thread::yield_now();
			thread::sleep(Duration::from_micros(100));
		}
		Ok(Verdict::Take(merged))
	}
}

/// The first value to arrive stands. Not commutative: peers that were first
/// given different values keep them.
struct FirstWins;

impl Resolver for FirstWins {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		match held {
			Some(_)	=> Ok(Verdict::Keep),
			None	=> Ok(Verdict::Take(incoming.to_vec())),
		}
	}
}

/// Takes a value no peer has held before, whatever it is offered, so no round
/// ever stores nothing.
struct NeverSettles(Arc<AtomicU64>);

impl Resolver for NeverSettles {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		_held:		Option<&[u8]>,
		_incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		Ok(Verdict::Take(self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes().to_vec()))
	}
}

/// Two records may claim one token, a (token, time) pair of bytes, and the
/// later claim loses it. The held and incoming claims are merged, then any
/// claim an earlier claim on another record contradicts is dropped, the other
/// record being confirmed by a point read.
struct ClaimRule;

const CLAIMANTS: [u8; 3] = [1, 2, 3];

impl Resolver for ClaimRule {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		view:		&V,
		table:		&str,
		id:			&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		// Claims are pairs, so merge them as pairs.
		let mut claims: Vec<[u8; 2]> = Vec::new();
		for bytes in [held.unwrap_or(&[]), incoming] {
			for pair in bytes.chunks_exact(2) {
				claims.push([pair[0], pair[1]]);
			}
		}
		claims.sort();
		claims.dedup();
		let mut kept = Vec::new();
		for claim in claims {
			let mut lost = false;
			for other in CLAIMANTS {
				if rid(other) == *id {
					continue;
				}
				if let Some(v) = res!(view.get(table, &rid(other))) {
					for theirs in v.chunks_exact(2) {
						// The earlier claim, by (time, record), keeps the token.
						if theirs[0] == claim[0] && (theirs[1], rid(other)) < (claim[1], *id) {
							lost = true;
						}
					}
				}
			}
			if !lost {
				kept.extend_from_slice(&claim);
			}
		}
		Ok(Verdict::Take(kept))
	}
}

fn converge(seeds: std::ops::Range<u64>, max_rounds: usize) -> Convergence {
	Convergence::new(3, seeds, 0, max_rounds)
}

/// The text of an error, with the seed it names.
fn failure_text(r: Outcome<()>) -> String {
	match r {
		Ok(())	=> String::new(),
		Err(e)	=> fmt!("{}", e),
	}
}

#[test]
fn resolve_and_persist_atomic() -> Outcome<()> {
	// Four threads each make 50 puts on one key, alternating a local put and an
	// inbound ReplicatePut, with a resolver that gives the processor away
	// between reading the held value and returning the merge. Every element is
	// distinct, so a lost update leaves the final set short of 200.
	let e = res!(holder(1, 2, MemoryStorage::new(), Union { pause: true }));
	let all = thread::scope(|sc| -> Outcome<()> {
		let mut workers = Vec::new();
		for t in 0..4u8 {
			let e = &e;
			workers.push(sc.spawn(move || -> Outcome<()> {
				for k in 0..50u8 {
					let record = rec("identity", rid(1), &[t * 50 + k]);
					if k % 2 == 0 {
						res!(e.put_at(record, 0));
					} else {
						res!(e.handle_envelope_at(replicate(2, 1, record), 0));
					}
				}
				Ok(())
			}));
		}
		for w in workers {
			match w.join() {
				Ok(r)	=> res!(r),
				Err(_)	=> return Err(err!("A worker thread panicked."; Test, Thread)),
			}
		}
		Ok(())
	});
	res!(all);
	let want: Vec<u8> = (0..200u8).collect();
	assert_eq!(res!(stored(&e, "identity", rid(1))), Some(want));
	Ok(())
}

#[test]
fn non_commutative_fails_convergence() -> Outcome<()> {
	let records = vec![
		rec("identity", rid(1), b"x"),
		rec("identity", rid(1), b"y"),
		rec("identity", rid(1), b"z"),
		rec("identity", rid(2), b"p"),
	];
	let cfg = converge(0..32, 16);
	// Last arrival wins: the peers pass values to each other and the final value
	// follows the order of the exchanges, so it never settles on one answer.
	let last = check_convergence(&cfg, &["identity"], &records, |_| TakeIncoming, |_, _, _| Ok(Vec::new()));
	assert!(last.is_err(), "last-arrival-wins passed the convergence check");
	// First arrival wins: the peers settle at once, on different values.
	let first = failure_text(check_convergence(
		&cfg, &["identity"], &records, |_| FirstWins, |_, _, _| Ok(Vec::new()),
	));
	assert!(first.contains("differs from peer 0"), "first-arrival-wins: {:?}", first);
	assert!(first.contains("seed"), "no seed named: {:?}", first);
	let id1: String = rid(1).as_bytes().iter().map(|b| fmt!("{:02x}", b)).collect();
	assert!(first.contains(&id1), "the differing id was not named: {:?}", first);
	Ok(())
}

#[test]
fn commutative_passes_convergence() -> Outcome<()> {
	let records = vec![
		rec("identity", rid(1), &[1]),
		rec("identity", rid(1), &[2]),
		rec("identity", rid(1), &[3, 9]),
		rec("identity", rid(2), &[4]),
		rec("escrow", rid(1), &[5]),
		rec("escrow", rid(1), &[6]),
	];
	let cfg = converge(0..32, 16);
	let tables = ["identity", "escrow"];
	let calls = AtomicUsize::new(0);
	res!(check_convergence(
		&cfg, &tables, &records, |_| Union { pause: false },
		|_, _, _| {
			calls.fetch_add(1, Ordering::SeqCst);
			Ok(Vec::new())
		},
	));
	// The helper ran: every seed stores at least the three keys on some peer.
	assert!(calls.load(Ordering::SeqCst) >= 32 * 3, "after ran {} times", calls.load(Ordering::SeqCst));

	let versioned = vec![
		rec("identity", rid(1), &LastVersionWins::value(1, b"a")),
		rec("identity", rid(1), &LastVersionWins::value(3, b"b")),
		rec("identity", rid(1), &LastVersionWins::value(2, b"c")),
		rec("identity", rid(2), &LastVersionWins::value(7, b"d")),
		rec("escrow", rid(1), &LastVersionWins::value(4, b"e")),
	];
	res!(check_convergence(&cfg, &tables, &versioned, |_| LastVersionWins, |_, _, _| Ok(Vec::new())));
	Ok(())
}

#[test]
fn cross_key_needs_after_hook() -> Outcome<()> {
	// Records 1 and 2 both claim the token 'o'; record 1's claim is earlier. Where
	// record 2 arrives first, it keeps the claim, and nothing offers it to the
	// resolver again when record 1 turns up.
	let records = vec![
		rec("identity", rid(1), &[b'o', 1]),
		rec("identity", rid(2), &[b'o', 2]),
		rec("identity", rid(3), &[b'q', 3]),
	];
	let cfg = converge(0..32, 16);
	let without = failure_text(check_convergence(
		&cfg, &["identity"], &records, |_| ClaimRule, |_, _, _| Ok(Vec::new()),
	));
	assert!(without.contains("differ"), "no divergence without the pass: {:?}", without);

	// The caller's pass: whenever a peer stores a record, offer it the current
	// bytes of the other claimants, so the rule runs again with the new record in view.
	let pass = |_peer: usize, e: &DistOzone<MemoryStorage, ClaimRule>, persisted: &[(String, RecordId)]| {
		let mut puts = Vec::new();
		for (table, id) in persisted {
			for other in CLAIMANTS {
				if rid(other) != *id {
					if let Some(r) = res!(e.storage().get(table, &rid(other))) {
						puts.push(r);
					}
				}
			}
		}
		Ok(puts)
	};
	res!(check_convergence(&cfg, &["identity"], &records, |_| ClaimRule, pass));
	Ok(())
}

#[test]
fn convergence_bounded() -> Outcome<()> {
	// A resolver that always takes a new value never lets a round store nothing.
	// The check must report that, not run for ever: it runs on its own thread, so
	// that a hang fails this test rather than stopping it.
	let (tx, rx) = mpsc::channel();
	thread::spawn(move || {
		let counter = Arc::new(AtomicU64::new(0));
		let records = vec![
			rec("identity", rid(1), b"x"),
			rec("identity", rid(1), b"y"),
		];
		let cfg = converge(0..4, 6);
		let r = check_convergence(
			&cfg,
			&["identity"],
			&records,
			|_| NeverSettles(counter.clone()),
			|_, _, _| Ok(Vec::new()),
		);
		let first = failure_text(r);

		// The same bound holds for a post-write pass that stores something new
		// each time it runs.
		let r = check_convergence(
			&converge(0..1, 4),
			&["identity"],
			&records,
			|_| NeverSettles(counter.clone()),
			|_, _, _| Ok(vec![rec("identity", rid(9), b"again")]),
		);
		let _ = tx.send((first, failure_text(r)));
	});
	let (text, deep) = match rx.recv_timeout(Duration::from_secs(20)) {
		Ok(texts)	=> texts,
		Err(_)		=> return Err(err!("check_convergence did not return within 20 s."; Test, Timeout)),
	};
	assert!(text.contains("did not quiesce"), "{:?}", text);
	assert!(text.contains("seed 0"), "{:?}", text);
	assert!(deep.contains("did not quiesce") && deep.contains("post-write"), "{:?}", deep);
	Ok(())
}


// ---------------------------------------------------------------------------
// Post-QA fixes (2026-10-05): cohort tables, a poisoned lock, one listing per
// record, a failed Decide store, byte comparison and a missing dependency.
// ---------------------------------------------------------------------------

/// A holder that also has a cohort table, "ledger", beside the eventual ones.
fn with_ledger<S: Storage, R: Resolver>(storage: S, resolver: R) -> Outcome<DistOzone<S, R>> {
	let oam = res!(OamConfig::new(2, 2));
	let tables = vec![
		res!(TableConfig::eventual("identity")),
		res!(TableConfig::eventual("escrow")),
		res!(TableConfig::cohort_default("ledger")),
	];
	let cfg = res!(DistOzoneConfig::new(node(1), vec![node(2)], oam, tables));
	DistOzone::with_resolver(cfg, storage, resolver)
}

#[test]
fn cohort_table_refuses_replicate_put() -> Outcome<()> {
	let e = res!(with_ledger(MemoryStorage::new(), LastVersionWins));
	// Stand in for a value a HotStuff Decide stored.
	res!(e.storage().put(&rec("ledger", rid(7), b"\x01decided")));
	let out = res!(e.handle_envelope_at(replicate(2, 1, rec("ledger", rid(7), b"\x09forged")), 0));
	assert_eq!(res!(stored(&e, "ledger", rid(7))), Some(b"\x01decided".to_vec()),
		"a ReplicatePut overwrote a cohort table's decided value; persisted={:?}", out.persisted);
	assert!(out.persisted.is_empty());
	assert_eq!(out.failed.len(), 1, "the refused record is in failed");
	assert_eq!((out.failed[0].0.as_str(), out.failed[0].1), ("ledger", rid(7)));
	Ok(())
}

#[test]
fn cohort_table_refuses_anti_entropy_push() -> Outcome<()> {
	let e = res!(with_ledger(TestStorage::new(), LastVersionWins));
	let out = res!(e.handle_envelope_at(
		push(2, 1, "ledger", vec![rec("ledger", rid(8), b"undecided")]), 0));
	assert_eq!(res!(stored(&e, "ledger", rid(8))), None,
		"an AntiEntropyPush stored a never-decided record in a cohort table; persisted={:?}", out.persisted);
	assert!(out.persisted.is_empty());
	assert_eq!(out.failed.len(), 1, "the refused record is in failed");
	// An anti-entropy reply is the same door.
	let out = res!(e.handle_envelope_at(
		reply(2, 1, "ledger", vec![rec("ledger", rid(9), b"undecided")]), 0));
	assert_eq!(res!(stored(&e, "ledger", rid(9))), None);
	assert_eq!(out.failed.len(), 1);
	assert_eq!(e.storage().puts(), 0);
	Ok(())
}

/// Panics on a value that begins with "PANIC", and takes any other.
struct Panicky;

impl Resolver for Panicky {
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
		if incoming.starts_with(b"PANIC") {
			panic!("resolver bug on one malformed record");
		}
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

#[test]
fn resolver_panic_does_not_wedge_engine() -> Outcome<()> {
	let e = res!(holder(1, 2, MemoryStorage::new(), Panicky));
	let hit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		e.handle_envelope_at(replicate(2, 1, rec("identity", rid(1), b"PANIC")), 0)
	}));
	assert!(hit.is_err(), "the resolver was meant to panic");
	// A different, well-formed record afterwards, by each door.
	let later = e.put_at(rec("identity", rid(2), b"fine"), 0);
	assert!(later.is_ok(), "a put after one resolver panic failed: {:?}", later.err());
	let inbound = res!(e.handle_envelope_at(replicate(2, 1, rec("identity", rid(3), b"fine")), 0));
	assert!(inbound.failed.is_empty(), "inbound after one resolver panic failed: {:?}", inbound.failed);
	assert_eq!(inbound.persisted, vec![(s("identity"), rid(3))]);
	assert_eq!(res!(stored(&e, "identity", rid(1))), None);
	Ok(())
}

/// Reads of an id fail once that id has been written.
struct ReadFailsAfterPut {
	inner:	MemoryStorage,
	armed:	Mutex<Vec<RecordId>>,
}

impl Storage for ReadFailsAfterPut {
	fn put(&self, record: &Record) -> Outcome<()> {
		lock_mutex!(self.armed).push(record.id);
		self.inner.put(record)
	}

	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Record>> {
		if lock_mutex!(self.armed).contains(id) {
			return Err(err!("Injected read fault."; IO, Read));
		}
		self.inner.get(table, id)
	}

	fn delete(&self, table: &str, id: &RecordId) -> Outcome<bool> {
		self.inner.delete(table, id)
	}

	fn digests(&self, table: &str) -> Outcome<Vec<RecordDigest>> {
		self.inner.digests(table)
	}
}

#[test]
fn reply_lists_each_record_at_most_once() -> Outcome<()> {
	let st = ReadFailsAfterPut { inner: MemoryStorage::new(), armed: Mutex::new(Vec::new()) };
	let e = res!(holder(1, 2, st, LastVersionWins));
	// An id whose content differs on the two sides is in both halves of a
	// decode, so it arrives in `records` and is also requested back.
	let out = res!(e.handle_envelope_at(
		reply_asking(2, 1, "identity", vec![rec("identity", rid(4), b"v")], vec![rid(4)]),
		0,
	));
	assert_eq!(out.persisted, vec![(s("identity"), rid(4))]);
	assert!(out.failed.is_empty(),
		"a local read fault was blamed on the sender: persisted={:?} failed={:?}", out.persisted, out.failed);
	Ok(())
}

/// A reply that also asks for ids back.
fn reply_asking(from: u8, to: u8, table: &str, records: Vec<Record>, requested_ids: Vec<RecordId>) -> Envelope {
	Envelope::new(node(from), node(to), MsgKind::AntiEntropyReply {
		table:	s(table),
		records,
		requested_ids,
		bulk:	false,
	})
}

/// The leader and members of a cohort, by the engine's own selection.
fn cohort_for(ids: &[NodeId], table: &str, id: &RecordId, lambda: u64) -> Outcome<(NodeId, Vec<NodeId>)> {
	let local = ids[0];
	let mut peers = PeerSet::new();
	for p in ids.iter().filter(|p| **p != local) {
		peers.insert(*p);
	}
	let c = res!(cohort::select(table, id, &peers, &local, lambda));
	Ok((c.leader, c.members))
}

#[test]
fn decide_store_fault_leaves_instance_undecided() -> Outcome<()> {
	let ids: Vec<NodeId> = (1..=5).map(node).collect();
	let mut engines: HashMap<NodeId, DistOzone<TestStorage>> = HashMap::new();
	for me in &ids {
		let peers = ids.iter().filter(|p| *p != me).copied().collect();
		let oam = res!(OamConfig::new(5, 5));
		let tables = vec![res!(TableConfig::new(
			"treasury",
			Consistency::Cohort { lambda: 5 },
			TableConfig::DEFAULT_AE,
			TableConfig::DEFAULT_IBLT_CELLS,
		))];
		let cfg = res!(DistOzoneConfig::new(*me, peers, oam, tables));
		engines.insert(*me, res!(DistOzone::new(cfg, TestStorage::new())));
	}
	let id = rid(0x42);
	let record = rec("treasury", id, b"payload");
	let (leader, members) = res!(cohort_for(&ids, "treasury", &id, 5));
	// A follower whose store fails when the Decide reaches it.
	let victim = members[3];
	res!(engines[&victim].storage().fail_put_of(Some(id)));

	let mut pending = res!(engines[&leader].put(record.clone())).outbound;
	let mut faulted: Vec<Envelope> = Vec::new();
	for _ in 0..64 {
		if pending.is_empty() {
			break;
		}
		let mut next = Vec::new();
		for env in pending.drain(..) {
			let to = env.to;
			match engines[&to].handle_envelope(env.clone()) {
				Ok(out)					=> next.extend(out.outbound),
				Err(_) if to == victim	=> faulted.push(env),
				Err(e)					=> return Err(e),
			}
		}
		pending = next;
	}
	assert!(!faulted.is_empty(), "the victim's store fault was never reached");
	assert_eq!(res!(stored(&engines[&victim], "treasury", id)), None);

	// The replica is inert once it has emitted its Decide, so the decided record
	// must be held and stored again when the caller's timer fires: still failing
	// while the fault lasts, stored once it clears.
	let victim_engine = &engines[&victim];
	assert!(victim_engine.cohort_timeout("treasury", &id).is_err(),
		"a failed Decide store was forgotten, not held for a retry");
	res!(victim_engine.storage().fail_put_of(None));
	res!(victim_engine.cohort_timeout("treasury", &id));
	assert_eq!(res!(stored(victim_engine, "treasury", id)), Some(b"payload".to_vec()),
		"a failed Decide store left the replica decided with no record");
	// Decided now, so a further timeout is quiet.
	assert!(res!(victim_engine.cohort_timeout("treasury", &id)).is_empty());
	Ok(())
}

#[test]
fn check_detects_trailing_zero_divergence() -> Outcome<()> {
	// Control: first-arrival-wins on [1] and [2] is caught.
	let ctl = vec![rec("identity", rid(1), &[1]), rec("identity", rid(1), &[2])];
	let r = check_convergence(&converge(0..32, 16), &["identity"], &ctl, |_| FirstWins, |_, _, _| Ok(Vec::new()));
	assert!(r.is_err(), "control: FirstWins on [1] and [2] passed");

	// The same resolver on [1] and [1, 0]: the peers hold different bytes whose
	// content digests are equal, so only a comparison of the bytes sees it.
	let recs = vec![rec("identity", rid(1), &[1]), rec("identity", rid(1), &[1, 0])];
	let text = failure_text(check_convergence(
		&converge(0..32, 16), &["identity"], &recs, |_| FirstWins, |_, _, _| Ok(Vec::new()),
	));
	assert!(text.contains("differs from peer 0"), "FirstWins on [1] and [1, 0] passed: {:?}", text);
	Ok(())
}

/// An escrow record needs the identity at its own id: with it missing the rule
/// says Defer, or Refuse when `refuse` is set (the bug the contract warns of).
struct NeedsIdentity {
	refuse: bool,
}

impl Resolver for NeedsIdentity {
	fn resolve<V: ReadView>(
		&self,
		ctx:		&ResolveCtx,
		view:		&V,
		table:		&str,
		id:			&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		if table == "escrow" && res!(view.get("identity", id)).is_none() {
			return Ok(if self.refuse {
				Verdict::Refuse(s("no identity"))
			} else {
				Verdict::Defer
			});
		}
		LastVersionWins.resolve(ctx, view, table, id, held, incoming)
	}
}

#[test]
fn refuse_on_missing_dependency_fails_check() -> Outcome<()> {
	// The escrow is listed before its identity.
	let recs = vec![
		rec("escrow", rid(1), &LastVersionWins::value(1, b"e")),
		rec("identity", rid(1), &LastVersionWins::value(1, b"i")),
	];
	let cfg = converge(0..32, 16);
	let tables = ["identity", "escrow"];
	let good = check_convergence(&cfg, &tables, &recs, |_| NeedsIdentity { refuse: false }, |_, _, _| Ok(Vec::new()));
	assert!(good.is_ok(), "Defer on a missing dependency failed: {:?}", good.err());
	let bad = failure_text(check_convergence(
		&cfg, &tables, &recs, |_| NeedsIdentity { refuse: true }, |_, _, _| Ok(Vec::new()),
	));
	assert!(!bad.is_empty(), "Refuse on a missing dependency passed the check");
	Ok(())
}

// ---------------------------------------------------------------------------
// Known red, carried by A2: the content hash ignores length (2-F1) and
// anti-entropy repairs only the dialling side (2-F3).
// ---------------------------------------------------------------------------

/// A mesh of `n` peers on the eventual table "identity", each holding everything.
fn mesh(n: usize) -> Outcome<Vec<DistOzone<MemoryStorage>>> {
	let ids: Vec<NodeId> = (0..n).map(|i| node((i + 1) as u8)).collect();
	let mut out = Vec::new();
	for me in &ids {
		let others = ids.iter().filter(|x| *x != me).copied().collect();
		let oam = res!(OamConfig::new(n as u64, n as u64));
		let cfg = res!(DistOzoneConfig::new(*me, others, oam, vec![res!(TableConfig::eventual("identity"))]));
		out.push(res!(DistOzone::new(cfg, MemoryStorage::new())));
	}
	Ok(out)
}

/// One whole anti-entropy exchange, `a` dialling `b`.
fn exchange(e: &[DistOzone<MemoryStorage>], a: usize, b: usize) -> Outcome<()> {
	let mut pending = vec![res!(e[a].build_anti_entropy_request("identity", node((b + 1) as u8)))];
	while let Some(env) = pending.pop() {
		let to = (env.to.as_bytes()[31] - 1) as usize;
		let out = res!(e[to].handle_envelope_at(env, 0));
		assert!(out.failed.is_empty());
		pending.extend(out.outbound);
	}
	Ok(())
}

#[test]
fn lww_lost_replicate_not_repaired() -> Outcome<()> {
	let e = res!(mesh(2));
	let hi = LastVersionWins::value(7, &[0xAB, 0]);
	let lo = LastVersionWins::value(7, &[0xAB]);
	// Each peer writes locally and both replications are lost.
	res!(e[0].put_at(rec("identity", rid(1), &hi), 0));
	res!(e[1].put_at(rec("identity", rid(1), &lo), 0));
	for _ in 0..4 {
		res!(exchange(&e, 0, 1));
		res!(exchange(&e, 1, 0));
	}
	assert_eq!(res!(stored(&e[0], "identity", rid(1))), Some(hi.clone()));
	assert_eq!(res!(stored(&e[1], "identity", rid(1))), Some(hi));
	Ok(())
}

#[test]
fn forged_same_digest_value_never_repaired() -> Outcome<()> {
	// A value is a version then two 8-byte chunks. The second chunk of the
	// forgery is solved so the chained hash after it matches the original's.
	fn mix(mut x: u64) -> u64 {
		x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
		x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
		x ^ (x >> 31)
	}
	let s_ver = mix(0x9E3779B97F4A7C15u64.wrapping_add(u64::from_le_bytes(9u64.to_be_bytes())));
	let (p0, p1, p0f) = (0x1111_1111_1111_1111u64, 0x2222_2222_2222_2222u64, 0x3333_3333_3333_3333u64);
	let p1f = mix(s_ver.wrapping_add(p0)).wrapping_add(p1).wrapping_sub(mix(s_ver.wrapping_add(p0f)));
	let mut pa = p0.to_le_bytes().to_vec();
	pa.extend_from_slice(&p1.to_le_bytes());
	let mut pb = p0f.to_le_bytes().to_vec();
	pb.extend_from_slice(&p1f.to_le_bytes());
	let (va, vb) = (LastVersionWins::value(9, &pa), LastVersionWins::value(9, &pb));
	assert_ne!(va, vb);

	let e = res!(mesh(2));
	res!(e[0].put_at(rec("identity", rid(1), &va), 0));
	res!(e[1].put_at(rec("identity", rid(1), &vb), 0));
	for _ in 0..4 {
		res!(exchange(&e, 0, 1));
		res!(exchange(&e, 1, 0));
	}
	assert_eq!(res!(stored(&e[0], "identity", rid(1))), res!(stored(&e[1], "identity", rid(1))));
	Ok(())
}

#[test]
fn dialled_peer_not_offered_on_value_divergence() -> Outcome<()> {
	let e = res!(mesh(2));
	let v2 = LastVersionWins::value(2, b"new");
	let v1 = LastVersionWins::value(1, b"old");
	// Peer 0 holds the newer X and a Y; peer 1 missed both but holds the older X.
	res!(e[0].put_at(rec("identity", rid(1), &v2), 0));
	res!(e[0].put_at(rec("identity", rid(2), b"y"), 0));
	res!(e[1].put_at(rec("identity", rid(1), &v1), 0));
	res!(exchange(&e, 0, 1));
	assert_eq!(res!(stored(&e[1], "identity", rid(1))), Some(v2), "the dialled peer kept its stale X");
	assert_eq!(res!(stored(&e[1], "identity", rid(2))), Some(b"y".to_vec()), "the dialled peer never got Y");
	Ok(())
}
