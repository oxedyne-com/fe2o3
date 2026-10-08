//! Per-phase wall time of one compilation, for `austenite --eval --timings FILE`.
//!
//! The recorder is exclusive: while a phase runs inside another (realisation is called from flow and from
//! the page furniture, a sink's page from the pass), the time goes to the innermost phase only, so the
//! phases of a run add up to its wall time and nothing is counted twice. A phase that is never entered
//! shows as zero entries, which is how a missing record is told from a quick phase. The recorder lives on
//! the [`Engine`](crate::eval::Engine) as an `Option`: with none, a phase costs one test of the option,
//! and with one, a clock read at each end of a phase and no allocation per page (a bucket is added once
//! per fixpoint pass).
//!
//! The record also carries the shaped-run cache's counters, which say whether the time in flow was spent
//! shaping again what had been shaped.
//!
//! Realisation is split further. Five sub-phases (rules, regex, show, repack, styles) are opened inside
//! `Realise` and are exclusive in the same way, so the time of `Realise` outside them is its own. A pass's
//! `realise` cell is the whole of it, the sub-phases and the rest, so a reader of the phase table sees one
//! realisation total; the `sub` object splits it, with `self` the part outside every sub-phase. Each pass
//! also counts the realise calls by the mode and the shape of the content realised and by cause, and a few
//! work counters (elements visited, show rules matched against, elements copied before they are changed).
//!
//! It reads `std::time::Instant`, which the wasm32-unknown-unknown target does not provide, so only a
//! native run switches it on.

use crate::fonts::ShapeStats;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::hash::Hasher;
use std::time::Instant;

// One entry per kind of work the fixpoint does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
	// Once a run.
	Load,
	Eval,
	Finish,
	Write,
	// Once a pass.
	Realise,
	Flow,
	Place,
	Decorate,
	Sink,
	Settle,
	// Inside realisation.
	Rules,
	Regex,
	Show,
	Repack,
	Styles,
}

impl Phase {
	pub const COUNT:	usize = 15;
	pub const RUN:		[Phase; 4] = [Phase::Load, Phase::Eval, Phase::Finish, Phase::Write];
	pub const PASS:		[Phase; 6] = [Phase::Realise, Phase::Flow, Phase::Place, Phase::Decorate, Phase::Sink, Phase::Settle];
	pub const SUB:		[Phase; 5] = [Phase::Rules, Phase::Regex, Phase::Show, Phase::Repack, Phase::Styles];

	pub fn name(&self) -> &'static str {
		match self {
			Self::Load		=> "load",
			Self::Eval		=> "eval",
			Self::Finish	=> "finish",
			Self::Write		=> "write",
			Self::Realise	=> "realise",
			Self::Flow		=> "flow",
			Self::Place		=> "place",
			Self::Decorate	=> "decorate",
			Self::Sink		=> "sink",
			Self::Settle	=> "settle",
			Self::Rules		=> "rules",
			Self::Regex		=> "regex",
			Self::Show		=> "show",
			Self::Repack	=> "repack",
			Self::Styles	=> "styles",
		}
	}

	fn index(&self) -> usize { *self as usize }

	fn per_pass(&self) -> bool { Self::PASS.contains(self) || Self::SUB.contains(self) }
}

// What a pass counts besides time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
	Visits,		// elements, sequences and styled runs met by the realisation walk
	Leaves,		// elements that no rule transformed and that reached grouping
	Shown,		// elements and labelled sequences put through the show-rule match
	Recipes,	// recipes in force, summed over the elements shown
	Matched,	// recipes whose selector was tried against an element
	Prepared,	// elements located and given their style values
	Copied,		// elements copied whole because a second holder shared them when they were changed
	Textual,	// textual runs searched for a text or regex rule
	Finished,	// groups finished (paragraphs, lists, cites, textual runs)
}

impl Counter {
	pub const COUNT:	usize = 9;
	pub const ALL:		[Counter; 9] = [
		Counter::Visits, Counter::Leaves, Counter::Shown, Counter::Recipes, Counter::Matched,
		Counter::Prepared, Counter::Copied, Counter::Textual, Counter::Finished,
	];

