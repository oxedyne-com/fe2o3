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
	config::{
		DistOzoneConfig,
		TableConfig,
	},
	engine::{
		DistOzone,
		InboundOutcome,
	},
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
};

use std::sync::{
	Arc,
	Mutex,
	atomic::{
		AtomicBool,
		AtomicUsize,
		Ordering,
	},
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
