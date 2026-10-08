//! The settings of `austenite watch` and `austenite build`, and the loop that keeps a PDF fresh.
//!
//! The cases hold the precedence (default, then file, then each `--set`, and a named source over the
//! `document` setting), every refusal by the key's name, the dotted `--set`, the choice of a document, the
//! atomic swap of the finished PDF under readers that interleave their opens, the last good PDF kept when
//! a compile or a settings reload fails, the cold compile a settings edit forces, and the absence of every
//! metadata key from a default `build`.

use oxedyne_fe2o3_austenite::compile::Session;
use oxedyne_fe2o3_austenite::emit::pdf::PdfOptions;
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::settings::{
	self,
	Settings,
};
use oxedyne_fe2o3_austenite::watch::{
	self,
	Plan,
	Run,
	Spec,
	Tick,
};
use oxedyne_fe2o3_core::prelude::Outcome;

use std::path::{
	Path,
	PathBuf,
};
use std::sync::atomic::{
	AtomicBool,
	Ordering,
};
use std::sync::Arc;
use std::time::Duration;

// The footer reads the final page count, so a cold compile needs a second pass to learn it.
const HEAD: &str = "#set page(width: 220pt, height: 130pt, margin: 16pt, numbering: \"1\",\n\
	footer: context [Page #counter(page).display() of #counter(page).final().first()])\n\
	#set text(size: 9pt)\n";

const PARA: &str = "A paragraph with enough ordinary words in it to run on to a second line of the page.";

// The invariant keys of a PDF that carries metadata, none of which a default build may hold.
const META: &[&str] = &[
	"/Info", "/Metadata", "xmpmeta", "/CreationDate", "/ModDate", "/Author", "/Creator", "/Producer", "/Keywords",
];

fn dir(name: &str) -> PathBuf {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("watch").join(name);
	let _ = std::fs::remove_dir_all(&d);
	std::fs::create_dir_all(&d).expect("the case's directory");
	std::fs::canonicalize(&d).expect("the case's canonical path")
}

fn write(path: &Path, text: &str) {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).expect("a project directory");
	}
	std::fs::write(path, text).expect("a project file");
}

fn source(paras: usize, tag: &str) -> String {
	let mut s = String::from(HEAD);
	for i in 0..paras {
		s.push_str(&format!("{} {}\n\n", PARA, if i == 0 { tag } else { "" }));
	}
	s
}

// Waits out the clock's grain so that a file written next carries a different modification time.
fn later() {
	std::thread::sleep(Duration::from_millis(30));
}

fn sets(items: &[&str]) -> Vec<String> {
	items.iter().map(|s| s.to_string()).collect()
}

// A settings file's text: the viewer is off whatever else it holds, so that no test opens a window.
fn conf(extra: &str) -> String {
	if extra.is_empty() {
		"{\"view\": {\"open\": false}}".to_string()
	} else {
		format!("{{\"view\": {{\"open\": false}}, {}}}", extra)
	}
}

// A project of one document with a settings file; the viewer is always off.
fn project(name: &str, extra: &str) -> (PathBuf, PathBuf) {
	let d = dir(name);
	let main = d.join("main.typ");
	write(&main, &source(9, "q"));
	write(&d.join(settings::FILE), &conf(extra));
	// A file saved within a few milliseconds of a build's start is built again once, so a loop under test
	// begins after the files are old enough to count as seen.
	later();
	(d, main)
}

// The reason a result was refused.
fn why<T>(r: Outcome<T>) -> String {
	match r {
		Ok(_)	=> panic!("expected a refusal"),
		Err(e)	=> e.plain(),
	}
}

fn count(bytes: &[u8], needle: &str) -> usize {
	let n = needle.as_bytes();
	bytes.windows(n.len()).filter(|w| *w == n).count()
}