	pub fn name(&self) -> &'static str {
		match self {
			Self::Visits	=> "visits",
			Self::Leaves	=> "leaves",
			Self::Shown		=> "shown",
			Self::Recipes	=> "recipes",
			Self::Matched	=> "matched",
			Self::Prepared	=> "prepared",
			Self::Copied	=> "copied",
			Self::Textual	=> "textual",
			Self::Finished	=> "finished",
		}
	}
}

// Why a realise call was made, judged by whether the same body was realised before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
	First,		// pass one, a body never realised
	New,		// a later pass, a body no earlier pass realised: an answer has arrived that gives it content
	Repass,		// a body the previous pass realised, realised again for the new pass
	Again,		// a body already realised in this pass, realised once more
}

impl Cause {
	pub const ALL: [Cause; 4] = [Cause::First, Cause::New, Cause::Repass, Cause::Again];

	pub fn name(&self) -> &'static str {
		match self {
			Self::First		=> "first",
			Self::New		=> "new",
			Self::Repass	=> "repass",
			Self::Again		=> "again",
		}
	}
}

// The time and the entries of each phase, indexed by `Phase::index`.
#[derive(Clone, Debug, Default)]
struct Bucket {
	ns:		[u64; Phase::COUNT],
	n:		[u64; Phase::COUNT],
	ctr:	[u64; Counter::COUNT],
	calls:	BTreeMap<String, [u64; 4]>,	// realise calls by mode and shape of the content, then by cause
}

impl Bucket {
	// The time of a phase. Realisation is reported whole, with the sub-phases inside it.
	fn whole(&self, p: Phase) -> u64 {
		match p {
			Phase::Realise	=> self.ns[p.index()] + Phase::SUB.iter().map(|q| self.ns[q.index()]).sum::<u64>(),
			_				=> self.ns[p.index()],
		}
	}

	fn cell(&self, p: Phase) -> String {
		fmt!("\"{}\":{{\"ns\":{},\"n\":{}}}", p.name(), self.whole(p), self.n[p.index()])
	}

	fn json(&self, phases: &[Phase]) -> String {
		let cells: Vec<String> = phases.iter().map(|p| self.cell(*p)).collect();
		fmt!("{{{}}}", cells.join(","))
	}

	// The pass's own record: its phases, then the realisation's split, counters and calls.
	fn pass_json(&self) -> String {
		let cells: Vec<String> = Phase::PASS.iter().map(|p| self.cell(*p)).collect();
		let mut subs: Vec<String> = Phase::SUB.iter().map(|p|
			fmt!("\"{}\":{{\"ns\":{},\"n\":{}}}", p.name(), self.ns[p.index()], self.n[p.index()])).collect();
		subs.push(fmt!("\"self\":{{\"ns\":{},\"n\":{}}}", self.ns[Phase::Realise.index()], self.n[Phase::Realise.index()]));
		let ctrs: Vec<String> = Counter::ALL.iter().map(|c| fmt!("\"{}\":{}", c.name(), self.ctr[*c as usize])).collect();
		let calls: Vec<String> = self.calls.iter().map(|(k, v)| {
			let by: Vec<String> = Cause::ALL.iter().map(|c| fmt!("\"{}\":{}", c.name(), v[*c as usize])).collect();
			fmt!("\"{}\":{{{}}}", k.replace('\\', "\\\\").replace('"', "\\\""), by.join(","))
		}).collect();
		fmt!("{{{},\"sub\":{{{}}},\"counts\":{{{}}},\"calls\":{{{}}}}}",
			cells.join(","), subs.join(","), ctrs.join(","), calls.join(","))
	}
}

#[derive(Debug)]
pub struct Timings {
	last:	Instant,		// the last clock read
	stack:	Vec<Phase>,		// the phases open, outermost first
	run:	Bucket,
	passes:	Vec<Bucket>,	// one for each fixpoint pass
	idle:	u64,			// ns spent outside every phase
	clock:	u64,			// ns one clock read costs, measured when the recorder starts
	shape:	Option<ShapeStats>,	// the shaped-run cache at the end of the run, once the caller has read it
	seen:	HashMap<u64, usize>,	// realised bodies by key: the last pass that realised one
}

impl Timings {
	/// Starts the recorder with `Load` open, since the clock runs from the moment the compile is asked for.
	pub fn start() -> Self {
		let clock = Self::clock_cost();
		let mut stack = Vec::with_capacity(64);
		stack.push(Phase::Load);
		let mut run = Bucket::default();
		run.n[Phase::Load.index()] = 1;
		Self { last: Instant::now(), stack, run, passes: Vec::with_capacity(Phase::PASS.len()), idle: 0, clock, shape: None, seen: HashMap::new() }
	}

