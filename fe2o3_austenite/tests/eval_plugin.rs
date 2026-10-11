//! Austenite's limits on a WebAssembly plugin, each met by a hostile module from `tools/plugin/mkwasm.py`:
//! a loop in a call and in the module's `start`, which fuel stops, and a memory that grows without end or
//! is declared past the ceiling, which the limiter stops. Typst sets no such limits, so the oracle cannot
//! check these; each must end in a `Limit` diagnostic that names the limit, and never hang or abort.
//! `grow` really allocates up to the ceiling, so run this file under `rc-build 2G`. The memo of loads,
//! transitions and calls (W2) is checked over three compiles of one session.

#![cfg(feature = "plugin")]

use oxedyne_fe2o3_austenite::caches::Counters;
use oxedyne_fe2o3_austenite::compile::Session;
use oxedyne_fe2o3_austenite::diag::DiagnosticKind;
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::eval::eval::{
	eval_string,
	EvalMode,
};
use oxedyne_fe2o3_austenite::eval::plugin::{
	FUEL,
	MEMORY,
};
use oxedyne_fe2o3_austenite::eval::scope::Scope;
use oxedyne_fe2o3_austenite::eval::{
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};
use std::time::{
	Duration,
	Instant,
};

// Evaluates `code` from a source beside the fixture modules, answering the first error's kind and message.
fn first_error(code: &str) -> Outcome<(DiagnosticKind, String)> {
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/plugin");
	let mut engine = Engine::new(World::new(dir.clone()));
	let id = res!(engine.world.add_source(dir.join("case.typ"), String::new()));
	let _ = eval_string(&mut engine, code, EvalMode::Code, Scope::new(), Span::new(id, 0, 0));
	match engine.diags.iter().find(|d| d.is_error()) {
		Some(d)	=> Ok((d.kind, d.message.clone())),
		None	=> Err(err!("{}: no error raised", code; Missing)),
	}
}

fn assert_limit(code: &str, word: &str) -> Outcome<()> {
	let t = Instant::now();
	let (kind, msg) = res!(first_error(code));
	assert_eq!(kind, DiagnosticKind::Limit, "{}: kind {} ({})", code, kind, msg);
	assert!(msg.contains(word), "{}: '{}' does not name the {}", code, msg, word);
	// Fuel is counted, not timed, but a limit that took this long would not protect a compile.
	assert!(t.elapsed() < Duration::from_secs(60), "{}: took {:?}", code, t.elapsed());
	Ok(())
}

#[test]
fn a_call_that_loops_forever_runs_out_of_fuel() -> Outcome<()> {
	assert_limit("plugin(\"spin.wasm\").spin()", "fuel")
}

#[test]
fn a_start_function_that_loops_forever_runs_out_of_fuel_while_loading() -> Outcome<()> {
	// FIX-10: the load runs `start`, so the fuel must already be set.
	assert_limit("plugin(\"spin_start.wasm\")", "fuel")
}

#[test]
fn a_memory_that_grows_without_end_stops_at_the_ceiling() -> Outcome<()> {
	assert_limit("plugin(\"grow.wasm\").grow()", "memory limit")
}

#[test]
fn a_declared_memory_past_the_ceiling_fails_the_load() -> Outcome<()> {
	// 32768 pages, 2 GiB, past the ceiling on every target.
	assert!(32768usize << 16 > MEMORY);
	assert_limit("plugin(\"huge_initial.wasm\")", "memory limit")
}

#[test]
fn the_limits_are_named_in_the_message() -> Outcome<()> {
	let (_, msg) = res!(first_error("plugin(\"spin.wasm\").spin()"));
	assert!(msg.contains(&FUEL.to_string()), "'{}' does not give the fuel figure {}", msg, FUEL);
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MEMO                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

fn fixture(name: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval/plugin").join(name)
}

// Two loads, a transition and three calls, one of them on the transitioned plugin; `arg` is the echo's.
fn memo_doc(arg: &str) -> String {
	fmt!("#let p = plugin(\"echo.wasm\")\n#let c = plugin(\"counter.wasm\")\n#let one = plugin.transition(c.inc)\n\
		#str(p.echo(bytes(\"{}\"))) #array(c.get()).at(0) #array(one.get()).at(0)\n", arg)
}

fn build(session: &mut Session, dir: &Path, text: &str) -> Outcome<(Vec<u8>, Counters)> {
	let main = dir.join("main.typ");
	res!(std::fs::write(&main, text).map_err(|e| err!("Cannot write {}: {}", main.display(), e; IO, File, Write)));
	let mut sink = res!(PdfSink::new());
	let done = res!(session.compile(&main, dir, &mut sink, None, None, false));
	if let Err(e) = &done.laid {
		return Err(err!("the compile failed: {}", e.plain(); Invalid));
	}
	let pdf = res!(sink.output().ok_or_else(|| err!("the compile finished no PDF"; Missing)));
	Ok((pdf.to_vec(), done.counters))
}

#[test]
fn loads_transitions_and_calls_are_memoised_across_compiles() -> Outcome<()> {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_plugin_memo");
	let _ = std::fs::remove_dir_all(&dir);
	res!(std::fs::create_dir_all(&dir).map_err(|e| err!("Cannot create {}: {}", dir.display(), e; IO, File, Write)));
	for m in ["echo.wasm", "counter.wasm"] {
		res!(std::fs::copy(fixture(m), dir.join(m)).map_err(|e| err!("Cannot copy {}: {}", m, e; IO, File, Write)));
	}
	let mut s = Session::new(FontStore::default());
	let (first, c1) = res!(build(&mut s, &dir, &memo_doc("one")));
	assert_eq!((c1.plugin_hits, c1.plugin_misses), (0, 6), "a first compile runs everything: {:?}", c1);
	let (second, c2) = res!(build(&mut s, &dir, &memo_doc("one")));
	assert_eq!((c2.plugin_hits, c2.plugin_misses), (6, 0), "an unchanged recompile runs nothing: {:?}", c2);
	assert!(first == second, "the recompile made different bytes");
	// Only the call whose argument changed runs again.
	let (_, c3) = res!(build(&mut s, &dir, &memo_doc("two")));
	assert_eq!((c3.plugin_hits, c3.plugin_misses), (5, 1), "an edited argument ran more than its call: {:?}", c3);
	assert_eq!(s.caches().plugins.len(), (2, 1, 4), "two loads, one transition, four calls held");
	Ok(())
}
