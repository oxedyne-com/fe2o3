//! `sys.inputs` is the dictionary of strings a host gives a compile, as Typst's `--input key=value` gives
//! it (D-20261018-03). A [`Session`] carries the inputs; each compile's library presents them, so two compiles
//! with different inputs each see their own, and a change of inputs moves the caches' configuration. Each
//! source asserts what it should see, and a failed assertion fails the compile.

use oxedyne_fe2o3_austenite::caches;
use oxedyne_fe2o3_austenite::compile::{
	self,
	Session,
};
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::watch;

use std::path::PathBuf;

fn dir(name: &str) -> PathBuf {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sys_inputs").join(name);
	let _ = std::fs::remove_dir_all(&d);
	std::fs::create_dir_all(&d).expect("the case's directory");
	std::fs::canonicalize(&d).expect("the case's canonical path")
}

fn pairs(kv: &[(&str, &str)]) -> Vec<(String, String)> {
	kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

// Compiles `src` under `session`, giving the error messages the compile recorded, empty when it laid cleanly.
fn errors(session: &mut Session, name: &str, src: &str) -> Vec<String> {
	let d = dir(name);
	let main = d.join("main.typ");
	std::fs::write(&main, src).expect("the case's source");
	let mut sink = PdfSink::new().expect("a PDF sink");
	let done = session.compile(&main, &d, &mut sink, None, None, true).expect("the main is readable");
	let mut out: Vec<String> = done.engine.diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
	if let Err(e) = &done.laid {
		out.push(e.plain());
	}
	out
}

fn with(kv: &[(&str, &str)]) -> Session {
	let mut s = Session::new(FontStore::default());
	s.set_inputs(pairs(kv));
	s
}

#[test]
fn no_input_gives_an_empty_dictionary() {
	let mut s = Session::new(FontStore::default());
	let e = errors(&mut s, "none", "#assert.eq(type(sys.inputs), dictionary)\n#assert.eq(sys.inputs, (:))\n\
		#assert.eq(sys.inputs.len(), 0)\nText.\n");
	assert!(e.is_empty(), "{:?}", e);
}

#[test]
fn one_input_is_a_string_under_its_key() {
	let mut s = with(&[("engine", "austenite")]);
	let e = errors(&mut s, "one", "#assert.eq(sys.inputs, (engine: \"austenite\"))\n\
		#assert.eq(type(sys.inputs.engine), str)\nText.\n");
	assert!(e.is_empty(), "{:?}", e);
}

#[test]
fn several_inputs_keep_their_order_and_a_repeated_key_its_last_value() {
	// As Typst's dictionary collects them: the first place a key took, the last value given it.
	let mut s = with(&[("a", "1"), ("b", "2"), ("a", "3")]);
	let e = errors(&mut s, "several", "#assert.eq(sys.inputs.keys(), (\"a\", \"b\"))\n\
		#assert.eq(sys.inputs.values(), (\"3\", \"2\"))\n#assert.eq(repr(sys.inputs), \"(a: \\\"3\\\", b: \\\"2\\\")\")\nText.\n");
	assert!(e.is_empty(), "{:?}", e);
}

#[test]
fn at_with_a_default_reads_a_given_key_and_falls_back_for_a_missing_one() {
	let src = "#let e = sys.inputs.at(\"engine\", default: none)\n#assert.eq(e, EXPECT)\nText.\n";
	let mut s = with(&[("engine", "austenite")]);
	let e = errors(&mut s, "at_given", &src.replace("EXPECT", "\"austenite\""));
	assert!(e.is_empty(), "{:?}", e);
	let mut s = Session::new(FontStore::default());
	let e = errors(&mut s, "at_missing", &src.replace("EXPECT", "none"));
	assert!(e.is_empty(), "{:?}", e);
}

#[test]
fn a_wrong_expectation_fails_the_compile() {
	// The control: a success above means the document saw what it asserted.
	let mut s = with(&[("engine", "typst")]);
	let e = errors(&mut s, "control", "#assert.eq(sys.inputs.engine, \"austenite\")\nText.\n");
	assert!(!e.is_empty(), "a document that sees another engine fails");
}

#[test]
fn sys_version_is_unchanged_by_inputs() {
	let mut s = with(&[("engine", "austenite")]);
	let e = errors(&mut s, "version", "#assert.eq(sys.version, version(0, 15, 1))\nText.\n");
	assert!(e.is_empty(), "{:?}", e);
}

#[test]
fn a_session_sees_its_new_inputs_and_they_move_the_configuration() {
	let src = "#assert.eq(sys.inputs.at(\"engine\", default: none), EXPECT)\nText.\n";
	let mut s = with(&[("engine", "typst")]);
	assert!(errors(&mut s, "first", &src.replace("EXPECT", "\"typst\"")).is_empty());
	let first = s.caches().config().expect("a compile sets the configuration");
	let faces = s.fonts().clone().book().expect("a book").fingerprint();
	assert_eq!(first, caches::configuration(faces, s.inputs(), compile::engine_git_hash()));
	s.set_inputs(pairs(&[("engine", "austenite")]));
	assert!(errors(&mut s, "second", &src.replace("EXPECT", "\"austenite\"")).is_empty());
	assert_ne!(s.caches().config(), Some(first), "new inputs move the configuration");
	// A fresh session in the same thread is not left the last session's inputs.
	let mut s = Session::new(FontStore::default());
	assert!(errors(&mut s, "third", &src.replace("EXPECT", "none")).is_empty());
}

#[test]
fn an_input_argument_splits_on_its_first_equals_sign_as_typst_does() {
	let ok = |a: &str| compile::parse_input(a).expect("a well-formed input");
	assert_eq!(ok("engine=austenite"),	("engine".to_string(), "austenite".to_string()));
	assert_eq!(ok("k=v=w"),				("k".to_string(), "v=w".to_string()));
	assert_eq!(ok(" k = v=w "),			("k".to_string(), "v=w".to_string()));
	assert_eq!(ok("k="),				("k".to_string(), String::new()));
	for (arg, why) in [("=x", "key"), (" =x", "key"), ("noeq", "equals sign"), ("", "equals sign")] {
		let m = match compile::parse_input(arg) {
			Ok(p)	=> panic!("{:?} parsed as {:?}", arg, p),
			Err(e)	=> e.plain(),
		};
		assert!(m.contains("--input") && m.contains(why) && m.contains(&format!("'{}'", arg)), "{:?}: {}", arg, m);
	}
}

#[test]
fn watch_compiles_with_its_inputs() {
	let d = dir("watch");
	let main = d.join("main.typ");
	std::fs::write(&main, "#assert.eq(sys.inputs.engine, \"austenite\")\nText.\n").expect("the source");
	let sets = vec!["view.open=false".to_string()];
	let mut run = watch::Run::new(Some(main.clone()), sets, pairs(&[("engine", "austenite")]), true, None)
		.expect("a watch run");
	match run.tick().expect("a tick") {
		watch::Tick::Built(_)	=> (),
		other					=> panic!("the watch did not build: {:?}", other),
	}
	// `austenite SOURCE --watch`, the run with no settings file.
	let bare = watch::Bare {
		source:			main.clone(),
		out_dir:		d.join("bare"),
		root:			None,
		fonts:			Vec::new(),
		strict:			false,
		diag_summary:	false,
		timings:		None,
		timings_fine:	false,
		inputs:			pairs(&[("engine", "austenite")]),
	};
	let mut run = watch::Run::bare(&d, &bare).expect("a bare watch run");
	match run.tick().expect("a tick") {
		watch::Tick::Built(_)	=> (),
		other					=> panic!("the bare watch did not build: {:?}", other),
	}
	let (_, report) = watch::build(Some(&main), &["view.open=false".to_string()], &pairs(&[("engine", "austenite")]), None)
		.expect("a build with its inputs");
	assert!(report.pages > 0);
}
