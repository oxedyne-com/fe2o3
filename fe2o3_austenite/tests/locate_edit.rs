//! Locations under a keystroke (speed programme U6): a location is made from where the element stands, its
//! kind, what it is and its ordinal among equals there, never from where it was written. Two compiles one
//! letter apart therefore differ in location only where the content differs (the edited element and the
//! elements that hold it) and under an element that lays a body out and holds the edit, whose place moves
//! and takes its body with it. Everything else keeps its location.
//!
//! Each case compiles a document, types one letter into it, compiles again, and lines the two compiles'
//! located elements up in document order (the structure is the same, only a letter differs). An element is
//! edited when its source span holds the letter. Three things must hold of every pair of compiles:
//!
//!  1. an edited element whose content differs has a new location, so a location never stands for two
//!     contents (an element that merely reads what the edit changed, as a reference reads its target's
//!     text, is not edited and keeps its location);
//!  2. an element of the same content moves only under a changed element that lays a body out, whose own
//!     location moved and so took its place with it; and
//!  3. the elements outside the edit's section of the document do not move at all (the synthetic case).
//!
//! `AUST_LOC_DOCS=a.typ,b.typ cargo test --release --test locate_edit -- --ignored --nocapture` runs the same
//! comparison over whole documents, at `AUST_LOC_EDITS` (default 6) body paragraphs spread through each.

use oxedyne_fe2o3_austenite::compile::Session;
use oxedyne_fe2o3_austenite::emit::sinks::CountSink;
use oxedyne_fe2o3_austenite::eval::content::ElemKind;
use oxedyne_fe2o3_austenite::flow::text::FontStore;
use oxedyne_fe2o3_austenite::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

// The compiles

// A located element as one compile placed it.
#[derive(Clone, Debug, PartialEq)]
struct Rec {
	loc:	u64,
	kind:	Option<ElemKind>,	// none for a sequence
	fp:		u128,
	place:	bool,				// lays a body out, so gives what it holds a place of its own
	span:	Span,
}

fn dir(name: &str) -> Outcome<PathBuf> {
	let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("locate_edit").join(name);
	let _ = std::fs::remove_dir_all(&d);
	res!(std::fs::create_dir_all(&d));
	Ok(res!(std::fs::canonicalize(&d)))
}

/// The located elements of `src`, compiled cold in a directory of its own, in document order.
fn compiled(name: &str, src: &str) -> Outcome<Vec<Rec>> {
	let root: PathBuf	= res!(dir(name));
	let main: PathBuf	= root.join("main.typ");
	res!(std::fs::write(&main, src));
	let mut session		= Session::new(FontStore::default());
	let done			= res!(session.compile(&main, &root, &mut CountSink::default(), None, None, true));
	let laid			= res!(done.laid.map_err(|e| err!("{}: the compile failed: {}", name, e.plain(); Test)));
	Ok(laid.intro.iter().filter_map(|r| r.elem.location().map(|l| Rec {
		loc:	l.0,
		kind:	r.elem.kind(),
		fp:		r.elem.fingerprint().as_u128(),
		place:	r.elem.place().is_some(),
		span:	r.elem.span(),
	})).collect())
}

// The edits

/// Types `letter` into `text` at byte `at`.
fn typed(text: &str, at: usize, letter: &str) -> String {
	let mut s = String::with_capacity(text.len() + letter.len());
	s.push_str(&text[..at]);
	s.push_str(letter);
	s.push_str(&text[at..]);
	s
}

/// Where a letter is typed to edit each body paragraph and each heading: just after the first word of a line
/// that follows a blank line, a heading or the top of the file, as the incremental harness does.
fn sites(text: &str) -> Vec<usize> {
	let mut out		= Vec::new();
	let mut off		= 0;
	let mut after	= true;
	for line in text.split('\n') {
		let b		= line.as_bytes();
		let first	= b.first().copied().unwrap_or(b' ');
		let heading	= line.starts_with("= ") || line.starts_with("== ");
		if after && (first.is_ascii_uppercase() || heading) {
			let skip	= if heading { line.find(' ').map(|i| i + 1).unwrap_or(0) } else { 0 };
			let w		= b[skip..].iter().take_while(|c| c.is_ascii_alphanumeric()).count();
			if w > 0 && skip + w < b.len() && b[skip + w] == b' ' {
				out.push(off + skip + w);
			}
		}
		after = line.trim().is_empty() || line.starts_with('=');
		off += line.len() + 1;
	}
	out
}

// The comparison

