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
//! It reads `std::time::Instant`, which the wasm32-unknown-unknown target does not provide, so only a
//! native run switches it on.

use crate::fonts::ShapeStats;

use oxedyne_fe2o3_core::prelude::*;

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
}

impl Phase {
	pub const COUNT:	usize = 10;
	pub const RUN:		[Phase; 4] = [Phase::Load, Phase::Eval, Phase::Finish, Phase::Write];
	pub const PASS:		[Phase; 6] = [Phase::Realise, Phase::Flow, Phase::Place, Phase::Decorate, Phase::Sink, Phase::Settle];

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
		}
	}

	fn index(&self) -> usize { *self as usize }

	fn per_pass(&self) -> bool { Self::PASS.contains(self) }
}

// The time and the entries of each phase, indexed by `Phase::index`.
#[derive(Clone, Debug, Default)]
struct Bucket {
	ns:	[u64; Phase::COUNT],
	n:	[u64; Phase::COUNT],
}

impl Bucket {
	fn cell(&self, p: Phase) -> String {
		fmt!("\"{}\":{{\"ns\":{},\"n\":{}}}", p.name(), self.ns[p.index()], self.n[p.index()])
	}

	fn json(&self, phases: &[Phase]) -> String {
		let cells: Vec<String> = phases.iter().map(|p| self.cell(*p)).collect();
		fmt!("{{{}}}", cells.join(","))
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
}

impl Timings {
	/// Starts the recorder with `Load` open, since the clock runs from the moment the compile is asked for.
	pub fn start() -> Self {
		let clock = Self::clock_cost();
		let mut stack = Vec::with_capacity(64);
		stack.push(Phase::Load);
		let mut run = Bucket::default();
		run.n[Phase::Load.index()] = 1;
		Self { last: Instant::now(), stack, run, passes: Vec::with_capacity(Phase::PASS.len()), idle: 0, clock, shape: None }
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
		let passes: Vec<String> = self.passes.iter().map(|b| b.json(&Phase::PASS)).collect();
		fmt!(
			"{{\"unit\":\"ns\",\"total\":{},\"idle\":{},\"clock\":{},\"passes\":{},\"run\":{},\"pass\":[{}],\"shape\":{}}}\n",
			total, self.idle, self.clock, self.passes.len(), self.run.json(&Phase::RUN), passes.join(","), self.shape_json())
	}
}
