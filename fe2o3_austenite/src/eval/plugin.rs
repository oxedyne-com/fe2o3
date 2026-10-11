//! WebAssembly plugins, as Typst 0.15.1 runs them (`typst-library` `foundations/plugin.rs`), through the same
//! interpreter at the same pin: wasmi 1.0.9, relaxed SIMD off, so a plugin's answer is the one Typst gets.
//!
//! The protocol is `wasm_minimal_protocol`: a module imports at most the two `typst_env` host functions,
//! exports its `memory`, and every exported function becomes a member of the module `plugin()` returns.
//! Each argument is bytes, its length is what the function receives, and it fetches the bytes with
//! `wasm_minimal_protocol_write_args_to_buffer`. It hands its result back with
//! `wasm_minimal_protocol_send_result_to_host` and returns 0 for success or 1 for an error whose message is
//! the result. The messages below are Typst's, word for word.
//!
//! Typst sets no limit on a plugin. Austenite runs untrusted modules in a browser tab and a compile server,
//! so it adds four. Three are counted, so a document's outcome never depends on the machine: fuel (`FUEL` by
//! default, for a load and again for each call; `Engine::plugin_fuel` may set another), a ceiling on linear
//! memory (`MEMORY`), and wasmi's own recursion limit. Both counted limits hold while the module loads as well
//! as on a call, because loading runs the module's `start` function and allocates its declared memory.
//!
//! The fourth is timed, since no count can tell jogs' 8.5e10 fuel of bulk copies (about a second) from a
//! plain loop's (about two minutes). A call runs in slices of `SLICE` fuel through wasmi's resumable calls,
//! and between slices the clock is read against the plugin time a layout pass allows (`TIME_MS`), which
//! applies only when the host bounds a compile's loops (`Engine::fuel`, the door's loop budget): the host
//! that sets none, the command line among them, can stop a compile itself, as with Typst. A `start`
//! function cannot be resumed, so a load runs it afresh with doubling fuel and the same check between tries.

use crate::eval::scope::Scope;
use crate::eval::value::{
	Module,
	Value,
};
use crate::eval::func::Func;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;
use oxedyne_fe2o3_hash::fingerprint::Fingerprinter;

use std::fmt;
use std::sync::{
	Arc,
	Mutex,
};

use wasmi_core::LimiterError;

// Limits
// The default fuel for a load or a call, `Engine::plugin_fuel`. Typst sets none, so a document Typst compiles
// must never meet it: the most the corpus spends is 84,908,412,112, by jogs 0.2.4's `call_function`
// (classy-german-invoice, german-fx-invoice; ctxjs 0.5.0's `new_context` in starchy-junia spends
// 4,204,020,394), most of it charged for bulk memory copies, so that call takes about a second. This allows
// 11.8 times that. A plain loop spends it in 1,577 s natively (release, 2026-10-12), about 26 minutes, so
// what stops a hostile module sooner is `TIME_MS`, and this is the last resort.
pub const FUEL:			u64		= 1_000_000_000_000;
pub const SLICE:		u64		= 1_000_000_000;	// fuel between reads of the clock, about 1.5 s of a plain loop natively
#[cfg(not(target_arch = "wasm32"))]
pub const TIME_MS:		u64		= 10_000;			// plugin time a pass allows under a loop budget; jogs takes about 1 s
#[cfg(target_arch = "wasm32")]
pub const TIME_MS:		u64		= 30_000;			// an interpreter inside the tab's interpreter, on any device
#[cfg(not(target_arch = "wasm32"))]
pub const MEMORY:		usize	= 1 << 30;			// bytes of linear memory an instance may hold
#[cfg(target_arch = "wasm32")]
pub const MEMORY:		usize	= 256 << 20;		// inside the tab's own linear memory, so lower
const TABLE_ELEMS:		usize	= 1 << 20;
const MAX_TABLES:		usize	= 8;

