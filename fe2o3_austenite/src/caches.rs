//! The caches a [`Session`](crate::compile::Session) keeps from one compile to the next, and what scopes
//! them.
//!
//! A cache here is keyed on content, so an entry made by a compile that went on to fail is as good as one
//! from a compile that finished, and a hit never depends on which compile made the entry. What a content
//! key cannot name is the configuration the compile ran under: the face set, `sys.inputs` and the engine.
//! A compile folds those into one [`configuration`] fingerprint, and when it differs from the one the
//! entries were made under every cache empties. Nothing is keyed across engines: the caches live in the
//! session, and the engine's identity is in the fingerprint.
//!
//! The session lends its [`Caches`] to the [`Engine`](crate::eval::Engine) for the length of a compile and
//! takes them back. The paragraph memo and the page ledger each add a [`GenMap`](oxedyne_fe2o3_data::gen_map::GenMap)
//! here, stepped by [`Caches::begin`], swept by [`Caches::end`] and emptied by [`Caches::clear`].

use crate::eval::eval::library;
use crate::eval::value::Value;
use crate::fonts::ShapeStats;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;
use oxedyne_fe2o3_hash::fingerprint::Fingerprinter;

pub const PAR_BUDGET:	usize = 16 << 20;	// bytes of laid paragraphs a session keeps by default
pub const SHAPE_BUDGET:	usize = 32 << 20;	// bytes of shaped runs a session keeps by default

// The largest whole number JavaScript holds exactly.
const JS_WHOLE: f64 = 9_007_199_254_740_991.0;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BUDGETS                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// The bytes each cache may hold, set by the host. A cache that reaches its budget stops taking entries and
/// loses none, so a lower budget slows a compile and never changes what it makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
	pub shapes:	usize,
	pub pars:	usize,
	pub ledger:	usize,	// the ledger's share is set with the ledger
}

impl Default for Budgets {
	fn default() -> Self {
		Self { shapes: SHAPE_BUDGET, pars: PAR_BUDGET, ledger: 0 }
	}
}

impl Budgets {
	/// These budgets with each of `shapes`, `pars` and `ledger` replaced by the number of bytes given, as a
	/// host that counts in JavaScript numbers gives it; a field left `None` stands. A number that is not a
	/// whole count of bytes this target can hold refuses the lot, naming the field, so a host never
	/// half-applies a change.
	pub fn with(self, shapes: Option<f64>, pars: Option<f64>, ledger: Option<f64>) -> Outcome<Self> {
		let mut out = self;
		for (name, given, slot) in [
			("shapes",	shapes,	&mut out.shapes),
			("pars",	pars,	&mut out.pars),
			("ledger",	ledger,	&mut out.ledger),
		] {
			if let Some(n) = given {
				*slot = res!(whole_bytes(name, n));
			}
		}
		Ok(out)
	}
}

fn whole_bytes(name: &str, n: f64) -> Outcome<usize> {
	if !(n >= 0.0 && n.fract() == 0.0 && n <= JS_WHOLE) {
		return Err(err!("The {} budget is {}, which is not a whole number of bytes.", name, n; Input, Invalid));
	}
	match usize::try_from(n as u64) {
		Ok(b)	=> Ok(b),
		Err(_)	=> Err(err!("The {} budget of {} bytes is more than this target can address.", name, n; Input, Invalid)),
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COUNTERS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// What a compile's caches did, read off its result. The paragraph and page counts are filled by the units
/// that own those caches and are zero until then; the shape stats are the shaped-run cache's, as the
/// compile left it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
	pub replayed:		u64,	// pages replayed from the ledger
	pub relaid:			u64,	// pages laid again
	pub par_hits:		u64,
	pub par_misses:		u64,
	pub shapes:			ShapeStats,
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CONFIGURATION                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// The fingerprint of everything outside the source that decides what a compile makes: the face set
/// (`faces`, [`FontBook::fingerprint`](crate::fonts::FontBook::fingerprint)), the `sys.inputs` pairs, and
/// the engine, by the commit it was built from. The inputs go in sorted, so their order is not a change.
pub fn configuration(faces: Fingerprint, inputs: &[(String, String)], engine: &str) -> Fingerprint {
	let mut sorted: Vec<&(String, String)> = inputs.iter().collect();
	sorted.sort();
	let mut f = Fingerprinter::new();
	f.write_fingerprint(faces);
	f.write_usize(sorted.len());
	for (key, val) in sorted {
		f.write_str(key);
		f.write_str(val);
	}
	f.write_str(engine);
	f.finish()
}

/// The pairs of the `sys.inputs` the evaluator presents to a document. A host has no route to set them
/// yet, so the dictionary is empty; reading it from the library keeps the fingerprint on whatever the
/// document would see.
pub fn sys_inputs() -> Vec<(String, String)> {
	let lib = library();
	let sys = match lib.get("sys") {
		Some(Value::Module(m))	=> m.clone(),
		_						=> return Vec::new(),
	};
	let dict = match sys.scope.get("inputs") {
		Some(Value::Dict(d))	=> d.clone(),
		_						=> return Vec::new(),
	};
	dict.iter().map(|(k, v)| {
		let text = match v {
			Value::Str(s)	=> s.as_str().to_string(),
			other			=> fmt!("{:?}", other),
		};
		(k.to_string(), text)
	}).collect()
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CACHES                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// The session's caches. Its entries (the paragraph memo and the page ledger add theirs) are all scoped by
/// one configuration fingerprint and swept by generation.
#[derive(Debug, Default)]
pub struct Caches {
	pub budgets:	Budgets,
	pub counters:	Counters,			// this compile's
	config:			Option<Fingerprint>,	// what the entries were made under, none before the first compile
	resets:			u64,				// how many times a changed configuration emptied them
}

impl Caches {
	pub fn new(budgets: Budgets) -> Self {
		Self { budgets, ..Self::default() }
	}

	/// Opens a compile under `config`: the counters start from zero, every cache's generation steps, and a
	/// configuration that is not the one the entries were made under empties them first. Answers whether
	/// it emptied them. The first compile has no entries to empty and does not count as a reset.
	pub fn begin(&mut self, config: Fingerprint) -> bool {
		let moved = match self.config {
			Some(held)	=> held != config,
			None		=> false,
		};
		if moved {
			self.clear();
			self.resets += 1;
		}
		self.config		= Some(config);
		self.counters	= Counters::default();
		moved
	}

	/// Closes the compile: each cache drops what neither this compile nor the one before touched.
	pub fn end(&mut self) {}

	/// Empties every cache. A panic, which leaves entries of unknown worth, drops the whole session instead.
	pub fn clear(&mut self) {}

	/// The configuration the entries were made under.
	pub fn config(&self) -> Option<Fingerprint> { self.config }

	/// How many times a changed configuration has emptied the caches.
	pub fn resets(&self) -> u64 { self.resets }
}
