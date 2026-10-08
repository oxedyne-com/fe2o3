//! A [`Session`] compiles warm: it starts a compile from the introspector of the one before, keeps its fonts,
//! and keeps what it keeps even when a compile fails. Warm must end where a cold compile ends, so each case
//! compares a session's bytes with a fresh session's for the same source, and counts the passes the warm
//! start saves.

use oxedyne_fe2o3_austenite::caches::{
	self,
	Budgets,
};
use oxedyne_fe2o3_austenite::compile::{
	self,
	Evaluated,
	Session,
};
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;

use std::path::{
	Path,
	PathBuf,
};
use std::sync::Arc;
use std::time::Duration;

// The footer reads the final page count, so a cold compile needs a second pass to learn it.
const HEAD: &str = "#set page(width: 220pt, height: 130pt, margin: 16pt, numbering: \"1\",\n\
	footer: context [Page #counter(page).display() of #counter(page).final().first()])\n\
	#set text(size: 9pt)\n";

const PARA: &str = "A paragraph with enough ordinary words in it to run on to a second line of the page.\n\n";

fn dir(name: &str) -> PathBuf {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_session").join(name);
	let _ = std::fs::remove_dir_all(&d);
	std::fs::create_dir_all(&d).expect("the case's directory");
	std::fs::canonicalize(&d).expect("the case's canonical path")
}

fn source(paras: usize, tag: &str) -> String {
	let mut s = String::from(HEAD);
	for i in 0..paras {
		s.push_str(&format!("{} {}\n\n", PARA.trim_end(), if i == 0 { tag } else { "" }));
	}
	s
}

// Compiles the file `main` under `session`, giving the PDF, the passes, and whether it started warm.
fn pdf(session: &mut Session, main: &Path, cold: bool) -> (Vec<u8>, u32, bool) {
	let mut sink = PdfSink::new().expect("a PDF sink");
	let root = main.parent().expect("the main's directory");
	let done: Evaluated = session.compile(main, root, &mut sink, None, None, cold).expect("the main is readable");
	let passes = match &done.laid {
		Ok(l)	=> l.passes,
		Err(e)	=> panic!("the compile failed: {}", e.plain()),
	};
	let bytes = sink.output().expect("a finished PDF").to_vec();
	(bytes, passes, done.warm)
}

fn fresh() -> Session { Session::new(FontStore::default()) }

fn write(path: &Path, text: &str) {
	std::fs::write(path, text).expect("a project file");
}

#[test]
fn a_second_compile_starts_warm_and_settles_in_one_pass_to_the_cold_bytes() {
	let main = dir("again").join("main.typ");
	write(&main, &source(9, "q"));
	let mut s = fresh();
	let (cold, cold_passes, was_warm) = pdf(&mut s, &main, false);
	assert!(!was_warm, "the first compile has nothing to start from");
	assert!(cold_passes >= 2, "the footer needs a second pass cold, took {}", cold_passes);
	let (warm, warm_passes, was_warm) = pdf(&mut s, &main, false);
	assert!(was_warm, "the second compile starts from the first's introspector");
	assert_eq!(warm_passes, 1, "and settles in one pass");
	assert!(warm == cold, "warm and cold bytes differ");
}

#[test]
fn an_edit_compiled_warm_equals_the_same_source_compiled_cold() {
	let main = dir("edit").join("main.typ");
	write(&main, &source(9, "q"));
	let mut s = fresh();
	let _ = pdf(&mut s, &main, false);
	// A letter, then enough text to add pages, then back to fewer: the page count the footer reads moves.
	for (paras, tag) in [(9, "x"), (24, "x"), (24, "z"), (3, "z")] {
		write(&main, &source(paras, tag));
		let (warm, _, was_warm) = pdf(&mut s, &main, false);
		let (cold, _, _) = pdf(&mut fresh(), &main, false);
		assert!(was_warm, "the compile of {} paragraphs starts warm", paras);
		assert!(warm == cold, "warm and cold differ at {} paragraphs, tag {}", paras, tag);
	}
}

#[test]
fn a_cold_request_ignores_the_kept_introspector_and_still_keeps_the_new_one() {
	let main = dir("cold").join("main.typ");
	write(&main, &source(9, "q"));
	let mut s = fresh();
	let (first, first_passes, _) = pdf(&mut s, &main, false);
	let (again, again_passes, was_warm) = pdf(&mut s, &main, true);
	assert!(!was_warm, "an explicit output starts cold");
	assert_eq!(again_passes, first_passes, "and takes the passes a first compile takes");
	assert!(again == first, "and gives the same bytes");
	let (_, passes, was_warm) = pdf(&mut s, &main, false);
	assert!(was_warm && passes == 1, "yet it left its introspector for the compile after it");
}

