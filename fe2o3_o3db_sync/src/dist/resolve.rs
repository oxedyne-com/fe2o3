//! The resolver seam: the application's rule for which value a record holds.
//!
//! Every record that reaches an eventual table -- a local put, a
//! `ReplicatePut`, an anti-entropy reply or push -- is offered to a
//! [`Resolver`] together with the value already held, and the engine stores
//! only what the resolver says to. For peers to converge, `resolve` must be a
//! function of `(held, incoming)` and the records its [`ReadView`] returns,
//! must not depend on arrival order or on who sent the record, and must answer
//! [`Verdict::Defer`], never [`Verdict::Refuse`], where only time or a missing
//! record can cure the problem. [`LastVersionWins`] is the default.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::{
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
	transport::Envelope,
};
use crate::{
	kademlia::id::NodeId,
	oam::config::OamConfig,
};

use oxedyne_fe2o3_core::prelude::*;

use std::ops::Range;


#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
	Take(Vec<u8>),		// store these bytes: the incoming value, or a merge of held and incoming
	Keep,				// the held value stands
	Defer,				// valid once something else arrives; not stored, offered again by anti-entropy
	Refuse(String),		// invalid; not stored, the held value is untouched; the reason is for the log
}

/// Built with [`ResolveCtx::at`], so that fields can be added later.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ResolveCtx {
	pub now_ms:	u64,	// the caller's clock, in milliseconds since the Unix epoch
}

impl ResolveCtx {
	pub fn at(now_ms: u64) -> Self {
		Self { now_ms }
	}
}

/// Point reads only, so a resolver cannot write.
pub trait ReadView {
	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Vec<u8>>>;
}

/// What the engine passes to a resolver. It is a wrapper, not a blanket
/// `impl ReadView for S: Storage`, which would make `get` ambiguous for a
/// caller with both traits in scope.
pub struct StorageView<'a, S: Storage>(pub &'a S);

impl<'a, S: Storage> ReadView for StorageView<'a, S> {
	fn get(&self, table: &str, id: &RecordId) -> Outcome<Option<Vec<u8>>> {
		Ok(res!(self.0.get(table, id)).map(|r| r.value))
	}
}

pub trait Resolver {
	/// `held` is `None` on a first arrival, which is validated too. The engine
	/// calls this under its write lock, for one record, only on a peer that
	/// holds the record. An `Err` stores nothing and is reported in
	/// `InboundOutcome::failed`, or returned from a local put.
	fn resolve<V: ReadView>(
		&self,
		ctx:		&ResolveCtx,
		view:		&V,
		table:		&str,
		id:			&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>;
}

/// The default resolver: the greater byte string wins. It is a semilattice
/// join, so any arrival order converges, and equal values are a [`Verdict::Keep`].
#[derive(Clone, Copy, Debug, Default)]
pub struct LastVersionWins;

impl LastVersionWins {
	/// A big-endian `u64` version, then the payload. Byte order then sorts by
	/// version first and payload second.
	pub fn value(version: u64, payload: &[u8]) -> Vec<u8> {
		let mut v = Vec::with_capacity(8 + payload.len());
		v.extend_from_slice(&version.to_be_bytes());
		v.extend_from_slice(payload);
		v
	}

	/// `None` when the value is shorter than the eight version bytes.
	pub fn version(value: &[u8]) -> Option<u64> {
		value.first_chunk::<8>().map(|b| u64::from_be_bytes(*b))
	}
}

impl Resolver for LastVersionWins {
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
			Some(h) if incoming <= h	=> Ok(Verdict::Keep),
			_							=> Ok(Verdict::Take(incoming.to_vec())),
		}
	}
}


/// The shape of a [`check_convergence`] run.
#[derive(Clone, Debug)]
pub struct Convergence {
	pub peers:		usize,			// 2 to 5; every peer holds every record
	pub seeds:		Range<u64>,		// one scatter-and-settle run per seed
	pub now_ms:		u64,			// the clock every peer is given
	pub max_rounds:	usize,			// full rounds, and post-write depth, allowed before "did not quiesce"
}

