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
#[cfg(feature = "plugin")]
use crate::eval::plugin::{
	Plugin,
	PluginFunc,
};
use crate::eval::value::Value;
use crate::flow::par::ParMemo;
use crate::fonts::ShapeStats;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;
use oxedyne_fe2o3_hash::fingerprint::Fingerprinter;
#[cfg(feature = "plugin")]
use oxedyne_fe2o3_data::gen_map::GenMap;

#[cfg(feature = "plugin")]
use std::sync::Arc;

pub const PAR_BUDGET:	usize = 16 << 20;	// bytes of laid paragraphs a session keeps by default
pub const SHAPE_BUDGET:	usize = 32 << 20;	// bytes of shaped runs a session keeps by default
pub const PLUGIN_BUDGET:	usize = 64 << 20;	// bytes of loaded plugins and their results a session keeps

// The largest whole number JavaScript holds exactly.
const JS_WHOLE: f64 = 9_007_199_254_740_991.0;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BUDGETS                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// The bytes each cache may hold, set by the host. The shaped-run cache at its budget evicts its least recently
/// used result; a cache swept by generation refuses a new entry and loses none. A lower budget slows a compile
/// and never changes what it makes.
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

/// What a compile's caches did, read off its result. Every paragraph laid counts as a hit or a miss of the
/// paragraph memo, one that is not kept (it holds an inline box or equation) as a miss; the page counts are
/// filled by the unit that owns the ledger and are zero until then; the shape stats are the shaped-run
/// cache's, as the compile left it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
	pub replayed:		u64,	// pages replayed from the ledger
	pub relaid:			u64,	// pages laid again
	pub par_hits:		u64,
	pub par_misses:		u64,
	pub plugin_hits:	u64,	// loads, transitions and calls answered from the memo
	pub plugin_misses:	u64,	// and those run
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

/// The session's caches. Its entries (the paragraph memo, and the page ledger when it comes) are all scoped by
/// one configuration fingerprint and swept by generation.
#[derive(Debug, Default)]
pub struct Caches {
	pub budgets:	Budgets,
	pub counters:	Counters,			// this compile's
	pub pars:		ParMemo,			// the paragraphs broken into lines
	#[cfg(feature = "plugin")]
	pub plugins:	PluginMemo,			// plugins loaded and transitioned, and their calls' results
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
		self.pars.begin(self.budgets.pars);
		#[cfg(feature = "plugin")]
		self.plugins.begin();
		moved
	}

	/// Closes the compile: each cache drops what neither this compile nor the one before touched.
	pub fn end(&mut self) {
		self.pars.sweep();
		#[cfg(feature = "plugin")]
		self.plugins.sweep();
	}

	/// Empties every cache. A panic, which leaves entries of unknown worth, drops the whole session instead.
	pub fn clear(&mut self) {
		self.pars.clear();
		#[cfg(feature = "plugin")]
		self.plugins.clear();
	}

	/// The configuration the entries were made under.
	pub fn config(&self) -> Option<Fingerprint> { self.config }

	/// How many times a changed configuration has emptied the caches.
	pub fn resets(&self) -> u64 { self.resets }
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PLUGINS                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Plugins memoised as Typst memoises them: a load by the module's bytes, and a call or a transition by the
/// function's fingerprint, which names the module, the transitions its plugin came through (FIX-9) and the
/// function, together with the bytes of every argument. A plugin is deterministic, so a hit is the answer
/// the call would give. Failures are not kept; they run again, under the same limits.
#[cfg(feature = "plugin")]
#[derive(Debug)]
pub struct PluginMemo {
	loads:	GenMap<Fingerprint, Arc<Plugin>>,
	steps:	GenMap<Fingerprint, Arc<Plugin>>,	// transitions
	calls:	GenMap<Fingerprint, Arc<Vec<u8>>>,
}

#[cfg(feature = "plugin")]
impl Default for PluginMemo {
	fn default() -> Self {
		Self {
			loads:	GenMap::new(PLUGIN_BUDGET),
			steps:	GenMap::new(PLUGIN_BUDGET),
			calls:	GenMap::new(PLUGIN_BUDGET),
		}
	}
}

#[cfg(feature = "plugin")]
impl PluginMemo {
	fn begin(&mut self) {
		self.loads.begin();
		self.steps.begin();
		self.calls.begin();
	}

	fn sweep(&mut self) {
		self.loads.sweep();
		self.steps.sweep();
		self.calls.sweep();
	}

	fn clear(&mut self) {
		self.loads.clear();
		self.steps.clear();
		self.calls.clear();
	}

	/// The entries held: loads, transitions and calls.
	pub fn len(&self) -> (usize, usize, usize) { (self.loads.len(), self.steps.len(), self.calls.len()) }
}

// A call or transition: the function, then each argument's length and bytes.
#[cfg(feature = "plugin")]
fn plugin_key(tag: u8, f: &PluginFunc, args: &[Arc<Vec<u8>>]) -> Fingerprint {
	let mut h = Fingerprinter::new();
	h.write_u8(tag);
	h.write_fingerprint(f.fingerprint());
	h.write_usize(args.len());
	for a in args {
		h.write_usize(a.len());
		h.write_fingerprint(Fingerprint::of(a));
	}
	h.finish()
}

#[cfg(feature = "plugin")]
impl Caches {
	/// `plugin(source)`, memoised on the module's bytes.
	pub fn plugin_load(&mut self, bytes: &[u8]) -> Outcome<Arc<Plugin>> {
		let key = Fingerprint::of(bytes);
		if let Some(p) = self.plugins.loads.get(&key) {
			self.counters.plugin_hits += 1;
			return Ok(p.clone());
		}
		self.counters.plugin_misses += 1;
		let p = Arc::new(res!(Plugin::load(bytes)));
		let size = p.size();
		self.plugins.loads.insert(key, p.clone(), size);
		Ok(p)
	}

	/// `plugin.transition(f, ..args)`, memoised on the function and the arguments' bytes.
	pub fn plugin_transition(&mut self, f: &PluginFunc, args: Vec<Arc<Vec<u8>>>) -> Outcome<Arc<Plugin>> {
		let key = plugin_key(b't', f, &args);
		if let Some(p) = self.plugins.steps.get(&key) {
			self.counters.plugin_hits += 1;
			return Ok(p.clone());
		}
		self.counters.plugin_misses += 1;
		let p = Arc::new(res!(f.transition(args)));
		let size = p.size();
		self.plugins.steps.insert(key, p.clone(), size);
		Ok(p)
	}

	/// `f(..args)` for a plugin function, memoised on the function and the arguments' bytes.
	pub fn plugin_call(&mut self, f: &PluginFunc, args: Vec<Arc<Vec<u8>>>) -> Outcome<Arc<Vec<u8>>> {
		let key = plugin_key(b'c', f, &args);
		if let Some(out) = self.plugins.calls.get(&key) {
			self.counters.plugin_hits += 1;
			return Ok(out.clone());
		}
		self.counters.plugin_misses += 1;
		let out = Arc::new(res!(f.call(args)));
		self.plugins.calls.insert(key, out.clone(), out.len() + std::mem::size_of::<Vec<u8>>());
		Ok(out)
	}
}