#[test]
fn a_compile_that_lays_no_pages_keeps_the_last_introspector() {
	let main = dir("broken").join("main.typ");
	write(&main, &source(9, "q"));
	let mut s = fresh();
	let _ = pdf(&mut s, &main, false);
	let kept = s.intro().expect("pages were laid").clone();
	// A bracket left open: the source does not evaluate, so no page is laid.
	write(&main, &format!("{}#[an open bracket\n", source(9, "q")));
	let mut sink = PdfSink::new().expect("a PDF sink");
	let root = main.parent().expect("the main's directory");
	let done = s.compile(&main, root, &mut sink, None, None, false).expect("the main is readable");
	assert!(done.laid.is_err(), "the broken source must fail");
	assert!(Arc::ptr_eq(&kept, s.intro().expect("still kept")), "a failed compile leaves the introspector alone");
	write(&main, &source(9, "x"));
	let (warm, passes, was_warm) = pdf(&mut s, &main, false);
	let (cold, _, _) = pdf(&mut fresh(), &main, false);
	assert!(was_warm && passes == 1, "the compile after the failure starts warm");
	assert!(warm == cold, "and ends where a cold one ends");
}

#[test]
fn a_font_added_on_disc_renews_the_store_and_an_unchanged_one_keeps_it() {
	let root = dir("fonts");
	let fonts = root.join("fonts");
	std::fs::create_dir_all(&fonts).expect("the font directory");
	write(&fonts.join("one.ttf"), "not a font");
	let main = root.join("main.typ");
	write(&main, &source(2, "q"));
	let mut store = FontStore::default();
	store.add_dir(fonts.clone());
	let mut s = Session::new(store);
	let _ = pdf(&mut s, &main, false);
	let held = |s: &Session| s.fonts().scanned().iter().filter(|p| p.extension().is_some()).count();
	assert_eq!(held(&s), 1, "the store has met the one file");
	let _ = pdf(&mut s, &main, false);
	assert_eq!(held(&s), 1, "an unchanged directory keeps the store");
	std::thread::sleep(Duration::from_millis(60));
	write(&fonts.join("two.ttf"), "not a font either");
	let _ = pdf(&mut s, &main, false);
	assert_eq!(held(&s), 2, "a file added beside it renews the store, which then meets both");
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE CONFIGURATION THE CACHES ARE SCOPED BY                                 │
// └───────────────────────────────────────────────────────────────────────────┘

fn noto() -> Vec<u8> {
	let at = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("fe2o3_font").join("fonts").join("NotoSans-Regular.ttf");
	std::fs::read(at).expect("the Noto Sans file")
}

fn pair(k: &str, v: &str) -> (String, String) { (k.to_string(), v.to_string()) }

#[test]
fn the_configuration_is_the_faces_the_inputs_and_the_engine_and_the_order_of_inputs_is_not_a_change() {
	let faces = Fingerprint::of(b"faces");
	let one = [pair("lang", "en"), pair("draft", "yes")];
	let swapped = [pair("draft", "yes"), pair("lang", "en")];
	let base = caches::configuration(faces, &one, "abc");
	assert_eq!(base, caches::configuration(faces, &one, "abc"), "the same configuration is the same fingerprint");
	assert_eq!(base, caches::configuration(faces, &swapped, "abc"), "the order the inputs come in is not a change");
	assert_ne!(base, caches::configuration(faces, &[pair("lang", "en")], "abc"), "an input dropped is a change");
	assert_ne!(base, caches::configuration(faces, &[pair("lang", "en"), pair("draft", "no")], "abc"),
		"an input's value changed is a change");
	assert_ne!(base, caches::configuration(faces, &[pair("lang", "en"), pair("final", "yes")], "abc"),
		"an input's name changed is a change");
	assert_ne!(base, caches::configuration(faces, &[], "abc"), "no inputs at all is a change");
	assert_ne!(base, caches::configuration(Fingerprint::of(b"other faces"), &one, "abc"), "other faces are a change");
	assert_ne!(base, caches::configuration(faces, &one, "abd"), "another engine is a change");
	// A key and a value that run together read as another pair.
	assert_ne!(caches::configuration(faces, &[pair("ab", "c")], "abc"),
		caches::configuration(faces, &[pair("a", "bc")], "abc"), "where a pair divides is part of it");
}

#[test]
fn the_face_set_is_told_by_its_families_and_its_bytes_in_the_order_added() {
	let mut plain = FontStore::default();
	let mut again = FontStore::default();
	let mut more = FontStore::default();
	more.add_bytes(noto());
	let a = plain.book().expect("a book").fingerprint();
	let b = again.book().expect("a book").fingerprint();
	let c = more.book().expect("a book").fingerprint();
	assert_eq!(a, b, "two books made alike have one fingerprint");
	assert_ne!(a, c, "a face added moves it");
	let mut flipped = noto();
	let last = flipped.len() - 1;
	flipped[last] ^= 1;
	let mut changed = FontStore::default();
	changed.add_bytes(flipped);
	let altered = changed.book().expect("a book");
	assert_eq!(altered.len(), more.book().expect("a book").len(), "the altered file still parses, as a face");
	assert_ne!(c, altered.fingerprint(), "a byte of the file changed moves it");
}

#[test]
fn a_new_face_empties_the_caches_and_the_same_faces_do_not() {
	let main = dir("config").join("main.typ");
	write(&main, &source(3, "q"));
	let mut s = fresh();
	assert!(s.caches().config().is_none(), "a session that has compiled nothing holds no configuration");
	let _ = pdf(&mut s, &main, false);
	let first = s.caches().config().expect("a compile sets the configuration");
	assert_eq!(s.caches().resets(), 0, "the first compile has nothing to empty");
	let _ = pdf(&mut s, &main, false);
	assert_eq!((s.caches().config(), s.caches().resets()), (Some(first), 0), "the same faces leave the caches");
	// The configuration is the one the parts name, read where the compile read them.
	let faces = s.fonts().clone().book().expect("a book").fingerprint();
	assert_eq!(first, caches::configuration(faces, &caches::sys_inputs(), compile::engine_git_hash()));
	let mut store = FontStore::default();
	store.add_bytes(noto());
	s.set_fonts(store);
	let _ = pdf(&mut s, &main, false);
	assert_ne!(s.caches().config(), Some(first), "a face added moves the configuration");
	assert_eq!(s.caches().resets(), 1, "and the caches were emptied once for it");
	let _ = pdf(&mut s, &main, false);
	assert_eq!(s.caches().resets(), 1, "and not again while the faces stand");
}

#[test]
fn the_budgets_are_the_hosts_the_shaped_run_cache_takes_its_share_and_the_counters_read_it() {
	let main = dir("budgets").join("main.typ");
	write(&main, &source(6, "q"));
	let mut sink = PdfSink::new().expect("a PDF sink");
	let root = main.parent().expect("the main's directory");
	let mut s = fresh();
	assert_eq!(s.budgets(), Budgets::default());
	assert_eq!((s.budgets().shapes, s.budgets().pars), (32 << 20, 16 << 20), "the defaults");
	let done = s.compile(&main, root, &mut sink, None, None, false).expect("the main is readable");
	assert_eq!(done.counters.shapes.budget, 32 << 20, "the shaped-run cache holds the default share");
	assert!(done.counters.shapes.misses > 0 && done.counters.shapes.entries > 0, "and the compile shaped into it");
	assert_eq!(s.caches().counters, done.counters, "the session keeps what the result reports");
	assert_eq!((done.counters.replayed, done.counters.relaid, done.counters.par_hits, done.counters.par_misses),
		(0, 0, 0, 0), "the page and paragraph counts are zero until those caches exist");
	let set = Budgets { shapes: 1 << 20, pars: 2 << 20, ledger: 3 << 20 };
	s.set_budgets(set);
	assert_eq!(s.budgets(), set);
	assert_eq!(s.fonts().clone().book().expect("a book").shape_stats().expect("stats").budget, 1 << 20,
		"the shaped-run cache takes its budget at once");
	let mut sink = PdfSink::new().expect("a PDF sink");
	let done = s.compile(&main, root, &mut sink, None, None, false).expect("the main is readable");
	assert_eq!(done.counters.shapes.budget, 1 << 20);
	assert_eq!(s.caches().resets(), 0, "a budget is not part of the configuration");
}