/// Does the resolver (and the application's `after` pass) make the converged
/// state a function of the set of records alone?
///
/// For each seed it builds `cfg.peers` peers on [`MemoryStorage`], each holding
/// everything, and delivers every record as a local put on a random peer in
/// random order. Between the puts it delivers in-flight replication in random
/// order, runs random anti-entropy exchanges and offers a deferred local put
/// again. Whatever `after` returns for a peer's newly stored records is put on
/// that peer, and its stores are passed to `after` in turn. It then runs full
/// rounds (the in-flight messages, the deferred puts, and an anti-entropy
/// exchange between every ordered pair of peers on every table) until a round
/// stores nothing, and compares the bytes the peers hold, and each seed's final state
/// with the first seed's. The `Err` names the seed, the table and the first
/// differing id; it also reports a run that is still storing after
/// `cfg.max_rounds` ("did not quiesce"), a resolver that fails, and an `after`
/// chain deeper than `cfg.max_rounds`.
pub fn check_convergence<R, F, P>(
	cfg:				&Convergence,
	tables:				&[&str],
	records:			&[Record],
	mut new_resolver:	F,
	mut after:			P,
)
	-> Outcome<()>
	where
		R:	Resolver,
		F:	FnMut(usize) -> R,
		P:	FnMut(usize, &DistOzone<MemoryStorage, R>, &[(String, RecordId)])
				-> Outcome<Vec<Record>>,
{
	if cfg.peers < 2 || cfg.peers > 5 {
		return Err(err!(
			"check_convergence needs 2 to 5 peers, got {}.", cfg.peers;
			Invalid, Input, Range));
	}
	if cfg.max_rounds == 0 || tables.is_empty() {
		return Err(err!(
			"check_convergence needs max_rounds >= 1 and at least one table.";
			Invalid, Input, Missing));
	}
	let mut first: Option<(u64, Vec<Held>)> = None;
	for seed in cfg.seeds.clone() {
		let state = match run_seed(cfg, seed, tables, records, &mut new_resolver, &mut after) {
			Ok(state)	=> state,
			Err(e)		=> return Err(err!(e,
				"check_convergence: seed {}.", seed; Test, Mismatch)),
		};
		match &first {
			None => first = Some((seed, state)),
			Some((seed0, state0)) => {
				for (t, table) in tables.iter().enumerate() {
					if let Some(id) = first_diff(&state0[t], &state[t]) {
						return Err(err!(
							"check_convergence: seed {} ends in a different state \
							from seed {} on table '{}', first differing id {}: the \
							converged state depends on the order of arrival.",
							seed, seed0, table, hex(&id);
							Test, Mismatch));
					}
				}
			},
		}
	}
	Ok(())
}

// What a peer holds on one table, by id: the stored bytes themselves. A content
// digest is only a summary and two values can share one, so it is used to list
// the ids and never to compare the values.
type Held = Vec<(RecordId, Vec<u8>)>;