const ENV:			&str = "typst_env";
const WRITE_ARGS:	&str = "wasm_minimal_protocol_write_args_to_buffer";
const SEND_RESULT:	&str = "wasm_minimal_protocol_send_result_to_host";

/// What a load, transition or call may spend: the fuel ceiling on each, and the plugin time the layout pass
/// has left, which `spent` carries from one to the next. [`Engine::plugin_bound`](crate::eval::Engine) makes
/// one and takes `spent` back.
#[derive(Clone, Copy, Debug)]
pub struct Bound {
	pub fuel:	u64,			// the ceiling on each load or call
	pub time:	Option<u64>,	// milliseconds a pass allows plugins, none for no bound
	pub spent:	f64,			// milliseconds they have run in this pass
}

impl Bound {
	/// The fuel ceiling alone, as the command line runs.
	pub fn fuel(fuel: u64) -> Self { Self { fuel, time: None, spent: 0.0 } }

	// Charges the time since `t0` and refuses to go on once the pass's allowance is spent.
	fn tick(&mut self, t0: &mut f64) -> Outcome<()> {
		let now = now_ms();
		self.spent += now - *t0;
		*t0 = now;
		match self.time {
			Some(t) if self.spent > t as f64	=> Err(err!(
				"plugin ran past the {} s a layout pass allows plugins when the host bounds a compile's loops, \
				so it was stopped (does it loop forever?)", t as f64 / 1000.0; LimitReached)),
			_									=> Ok(()),
		}
	}

	// The milliseconds left, none when unbounded.
	fn left(&self) -> Option<f64> { self.time.map(|t| t as f64 - self.spent) }
}

// Milliseconds on the host's clock: a monotonic `Instant` natively, `Date.now()` in a browser, where
// `Instant` panics.
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
	static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
	EPOCH.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 { js_sys::Date::now() }

// Runs `func` to its end in slices of `SLICE` fuel, reading the clock between them. The fuel each slice
// leaves unspent is carried, so the ceiling is the same `bound.fuel` an unsliced call has, and a resumed call
// computes what an uninterrupted one would.
fn run(
	store:		&mut wasmi::Store<CallData>,
	func:		&wasmi::Func,
	params:		&[wasmi::Val],
	results:	&mut [wasmi::Val],
	bound:		&mut Bound,
)
	-> Outcome<()>
{
	let mut t0 = now_ms();
	let mut left = bound.fuel;	// of the ceiling
	let mut step = SLICE.min(left);
	res!(fuel_store(store, step));
	let mut call = func.call_resumable(&mut *store, params, results);
	loop {
		let paused = match call {
			Ok(wasmi::ResumableCall::Finished)		=> return bound.tick(&mut t0),
			Ok(wasmi::ResumableCall::OutOfFuel(r))	=> r,
			Ok(wasmi::ResumableCall::HostTrap(h))	=> {
				// The protocol's host functions never trap, so this is not a pause but a failure.
				let e = h.into_host_error();
				let _ = bound.tick(&mut t0);
				return Err(limit_or(store, &e, fmt!("plugin panicked: {}", e), bound.fuel));
			}
			Err(e)									=> {
				let _ = bound.tick(&mut t0);
				return Err(limit_or(store, &e, fmt!("plugin panicked: {}", e), bound.fuel));
			}
		};
		let unspent = store.get_fuel().unwrap_or(0);
		left = left.saturating_sub(step.saturating_sub(unspent));
		res!(bound.tick(&mut t0));
		let need = paused.required_fuel();
		if need > left {
			return Err(out_of_fuel(bound.fuel));
		}
		// An instruction charged at once, such as a bulk copy, may need more than a slice.
		step = SLICE.min(left).max(need);
		res!(fuel_store(store, step));
		call = paused.resume(&mut *store, results);
	}
}

