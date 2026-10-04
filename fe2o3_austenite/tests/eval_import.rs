//! U10a: imports, includes and packages, against the host's `typst` 0.15.x.
//!
//! - The `import` corpus (`tests/fixtures/eval/import/`) holds named, renamed, wildcard and module imports,
//!   a bare import, a path that climbs from a nested file, a root-absolute path, `include`, the template
//!   pattern (`#show: doc.with(..)` from a file), data read by a relative path and strings given to `eval`.
//!   Each accepted fixture is held to Typst's values, and the template and include fixtures to the structure
//!   of Typst's realisation. Its rejected fixtures (a missing file, a cycle, a path out of the root, a
//!   backslash, an unresolved name, a file that is not UTF-8, a directory) are held to Typst's first error:
//!   message, file, line and column.
//! - The `package` corpus holds packages the area supplies itself (`packages/local/<name>/<version>`,
//!   laid out as Typst's package path is), with the entrypoints, manifests and paths of Typst's own rules.
//! - A module is evaluated once per canonical path; a diagnostic carries the kind of what failed, so a
//!   caller never switches on wording; a package is supplied by the host and never fetched.
//! - The real packages the book will import (cetz, fletcher, tablex, oxifmt) evaluate without a panic or a
//!   run that does not end.

#![allow(dead_code)]

#[path = "eval_oracle/mod.rs"]
mod harness;

use harness::austenite;
use harness::corpus::{
	discover,
	Expect,
	Filter,
	Fixture,
};
use harness::oracle::Oracle;
use harness::{
	check,
	FixtureReport,
	Verdict,
};

use oxedyne_fe2o3_austenite::diag::{
	Diagnostic,
	DiagnosticKind,
};
use oxedyne_fe2o3_austenite::eval::eval::{
	eval_string,
	EvalMode,
};
use oxedyne_fe2o3_austenite::eval::package;
use oxedyne_fe2o3_austenite::eval::scope::Scope;
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::syntax::Span;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

fn oracle() -> Outcome<Option<Oracle>> {
	let o = res!(Oracle::find());
	if o.is_none() {
		println!("[eval-import] SKIPPED: EVAL_ORACLE_SKIP=1 and no typst 0.15.x -- nothing was compared");
	}
	Ok(o)
}

fn fixtures(area: &str) -> Outcome<Vec<Fixture>> {
	discover(&Filter::only(area))
}

fn named(area: &str, name: &str) -> Outcome<Fixture> {
	let all = res!(fixtures(area));
	match all.into_iter().find(|f| f.name == name) {
		Some(f)	=> Ok(f),
		None	=> Err(err!("The fixture {}/{} is missing from tests/fixtures/eval/{}.", area, name, area; Missing)),
	}
}

// Every level a fixture asked for, and its rejection, must pass: the two corpora here are held strictly. The
// numbers compared come back so a test can insist the corpus compared something.
fn hold(area: &str, o: &Oracle, floor: usize) -> Outcome<(usize, usize)> {
	let all = res!(fixtures(area));
	assert!(all.len() >= floor, "{} fixtures under tests/fixtures/eval/{}, {} expected", all.len(), area, floor);
	let limit = harness::timeout();
	let mut bad = Vec::new();
	let (mut structure, mut errors) = (0, 0);
	for fx in &all {
		let rep: FixtureReport = res!(check(fx, o, limit));
		if let Some(f) = &rep.fault {
			bad.push(fmt!("{}: {}", rep.id, f));
		}
		for (i, v) in rep.levels.iter().enumerate() {
			if let Verdict::Fail(d) = v {
				bad.push(fmt!("{} L{}: {}", rep.id, i + 1, d.join("; ")));
			}
		}
		if let Verdict::Fail(d) = &rep.rejection {
			bad.push(fmt!("{} Err: {}", rep.id, d.join("; ")));
		}
		structure	+= rep.levels[2].is_pass() as usize;
		errors		+= rep.rejection.is_pass() as usize;
	}
	assert!(bad.is_empty(), "{} disagreement(s) with typst in {}:\n{}", bad.len(), area, bad.join("\n"));
	Ok((structure, errors))
}