	fn clock_cost() -> u64 {
		const READS: u32 = 1_000;
		let t = Instant::now();
		for _ in 0..READS {
			std::hint::black_box(Instant::now());
		}
		(t.elapsed().as_nanos() / READS as u128) as u64
	}

	/// Opens a phase inside whatever is open, the enclosing phase's clock stopping until it ends.
	pub fn enter(&mut self, p: Phase) {
		self.charge_top();
		self.stack.push(p);
		self.count(p);
	}

	/// Closes the innermost phase.
	pub fn leave(&mut self) {
		self.charge_top();
		self.stack.pop();
	}

	/// Begins a fixpoint pass: its phases are counted in a bucket of their own.
	pub fn begin_pass(&mut self) {
		self.passes.push(Bucket::default());
	}

	// Adds the time since the last read to the innermost phase, or to the idle time with none open.
	fn charge_top(&mut self) {
		let now = Instant::now();
		let dt = now.duration_since(self.last).as_nanos() as u64;
		self.last = now;
		match self.stack.last().copied() {
			Some(p)	=> self.bucket(p).ns[p.index()] += dt,
			None	=> self.idle += dt,
		}
	}

	fn count(&mut self, p: Phase) {
		self.bucket(p).n[p.index()] += 1;
	}

	// The bucket a phase is charged to: the run's, or the current pass's (the first is made if no pass was begun).
	fn bucket(&mut self, p: Phase) -> &mut Bucket {
		if p.per_pass() {
			if self.passes.is_empty() {
				self.passes.push(Bucket::default());
			}
			let last = self.passes.len() - 1;
			&mut self.passes[last]
		} else {
			&mut self.run
		}
	}

	pub fn passes(&self) -> usize { self.passes.len() }

	/// Adds `by` to a counter of the current pass.
	pub fn bump(&mut self, c: Counter, by: u64) {
		self.bucket(Phase::Realise).ctr[c as usize] += by;
	}

	/// Records one realise call: under the label of its mode and the shape of its content, and under the
	/// cause its `key` gives, the key being the hash of where the body is realised (the place, the span, the
	/// kind of the content). A key not met in any pass is `First` in pass one and `New` after; one met in
	/// an earlier pass is `Repass`; one already met in this pass is `Again`.
	pub fn note_call(&mut self, label: String, key: u64) {
		let pass = self.passes.len().max(1) - 1;
		let cause = match self.seen.insert(key, pass) {
			Some(p) if p == pass	=> Cause::Again,
			Some(_)					=> Cause::Repass,
			None if pass == 0		=> Cause::First,
			None					=> Cause::New,
		};
		self.bucket(Phase::Realise).calls.entry(label).or_insert([0; 4])[cause as usize] += 1;
	}

	/// The hash that tells one realised body from another, from what places it.
	pub fn key<H: Hash>(placing: &H) -> u64 {
		let mut h = std::collections::hash_map::DefaultHasher::new();
		placing.hash(&mut h);
		h.finish()
	}

	/// Keeps the shaped-run cache's counters for the record.
	pub fn set_shape(&mut self, stats: ShapeStats) { self.shape = Some(stats); }

	fn shape_json(&self) -> String {
		match &self.shape {
			Some(s)	=> fmt!(
				"{{\"entries\":{},\"bytes\":{},\"budget\":{},\"hits\":{},\"misses\":{},\"evictions\":{}}}",
				s.entries, s.bytes, s.budget, s.hits, s.misses, s.evictions),
			None	=> "null".to_string(),
		}
	}

	/// The recorded times as one JSON object, in nanoseconds, with `total` the run's own wall time as the
	/// caller read it.
	pub fn json(&self, total: u64) -> String {
		let passes: Vec<String> = self.passes.iter().map(|b| b.pass_json()).collect();
		fmt!(
			"{{\"unit\":\"ns\",\"total\":{},\"idle\":{},\"clock\":{},\"passes\":{},\"run\":{},\"pass\":[{}],\"shape\":{}}}\n",
			total, self.idle, self.clock, self.passes.len(), self.run.json(&Phase::RUN), passes.join(","), self.shape_json())
	}
}