// Builds the peers for one seed, drives them to quiescence and returns the
// final bytes held on each table, after checking that every peer agrees.
fn run_seed<R, F, P>(
	cfg:			&Convergence,
	seed:			u64,
	tables:			&[&str],
	records:		&[Record],
	new_resolver:	&mut F,
	after:			&mut P,
)
	-> Outcome<Vec<Held>>
	where
		R:	Resolver,
		F:	FnMut(usize) -> R,
		P:	FnMut(usize, &DistOzone<MemoryStorage, R>, &[(String, RecordId)])
				-> Outcome<Vec<Record>>,
{
	let n = cfg.peers;
	let mut ids = Vec::with_capacity(n);
	for i in 0..n {
		let mut bytes = [0u8; 32];
		bytes[31] = (i + 1) as u8;
		ids.push(NodeId::from_bytes(bytes));
	}
	let mut engines = Vec::with_capacity(n);
	for i in 0..n {
		let mut tcs = Vec::with_capacity(tables.len());
		for table in tables {
			tcs.push(res!(TableConfig::eventual(*table)));
		}
		let others = ids.iter().copied().filter(|id| *id != ids[i]).collect();
		let oam = res!(OamConfig::new(n as u64, n as u64));
		let dcfg = res!(DistOzoneConfig::new(ids[i], others, oam, tcs));
		engines.push(res!(DistOzone::with_resolver(
			dcfg,
			MemoryStorage::new(),
			new_resolver(i),
		)));
	}
	let mut run = Run {
		seed,
		engines,
		ids,
		rng:		SplitMix64(seed),
		now_ms:		cfg.now_ms,
		max_depth:	cfg.max_rounds,
		after,
		flight:		Vec::new(),
		deferred:	Vec::new(),
		stored:		0,
	};

	let mut order: Vec<usize> = (0..records.len()).collect();
	run.rng.shuffle(&mut order);
	for i in order {
		let peer = run.rng.below(n);
		res!(run.local_put(peer, records[i].clone()));
		for _ in 0..run.rng.below(3) {
			res!(run.step(tables));
		}
	}

	let mut quiet = false;
	for _ in 0..cfg.max_rounds {
		if res!(run.round(tables)) == 0 {
			quiet = true;
			break;
		}
	}
	if !quiet {
		return Err(err!(
			"check_convergence: seed {} did not quiesce: every one of {} full \
			rounds still stored records.", seed, cfg.max_rounds;
			Test, LimitReached));
	}

	let mut state = Vec::with_capacity(tables.len());
	for table in tables {
		let mut mine = Vec::with_capacity(n);
		for engine in &run.engines {
			let mut d = res!(engine.storage().digests(table));
			d.sort_by(|a, b| a.id.cmp(&b.id));
			let mut held = Vec::with_capacity(d.len());
			for digest in d {
				let record = res!(engine.storage().get(table, &digest.id));
				let record = res!(record.ok_or_else(|| err!(
					"check_convergence: seed {}: table '{}' lists id {} in its digests \
					but cannot read it.", seed, table, hex(&digest.id);
					Test, Missing)));
				held.push((digest.id, record.value));
			}
			mine.push(held);
		}
		for p in 1..n {
			if let Some(id) = first_diff(&mine[0], &mine[p]) {
				return Err(err!(
					"check_convergence: seed {}: on table '{}' peer {} differs \
					from peer 0, first differing id {}.", seed, table, p, hex(&id);
					Test, Mismatch));
			}
		}
		state.push(mine.swap_remove(0));
	}
	Ok(state)
}

// One seed's world: the peers, the messages in flight and the puts to offer again.
struct Run<'a, R: Resolver, P> {
	seed:		u64,
	engines:	Vec<DistOzone<MemoryStorage, R>>,
	ids:		Vec<NodeId>,
	rng:		SplitMix64,
	now_ms:		u64,
	max_depth:	usize,
	after:		&'a mut P,
	flight:		Vec<Envelope>,
	deferred:	Vec<(usize, Record)>,		// a local put that answered Defer
	stored:		usize,						// stores made so far, counted by the round
}