#[test]
fn imports_and_includes_agree_with_the_oracle() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let (structure, errors) = res!(hold("import", &o, 28));
	// Agreement is only worth something when the corpus compared things of each kind.
	assert!(structure >= 12, "{} fixtures agreed on realised structure, 12 expected", structure);
	assert!(errors >= 16, "{} rejected fixtures agreed on the first error, 16 expected", errors);
	Ok(())
}

#[test]
fn packages_agree_with_the_oracle() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let (structure, errors) = res!(hold("package", &o, 47));
	assert!(structure >= 14, "{} package fixtures agreed on realised structure, 14 expected", structure);
	assert!(errors >= 33, "{} rejected package fixtures agreed on the first error, 33 expected", errors);
	Ok(())
}

// Values through imports, as `typst eval` serialises them: each expression carries its own `import`s, so it
// reaches the area's files the way a document does, and Austenite evaluates the same string.
const VALUES: [(&str, &str); 14] = [
	("names",		r#"import "_lib.typ": a, b as other, g; (a, other, g(1))"#),
	("star",		r#"import "_lib.typ": *; (a, b, c, f(1), g(1))"#),
	("module_alias", r#"import "_lib.typ" as m; (m.a, m.b, m.c, m.f(2), m.g(2), m.hidden)"#),
	("bare",		r#"import "_lib.typ"; (_lib.a, _lib.c)"#),
	("nested",		r#"import "sub/_inner.typ" as inner; inner.sees"#),
	("nested",		r#"import "sub/deeper/_leaf.typ": leaf; leaf"#),
	("root_abs",	r#"import "/_lib.typ": a, c; import "/sub/peer.typ": p; (a, c, p)"#),
	("dotted",		r#"import "_lib.typ": a; import "./_lib.typ": b; import "sub/_up.typ": c; (a, b, c)"#),
	("include",		r#"import "_part.typ": hidden; hidden"#),
	("read_ok",		r#"import "sub/_reader.typ": from_sub; (read("_data.txt"), from_sub)"#),
	("eval_paths",	r#"(eval("read(\"_data.txt\")"), eval("import \"_lib.typ\": a, c; (a, c)"))"#),
	("once",		r#"import "_warn.typ": w; import "./_warn.typ": w as w2; import "sub/_again.typ": w as w3; (w, w2, w3)"#),
	("names",		r#"import "_lib.typ" as m; import m: a, c as d; (a, d)"#),
	// An escape a string does not define keeps its backslash, so a package's regex pattern reads as written.
	("names",		r#"("\*", "\$", "a\qb", "\u{41}\t|", "\\")"#),
];

// What Austenite makes of an expression, evaluated from a source in the fixture's area.
fn austenite_value(fx: &Fixture, code: &str) -> Outcome<std::result::Result<harness::json::J, String>> {
	let path = fx.path.clone();
	let root = fx.root.clone();
	let code = code.to_string();
	let handle = res!(std::thread::Builder::new().stack_size(256 << 20).spawn(move || -> Outcome<std::result::Result<harness::json::J, String>> {
		austenite::supply_packages(&root);
		let mut world = World::new(root);
		let id = res!(world.load(&path));
		let mut engine = Engine::new(world);
		let span = Span::new(id, 0, 0);
		Ok(match eval_string(&mut engine, &code, EvalMode::Code, Scope::new(), span) {
			Ok(v)	=> Ok(austenite::to_json(&v)),
			Err(_)	=> Err(engine.diags.iter().find(|d| d.is_error()).map(|d| d.message.clone()).unwrap_or_default()),
		})
	}).map_err(|e| err!("Cannot start the evaluation thread: {}", e; IO)));
	match handle.join() {
		Ok(r)	=> r,
		Err(_)	=> Err(err!("The evaluation panicked."; Bug)),
	}
}

#[test]
fn values_through_imports_agree_with_typst_eval() -> Outcome<()> {
	let o = match res!(oracle()) {
		Some(o)	=> o,
		None	=> return Ok(()),
	};
	let mut bad = Vec::new();
	for (name, code) in VALUES {
		let fx = res!(named("import", name));
		let want = match res!(o.value(&fx, code)) {
			Ok(v)	=> v,
			Err(e)	=> {
				bad.push(fmt!("{}: typst rejects `{}`: {}", name, code, e));
				continue;
			}
		};
		match res!(austenite_value(&fx, code)) {
			Ok(got)	=> {
				let mut d = Vec::new();
				harness::json::diff(name, &want, &got, &mut d);
				if !d.is_empty() {
					bad.push(fmt!("{}: `{}`: {}", name, code, d.join("; ")));
				}
			}
			Err(e)	=> bad.push(fmt!("{}: `{}`: austenite failed: {}", name, code, e)),
		}
	}
	assert!(bad.is_empty(), "{} of {} expressions differ:\n{}", bad.len(), VALUES.len(), bad.join("\n"));
	Ok(())
}

// What evaluating one fixture left behind.
struct Run {
	failed:		bool,
	diags:		Vec<Diagnostic>,
	modules:	usize,
	root:		PathBuf,
	sources:	Vec<PathBuf>,	// by file id
}

impl Run {
	// The file a diagnostic is placed in, relative to the project root.
	fn file_of(&self, d: &Diagnostic) -> Option<String> {
		let path = self.sources.get(d.span.file.0 as usize)?;
		Some(path.strip_prefix(&self.root).unwrap_or(path).display().to_string())
	}

	fn first_error(&self) -> Option<&Diagnostic> { self.diags.iter().find(|d| d.is_error()) }

	fn warnings(&self, containing: &str) -> usize {
		self.diags.iter().filter(|d| !d.is_error() && d.message.contains(containing)).count()
	}
}

// Evaluates a fixture on a thread with room for the evaluator's recursion.
fn evaluate(fx: &Fixture) -> Outcome<Run> {
	let path = fx.path.clone();
	let root = fx.root.clone();
	let handle = res!(std::thread::Builder::new().stack_size(256 << 20).spawn(move || -> Outcome<Run> {
		austenite::supply_packages(&root);
		let mut world = World::new(root.clone());
		let id = res!(world.load(&path));
		let mut engine = Engine::new(world);
		let failed = eval_source(&mut engine, id).is_err();
		let sources = engine.world.sources.iter().map(|s| s.path.clone()).collect();
		Ok(Run { failed, diags: engine.diags.clone(), modules: engine.world.modules.len(), root, sources })
	}).map_err(|e| err!("Cannot start the evaluation thread: {}", e; IO)));
	match handle.join() {
		Ok(r)	=> r,
		Err(_)	=> Err(err!("The evaluation of {} panicked.", fx.id(); Bug)),
	}
}

#[test]
fn a_module_is_evaluated_once_per_canonical_path() -> Outcome<()> {
	// `once.typ` imports `_warn.typ` four times, under three spellings of one path and from a nested file,
	// and the module warns each time it is evaluated.
	let run = res!(evaluate(&res!(named("import", "once"))));
	assert!(!run.failed, "once.typ failed: {:?}", run.first_error());
	assert_eq!(run.warnings("unnecessary import rename"), 1, "the module's warning must be raised once: {:?}", run.diags);
	// `_warn.typ` and `sub/_again.typ`, the only modules the fixture reaches.
	assert_eq!(run.modules, 2, "one cache entry per module file");
	Ok(())
}

#[test]
fn a_diagnostic_carries_the_kind_of_what_failed() -> Outcome<()> {
	use DiagnosticKind as K;
	let cases = [
		("import",	"missing",			K::MissingFile,	"file not found (searched at "),
		("import",	"missing_read",		K::MissingFile,	"file not found (searched at "),
		("import",	"directory",		K::MissingFile,	"failed to load file (is a directory)"),
		("import",	"not_utf8",			K::Encoding,	"file is not valid UTF-8"),
		("import",	"unresolved",		K::UnknownVariable,	"unresolved import"),
		("import",	"escape",			K::Type,		"would escape the project root"),
		("import",	"backslash",		K::Type,		"path must not contain a backslash"),
		("package",	"missing",			K::Package,		"package not found (searched for @local/nope:0.1.0)"),
		("package",	"multi_missing",	K::Package,		"package not found (searched for @local/multi:0.3.0)"),
		("package",	"badc",				K::Package,		"package manifest is malformed"),
		("package",	"nomanifest",		K::Package,		"file not found"),
		("package",	"namemm",			K::Package,		"package manifest contains mismatched name"),
		("package",	"newc",				K::Package,		"package requires Typst 0.99.0 or newer"),
		("package",	"noentfile",		K::MissingFile,	"file not found"),
		("package",	"esc_read_missing",	K::MissingFile,	"file not found (searched at "),
	];
	let mut bad = Vec::new();
	for (area, name, kind, text) in cases {
		let run = res!(evaluate(&res!(named(area, name))));
		match run.first_error() {
			Some(d) if d.kind == kind && d.message.contains(text)	=> (),
			Some(d)	=> bad.push(fmt!("{}/{}: kind {} ({}), wanted {} ({})", area, name, d.kind, d.message, kind, text)),
			None	=> bad.push(fmt!("{}/{}: no error", area, name)),
		}
	}
	assert!(bad.is_empty(), "{} case(s) differ:\n{}", bad.len(), bad.join("\n"));
	Ok(())
}

#[test]
fn a_cycle_is_reported_where_it_closes_and_names_its_files() -> Outcome<()> {
	let want = [
		("cycle",		"_cyc_b.typ",	"_cyc_a.typ -> _cyc_b.typ -> _cyc_a.typ"),
		("self_cycle",	"self_cycle.typ", "self_cycle.typ -> self_cycle.typ"),
		("main_cycle",	"_back.typ",	"main_cycle.typ -> _back.typ -> main_cycle.typ"),
	];
	for (name, file, chain) in want {
		let run = res!(evaluate(&res!(named("import", name))));
		let d = match run.first_error() {
			Some(d)	=> d,
			None	=> return Err(err!("{} raised no error", name; Invalid)),
		};
		assert_eq!(d.message, "cyclic import", "{}", name);
		assert_eq!(d.kind, DiagnosticKind::Type, "{}", name);
		assert!(d.hints.iter().any(|h| h.ends_with(chain)), "{}: the hint must name the chain {}: {:?}", name, chain, d.hints);
		// The error is placed in the file whose import closes the cycle.
		assert_eq!(run.file_of(d).as_deref(), Some(file), "{}", name);
	}
	Ok(())
}

#[test]
fn a_chain_of_imports_is_bounded_and_a_shorter_one_is_not() -> Outcome<()> {
	let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_import_chain");
	res!(std::fs::create_dir_all(&root).map_err(|e| err!("Cannot create {}: {}", root.display(), e; IO, File, Write)));
	// `m0.typ` imports `m1.typ`, which imports `m2.typ`, and so on; the last file ends the chain.
	let n = 120;
	for i in 0..n {
		let text = if i + 1 < n { fmt!("#import \"m{}.typ\"\n#let top = {}\n", i + 1, i) } else { "#let top = 0\n".to_string() };
		res!(std::fs::write(root.join(fmt!("m{}.typ", i)), text).map_err(|e| err!("Cannot write m{}.typ: {}", i, e; IO, File, Write)));
	}
	let long = res!(run_text(&root, "main_long.typ", "#import \"m0.typ\": top\n"));
	let d = res!(long.first_error().ok_or_else(|| err!("A chain of {} imports was not bounded.", n; Invalid)));
	assert_eq!(d.kind, DiagnosticKind::Limit, "{}", d.message);
	// The same files, entered sixty files from the end, are within the bound.
	let short = res!(run_text(&root, "main_short.typ", "#import \"m60.typ\": top\n"));
	assert!(!short.failed, "a chain of {} imports failed: {:?}", n - 60, short.first_error().map(|d| d.message.clone()));
	Ok(())
}

// A package the host supplies in memory, as Daimond's origin does through the wasm surface.
fn supply_demo(version: &str, value: i64) -> Outcome<package::PackageSpec> {
	let spec = res!(package::parse_spec(&fmt!("@local/memdemo:{}", version)));
	let manifest = fmt!("[package]\nname = \"memdemo\"\nversion = \"{}\"\nentrypoint = \"src/lib.typ\"\n", version);
	let lib = fmt!("#import \"util.typ\": twice\n#let v = {}\n#let doubled = twice(v)\n", value);
	let files = vec![
		("typst.toml".to_string(), manifest.into_bytes()),
		("/src/lib.typ".to_string(), lib.into_bytes()),
		("src/util.typ".to_string(), b"#let twice(x) = x * 2\n".to_vec()),
		("data/d.txt".to_string(), b"memory data".to_vec()),
	];
	res!(package::supply(&spec, files));
	Ok(spec)
}

fn diag_message(e: &Error<ErrTag>) -> String { oxedyne_fe2o3_austenite::diag::message_of(e) }

fn run_text(root: &Path, name: &str, text: &str) -> Outcome<Run> {
	let root = root.to_path_buf();
	let name = name.to_string();
	let text = text.to_string();
	let handle = res!(std::thread::Builder::new().stack_size(256 << 20).spawn(move || -> Outcome<Run> {
		let mut world = World::new(root.clone());
		let id = res!(world.add_source(root.join(&name), text));
		let mut engine = Engine::new(world);
		let failed = eval_source(&mut engine, id).is_err();
		let sources = engine.world.sources.iter().map(|s| s.path.clone()).collect();
		Ok(Run { failed, diags: engine.diags.clone(), modules: engine.world.modules.len(), root, sources })
	}).map_err(|e| err!("Cannot start the evaluation thread: {}", e; IO)));
	match handle.join() {
		Ok(r)	=> r,
		Err(_)	=> Err(err!("The evaluation of the text panicked."; Bug)),
	}
}

#[test]
fn a_package_is_supplied_by_the_host_and_withdrawn_again() -> Outcome<()> {
	let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_import_memory");
	res!(std::fs::create_dir_all(&root).map_err(|e| err!("Cannot create {}: {}", root.display(), e; IO, File, Write)));
	let code = "#import \"@local/memdemo:1.2.3\": v, doubled\n#let r = (v, doubled)\n";
	// Nothing is supplied: the import fails with a package error and no download is attempted.
	let before = res!(run_text(&root, "before.typ", code));
	let d = res!(before.first_error().ok_or_else(|| err!("The import of an absent package succeeded."; Invalid)));
	assert_eq!(d.kind, DiagnosticKind::Package);
	assert_eq!(d.message, "package not found (searched for @local/memdemo:1.2.3)");
	assert!(d.hints.iter().any(|h| h.contains("never downloads")), "{:?}", d.hints);
	// Supplied, the entrypoint's own imports resolve inside the package, and its data reads by package path.
	let spec = res!(supply_demo("1.2.3", 21));
	assert!(res!(package::supplied()).contains(&spec));
	// A version is immutable: the same files again change nothing, different files are refused.
	res!(supply_demo("1.2.3", 21));
	let changed = supply_demo("1.2.3", 22);
	let refused = res!(changed.err().ok_or_else(|| err!("A supplied version was replaced by different files."; Invalid)));
	assert!(diag_message(&refused).contains("immutable"), "{}", diag_message(&refused));
	let after = res!(run_text(&root, "after.typ", &fmt!("{}#let d = read(\"/x\")\n", code)));
	// `read("/x")` is the project's file, absent here: the package supply must not have shadowed the project.
	let e = res!(after.first_error().ok_or_else(|| err!("The read of an absent project file succeeded."; Invalid)));
	assert_eq!(e.kind, DiagnosticKind::MissingFile, "{}", e.message);
	let ok = res!(run_text(&root, "ok.typ", code));
	assert!(!ok.failed, "the supplied package did not import: {:?}", ok.first_error());
	assert_eq!(ok.modules, 3, "the package, its entrypoint and the file the entrypoint imports are cached once each");
	// Its files are in the vfs under the package's own root, and read-only.
	let file = spec.root().join("data/d.txt");
	assert_eq!(res!(vfs::read(&file).map_err(|e| err!("{}", e; IO))), b"memory data".to_vec());
	assert!(vfs::is_file(&file) && !vfs::is_dir(&file) && vfs::is_dir(&spec.root().join("data")));
	assert_eq!(vfs::write(&file, b"x").map_err(|e| e.kind()), Err(std::io::ErrorKind::PermissionDenied),
		"a package file must be refused as read-only, not merely fail to be written");
	assert!(vfs::read(&spec.root().join("data/absent.txt")).is_err());
	// Another version is a different package.
	let other = res!(run_text(&root, "other.typ", "#import \"@local/memdemo:1.2.4\"\n"));
	let o = res!(other.first_error().ok_or_else(|| err!("An unsupplied version imported."; Invalid)));
	assert_eq!(o.kind, DiagnosticKind::Package);
	assert!(o.hints.iter().any(|h| h.contains("1.2.3")), "the hint names the version the host did supply: {:?}", o.hints);
	// Withdrawn, it is gone.
	assert!(res!(package::withdraw(&spec)));
	assert!(!res!(package::withdraw(&spec)), "a package withdrawn twice");
	let gone = res!(run_text(&root, "gone.typ", code));
	assert!(gone.failed);
	assert!(vfs::read(&file).is_err());
	// Once withdrawn, the version may be supplied afresh, with other files.
	let again = res!(supply_demo("1.2.3", 22));
	let fresh = res!(run_text(&root, "fresh.typ", "#import \"@local/memdemo:1.2.3\": v\n#assert.eq(v, 22)\n"));
	assert!(!fresh.failed, "{:?}", fresh.first_error().map(|d| d.message.clone()));
	assert!(res!(package::withdraw(&again)));
	Ok(())
}

// The examples below import a real package from Typst's own cache and use it. Austenite's library is not
// complete, so an evaluation error is reported and not failed; a panic, a run that does not end, and a
// failure to find the package or one of its own files are failures.
const REAL: [&str; 4] = ["real_oxifmt", "real_tablex", "real_fletcher", "real_cetz"];

#[test]
fn real_packages_import_without_a_panic_or_a_runaway() -> Outcome<()> {
	let cache = match std::env::var("HOME") {
		Ok(h)	=> PathBuf::from(h).join(".cache/typst/packages/preview"),
		Err(_)	=> PathBuf::new(),
	};
	if !cache.is_dir() {
		let skip = std::env::var("EVAL_ORACLE_SKIP").map(|v| v == "1").unwrap_or(false);
		assert!(skip, "no package cache at {} (set EVAL_ORACLE_SKIP=1 to skip explicitly)", cache.display());
		println!("[eval-import] SKIPPED: no package cache at {}", cache.display());
		return Ok(());
	}
	let area = corpus_dir().join("import");
	let limit = harness::timeout();
	let mut faults = Vec::new();
	for name in REAL {
		let path = area.join(fmt!("{}.typ", name));
		assert!(path.is_file(), "{} is missing", path.display());
		match austenite::eval_only(&path, &area, limit) {
			harness::austenite::RunEnd::Done(out) => match &out.eval {
				None	=> println!("[eval-import] {}: evaluated without error", name),
				Some(e)	=> {
					println!("[eval-import] {}: {}", name, e);
					for l in ["package not found", "file not found", "cyclic import", "would escape", "package manifest"] {
						if e.contains(l) {
							faults.push(fmt!("{}: the package did not load ({})", name, e));
						}
					}
				}
			},
			harness::austenite::RunEnd::Panic(m)	=> faults.push(fmt!("{}: evaluation panicked: {}", name, m)),
			harness::austenite::RunEnd::Timeout(s)	=> faults.push(fmt!("{}: evaluation ran past {}s", name, s)),
		}
	}
	assert!(faults.is_empty(), "{} fault(s):\n{}", faults.len(), faults.join("\n"));
	Ok(())
}

fn corpus_dir() -> PathBuf { harness::corpus::fixtures_dir() }

#[test]
fn the_import_corpus_stays_synthetic_and_inside_its_area() -> Outcome<()> {
	for area in ["import", "package"] {
		for fx in res!(fixtures(area)) {
			if let Some(f) = harness::corpus::provenance_fault(&fx) {
				return Err(err!("{}", f; Invalid));
			}
			assert!(fx.expect == Expect::Accepts || fx.expect == Expect::Rejects);
		}
	}
	Ok(())
}
