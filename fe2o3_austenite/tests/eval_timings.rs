//! `austenite --eval --timings FILE.json`: the wall time of each phase of a compile.
//!
//! Each case compiles a synthetic document with the built binary. The record is judged on what it must
//! always hold: the PDF is the same bytes with the recorder on or off, every phase of every fixpoint pass
//! was entered, and the phases add up to the run's wall time within a tenth. The realisation's split is
//! held to its own sums: the sub-phases and the part outside them are the `realise` cell, and the calls by
//! cause are its entries.

#[allow(dead_code)]
#[path = "eval_oracle/json.rs"]
mod json;

use json::J;

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::emit::sinks::PdfSink;
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::timings::Phase;
use oxedyne_fe2o3_austenite::timings::Timings;

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

// A forward read, the page total on its first page, so the fixpoint takes more than one pass; furniture on
// every page; a show rule on an element and one on a word, so realisation runs user code and searches text;
// and enough pages for each phase to be measurable.
const DOC: &str = "\
#set page(width: 220pt, height: 260pt, margin: 24pt, header: [Header], footer: context counter(page).display())
#show heading: it => block(fill: luma(235), inset: 4pt, it.body)
#show \"Sentence\": strong
#context [Pages: #counter(page).final().first()]

#for i in range(1, 15) [
= Section #i
#for j in range(1, 16) [Sentence #j of section #i sets some words in a paragraph. ]
]
";

struct Runs {
	plain:		Vec<u8>,	// the PDF with no recorder
	timed:		Vec<u8>,	// the PDF with it
	record:		String,		// the record's JSON
	fine:		Vec<u8>,	// the PDF with the fine recorder
	fine_rec:	String,		// the fine record's JSON
}

fn case_dir(name: &str) -> PathBuf {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_timings").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).expect("the case's directory");
	dir
}

fn austenite(args: &[&str]) -> std::process::Output {
	Command::new(env!("CARGO_BIN_EXE_austenite"))
		.args(args)
		.output()
		.expect("the built austenite binary")
}

// The document compiled once without `--timings` and once with, shared by the cases that read the pair.
fn runs() -> &'static Runs {
	static RUNS: OnceLock<Runs> = OnceLock::new();
	RUNS.get_or_init(|| {
		let dir = case_dir("pair");
		let main = dir.join("main.typ");
		std::fs::write(&main, DOC).expect("the case's source");
		let (plain, timed, record) = (dir.join("plain"), dir.join("timed"), dir.join("record.json"));
		let a = austenite(&["--eval", main.to_str().expect("a path"), plain.to_str().expect("a path")]);
		assert!(a.status.success(), "the plain compile succeeds:\n{}", String::from_utf8_lossy(&a.stderr));
		let b = austenite(&["--eval", "--timings", record.to_str().expect("a path"),
			main.to_str().expect("a path"), timed.to_str().expect("a path")]);
		assert!(b.status.success(), "the timed compile succeeds:\n{}", String::from_utf8_lossy(&b.stderr));
		let (fine, fine_rec) = (dir.join("fine"), dir.join("fine.json"));
		let c = austenite(&["--eval", "--timings", fine_rec.to_str().expect("a path"), "--timings-fine",
			main.to_str().expect("a path"), fine.to_str().expect("a path")]);
		assert!(c.status.success(), "the fine compile succeeds:\n{}", String::from_utf8_lossy(&c.stderr));
		Runs {
			plain:		std::fs::read(plain.join("document.pdf")).expect("the plain PDF"),
			timed:		std::fs::read(timed.join("document.pdf")).expect("the timed PDF"),
			record:		std::fs::read_to_string(&record).expect("the record"),
			fine:		std::fs::read(fine.join("document.pdf")).expect("the fine PDF"),
			fine_rec:	std::fs::read_to_string(&fine_rec).expect("the fine record"),
		}
	})
}