/// What differs between two compiles one letter apart, as indices into the located elements.
#[derive(Debug, Default)]
struct Diff {
	n:			usize,
	edited:		Vec<usize>,	// the source span holds the letter
	readers:	Vec<usize>,	// not edited, but the content differs: a reference or an outline entry carrying the text
	moved:		Vec<usize>,	// not edited, the content the same, the location not
	holders:	Vec<usize>,	// edited or reading, moved, and laying a body out: whatever follows them may move
	shifted:	Vec<usize>,	// every element whose location differs
}

// Does the span of an element of the compile before the edit hold a letter typed at byte `at`?
fn holds(s: Span, at: usize) -> bool { !s.is_detached() && (s.start as usize) < at && at <= s.end as usize }

fn diff(name: &str, a: &[Rec], b: &[Rec], at: usize) -> Outcome<Diff> {
	if a.len() != b.len() {
		return Err(err!("{}: {} located elements before the edit and {} after.", name, a.len(), b.len(); Test));
	}
	let mut d = Diff { n: a.len(), ..Diff::default() };
	for (i, (x, y)) in a.iter().zip(b).enumerate() {
		if x.kind != y.kind {
			return Err(err!("{}: element {} is {:?} before the edit and {:?} after.", name, i, x.kind, y.kind; Test));
		}
		let edited = holds(x.span, at);
		if x.loc != y.loc {
			d.shifted.push(i);
		}
		if edited {
			d.edited.push(i);
			if x.fp != y.fp && x.loc == y.loc {
				return Err(err!("{}: element {} ({:?}) holds the edit, has different content and kept its location.",
					name, i, y.kind; Test));
			}
		} else if x.fp != y.fp {
			d.readers.push(i);
		} else if x.loc != y.loc {
			d.moved.push(i);
		}
		if (edited || x.fp != y.fp) && x.loc != y.loc && y.place {
			d.holders.push(i);
		}
	}
	Ok(d)
}

/// An element whose content is the same moves only under one that changed, moved and lays a body out.
fn check(name: &str, b: &[Rec], d: &Diff) -> Outcome<()> {
	for &i in &d.moved {
		if !d.holders.iter().any(|&j| j < i) {
			return Err(err!("{}: element {} ({:?}) moved and is held by no changed element that lays a body out; \
				edited {:?}, reading {:?}, moved {:?}.", name, i, b[i].kind, d.edited, d.readers, d.moved; Test));
		}
	}
	Ok(())
}

// The cases

const FIXTURES: &[&str] = &[
	"counters_context", "floats", "footnote_spill", "headings_refs_outline",
	"page_columns", "pagebreak_parity", "table_split",
];

fn fixture(name: &str) -> Outcome<String> {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/incremental").join(fmt!("{}.typ", name));
	Ok(res!(std::fs::read_to_string(&path)))
}

#[test]
fn a_letter_typed_into_a_fixture_moves_only_what_holds_it() -> Outcome<()> {
	let mut cases	= 0;
	let mut edited	= 0;
	let mut total	= Diff::default();
	for name in FIXTURES {
		let base	= res!(fixture(name));
		let before	= res!(compiled(name, &base));
		assert!(!before.is_empty(), "{}: no located element at all", name);
		for (k, at) in sites(&base).into_iter().enumerate() {
			let label	= fmt!("{} site {}", name, k);
			let after	= res!(compiled(&label.replace(' ', "_"), &typed(&base, at, "q")));
			let d		= res!(diff(&label, &before, &after, at));
			res!(check(&label, &after, &d));
			cases		+= 1;
			edited		+= d.edited.len().min(1);
			total.n			+= d.n;
			total.readers.extend(d.readers.iter().map(|_| 0));
			total.moved.extend(d.moved.iter().map(|_| 0));
			total.shifted.extend(d.shifted.iter().map(|_| 0));
		}
	}
	println!("{} edits, {} located elements compared: {} shifted, of which {} read the edit and {} moved under it",
		cases, total.n, total.shifted.len(), total.readers.len(), total.moved.len());
	assert!(cases >= 20, "only {} cases ran", cases);
	assert!(edited >= 10, "only {} of the {} edits changed an element that is located", edited, cases);
	Ok(())
}

/// A document of forty sections, each a heading, a paragraph with a footnote and a labelled table. A letter in
/// one section's paragraph leaves every other section's elements where they were.
fn sections(n: usize, tag: &str) -> String {
	let mut s = String::from("#set page(width: 220pt, height: 130pt, margin: 16pt)\n#set text(size: 9pt)\n");
	for i in 0..n {
		s.push_str(&fmt!("\n= Section {}\n\nParagraph {} has words of its own{} and a note#footnote[Note {}.] to close.\n\n\
			#table(columns: 2, [A{}], [B{}]) <t{}>\n", i, i, if i == 17 { tag } else { "" }, i, i, i, i));
	}
	s
}