fn fuel_store(store: &mut wasmi::Store<CallData>, fuel: u64) -> Outcome<()> {
	match store.set_fuel(fuel) {
		Ok(())	=> Ok(()),
		Err(e)	=> Err(err!("cannot fuel the plugin's store: {}", e; Init)),
	}
}

fn out_of_fuel(fuel: u64) -> Error<ErrTag> {
	err!("plugin exceeded its fuel of {} instructions, so it was stopped (does it loop forever?)", fuel;
		LimitReached)
}

/// A loaded module, or one made from another by `plugin.transition`. Instances are pooled: a call takes one,
/// and gives it back only when the call succeeded, since a failed call may have left it in any state.
pub struct Plugin {
	base:		Arc<Base>,
	pool:		Mutex<Vec<Instance>>,
	snapshot:	Option<Arc<Snapshot>>,	// memory after the transition that made this plugin
	print:		Fingerprint,			// the transitions that made it, zero for a module as loaded
}

struct Base {
	id:			Fingerprint,			// of the module's bytes
	size:		usize,					// and their length
	module:		wasmi::Module,
	linker:		wasmi::Linker<CallData>,
}

struct Instance {
	instance:	wasmi::Instance,
	store:		wasmi::Store<CallData>,
}

struct Snapshot {
	pages:	u64,
	data:	Vec<u8>,
}

#[derive(Default)]
struct CallData {
	args:	Vec<Arc<Vec<u8>>>,
	output:	Vec<u8>,
	fault:	Option<Fault>,	// a host function's out-of-bounds access, reported after the call
	cap:	Cap,
}

struct Fault {
	offset:	u32,
	length:	u32,
	write:	bool,
}

/// The memory and table ceiling. A refused growth traps, so a plugin that asks for more than `MEMORY` stops
/// with a diagnostic that names the limit rather than limping on with an allocator that failed. wasmi
/// keeps `LimiterError` out of its public paths, so it is named from `wasmi_core`, at the same pin.
#[derive(Default)]
struct Cap {
	hit: bool,
}

impl wasmi::ResourceLimiter for Cap {
	fn memory_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>)
		-> std::result::Result<bool, LimiterError>
	{
		if desired > MEMORY {
			self.hit = true;
			return Err(LimiterError::ResourceLimiterDeniedAllocation);
		}
		Ok(maximum.map_or(true, |m| desired <= m))
	}

	fn table_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>)
		-> std::result::Result<bool, LimiterError>
	{
		if desired > TABLE_ELEMS {
			self.hit = true;
			return Err(LimiterError::ResourceLimiterDeniedAllocation);
		}
		Ok(maximum.map_or(true, |m| desired <= m))
	}

	fn instances(&self) -> usize	{ 1 }
	fn tables(&self) -> usize		{ MAX_TABLES }
	fn memories(&self) -> usize		{ 1 }
}

/// One exported function of a plugin, the value `p.name` is.
#[derive(Clone)]
pub struct PluginFunc {
	plugin:	Arc<Plugin>,
	name:	String,
}

impl PluginFunc {
	pub fn name(&self) -> &str { &self.name }

	/// What the function is: its module's bytes, the transitions its plugin came through, and its name.
	pub fn fingerprint(&self) -> Fingerprint {
		let mut f = Fingerprinter::new();
		f.write_fingerprint(self.plugin.base.id);
		f.write_fingerprint(self.plugin.print);
		f.write_str(&self.name);
		f.finish()
	}

	/// Runs the function on `args`, answering its result bytes or Typst's message for the failure.
	pub fn call(&self, args: Vec<Arc<Vec<u8>>>, bound: &mut Bound) -> Outcome<Vec<u8>> {
		self.plugin.call(&self.name, args, bound)
	}

	/// `plugin.transition(f, ..args)`: a new plugin whose instances start from the memory this call leaves.
	pub fn transition(&self, args: Vec<Arc<Vec<u8>>>, bound: &mut Bound) -> Outcome<Plugin> {
		self.plugin.transition(&self.name, args, bound)
	}
}