fn num(j: &J, path: &[&str]) -> i64 {
	let mut at = j;
	for key in path {
		at = at.get(key).unwrap_or_else(|| panic!("the record has no {:?} (looking for {})", key, path.join(".")));
	}
	at.as_i64().unwrap_or_else(|| panic!("{} is not an integer", path.join(".")))
}

// What is wrong with a record: a phase of a pass with no entry, a pass count that disagrees with the
// array, or phases that do not add up to the total within a tenth. Empty when the record is sound.
fn faults(record: &J) -> Vec<String> {
	let mut out = Vec::new();
	let total = num(record, &["total"]);
	let passes = num(record, &["passes"]);
	let mut sum = 0i64;
	for p in Phase::RUN {
		let (ns, n) = (num(record, &["run", p.name(), "ns"]), num(record, &["run", p.name(), "n"]));
		if n < 1 {
			out.push(format!("run phase {} was never entered", p.name()));
		}
		sum += ns;
	}
	let each = match record.get("pass") {
		Some(J::Arr(v))	=> v.as_slice(),
		_				=> {
			out.push("the record has no pass array".to_string());
			&[]
		},
	};
	if each.len() as i64 != passes {
		out.push(format!("the record says {} passes and holds {}", passes, each.len()));
	}
	for (i, pass) in each.iter().enumerate() {
		for p in Phase::PASS {
			let (ns, n) = (num(pass, &[p.name(), "ns"]), num(pass, &[p.name(), "n"]));
			if n < 1 {
				out.push(format!("pass {} never entered {}", i + 1, p.name()));
			}
			sum += ns;
		}
	}
	if (sum - total).abs() * 10 > total {
		out.push(format!("the phases sum to {} ns against a total of {} ns", sum, total));
	}
	// The shaped-run cache was used, and holds no more than its budget.
	let (hits, misses) = (num(record, &["shape", "hits"]), num(record, &["shape", "misses"]));
	if hits + misses < 1 || misses < 1 {
		out.push(format!("the shaped-run cache shows {} hits and {} misses", hits, misses));
	}
	if num(record, &["shape", "bytes"]) > num(record, &["shape", "budget"]) {
		out.push("the shaped-run cache holds more than its budget".to_string());
	}
	out
}

#[test]
fn the_pdf_is_the_same_bytes_with_the_recorder_on_or_off() {
	let r = runs();
	assert!(r.plain.starts_with(b"%PDF-"), "the plain output is a PDF");
	assert!(r.plain.len() > 4_000, "the document made real pages: {} bytes", r.plain.len());
	assert!(r.plain == r.timed, "--timings changed the PDF ({} bytes against {})", r.plain.len(), r.timed.len());
	assert!(r.plain == r.fine, "--timings-fine changed the PDF ({} bytes against {})", r.plain.len(), r.fine.len());
}

#[test]
fn the_record_holds_every_phase_of_every_pass_and_the_phases_sum_to_the_wall_time() {
	let record = json::parse(&runs().record).expect("the record is JSON");
	assert!(num(&record, &["passes"]) >= 2, "the forward read takes a second pass: {}", runs().record);
	let found = faults(&record);
	assert!(found.is_empty(), "the record is unsound: {:#?}\n{}", found, runs().record);
	let fine = json::parse(&runs().fine_rec).expect("the fine record is JSON");
	let found = faults(&fine);
	assert!(found.is_empty(), "the fine record is unsound: {:#?}\n{}", found, runs().fine_rec);
}

// Sets the time and the count of every cell named `phase`, in the run or in any pass, to zero.
fn never_recorded(j: &mut J, phase: &str, cells: &mut usize) {
	match j {
		J::Obj(kv)	=> {
			for (k, v) in kv.iter_mut() {
				if k == phase {
					*v = J::Obj(vec![("ns".to_string(), J::Int(0)), ("n".to_string(), J::Int(0))]);
					*cells += 1;
				} else {
					never_recorded(v, phase, cells);
				}
			}
		},
		J::Arr(v)	=> for x in v.iter_mut() {
			never_recorded(x, phase, cells);
		},
		_			=> (),
	}
}