fn has(bytes: &[u8], needle: &str) -> bool {
	count(bytes, needle) > 0
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PRECEDENCE                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn with_no_file_and_no_set_every_setting_is_its_default() {
	let s = Settings::resolve(None, &[]).expect("the defaults");
	assert_eq!(s.output, "{stem}.pdf");
	assert_eq!(s.fonts, vec!["assets/fonts".to_string()]);
	assert!(s.document.is_empty() && s.root.is_empty());
	assert!(!s.strict);
	assert_eq!(s.diagnostics, "errors");
	assert_eq!((s.watch.poll_ms, s.watch.warm), (200, true));
	assert_eq!((s.figs.dir.as_str(), s.figs.render), ("figs", true));
	assert_eq!((s.colour.space.as_str(), s.colour.black.as_str()), ("native", "k"));
	assert_eq!((s.metadata.document, s.metadata.engine), (false, false));
	assert_eq!((s.pdf.version.as_str(), s.pdf.compress, s.pdf.outline), ("1.7", true, true));
	assert_eq!((s.view.open, s.view.app.as_str()), (true, "auto"));
}

#[test]
fn the_file_overrides_the_default_and_a_set_overrides_the_file() {
	let d = dir("precedence");
	let f = d.join(settings::FILE);
	write(&f, "{\"strict\": true, \"pdf\": {\"compress\": false, \"version\": \"1.5\"}, \"watch\": {\"poll_ms\": 500}}");
	let s = Settings::resolve(Some(&f), &sets(&["watch.poll_ms=50", "pdf.compress=true"])).expect("file and sets");
	assert!(s.strict, "the file beats the default");
	assert_eq!(s.pdf.version, "1.5", "the file beats the default");
	assert_eq!(s.watch.poll_ms, 50, "a set beats the file");
	assert!(s.pdf.compress, "a set beats the file, whatever the key's order");
	assert!(s.watch.warm, "a key neither names keeps its default");
}

#[test]
fn of_two_sets_for_one_key_the_later_wins() {
	let s = Settings::resolve(None, &sets(&["watch.poll_ms=20", "watch.poll_ms=30"])).expect("two sets");
	assert_eq!(s.watch.poll_ms, 30);
}

#[test]
fn a_named_source_wins_over_the_document_setting() {
	let (d, main) = project("named", "\"document\": \"main.typ\"");
	let other = d.join("other.typ");
	write(&other, &source(2, "z"));
	let named = Plan::at(&d, Some(&other), &sets(&["view.open=false"]), None).expect("a named source");
	assert_eq!(named.spec.main, other, "the source on the command line");
	assert_eq!(named.spec.out, d.join("other.pdf"));
	let unnamed = Plan::at(&d, None, &sets(&["view.open=false"]), None).expect("the document setting");
	assert_eq!(unnamed.spec.main, main, "with no source the setting names the document");
	// A set on the command line beats the file's `document` too, when no source is named.
	let by_set = Plan::at(&d, None, &sets(&["document=other.typ"]), None).expect("a document by set");
	assert_eq!(by_set.spec.main, other);
}

#[test]
fn the_settings_file_is_found_by_walking_up_from_the_source() {
	let (d, _) = project("walk", "\"output\": \"out/{stem}.pdf\"");
	let deep = d.join("a").join("b").join("deep.typ");
	write(&deep, &source(1, "d"));
	let plan = Plan::at(&d, Some(&deep), &[], None).expect("a deep source");
	assert_eq!(plan.file, Some(d.join(settings::FILE)));
	assert_eq!(plan.spec.out, d.join("a").join("b").join("out").join("deep.pdf"), "an output beside the source");
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REFUSALS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn an_unknown_key_is_refused_by_its_dotted_name_in_the_file_and_in_a_set() {
	let d = dir("unknown");
	let f = d.join(settings::FILE);
	write(&f, "{\"pdf\": {\"compres\": true}}");
	let m = why(Settings::resolve(Some(&f), &[]));
	assert!(m.contains("pdf.compres"), "{}", m);
	write(&f, "{\"nope\": 1}");
	let m = why(Settings::resolve(Some(&f), &[]));
	assert!(m.contains("nope"), "{}", m);
	let m = why(Settings::resolve(None, &sets(&["colour.spce=rgb"])));
	assert!(m.contains("colour.spce"), "{}", m);
	let m = why(Settings::resolve(None, &sets(&["strict"])));
	assert!(m.contains("strict") && m.contains('='), "{}", m);
}

#[test]
fn a_value_of_the_wrong_type_is_refused_by_the_keys_name() {
	let d = dir("types");
	let f = d.join(settings::FILE);
	write(&f, "{\"pdf\": {\"compress\": \"yes\"}}");
	let m = why(Settings::resolve(Some(&f), &[]));
	assert!(m.contains("pdf.compress"), "{}", m);
	write(&f, "{\"output\": 5}");
	let m = why(Settings::resolve(Some(&f), &[]));
	assert!(m.contains("output"), "{}", m);
	for (arg, key) in [("watch.poll_ms=fast", "watch.poll_ms"), ("strict=maybe", "strict"), ("view.open=1", "view.open")] {
		let m = why(Settings::resolve(None, &sets(&[arg])));
		assert!(m.contains(key), "{}: {}", arg, m);
	}
}

#[test]
fn a_value_out_of_range_is_refused_by_the_keys_name() {
	let d = dir("range");
	let f = d.join(settings::FILE);
	write(&f, "{\"watch\": {\"poll_ms\": 5}}");
	let m = why(Settings::resolve(Some(&f), &[]));
	assert!(m.contains("watch.poll_ms"), "{}", m);
	for arg in ["watch.poll_ms=5", "watch.poll_ms=60001"] {
		let m = why(Settings::resolve(None, &sets(&[arg])));
		assert!(m.contains("watch.poll_ms"), "{}: {}", arg, m);
	}
	for (arg, key) in [("pdf.version=1.3", "pdf.version"), ("diagnostics=loud", "diagnostics"), ("colour.black=pink", "colour.black")] {
		let m = why(Settings::resolve(None, &sets(&[arg])));
		assert!(m.contains(key), "{}: {}", arg, m);
	}
	assert!(Settings::resolve(None, &sets(&["watch.poll_ms=10", "watch.poll_ms=60000"])).is_ok(), "the ends of the range stand");
}

#[test]
fn a_dotted_set_makes_the_levels_the_file_lacks_and_keeps_the_siblings() {
	let s = Settings::resolve(None, &sets(&["colour.space=rgb", "fonts=a,b", "view.open=false"])).expect("dotted sets");
	assert_eq!(s.colour.space, "rgb");
	assert_eq!(s.colour.black, "k", "a sibling keeps its default");
	assert_eq!(s.fonts, vec!["a".to_string(), "b".to_string()], "a list is comma separated");
	assert!(!s.view.open);
	let d = dir("dotted");
	let f = d.join(settings::FILE);
	write(&f, "{\"colour\": {\"black\": \"k\", \"intent\": \"perceptual\"}}");
	let s = Settings::resolve(Some(&f), &sets(&["colour.space=rgb"])).expect("a set into a group");
	assert_eq!((s.colour.space.as_str(), s.colour.intent.as_str()), ("rgb", "perceptual"));
	// A key whose parent in the file is no map cannot take the set.
	write(&f, "{\"colour\": 3}");
	assert!(Settings::resolve(Some(&f), &sets(&["colour.space=rgb"])).is_err());
}

#[test]
fn each_setting_that_asks_for_what_is_not_built_is_refused_by_its_key() {
	let (d, main) = project("unbuilt", "");
	for (arg, key) in [
		("colour.space=cmyk",				"colour.space"),
		("colour.space=grey",				"colour.space"),
		("colour.rgb_profile=adobe",		"colour.rgb_profile"),
		("colour.grey_profile=gamma",		"colour.grey_profile"),
		("colour.cmyk_profile=swop",		"colour.cmyk_profile"),
		("colour.intent=relative",			"colour.intent"),
		("colour.black=rich",				"colour.black"),
		("colour.black_point=false",		"colour.black_point"),
	] {
		let m = why(Plan::at(&d, Some(&main), &sets(&[arg]), None));
		assert!(m.contains(key), "{}: {}", arg, m);
		assert!(m.contains("not built"), "{}: {}", arg, m);
	}
	for arg in ["colour.space=rgb", "colour.space=native", "colour.rgb_profile=srgb", "colour.black=k", "colour.black_point=true"] {
		assert!(Plan::at(&d, Some(&main), &sets(&[arg]), None).is_ok(), "{} is built", arg);
	}
}

#[test]
fn figure_rendering_is_refused_while_a_figures_directory_stands_beside_the_document() {
	let (d, main) = project("figs", "");
	assert!(Plan::at(&d, Some(&main), &[], None).is_ok(), "no figures directory, nothing to render");
	write(&d.join("figs").join("one.typ"), "x");
	let m = why(Plan::at(&d, Some(&main), &[], None));
	assert!(m.contains("figs.render"), "{}", m);
	assert!(Plan::at(&d, Some(&main), &sets(&["figs.render=false"]), None).is_ok(), "switched off, it goes on");
	assert!(Plan::at(&d, Some(&main), &sets(&["figs.dir=pictures"]), None).is_ok(), "another name, nothing there");
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE DOCUMENT                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn the_document_is_the_one_typ_that_no_other_imports() {
	let d = dir("select_one");
	write(&d.join("main.typ"), "#include \"part.typ\"\n#import \"lib.typ\": f\n");
	write(&d.join("part.typ"), "x");
	write(&d.join("lib.typ"), "#let f = 1");
	write(&d.join("notes.txt"), "not a source");
	assert_eq!(settings::select_document(&d).expect("one root"), d.join("main.typ"));
}

#[test]
fn several_roots_are_refused_and_named() {
	let d = dir("select_many");
	write(&d.join("a.typ"), "a");
	write(&d.join("b.typ"), "b");
	let m = why(settings::select_document(&d));
	assert!(m.contains("a.typ") && m.contains("b.typ"), "{}", m);
}

#[test]
fn a_directory_with_no_root_or_no_source_is_refused() {
	let d = dir("select_none");
	let m = why(settings::select_document(&d));
	assert!(m.contains("No document"), "{}", m);
	write(&d.join("a.typ"), "#import \"b.typ\"");
	write(&d.join("b.typ"), "#import \"a.typ\"");
	let m = why(settings::select_document(&d));
	assert!(m.contains("No document"), "{}", m);
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ BUILD                                                                      │
// └───────────────────────────────────────────────────────────────────────────┘

#[test]
fn a_default_build_holds_none_of_the_metadata_keys() {
	let (_, main) = project("meta", "");
	let mut text = String::from("#set document(title: \"Title\", author: \"Author\", keywords: (\"kw\",))\n");
	text.push_str(&source(9, "q"));
	write(&main, &text);
	let (plan, report) = watch::build(&main, &sets(&["view.open=false"]), None).expect("a build");
	assert!(report.pages >= 1);
	let bytes = std::fs::read(&plan.spec.out).expect("the PDF");
	assert!(bytes.starts_with(b"%PDF-1.7"), "the default version");
	for key in META {
		assert!(!has(&bytes, key), "the default PDF holds {}", key);
	}
	// The same document asked for its metadata does carry it, so the absence above is the setting's doing.
	let (plan, _) = watch::build(&main, &sets(&["metadata.document=true", "metadata.engine=true"]), None).expect("a build");
	let bytes = std::fs::read(&plan.spec.out).expect("the PDF");
	for key in ["/Info", "/Title", "/Author", "/Keywords", "/Creator", "/Producer"] {
		assert!(has(&bytes, key), "the PDF with metadata lacks {}", key);
	}
	assert!(!has(&bytes, "/CreationDate") && !has(&bytes, "/ModDate"), "no date is ever written");
}

#[test]
fn the_pdf_settings_shape_the_file() {
	let (d, main) = project("shape", "");
	let (plan, _) = watch::build(&main, &[], None).expect("the default");
	let plain = std::fs::read(&plan.spec.out).expect("the PDF");
	assert!(has(&plain, "/FlateDecode"), "compressed by default");
	assert_eq!(plan.spec.out, d.join("main.pdf"));
	let (plan, _) = watch::build(&main, &sets(&["pdf.compress=false", "pdf.version=1.5", "output=sub/{stem}-x.pdf"]), None).expect("changed");
	let big = std::fs::read(&plan.spec.out).expect("the PDF");
	assert_eq!(plan.spec.out, d.join("sub").join("main-x.pdf"));
	assert!(big.starts_with(b"%PDF-1.5"));
	// The fonts' streams stay compressed whatever the setting; the pages' do not.
	assert!(count(&big, "/FlateDecode") < count(&plain, "/FlateDecode"), "page streams left whole");
	assert!(big.len() > plain.len());
	let (plan, _) = watch::build(&main, &sets(&["pdf.outline=false"]), None).expect("no outline");
	let flat = std::fs::read(&plan.spec.out).expect("the PDF");
	assert!(!has(&flat, "/Outlines"), "no bookmarks when the outline is off");
}

#[test]
fn two_builds_of_one_document_are_the_same_bytes() {
	let (_, main) = project("again", "");
	let (plan, _) = watch::build(&main, &[], None).expect("first");
	let one = std::fs::read(&plan.spec.out).expect("the PDF");
	let (plan, _) = watch::build(&main, &[], None).expect("second");
	let two = std::fs::read(&plan.spec.out).expect("the PDF");
	assert!(one == two, "a build is deterministic");
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SWAP                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

fn spec(main: &Path, out: &Path) -> Spec {
	Spec {
		main:			main.to_path_buf(),
		root:			main.parent().expect("the main's directory").to_path_buf(),
		out:			out.to_path_buf(),
		strict:			false,
		diag_summary:	false,
		timings:		None,
		timings_fine:	false,
		pdf:			PdfOptions { compress: false, ..PdfOptions::default() },
	}
}

#[test]
fn a_reader_meets_one_whole_pdf_or_the_other_while_the_compile_swaps_a_new_one_in() {
	let d = dir("swap");
	let a = d.join("a.typ");
	let b = d.join("b.typ");
	write(&a, &source(40, "a"));
	write(&b, &source(40, "b"));
	let mut session = Session::new(FontStore::default());
	let mut read = Vec::new();
	let mut known = Vec::new();
	for (main, name) in [(&a, "a.pdf"), (&b, "b.pdf")] {
		let out = d.join(name);
		watch::compile_pdf(&spec(main, &out), &mut session, true, false, &mut read).expect("a known PDF");
		known.push(std::fs::read(&out).expect("its bytes"));
	}
	let (known_a, known_b) = (Arc::new(known[0].clone()), Arc::new(known[1].clone()));
	assert!(known_a != known_b, "the two documents differ");
	let live = d.join("live.pdf");
	watch::compile_pdf(&spec(&a, &live), &mut session, true, false, &mut read).expect("the first live PDF");
	let stop = Arc::new(AtomicBool::new(false));
	let mut readers = Vec::new();
	for _ in 0..3 {
		let (live, stop, ka, kb) = (live.clone(), stop.clone(), known_a.clone(), known_b.clone());
		readers.push(std::thread::spawn(move || {
			let (mut reads, mut torn) = (0u32, 0u32);
			while !stop.load(Ordering::Relaxed) {
				match std::fs::read(&live) {
					Ok(bytes)	=> {
						reads += 1;
						if bytes != *ka && bytes != *kb {
							torn += 1;
						}
					},
					Err(_)		=> torn += 1,
				}
			}
			(reads, torn)
		}));
	}
	for i in 0..14 {
		let main = if i % 2 == 0 { &b } else { &a };
		watch::compile_pdf(&spec(main, &live), &mut session, true, false, &mut read).expect("a swap");
	}
	stop.store(true, Ordering::Relaxed);
	let (mut reads, mut torn) = (0, 0);
	for r in readers {
		let (n, t) = r.join().expect("a reader");
		reads += n;
		torn += t;
	}
	assert!(reads > 50, "the readers read {} times, too few to mean anything", reads);
	assert_eq!(torn, 0, "{} of {} reads met something other than a whole PDF", torn, reads);
	let mut beside = live.as_os_str().to_owned();
	beside.push(".part");
	assert!(!PathBuf::from(beside).exists(), "no half-written file is left beside the PDF");
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE LOOP                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

fn run_of(main: &Path, extra: &[&str]) -> Run {
	let mut s = vec!["view.open=false"];
	s.extend_from_slice(extra);
	Run::new(Some(main.to_path_buf()), sets(&s), false, None).expect("a run")
}

fn built(t: Tick) -> watch::Report {
	match t {
		Tick::Built(r)		=> r,
		Tick::Idle			=> panic!("expected a build, the tick was idle"),
		Tick::Failed(why)	=> panic!("expected a build, it failed: {}", why),
	}
}

#[test]
fn the_first_tick_builds_cold_and_an_untouched_project_is_idle() {
	let (d, main) = project("tick_first", "");
	let mut run = run_of(&main, &[]);
	let r = built(run.tick().expect("tick one"));
	assert!(!r.warm && r.passes >= 2, "cold: warm {}, passes {}", r.warm, r.passes);
	assert!(d.join("main.pdf").is_file());
	assert!(matches!(run.tick().expect("tick two"), Tick::Idle), "nothing changed");
	assert!(run.line(&r).contains("cold"), "{}", run.line(&r));
}

#[test]
fn an_edit_is_compiled_warm_in_one_pass_to_the_bytes_a_cold_build_gives() {
	let (d, main) = project("tick_warm", "");
	let mut run = run_of(&main, &[]);
	built(run.tick().expect("tick one"));
	later();
	write(&main, &source(9, "edited"));
	let r = built(run.tick().expect("tick two"));
	assert!(r.warm, "the second compile starts from the first's introspector");
	assert_eq!(r.passes, 1, "and settles in one pass");
	let warm = std::fs::read(d.join("main.pdf")).expect("the PDF");
	let cold_dir = dir("tick_warm_cold");
	let cold_main = cold_dir.join("main.typ");
	write(&cold_main, &source(9, "edited"));
	let (plan, _) = watch::build(&cold_main, &sets(&["view.open=false"]), None).expect("a cold build");
	assert!(warm == std::fs::read(&plan.spec.out).expect("the cold PDF"), "warm and cold bytes differ");
	assert!(run.line(&r).contains("warm"));
}

#[test]
fn a_compile_that_fails_leaves_the_last_good_pdf_as_it_was_and_the_loop_goes_on() {
	let (d, main) = project("tick_fail", "");
	let mut run = run_of(&main, &[]);
	built(run.tick().expect("tick one"));
	let out = d.join("main.pdf");
	let good = std::fs::read(&out).expect("the PDF");
	later();
	write(&main, &format!("{}\n#nosuchname\n", source(9, "q")));
	match run.tick().expect("tick two") {
		Tick::Failed(why)	=> assert!(!why.is_empty(), "the failure is told"),
		other				=> panic!("expected a failure, got {:?}", other),
	}
	assert!(std::fs::read(&out).expect("the PDF still there") == good, "the last good PDF is as it was");
	assert!(matches!(run.tick().expect("tick three"), Tick::Idle), "the failure is told once");
	later();
	write(&main, &source(9, "mended"));
	built(run.tick().expect("tick four"));
	assert!(std::fs::read(&out).expect("the PDF") != good, "the mended source gives a new PDF");
}

#[test]
fn a_settings_edit_reloads_the_settings_and_compiles_cold_once() {
	let (d, main) = project("tick_settings", "\"pdf\": {\"version\": \"1.7\"}");
	let mut run = run_of(&main, &[]);
	built(run.tick().expect("tick one"));
	let out = d.join("main.pdf");
	assert!(std::fs::read(&out).expect("the PDF").starts_with(b"%PDF-1.7"));
	later();
	write(&main, &source(9, "edited"));
	assert!(built(run.tick().expect("tick two")).warm, "an edit of the source compiles warm");
	later();
	write(&d.join(settings::FILE), &conf("\"pdf\": {\"version\": \"1.5\"}"));
	let r = built(run.tick().expect("tick three"));
	assert!(!r.warm, "a settings edit compiles cold");
	assert!(std::fs::read(&out).expect("the PDF").starts_with(b"%PDF-1.5"), "the new setting is in force");
	later();
	write(&main, &source(9, "edited again"));
	assert!(built(run.tick().expect("tick four")).warm, "and the compile after it is warm again");
}

#[test]
fn a_settings_file_that_no_longer_reads_keeps_the_old_settings_and_is_told_once() {
	let (d, main) = project("tick_badsettings", "\"pdf\": {\"version\": \"1.6\"}");
	let mut run = run_of(&main, &[]);
	built(run.tick().expect("tick one"));
	let out = d.join("main.pdf");
	let good = std::fs::read(&out).expect("the PDF");
	assert!(good.starts_with(b"%PDF-1.6"));
	later();
	write(&d.join(settings::FILE), &conf("\"pdf\": {\"version\": \"9.9\"}"));
	match run.tick().expect("tick two") {
		Tick::Failed(why)	=> assert!(why.contains("pdf.version"), "{}", why),
		other				=> panic!("expected a failure, got {:?}", other),
	}
	assert!(std::fs::read(&out).expect("the PDF") == good, "the last good PDF is as it was");
	assert!(matches!(run.tick().expect("tick three"), Tick::Idle), "told once");
	later();
	write(&d.join(settings::FILE), &conf("\"pdf\": {\"version\": \"1.4\"}"));
	built(run.tick().expect("tick four"));
	assert!(std::fs::read(&out).expect("the PDF").starts_with(b"%PDF-1.4"), "the mended file takes effect");
}

#[test]
fn watch_warm_off_compiles_cold_every_time() {
	let (_, main) = project("tick_nowarm", "\"watch\": {\"warm\": false}");
	let mut run = run_of(&main, &[]);
	built(run.tick().expect("tick one"));
	later();
	write(&main, &source(9, "edited"));
	assert!(!built(run.tick().expect("tick two")).warm);
}