impl PartialEq for PluginFunc {
	fn eq(&self, other: &Self) -> bool { self.fingerprint() == other.fingerprint() }
}

impl fmt::Debug for PluginFunc {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "PluginFunc({})", self.name) }
}

impl fmt::Debug for Plugin {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { f.pad("Plugin(..)") }
}

impl Plugin {
	/// Loads a module from its bytes, as `plugin(source)` does: it must validate and export its memory, and
	/// one instance is made at once, so a module that cannot start fails here rather than at its first call.
	pub fn load(bytes: &[u8], bound: &mut Bound) -> Outcome<Self> {
		let mut config = wasmi::Config::default();
		config.wasm_relaxed_simd(false);	// as Typst: relaxed SIMD may differ between hosts
		config.consume_fuel(true);
		let engine = wasmi::Engine::new(&config);
		let module = match wasmi::Module::new(&engine, bytes) {
			Ok(m)	=> m,
			Err(e)	=> return Err(err!("failed to load WebAssembly module ({})", e; Input, Invalid)),
		};
		if !matches!(module.get_export("memory"), Some(wasmi::ExternType::Memory(_))) {
			return Err(err!("plugin does not export its memory"; Input, Invalid));
		}
		let mut linker = wasmi::Linker::<CallData>::new(&engine);
		if let Err(e) = linker.func_wrap(ENV, SEND_RESULT, send_result) {
			return Err(err!("cannot link {}: {}", SEND_RESULT, e; Init));
		}
		if let Err(e) = linker.func_wrap(ENV, WRITE_ARGS, write_args) {
			return Err(err!("cannot link {}: {}", WRITE_ARGS, e; Init));
		}
		let base = Arc::new(Base { id: Fingerprint::of(bytes), size: bytes.len(), module, linker });
		let first = res!(Instance::new(&base, None, bound));
		Ok(Self {
			base,
			pool:		Mutex::new(vec![first]),
			snapshot:	None,
			print:		Fingerprint::from_u128(0),
		})
	}

	/// The bytes a memo holding this plugin should count: the module, the snapshot it starts from, and the
	/// memory of the instances pooled now.
	pub fn size(&self) -> usize {
		let held = match self.pool.lock() {
			Ok(pool)	=> pool.iter().map(|i| i.memory().map(|m| m.data(&i.store).len()).unwrap_or(0)).sum(),
			Err(_)		=> 0,
		};
		self.base.size + self.snapshot.as_ref().map_or(0, |s| s.data.len()) + held
	}

	/// The module `plugin()` returns: every exported function, by name.
	pub fn into_module(this: Arc<Self>) -> Module {
		let mut scope = Scope::new();
		for export in this.base.module.exports() {
			if matches!(export.ty(), wasmi::ExternType::Func(_)) {
				let func = PluginFunc { plugin: this.clone(), name: export.name().to_string() };
				scope.define(export.name(), Value::Func(Func::Plugin(Arc::new(func))));
			}
		}
		Module::new("", scope)
	}

	fn call(&self, name: &str, args: Vec<Arc<Vec<u8>>>, bound: &mut Bound) -> Outcome<Vec<u8>> {
		let mut inst = res!(self.acquire(bound));
		// A failed instance is dropped, never pooled again.
		let out = res!(inst.call(name, args, bound));
		let mut pool = lock_mutex!(self.pool);
		pool.push(inst);
		Ok(out)
	}

	fn transition(&self, name: &str, args: Vec<Arc<Vec<u8>>>, bound: &mut Bound) -> Outcome<Self> {
		let mut f = Fingerprinter::new();
		f.write_fingerprint(self.print);
		f.write_str(name);
		f.write_usize(args.len());
		for a in &args {
			f.write_usize(a.len());
			f.write_fingerprint(Fingerprint::of(a));
		}
		let print = f.finish();
		let mut inst = res!(self.acquire(bound));
		res!(inst.call(name, args, bound));
		let snapshot = Arc::new(res!(inst.snapshot()));
		// The instance that made the change moves into the new plugin, so the old one never sees it.
		Ok(Self {
			base:		self.base.clone(),
			pool:		Mutex::new(vec![inst]),
			snapshot:	Some(snapshot),
			print,
		})
	}