#[test]
fn a_record_with_a_phase_never_recorded_is_refused() {
	// The record as the engine would write it were flow never entered: every `flow` cell is empty.
	let mut record = json::parse(&runs().record).expect("the record is JSON");
	let mut cells = 0;
	never_recorded(&mut record, "flow", &mut cells);
	assert!(cells >= 2, "at least two passes had a flow cell to empty, found {}", cells);
	let found = faults(&record);
	assert!(found.iter().any(|f| f.contains("never entered flow")), "the missing phase is named: {:#?}", found);
	assert!(found.iter().any(|f| f.contains("sum to")), "the missing time breaks the sum: {:#?}", found);
}

#[test]
fn timings_needs_the_evaluator_and_a_file() {
	let dir = case_dir("flags");
	let main = dir.join("main.typ");
	std::fs::write(&main, "Text.\n").expect("the case's source");
	let m = main.to_str().expect("a path");
	let rec = dir.join("t.json");
	let out = austenite(&["--timings", rec.to_str().expect("a path"), m]);
	assert!(!out.status.success(), "--timings without --eval is refused");
	assert!(String::from_utf8_lossy(&out.stderr).contains("--timings needs --eval"), "{}", String::from_utf8_lossy(&out.stderr));
	let out = austenite(&["--eval", m, "--timings"]);
	assert!(!out.status.success(), "--timings with no file is refused");
	assert!(String::from_utf8_lossy(&out.stderr).contains("--timings needs a file"), "{}", String::from_utf8_lossy(&out.stderr));
	let out = austenite(&["--eval", "--timings-fine", m]);
	assert!(!out.status.success(), "--timings-fine without --timings is refused");
	assert!(String::from_utf8_lossy(&out.stderr).contains("--timings-fine needs --timings"), "{}", String::from_utf8_lossy(&out.stderr));
	assert!(!rec.exists(), "a refused run writes no record");
}

// The realisation's split, as a list of what is wrong with it. Each pass must hold: the `realise` cell equal
// to the sub-phases and `self` added up, in time and in entries; the calls by cause adding to the entries;
// no call in pass one but a first one or a repeat within the pass; no first call after pass one; and the
// work counters present. Empty when the split is sound.
fn split_faults(record: &J, fine: bool) -> Vec<String> {
	let mut out = Vec::new();
	let each = match record.get("pass") {
		Some(J::Arr(v))	=> v.as_slice(),
		_				=> return vec!["the record has no pass array".to_string()],
	};
	for (i, pass) in each.iter().enumerate() {
		let at = i + 1;
		let (ns, n) = (num(pass, &["realise", "ns"]), num(pass, &["realise", "n"]));
		let (mut sub_ns, mut sub_n) = (0i64, 0i64);
		for key in ["rules", "regex", "show", "repack", "styles", "self"] {
			sub_ns += num(pass, &["sub", key, "ns"]);
			sub_n += if key == "self" { num(pass, &["sub", key, "n"]) } else { 0 };
		}
		if sub_ns != ns {
			out.push(format!("pass {} realise holds {} ns and its sub-phases sum to {} ns", at, ns, sub_ns));
		}
		if sub_n != n {
			out.push(format!("pass {} realise was entered {} times and `self` says {}", at, n, sub_n));
		}
		// The phases entered for every element are counted always and clocked only in a fine record.
		for key in ["rules", "show", "styles"] {
			if num(pass, &["sub", key, "n"]) < 1 {
				out.push(format!("pass {} never entered {}", at, key));
			}
			if !fine && num(pass, &["sub", key, "ns"]) != 0 {
				out.push(format!("pass {} clocked {} in a coarse record", at, key));
			}
			if fine && num(pass, &["sub", key, "ns"]) < 1 {
				out.push(format!("pass {} left {} unclocked in a fine record", at, key));
			}
		}
		let mut by_cause = [0i64; 4];
		match pass.get("calls") {
			Some(J::Obj(kv))	=> for (label, causes) in kv {
				for (c, cause) in ["first", "new", "repass", "again"].iter().enumerate() {
					by_cause[c] += num(causes, &[cause]);
				}
				if label.is_empty() {
					out.push(format!("pass {} has a call with no label", at));
				}
			},
			_					=> out.push(format!("pass {} has no calls object", at)),
		}
		let calls: i64 = by_cause.iter().sum();
		if calls != n {
			out.push(format!("pass {} counts {} realise calls and realise was entered {} times", at, calls, n));
		}
		if i == 0 && (by_cause[1] != 0 || by_cause[2] != 0) {
			out.push(format!("pass 1 has {} new and {} repass calls", by_cause[1], by_cause[2]));
		}
		if i > 0 && by_cause[0] != 0 {
			out.push(format!("pass {} has {} first calls", at, by_cause[0]));
		}
		for c in ["visits", "leaves", "shown", "recipes", "matched", "prepared", "copied", "textual", "finished"] {
			if pass.get("counts").and_then(|k| k.get(c)).is_none() {
				out.push(format!("pass {} has no {} counter", at, c));
			}
		}
	}
	out
}

