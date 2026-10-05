//! The per-page carry-state record of `austenite --eval --timings FILE.json` (speed programme U1).
//!
//! Each case compiles synthetic documents with the built binary and compares the `pages` list of the last
//! fixpoint pass. A record holds, for every page, the fingerprint of the pairs it was made from and one
//! fingerprint for each part of the state it began in. The cases pin that a part of the state the walk is
//! meant to carry does change the record where the documents differ: a footnote that spills onto the next
//! page, and the unplaced lines of a paragraph that splits over a page break. And that a one-letter edit
//! changes nothing after the page it is on, which only holds if spans and locations are masked.

#[allow(dead_code)]
#[path = "eval_oracle/json.rs"]
mod json;

use json::J;

use std::path::PathBuf;
use std::process::Command;

struct Page {
	pairs:	i64,
	input:	String,
	entry:	Vec<(String, String)>,	// the entry's fields in the order the record names them
}

impl Page {
	fn field(&self, name: &str) -> &str {
		match self.entry.iter().find(|(k, _)| k == name) {
			Some((_, v))	=> v,
			None			=> panic!("the entry has no field {:?}: {:?}", name, self.entry),
		}
	}
}

fn case_dir(name: &str) -> PathBuf {
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("eval_carry").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).expect("the case's directory");
	dir
}

// The pages of the last pass of one compile of `doc`.
fn pages(case: &str, doc: &str) -> Vec<Page> {
	let dir = case_dir(case);
	let (main, rec, out) = (dir.join("main.typ"), dir.join("record.json"), dir.join("out"));
	std::fs::write(&main, doc).expect("the case's source");
	let run = Command::new(env!("CARGO_BIN_EXE_austenite"))
		.args(["--eval", "--timings", rec.to_str().expect("a path"), main.to_str().expect("a path"), out.to_str().expect("a path")])
		.output()
		.expect("the built austenite binary");
	assert!(run.status.success(), "the compile succeeds:\n{}", String::from_utf8_lossy(&run.stderr));
	let record = json::parse(&std::fs::read_to_string(&rec).expect("the record")).expect("the record is JSON");
	let last = match record.get("pages") {
		Some(J::Arr(passes))	=> passes.last().expect("a pass"),
		_						=> panic!("the record has no pages array"),
	};
	let list = match last {
		J::Arr(v)	=> v,
		_			=> panic!("a pass's pages are not a list"),
	};
	list.iter().map(|p| {
		let entry = match p.get("entry") {
			Some(J::Obj(kv))	=> kv.iter().map(|(k, v)| (k.clone(), v.render())).collect(),
			_					=> panic!("a page has no entry"),
		};
		Page {
			pairs:	p.get("pairs").and_then(|v| v.as_i64()).expect("the pair count"),
			input:	p.get("input").and_then(|v| v.as_str()).expect("the input fingerprint").to_string(),
			entry,
		}
	}).collect()
}

const SPILL_HEAD: &str = "#set page(width: 200pt, height: 150pt, margin: 16pt)\n";
const FILLER: &str = "Filler paragraph text sets some words so the document runs on to more pages than one.\n\n";

// A footnote on the first page, with `note` as its text, and enough after it for more pages.
fn footnoted(note: &str) -> String {
	format!("{}First page text, with a note.#footnote[{}] More first page text.\n\n{}", SPILL_HEAD, note, FILLER.repeat(8))
}

#[test]
fn a_document_compiled_twice_has_the_same_page_records() {
	let doc = footnoted("A short note.");
	let (a, b) = (pages("twice_a", &doc), pages("twice_b", &doc));
	assert!(a.len() >= 3, "the document makes several pages: {}", a.len());
	assert_eq!(a.len(), b.len(), "the same page count");
	for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
		assert_eq!((x.pairs, &x.input, &x.entry), (y.pairs, &y.input, &y.entry), "page {} differs between two compiles", i + 1);
	}
}

#[test]
fn a_footnote_that_spills_onto_the_next_page_changes_the_entry_there() {
	let long = "A long note that goes on and on, with words enough to fill more than the room the first page leaves. ".repeat(12);
	let (short, spilling) = (pages("note_short", &footnoted("A short note.")), pages("note_long", &footnoted(&long)));
	// The first page begins before any flow is open, so its fields are the empty ones; take the second.
	let none = short[1].field("foot_spill").to_string();
	assert!(short.iter().skip(1).all(|p| p.field("foot_spill") == none), "no spill in the short note's document");
	let at = spilling.iter().skip(1).position(|p| p.field("foot_spill") != none);
	assert!(at.is_some(), "some page of the long note's document begins with a spilled footnote: {:?}",
		spilling.iter().map(|p| p.field("foot_spill")).collect::<Vec<_>>());
	}

// A paragraph of `words` words, the last of which is `last`, on pages short enough that it splits.
fn splitting(last: &str) -> String {
	let body = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november ".repeat(14);
	format!("#set page(width: 200pt, height: 110pt, margin: 12pt)\n{}{}\n\nAfter.\n", body, last)
}

#[test]
fn a_paragraph_split_across_a_page_break_carries_its_unplaced_lines() {
	let (a, b) = (pages("split_a", &splitting("oscar")), pages("split_b", &splitting("papap")));
	assert!(a.len() >= 3 && a.len() == b.len(), "the paragraph runs over several pages: {} and {}", a.len(), b.len());
	let at = a.iter().zip(b.iter()).position(|(x, y)| x.field("children") != y.field("children"));
	assert!(at.is_some(), "the unplaced lines differ at some page: {:?}",
		a.iter().map(|p| p.field("children")).collect::<Vec<_>>());
	assert_eq!(at, Some(1), "the lines left after the first page are the first to differ");
}

// Paragraphs of a few lines each, the first of which carries `mark` as a number.
fn paragraphs(mark: u32) -> String {
	let mut doc = String::from("#set page(width: 200pt, height: 160pt, margin: 16pt)\n");
	for i in 1..=30u32 {
		let n = if i == 1 { mark } else { 11 };
		doc.push_str(&format!(
			"Paragraph {} carries the number {} among words that set the paragraph into several lines on the page.\n\n", i, n));
	}
	doc
}

#[test]
fn a_one_letter_edit_leaves_every_page_after_the_edited_one_equal() {
	let (before, after) = (pages("edit_before", &paragraphs(11)), pages("edit_after", &paragraphs(12)));
	assert!(before.len() >= 6 && before.len() == after.len(), "several pages, the same count: {} and {}", before.len(), after.len());
	assert!(before[0].input != after[0].input, "the edited page's own input differs");
	for i in 3..before.len() {
		assert_eq!(
			(before[i].pairs, &before[i].input, &before[i].entry),
			(after[i].pairs, &after[i].input, &after[i].entry),
			"page {} differs after a one-letter edit on the first page", i + 1);
	}
}