	fn acquire(&self, bound: &mut Bound) -> Outcome<Instance> {
		let held = {
			let mut pool = lock_mutex!(self.pool);
			pool.pop()
		};
		match held {
			Some(i)	=> Ok(i),
			None	=> Instance::new(&self.base, self.snapshot.as_deref(), bound),
		}
	}
}

impl Instance {
	fn new(base: &Base, snapshot: Option<&Snapshot>, bound: &mut Bound) -> Outcome<Self> {
		// wasmi cannot resume a `start` function, so a load that runs out of a try's fuel begins again in a
		// fresh store with twice the fuel, no more than the time left allows, reading the clock between tries.
		// A start that finishes costs at most twice its fuel; one that loops stops at the ceiling or the time.
		let mut t0 = now_ms();
		let mut fuel = SLICE.min(bound.fuel);
		loop {
			let tried = now_ms();
			let mut store = wasmi::Store::new(base.linker.engine(), CallData::default());
			store.limiter(|d: &mut CallData| &mut d.cap as &mut dyn wasmi::ResourceLimiter);
			// The fuel is set before the start function runs, so a module that loops in `start` stops too.
			res!(fuel_store(&mut store, fuel));
			let e = match base.linker.instantiate_and_start(&mut store, &base.module) {
				Ok(instance)	=> {
					res!(bound.tick(&mut t0));
					let mut out = Self { instance, store };
					if let Some(s) = snapshot {
						res!(out.restore(s));
					}
					return Ok(out);
				}
				Err(e)			=> e,
			};
			if e.as_trap_code() != Some(wasmi::TrapCode::OutOfFuel) || fuel >= bound.fuel {
				let _ = bound.tick(&mut t0);
				return Err(limit_or(&store, &e, fmt!("{}", e), bound.fuel));
			}
			let took = now_ms() - tried;
			res!(bound.tick(&mut t0));
			let mut next = fuel.saturating_mul(2).min(bound.fuel);
			if let Some(ms) = bound.left() {
				// At the rate the last try ran, the most the time left can spend.
				let rate = fuel as f64 / took.max(1.0);
				next = next.min(((rate * ms) as u64).max(SLICE));
			}
			fuel = next;
		}
	}

	fn memory(&self) -> Outcome<wasmi::Memory> {
		match self.instance.get_memory(&self.store, "memory") {
			Some(m)	=> Ok(m),
			None	=> Err(err!("plugin does not export its memory"; Input, Invalid)),
		}
	}

