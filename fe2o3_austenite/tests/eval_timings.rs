//! `austenite --eval --timings FILE.json`: the wall time of each phase of a compile.
//!
//! Each case compiles a synthetic document with the built binary. The record is judged on what it must
//! always hold: the PDF is the same bytes with the recorder on or off, every phase of every fixpoint pass
//! was entered, and the phases add up to the run's wall time within a tenth.

#[allow(dead_code)]
#[path = "eval_oracle/json.rs"]
mod json;

use json::J;

use oxedyne_fe2o3_austenite::timings::Phase;

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

// A forward read, the page total on its first page, so the fixpoint takes more than one pass; furniture on
// every page; a show rule, so realisation runs user code; and enough pages for each phase to be measurable.
const DOC: &str = "\
#set page(width: 220pt, height: 260pt, margin: 24pt, header: [Header], footer: context counter(page).display())
#show heading: it => block(fill: luma(235), inset: 4pt, it.body)
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
		Runs {
			plain:	std::fs::read(plain.join("document.pdf")).expect("the plain PDF"),
			timed:	std::fs::read(timed.join("document.pdf")).expect("the timed PDF"),
			record:	std::fs::read_to_string(&record).expect("the record"),
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
	out
}

#[test]
fn the_pdf_is_the_same_bytes_with_the_recorder_on_or_off() {
	let r = runs();
	assert!(r.plain.starts_with(b"%PDF-"), "the plain output is a PDF");
	assert!(r.plain.len() > 4_000, "the document made real pages: {} bytes", r.plain.len());
	assert!(r.plain == r.timed, "--timings changed the PDF ({} bytes against {})", r.plain.len(), r.timed.len());
}

#[test]
fn the_record_holds_every_phase_of_every_pass_and_the_phases_sum_to_the_wall_time() {
	let record = json::parse(&runs().record).expect("the record is JSON");
	assert!(num(&record, &["passes"]) >= 2, "the forward read takes a second pass: {}", runs().record);
	let found = faults(&record);
	assert!(found.is_empty(), "the record is unsound: {:#?}\n{}", found, runs().record);
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
	assert!(!rec.exists(), "a refused run writes no record");
}