impl<'a, R, P> Run<'a, R, P>
	where
		R:	Resolver,
		P:	FnMut(usize, &DistOzone<MemoryStorage, R>, &[(String, RecordId)])
				-> Outcome<Vec<Record>>,
{
	// A local put on one peer. Its replication joins the messages in flight.
	fn local_put(&mut self, peer: usize, record: Record) -> Outcome<()> {
		let out = res!(self.engines[peer].put_at(record.clone(), self.now_ms));
		self.flight.extend(out.outbound);
		match out.verdict {
			Some(Verdict::Take(_))	=> self.settle(peer, vec![(record.table, record.id)]),
			Some(Verdict::Defer)	=> {
				self.deferred.push((peer, record));
				Ok(())
			},
			_						=> Ok(()),
		}
	}

	// The caller's post-write pass over what a peer has just stored, and again
	// over what that pass stores in turn, to a depth of `max_rounds`.
	fn settle(&mut self, peer: usize, persisted: Vec<(String, RecordId)>) -> Outcome<()> {
		self.stored += persisted.len();
		let mut work = vec![(peer, persisted, 0usize)];
		while let Some((p, list, depth)) = work.pop() {
			if list.is_empty() {
				continue;
			}
			if depth >= self.max_depth {
				return Err(err!(
					"check_convergence: seed {} did not quiesce: the post-write \
					pass was still storing records at depth {}.", self.seed, depth;
					Test, LimitReached));
			}
			let puts = res!((self.after)(p, &self.engines[p], &list));
			for record in puts {
				let out = res!(self.engines[p].put_at(record.clone(), self.now_ms));
				self.flight.extend(out.outbound);
				match out.verdict {
					Some(Verdict::Take(_))	=> {
						self.stored += 1;
						work.push((p, vec![(record.table, record.id)], depth + 1));
					},
					Some(Verdict::Defer)	=> self.deferred.push((p, record)),
					_						=> {},
				}
			}
		}
		Ok(())
	}

	// Delivers an envelope and every reply it provokes.
	fn deliver(&mut self, env: Envelope) -> Outcome<()> {
		let mut pending = vec![env];
		while let Some(e) = pending.pop() {
			let dest = res!(self.ids.iter().position(|id| *id == e.to).ok_or_else(|| err!(
				"check_convergence: seed {}: envelope for an unknown peer.", self.seed;
				Invalid, Input, Missing)));
			let out = res!(self.engines[dest].handle_envelope_at(e, self.now_ms));
			// A resolver fault is a bug in the rule under test, not a verdict.
			if let Some((table, id, _)) = out.failed.first() {
				return Err(err!(
					"check_convergence: seed {}: the resolver failed on table '{}' \
					id {}.", self.seed, table, hex(id);
					Test, Unexpected));
			}
			pending.extend(out.outbound);
			res!(self.settle(dest, out.persisted));
		}
		Ok(())
	}

	// One anti-entropy exchange on one table: digest, reply, push.
	fn exchange(&mut self, a: usize, b: usize, table: &str) -> Outcome<()> {
		let req = res!(self.engines[a].build_anti_entropy_request(table, self.ids[b]));
		self.deliver(req)
	}

	// One random event between puts.
	fn step(&mut self, tables: &[&str]) -> Outcome<()> {
		let n = self.engines.len();
		match self.rng.below(4) {
			0 if !self.flight.is_empty()	=> {
				let i = self.rng.below(self.flight.len());
				let env = self.flight.swap_remove(i);
				self.deliver(env)
			},
			1								=> {
				let a = self.rng.below(n);
				let b = (a + 1 + self.rng.below(n - 1)) % n;
				let t = self.rng.below(tables.len());
				self.exchange(a, b, tables[t])
			},
			2 if !self.deferred.is_empty()	=> {
				let i = self.rng.below(self.deferred.len());
				let (peer, record) = self.deferred.swap_remove(i);
				self.local_put(peer, record)
			},
			_								=> Ok(()),
		}
	}

	// A full round: what is in flight, the deferred puts, then every ordered
	// pair on every table. Returns the number of stores the round made.
	fn round(&mut self, tables: &[&str]) -> Outcome<usize> {
		self.stored = 0;
		while !self.flight.is_empty() {
			let i = self.rng.below(self.flight.len());
			let env = self.flight.swap_remove(i);
			res!(self.deliver(env));
		}
		let again = std::mem::take(&mut self.deferred);
		for (peer, record) in again {
			res!(self.local_put(peer, record));
		}
		let n = self.engines.len();
		let mut pairs = Vec::with_capacity(n * (n - 1) * tables.len());
		for a in 0..n {
			for b in 0..n {
				for t in 0..tables.len() {
					if a != b {
						pairs.push((a, b, t));
					}
				}
			}
		}
		self.rng.shuffle(&mut pairs);
		for (a, b, t) in pairs {
			res!(self.exchange(a, b, tables[t]));
		}
		Ok(self.stored)
	}
}

// The first id at which two id-sorted lists differ in presence or in bytes.
fn first_diff(a: &[(RecordId, Vec<u8>)], b: &[(RecordId, Vec<u8>)]) -> Option<RecordId> {
	let (mut i, mut j) = (0, 0);
	loop {
		match (a.get(i), b.get(j)) {
			(None, None)		=> return None,
			(Some(x), None)		=> return Some(x.0),
			(None, Some(y))		=> return Some(y.0),
			(Some(x), Some(y))	=> {
				if x.0 < y.0 {
					return Some(x.0);
				}
				if y.0 < x.0 {
					return Some(y.0);
				}
				if x.1 != y.1 {
					return Some(x.0);
				}
				i += 1;
				j += 1;
			},
		}
	}
}

fn hex(id: &RecordId) -> String {
	id.as_bytes().iter().map(|b| fmt!("{:02x}", b)).collect()
}

// The seeded generator behind every random choice, so a failing seed replays.
struct SplitMix64(u64);

impl SplitMix64 {
	fn draw(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
		z ^ (z >> 31)
	}

	// Uniform enough for scheduling; `n` is at least one.
	fn below(&mut self, n: usize) -> usize {
		(self.draw() % n as u64) as usize
	}

	fn shuffle<T>(&mut self, v: &mut [T]) {
		for i in (1..v.len()).rev() {
			let j = self.below(i + 1);
			v.swap(i, j);
		}
	}
}