	fn call(&mut self, name: &str, args: Vec<Arc<Vec<u8>>>, bound: &mut Bound) -> Outcome<Vec<u8>> {
		let handle = match self.instance.get_func(&self.store, name) {
			Some(f)	=> f,
			None	=> return Err(err!("plugin has no function `{}`", name; Input, Invalid)),
		};
		let ty = handle.ty(&self.store);
		// The signature is checked at the call, not at the load, since a module may export functions such
		// as `_initialize` that do not follow the protocol.
		if ty.params().iter().any(|v| *v != wasmi::ValType::I32) {
			return Err(err!("plugin function `{}` has a parameter that is not a 32-bit integer", name;
				Input, Invalid));
		}
		if ty.results() != [wasmi::ValType::I32] {
			return Err(err!("plugin function `{}` does not return exactly one 32-bit integer", name;
				Input, Invalid));
		}
		let expected = ty.params().len();
		let given = args.len();
		if expected != given {
			return Err(err!("plugin function takes {} argument{}, but {} {} given",
				expected, if expected == 1 { "" } else { "s" },
				given, if given == 1 { "was" } else { "were" }; Input, Invalid));
		}
		let lengths: Vec<wasmi::Val> = args.iter().map(|a| wasmi::Val::I32(a.len() as i32)).collect();
		{
			let data = self.store.data_mut();
			data.args	= args;
			data.output	= Vec::new();
			data.fault	= None;
		}
		let mut code = [wasmi::Val::I32(-1)];
		res!(run(&mut self.store, &handle, &lengths, &mut code, bound));
		let data = self.store.data_mut();
		if let Some(Fault { offset, length, write }) = data.fault.take() {
			let kind = if write { "write" } else { "read" };
			return Err(err!("plugin tried to {} out of bounds: pointer {:#x} is out of bounds for {} of length {}",
				kind, offset, kind, length; Input, Invalid));
		}
		let output = std::mem::take(&mut data.output);
		match code[0] {
			wasmi::Val::I32(0)	=> Ok(output),
			wasmi::Val::I32(1)	=> match std::str::from_utf8(&output) {
				Ok(m)	=> Err(err!("plugin errored with: {}", m; Input, Invalid)),
				Err(_)	=> Err(err!("plugin errored, but did not return a valid error message"; Input, Invalid)),
			},
			_					=> Err(err!("plugin did not respect the protocol"; Input, Invalid)),
		}
	}

	fn snapshot(&self) -> Outcome<Snapshot> {
		let mem = res!(self.memory());
		Ok(Snapshot { pages: mem.size(&self.store), data: mem.data(&self.store).to_vec() })
	}

	fn restore(&mut self, s: &Snapshot) -> Outcome<()> {
		let mem = res!(self.memory());
		let now = mem.size(&self.store);
		if now < s.pages {
			if let Err(e) = mem.grow(&mut self.store, s.pages - now) {
				return Err(err!("cannot restore the plugin's memory: {}", e; LimitReached));
			}
		}
		let data = mem.data_mut(&mut self.store);
		match data.get_mut(..s.data.len()) {
			Some(d)	=> { d.copy_from_slice(&s.data); Ok(()) }
			None	=> Err(err!("cannot restore the plugin's memory: {} bytes into {}", s.data.len(), data.len();
				Bug)),
		}
	}
}

// A failure that one of Austenite's limits caused is named as such; any other keeps the message given.
fn limit_or(store: &wasmi::Store<CallData>, e: &wasmi::Error, msg: String, fuel: u64) -> Error<ErrTag> {
	if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) {
		return out_of_fuel(fuel);
	}
	if store.data().cap.hit {
		return err!("plugin exceeded its memory limit of {} MiB", MEMORY >> 20; LimitReached);
	}
	err!("{}", msg; Input, Invalid)
}

fn write_args(mut caller: wasmi::Caller<'_, CallData>, ptr: u32) {
	let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
		Some(m)	=> m,
		None	=> return,
	};
	let args = std::mem::take(&mut caller.data_mut().args);
	let mut at = ptr as usize;
	for a in args {
		if mem.write(&mut caller, at, a.as_slice()).is_err() {
			caller.data_mut().fault = Some(Fault { offset: at as u32, length: a.len() as u32, write: true });
			return;
		}
		at += a.len();
	}
}

fn send_result(mut caller: wasmi::Caller<'_, CallData>, ptr: u32, len: u32) {
	let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
		Some(m)	=> m,
		None	=> return,
	};
	// The bytes are read where they lie, so a length past the memory allocates nothing on the host.
	let (lo, hi) = (ptr as usize, ptr as usize + len as usize);
	let got = mem.data(&caller).get(lo..hi).map(|b| b.to_vec());
	let buf = match got {
		Some(b)	=> b,
		None	=> {
			caller.data_mut().fault = Some(Fault { offset: ptr, length: len, write: false });
			return;
		}
	};
	caller.data_mut().output = buf;
}