#[test]
fn the_realise_cell_is_its_sub_phases_and_its_calls_are_its_entries() {
	for (fine, text) in [(false, &runs().record), (true, &runs().fine_rec)] {
		check_split(text, fine);
	}
}

fn check_split(text: &str, fine: bool) {
	let record = json::parse(text).expect("the record is JSON");
	assert!(matches!(record.get("fine"), Some(J::Bool(b)) if *b == fine), "the record says whether it is fine:\n{}", text);
	let found = split_faults(&record, fine);
	assert!(found.is_empty(), "the split is unsound: {:#?}\n{}", found, text);
	// The document reaches every part of realisation in every pass, so a sub-phase that stays empty is not
	// being opened.
	let each = match record.get("pass") {
		Some(J::Arr(v))	=> v,
		_				=> panic!("the record has no pass array"),
	};
	for (i, pass) in each.iter().enumerate() {
		for key in ["show", "repack", "styles", "regex"] {
			assert!(num(pass, &["sub", key, "n"]) >= 1, "pass {} never entered the {} sub-phase:\n{}", i + 1, key, text);
		}
		for c in ["visits", "shown", "recipes", "matched", "prepared", "textual", "finished"] {
			assert!(num(pass, &["counts", c]) >= 1, "pass {} counted no {}:\n{}", i + 1, c, text);
		}
		assert!(num(pass, &["counts", "recipes"]) >= num(pass, &["counts", "shown"]),
			"pass {} has fewer recipes in force than elements shown:\n{}", i + 1, text);
	}
	// A later pass realises some bodies that the pass before it realised too.
	let later: i64 = each.iter().skip(1).map(|p| match p.get("calls") {
		Some(J::Obj(kv))	=> kv.iter().map(|(_, c)| num(c, &["repass"])).sum::<i64>(),
		_					=> 0,
	}).sum();
	assert!(later >= 1, "a second pass realised nothing it had realised before:\n{}", text);
}

// The value at `path` below `j`, if the objects on the way hold it.
fn at_mut<'a>(j: &'a mut J, path: &[&str]) -> Option<&'a mut J> {
	let mut at = j;
	for key in path {
		let J::Obj(kv) = at else { return None };
		match kv.iter_mut().find(|(k, _)| k == key) {
			Some((_, v))	=> at = v,
			None			=> return None,
		}
	}
	Some(at)
}