#[test]
fn a_letter_typed_into_one_section_leaves_the_other_sections_where_they_were() -> Outcome<()> {
	let (plain, typed)	= (sections(40, ""), sections(40, "q"));
	let at				= plain.bytes().zip(typed.bytes()).take_while(|(x, y)| x == y).count();
	let before	= res!(compiled("sections_a", &plain));
	let after	= res!(compiled("sections_b", &typed));
	let d		= res!(diff("sections", &before, &after, at));
	res!(check("sections", &after, &d));
	assert!(!d.edited.is_empty(), "the letter changed no located element");
	// A section is a heading, a footnote and a table; the table is a placed element, so it takes its cells.
	let per = before.len() / 40;
	assert!(per >= 3, "a section holds {} located elements", per);
	let outside = d.shifted.iter().filter(|&&i| i / per != 17).count();
	assert!(outside <= 2, "{} elements outside section 17 moved ({:?}), of {}", outside, d.shifted, d.n);
	assert!(d.shifted.len() <= per + 2, "{} of {} elements changed location", d.shifted.len(), d.n);
	Ok(())
}

/// Equal elements are told apart by their ordinal among the equals at one place, as Typst's locator does, so
/// an edit that makes one of them unequal renumbers the equal ones after it. This pins that reach: the later
/// equals move, the earlier ones and the unequal ones do not.
#[test]
fn an_edited_element_renumbers_only_the_equal_ones_after_it() -> Outcome<()> {
	let doc = |q: &str| {
		let mut s = String::from("#set page(width: 220pt, height: 400pt, margin: 16pt)\n= Title\n\n");
		for i in 0..5 {
			s.push_str(&fmt!("{}\n\n", if i == 1 { fmt!("Same words{} here.", q) } else { fmt!("Same words here.") }));
		}
		s.push_str("= Another\n");
		s
	};
	let (plain, typed) = (doc(""), doc("q"));
	let at			= plain.bytes().zip(typed.bytes()).take_while(|(x, y)| x == y).count();
	let before		= res!(compiled("equals_a", &plain));
	let after		= res!(compiled("equals_b", &typed));
	let d			= res!(diff("equals", &before, &after, at));
	// The paragraphs are the located elements of kind Par; the second is edited.
	let pars: Vec<usize> = before.iter().enumerate().filter(|(_, r)| r.kind == Some(ElemKind::Par)).map(|(i, _)| i).collect();
	assert_eq!(pars.len(), 5, "the document has {} paragraphs of the five typed", pars.len());
	assert!(d.shifted.contains(&pars[1]), "the edited paragraph kept its location");
	assert!(!d.shifted.contains(&pars[0]), "the equal paragraph before the edit moved");
	for &p in &pars[2..] {
		assert!(d.shifted.contains(&p), "an equal paragraph after the edit kept its ordinal");
	}
	let heads: Vec<usize> = before.iter().enumerate().filter(|(_, r)| r.kind == Some(ElemKind::Heading)).map(|(i, _)| i).collect();
	assert!(heads.iter().all(|h| !d.shifted.contains(h)), "a heading moved");
	Ok(())
}

/// Whole documents, named in `AUST_LOC_DOCS`, edited at `AUST_LOC_EDITS` paragraphs spread through each.
#[test]
#[ignore]
fn a_letter_typed_into_a_document_moves_only_what_holds_it() -> Outcome<()> {
	let docs	= res!(std::env::var("AUST_LOC_DOCS").map_err(|_| err!("Set AUST_LOC_DOCS to a list of .typ files."; Test)));
	let edits: usize = std::env::var("AUST_LOC_EDITS").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
	for doc in docs.split(',') {
		let base	= res!(std::fs::read_to_string(doc));
		let all		= sites(&base);
		assert!(!all.is_empty(), "{}: no site to edit", doc);
		let before	= res!(compiled("doc_a", &base));
		for k in 0..edits {
			let at		= all[k * all.len() / edits];
			let after	= res!(compiled("doc_b", &typed(&base, at, "q")));
			let d		= res!(diff(doc, &before, &after, at));
			res!(check(doc, &after, &d));
			println!("{}: edit {} at byte {}: {} located, {} edited, {} reading, {} moved, {} in all", doc, k, at, d.n,
				d.edited.len(), d.readers.len(), d.moved.len(), d.shifted.len());
		}
	}
	Ok(())
}