// Adds `by` to the integer at `path` in pass number `pass` (from zero) of the record.
fn add_in_pass(record: &mut J, pass: usize, path: &[&str], by: i64) {
	let Some(J::Arr(passes)) = at_mut(record, &["pass"]) else { panic!("the record has no pass array") };
	match at_mut(&mut passes[pass], path) {
		Some(J::Int(x))	=> *x += by,
		_				=> panic!("pass {} has no integer at {}", pass + 1, path.join(".")),
	}
}

#[test]
fn a_record_whose_sub_phases_or_calls_do_not_add_up_is_refused() {
	let sound = json::parse(&runs().record).expect("the record is JSON");
	assert!(split_faults(&sound, false).is_empty(), "the unmutated record is sound");
	// A sub-phase holding a thousand nanoseconds that `realise` does not.
	let mut r = sound.clone();
	add_in_pass(&mut r, 0, &["sub", "show", "ns"], 1_000);
	let found = split_faults(&r, false);
	assert!(found.iter().any(|f| f.contains("pass 1 realise holds") && f.contains("sub-phases sum to")), "{:#?}", found);
	// An entry that `self` counts and `realise` does not.
	let mut r = sound.clone();
	add_in_pass(&mut r, 1, &["sub", "self", "n"], 1);
	let found = split_faults(&r, false);
	assert!(found.iter().any(|f| f.contains("pass 2") && f.contains("`self` says")), "{:#?}", found);
	// A coarse record that clocked a phase entered for every element, and a fine one that did not.
	let mut r = sound.clone();
	add_in_pass(&mut r, 0, &["sub", "styles", "ns"], 5);
	let found = split_faults(&r, false);
	assert!(found.iter().any(|f| f.contains("pass 1 clocked styles in a coarse record")), "{:#?}", found);
	let found = split_faults(&sound, true);
	assert!(found.iter().any(|f| f.contains("left rules unclocked in a fine record")), "{:#?}", found);
	// A call in pass one with a cause that only a later pass can give it, and no entry behind it.
	let mut r = sound.clone();
	let label = match sound.get("pass") {
		Some(J::Arr(v))	=> match v[0].get("calls") {
			Some(J::Obj(kv))	=> kv[0].0.clone(),
			_					=> panic!("pass 1 has no calls"),
		},
		_				=> panic!("the record has no pass array"),
	};
	add_in_pass(&mut r, 0, &["calls", &label, "repass"], 1);
	let found = split_faults(&r, false);
	assert!(found.iter().any(|f| f.contains("pass 1 has") && f.contains("repass calls")), "{:#?}", found);
	assert!(found.iter().any(|f| f.contains("pass 1 counts") && f.contains("realise calls")), "{:#?}", found);
	// A first call after pass one.
	let mut r = sound.clone();
	add_in_pass(&mut r, 1, &["calls", &label, "first"], 1);
	let found = split_faults(&r, false);
	assert!(found.iter().any(|f| f.contains("pass 2 has") && f.contains("first calls")), "{:#?}", found);
}

// An engine given no recorder holds none after a whole compile, so the sub-phases and counters cost a test
// of the option and nothing more; one given a recorder holds it, with its passes.
#[test]
fn the_engine_holds_no_recorder_unless_it_was_given_one() {
	let dir = case_dir("lib");
	let main = dir.join("main.typ");
	std::fs::write(&main, DOC).expect("the case's source");
	let mut sink = PdfSink::new().expect("a PDF sink");
	let plain = compile::assemble_eval(&main, &dir, FontStore::default(), &mut sink).expect("the plain compile");
	assert!(plain.engine.timings.is_none(), "a compile without --timings made a recorder");
	let mut sink = PdfSink::new().expect("a PDF sink");
	let timed = compile::assemble_eval_timed(&main, &dir, FontStore::default(), &mut sink, Some(Timings::start()), None)
		.expect("the timed compile");
	let t = timed.engine.timings.as_ref().expect("the recorder stays in the engine");
	assert!(t.passes() >= 2, "the forward read takes a second pass: {}", t.passes());
}
