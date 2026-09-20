//! The incremental memo, proved on the native recompile path.
//!
//! The master goal is to swap Austenite in for Daimond's Typst-wasm compiler; the blocker is that a
//! straight swap re-authors and re-emits the whole document on every keystroke (~15x the live-view
//! latency). The [`memo`](oxedyne_fe2o3_austenite::memo) layer caches the two costly content-addressed
//! stages -- block authoring and page emit -- so an edit recomputes only what changed. These tests hold
//! that memo to the one standard that matters: the memo must never change a single output byte. A warm
//! recompile after an edit must be byte-identical to a cold compile of the edited source, while all but
//! the one edited block hit the authoring cache and every page outside the edit's pagination cascade hits
//! the emit cache.

use oxedyne_fe2o3_austenite::bib::Bibliography;
use oxedyne_fe2o3_austenite::compile::{
	author_and_run_memo,
	Assembled,
};
use oxedyne_fe2o3_austenite::doc::{
	Block,
	Segment,
};
use oxedyne_fe2o3_austenite::emit::svg;
use oxedyne_fe2o3_austenite::fonts::{
	self,
	FaceResolver,
};
use oxedyne_fe2o3_austenite::memo::Memo;
use oxedyne_fe2o3_austenite::page::PageGeometry;
use oxedyne_fe2o3_austenite::theme::Theme;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;
use std::time::Instant;

/// A deterministic body of prose paragraphs. Each is a few sentences of varying-length words, enough to
/// break into several lines and paginate over many pages, so a mid-document edit has both earlier pages
/// to leave untouched and later pages to cascade. `edit_at` names a paragraph to rewrite, standing in for
/// the one edit a keystroke makes; `None` is the untouched source.
fn build_doc(n: usize, edit_at: Option<usize>) -> Vec<Block> {
	let mut blocks = Vec::with_capacity(n);
	for k in 0..n {
		blocks.push(Block::paragraph(paragraph_text(k, edit_at == Some(k))));
	}
	blocks
}

/// One paragraph's text, keyed by its index so the corpus is stable run to run and, crucially, unique per
/// paragraph: each opens with its own ordinal word. Uniqueness matters to the hit-count assertions -- the
/// authoring memo is content-addressed, so two paragraphs of identical text would legitimately share one
/// cache entry (a real and desirable property: a moved or duplicated paragraph reuses its authored nodes),
/// which would blur "one edit, one miss". An edited paragraph swaps in a different but similarly shaped
/// body, so it genuinely re-breaks and re-paginates from that point -- the honest case for the cascade.
fn paragraph_text(k: usize, edited: bool) -> String {
	let words = [
		"typesetting", "streams", "a", "document", "through", "two", "passes", "of", "the", "driver",
		"while", "the", "ledger", "resolves", "each", "anchor", "into", "the", "page", "it", "landed",
		"on", "and", "the", "breaker", "sets", "every", "justified", "line", "to", "the", "measure",
		"without", "a", "single", "floating", "point", "along", "the", "way", "so", "the", "build",
		"stays", "byte", "identical", "from", "one", "run", "to", "the", "very", "next", "one", "here",
	];
	let seed = if edited { k * 7 + 3 } else { k * 5 + 1 };
	let count = 34 + (seed % 20);
	// The unique ordinal opener, distinct for every paragraph and for an edited one against its original.
	let mut s = fmt!("Paragraph{}{}.", k, if edited { "edited" } else { "" });
	for i in 0..count {
		s.push(' ');
		s.push_str(words[(seed + i * 3) % words.len()]);
	}
	s.push('.');
	s
}

/// Authors, runs and emits a document to a vector of per-page SVG strings, threading the memo (when given)
/// through both the authoring stage and the page emit, exactly as the native `--watch` path would. `None`
/// is the plain, un-memoised compile -- the honest cold baseline, and the byte reference every memo path
/// is held against. `bib` threads a marked bibliography through exactly as a real compile does (see
/// `book.rs:662-665`), so a `#cite` segment resolves and the cited set folds into the memo's global
/// fingerprint; plain callers with no citations pass `None`.
fn compile_pages(blocks: Vec<Block>, bib: Option<Bibliography>, memo: Option<&mut Memo>) -> Outcome<Vec<String>> {
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let geom	= PageGeometry::a4();
	let assembled = Assembled {
		blocks,
		fonts,
		geom,
		style:	Theme::default(),
		title:	String::new(),
		faces:	FaceResolver::default(),
		front:	None,
		bib,
	};
	let mut memo	= memo;
	let rendered	= res!(author_and_run_memo(assembled, memo.as_deref_mut()));
	let mut svgs	= Vec::with_capacity(rendered.out.pages.len());
	for page in &rendered.out.pages {
		let svg = match memo.as_deref_mut() {
			Some(m)	=> res!(svg::render_page_memo(page, m)),
			None	=> res!(svg::render_page(page)),
		};
		svgs.push(svg);
	}
	// The caller closes the generation once the pages are emitted, so the page cache survives to be reused.
	if let Some(m) = memo.as_deref_mut() {
		m.sweep();
	}
	Ok(svgs)
}

/// The heart of the gate: a warm recompile after a one-paragraph edit is byte-identical to a cold compile
/// of the edited source, every unedited block hits the authoring cache, and every page the edit does not
/// reach hits the emit cache.
#[test]
fn warm_recompile_is_byte_identical_after_a_one_paragraph_edit() -> Outcome<()> {
	let n		= 40;
	let edit_at	= 34;	// late, so many earlier pages fall outside the pagination cascade

	// The cold references: the original source and the edited source, each compiled with no memo at all.
	let cold_orig	= res!(compile_pages(build_doc(n, None), None, None));
	let cold_edited	= res!(compile_pages(build_doc(n, Some(edit_at)), None, None));

	// Prime a memo on the original, then recompile the edited source warm against it.
	let mut memo = Memo::new();
	let warm_orig = res!(compile_pages(build_doc(n, None), None, Some(&mut memo)));
	assert_eq!(warm_orig, cold_orig,
		"a first, cold compile through the memo must be byte-identical to a compile with no memo");
	// After the priming compile every block was a miss (the cache was empty) and every page was rendered.
	assert_eq!(memo.block_hits, 0, "the priming compile fills the cache, so nothing hits");
	assert_eq!(memo.block_misses as usize, n, "every block is authored on the priming compile");

	let warm_edited = res!(compile_pages(build_doc(n, Some(edit_at)), None, Some(&mut memo)));

	// (1) Byte identity -- the one property the memo must never break.
	assert_eq!(warm_edited, cold_edited,
		"a warm recompile after an edit must be byte-identical to a cold compile of the edited source");

	// (2) The authoring cache: only the edited block misses; every other block splices its cached nodes.
	assert_eq!(memo.block_misses, 1,
		"only the one edited block should miss the authoring cache, found {} misses", memo.block_misses);
	assert_eq!(memo.block_hits as usize, n - 1,
		"every block but the edited one should hit the authoring cache, found {} hits", memo.block_hits);

	// (3) The emit cache: every page the edit does not reach must hit. A page is provably outside the
	// cascade exactly when its cold SVG is unchanged by the edit; that is a rigorous lower bound on the
	// page hits the warm compile must score, since such a page's body frame is byte-identical to the one
	// the priming compile cached.
	let untouched = cold_orig.iter().zip(cold_edited.iter()).filter(|(a, b)| a == b).count();
	assert!(untouched > 0, "the edit must be late enough to leave earlier pages untouched");
	assert!(memo.page_hits as usize >= untouched,
		"every page outside the cascade must hit the emit cache: {} hits < {} untouched pages",
		memo.page_hits, untouched);
	assert_eq!((memo.page_hits + memo.page_misses) as usize, warm_edited.len(),
		"every page is either a hit or a miss");
	assert!(memo.page_misses >= 1, "the edited region must re-render at least one page");

	eprintln!(
		"[memo] {} blocks over {} pages: warm edit -> block {}/{} hit, page {}/{} hit ({} untouched)",
		n, warm_edited.len(), memo.block_hits, n - 1,
		memo.page_hits, warm_edited.len(), untouched);
	Ok(())
}

/// A warm recompile over a document of headings and paragraphs -- where a chapter heading keeps the first
/// line of the paragraph it introduces, consuming two source blocks as one authored unit -- must still be
/// byte-identical to a cold compile of the edited source. This exercises the splice of a cached heading
/// unit on a hit, the path plain prose never reaches.
#[test]
fn warm_recompile_is_byte_identical_with_headings() -> Outcome<()> {
	let doc = |edit: bool| -> Vec<Block> {
		let mut v = Vec::new();
		for c in 0..4 {
			v.push(Block::heading(1, fmt!("Chapter{}", c)));
			for p in 0..6 {
				let k = c * 6 + p;
				// Edit one paragraph in the third chapter, well clear of any heading's kept first line.
				v.push(Block::paragraph(paragraph_text(k, edit && k == 15)));
			}
		}
		v
	};
	let cold_edited	= res!(compile_pages(doc(true), None, None));
	let mut memo	= Memo::new();
	let _		= res!(compile_pages(doc(false), None, Some(&mut memo)));	// prime on the original
	let warm	= res!(compile_pages(doc(true), None, Some(&mut memo)));
	assert_eq!(warm, cold_edited,
		"a warm recompile with chapter headings must be byte-identical to a cold compile of the edit");
	assert!(memo.block_misses >= 1, "the edited paragraph must miss");
	assert!(memo.block_hits >= 1, "the unedited blocks, headings included, must hit");
	Ok(())
}

/// A cold compile with the memo present-but-empty must match a compile with no memo, page for page, over
/// a document exercising more than plain prose -- headings, a list and a code block -- so the byte-identity
/// guarantee is not resting on paragraphs alone.
#[test]
fn a_cold_memo_compile_matches_a_no_memo_compile_on_mixed_blocks() -> Outcome<()> {
	let blocks = || vec![
		Block::heading(1, "The Streaming Driver"),
		Block::paragraph(paragraph_text(0, false)),
		Block::paragraph(paragraph_text(1, false)),
		Block::heading(2, "Line Breaking"),
		Block::paragraph(paragraph_text(2, false)),
		Block::list(false, vec![]),
		Block::code(vec!["let x = 1;".to_string(), "let y = x + 1;".to_string()]),
		Block::paragraph(paragraph_text(3, false)),
	];
	let no_memo		= res!(compile_pages(blocks(), None, None));
	let mut memo	= Memo::new();
	let with_memo	= res!(compile_pages(blocks(), None, Some(&mut memo)));
	assert_eq!(no_memo, with_memo,
		"a present-but-cold memo must not change one output byte over mixed block kinds");
	Ok(())
}

/// The measurement the swap rests on: cold (no memo) versus warm (one-paragraph edit through the memo)
/// wall time on a document of a few hundred pages. Not an assertion -- machines differ -- but it prints
/// the speedup the live-view loop would see. Run with `--nocapture` to read it.
#[test]
fn measure_cold_versus_warm_recompile() -> Outcome<()> {
	let n		= 260;
	let edit_at	= 130;

	// Cold: a full compile with no memo, the latency a straight swap would pay on every edit.
	let t0		= Instant::now();
	let cold	= res!(compile_pages(build_doc(n, None), None, None));
	let cold_ms	= t0.elapsed().as_secs_f64() * 1000.0;

	// Prime the memo on the original document (the first open of the document, not the edit loop).
	let mut memo	= Memo::new();
	let _		= res!(compile_pages(build_doc(n, None), None, Some(&mut memo)));

	// Warm: the edit-and-re-render a keystroke triggers.
	let t1		= Instant::now();
	let warm	= res!(compile_pages(build_doc(n, Some(edit_at)), None, Some(&mut memo)));
	let warm_ms	= t1.elapsed().as_secs_f64() * 1000.0;

	let cold_ref = res!(compile_pages(build_doc(n, Some(edit_at)), None, None));
	assert_eq!(warm, cold_ref, "the measured warm recompile must still be byte-identical");

	eprintln!(
		"[memo] {} pages: cold {:.1} ms, warm {:.1} ms, speedup {:.1}x (block {}/{} hit, page {}/{} hit)",
		cold.len(), cold_ms, warm_ms, cold_ms / warm_ms.max(0.001),
		memo.block_hits, n - 1, memo.page_hits, warm.len());
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE LEDGER GATE: pagination-shifting edits against folios, cites, glossary │
// └───────────────────────────────────────────────────────────────────────────┘
//
// The tests above hold the memo to byte-identity over plain prose and headings, where an edit only ever
// lengthens or shortens text. The gate below tests the sharper claim: that byte-identity survives an edit
// that shifts which PAGE later content lands on (so folios printed in the furniture move) and an edit that
// moves WHICH block first uses a glossary term (so the term's bold-italic first-use markup moves). Both are
// ledger facts -- resolved after the driver's pages converge -- that the memo must never freeze stale.

/// Three entries, two of them Zuboff works dated 2019, so the reference list and the in-text label both
/// exercise the year-suffix disambiguation (`2019a`/`2019b`) the way a real bibliography does.
const LEDGER_BIB: &str = r#"
@book{scott1976,
  author    = {Scott, James C.},
  title     = {The Moral Economy of the Peasant},
  year      = {1976},
  publisher = {Yale University Press}
}
@book{zuboff2019a,
  author    = {Zuboff, Shoshana},
  title     = {The Age of Surveillance Capitalism},
  year      = {2019},
  publisher = {PublicAffairs}
}
@book{zuboff2019b,
  author    = {Zuboff, Shoshana},
  title     = {Big Other},
  year      = {2019},
  publisher = {Profile Books}
}
"#;

/// Parses [`LEDGER_BIB`] and marks `cited` as cited, mirroring `book.rs:662-665` -- the bibliography must
/// enter the assembled doc with its cited set already settled, not built up as the body walk goes.
fn ledger_bib(cited: &[&str]) -> Outcome<Bibliography> {
	let mut bib = res!(Bibliography::parse(LEDGER_BIB));
	for k in cited {
		bib.mark_cited(k);
	}
	Ok(bib)
}

/// The three ledger-fixture variants. `Orig` and `Shift` differ only in the edit site's length; `Orig` and
/// `DropFirstUse` differ only in whether the first chapter's rich paragraph carries the glossary segment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Variant { Orig, Shift, DropFirstUse }

/// The sentence the edit site repeats under `Shift`. Forty repeats is long enough to add a whole extra
/// page and push every later block -- the rest of chapter one, the two rich paragraphs, chapter two and
/// the back matter -- onto later pages than they land on under `Orig`.
fn edit_sentence() -> &'static str {
	"the ledger resolves this very anchor into the folio it lands on once the driver's second pass converges"
}

/// The one middle paragraph the ledger fixture edits. `shift` repeats its sentence forty times instead of
/// once; nothing else about the fixture changes, so this is the sole block whose content differs.
fn edit_site(shift: bool) -> Block {
	let reps = if shift { 40 } else { 1 };
	let mut s = String::from("EditSite.");
	for _ in 0..reps {
		s.push(' ');
		s.push_str(edit_sentence());
		s.push('.');
	}
	Block::paragraph(s)
}

/// Chapter one's rich paragraph: a glossary first use (omitted under `DropFirstUse`), a citation, an index
/// marker, a footnote and a margin note -- the one paragraph exercising every ledger-facing segment kind.
/// Everything but the glossary segment is fixed across variants, so a miss traces to exactly that change.
fn rich_paragraph_one(drop_first_use: bool) -> Block {
	let mut segments = vec![Segment::text("The driver's ledger resolves ")];
	if !drop_first_use {
		segments.push(Segment::glossary("memoisation", "memoisation"));
		segments.push(Segment::text(" as "));
	}
	segments.push(Segment::text("the technique that caches "));
	segments.push(Segment::cite(vec!["scott1976".to_string()]));
	segments.push(Segment::text("'s peasant economy against a repeated anchor, recording it "));
	segments.push(Segment::index("Ledger anchor", None, vec![Segment::text("Ledger anchor")]));
	segments.push(Segment::text(" once per pass"));
	segments.push(Segment::footnote(vec![Segment::text("A pass converges when no anchor moves twice.")]));
	segments.push(Segment::text(", and marks it "));
	segments.push(Segment::margin_note("R1", vec!["R1".to_string()]));
	segments.push(Segment::text("in the outside margin."));
	Block::rich(segments)
}

/// The paragraph carrying the term's second textual mention (the first paragraph's own topic sentence
/// leads into it): always carries the glossary term (a later use under `Orig`/`Shift`, the FIRST use under
/// `DropFirstUse`), a two-key citation exercising the Zuboff year-suffix pair, an index marker, a footnote
/// and a margin note. Fixed across all three variants; kept directly adjacent to `rich_paragraph_one` (see
/// `ledger_doc`) so `DropFirstUse`'s effect on the memo is isolated to exactly these two blocks.
fn rich_paragraph_two() -> Block {
	Block::rich(vec![
		Segment::text("The account returns to "),
		Segment::glossary("memoisation", "memoisation"),
		Segment::text(" once more, this time citing "),
		Segment::cite(vec!["zuboff2019a".to_string(), "zuboff2019b".to_string()]),
		Segment::text(" on the surveillance ledger, indexing "),
		Segment::index("Surveillance ledger", None, vec![Segment::text("Surveillance ledger")]),
		Segment::text(" and noting"),
		Segment::footnote(vec![Segment::text("Both works share the one 2019 imprint year.")]),
		Segment::text(" the citation in "),
		Segment::margin_note("R2", vec!["R2".to_string()]),
		Segment::text("the margin."),
	])
}

/// The ledger fixture: a chapter of leading filler enough to paginate over several pages before the edit
/// site, the edit site itself, more filler, the two rich paragraphs, a second chapter heading and its own
/// filler, then a reverse claim index and back-matter index to close it. `paragraph_text`'s indices are
/// unique per paragraph and identical across every variant, so only the edit site and (under
/// `DropFirstUse`) the two rich paragraphs can ever differ.
///
/// The two rich paragraphs sit directly adjacent, with no heading or filler between them. That is
/// deliberate, not merely tidy: the block-authoring memo folds the glossary first-use `seen` set into
/// *every* block's key (`Authoring::seen_hash`, `doc.rs:1266`), so once the term's first use moves from
/// the first rich paragraph to the second, any block sitting between them would key on a `seen` state that
/// differs from the one it was primed under -- a correct, safe, but non-minimal miss, since that block's
/// own rendered output never reads the term. Keeping the pair adjacent isolates `DropFirstUse`'s effect to
/// exactly the two blocks that do read it, which is what `warm_recompile_reauthors_a_later_glossary_use_*`
/// holds the memo to.
fn ledger_doc(variant: Variant) -> Vec<Block> {
	let drop_first_use	= variant == Variant::DropFirstUse;
	let shift			= variant == Variant::Shift;
	let mut blocks = Vec::new();

	blocks.push(Block::heading(1, "Chapter One"));
	for k in 0..24 {
		blocks.push(Block::paragraph(paragraph_text(k, false)));
	}
	blocks.push(edit_site(shift));	// the one edited paragraph, well after several leading pages
	for k in 24..30 {
		blocks.push(Block::paragraph(paragraph_text(k, false)));
	}
	blocks.push(rich_paragraph_one(drop_first_use));
	blocks.push(rich_paragraph_two());

	blocks.push(Block::heading(1, "Chapter Two"));
	for k in 30..40 {
		blocks.push(Block::paragraph(paragraph_text(k, false)));
	}

	blocks.push(Block::ClaimIndex);
	blocks.push(Block::Index);
	blocks
}

/// THE gate: a warm recompile after an edit that shifts pagination -- moving every later folio, the second
/// chapter's rich paragraph and the back matter onto later pages -- must still be byte-identical to a cold
/// compile of the shifted source, while only the one edited block misses the authoring cache.
#[test]
fn warm_recompile_is_byte_identical_when_pagination_shifts_ledger_facts() -> Outcome<()> {
	let full_cites = ["scott1976", "zuboff2019a", "zuboff2019b"];

	let cold_orig	= res!(compile_pages(ledger_doc(Variant::Orig), Some(res!(ledger_bib(&full_cites))), None));
	let cold_shift	= res!(compile_pages(ledger_doc(Variant::Shift), Some(res!(ledger_bib(&full_cites))), None));

	// Precondition: the edit really does shift pagination -- the tail page differs and the document grows.
	// A test that does not move the folios proves nothing about the ledger, only about plain byte diffing.
	assert_ne!(cold_orig.last(), cold_shift.last(),
		"the shifted edit must move the final page's content -- the precondition the whole gate rests on");
	assert!(cold_shift.len() > cold_orig.len(),
		"the shifted edit must add at least one page, found {} vs {} pages",
		cold_shift.len(), cold_orig.len());

	// Prime on the original, then warm-compile the shift.
	let mut memo	= Memo::new();
	let _			= res!(compile_pages(ledger_doc(Variant::Orig), Some(res!(ledger_bib(&full_cites))), Some(&mut memo)));
	let warm_shift	= res!(compile_pages(ledger_doc(Variant::Shift), Some(res!(ledger_bib(&full_cites))), Some(&mut memo)));

	// (1) Byte identity -- THE gate.
	assert_eq!(warm_shift, cold_shift,
		"a warm recompile after a pagination-shifting edit must be byte-identical to the cold shifted compile");

	// (2) Only the edited paragraph misses; the rich paragraphs, headings and back-matter blocks all hit,
	// even though every one of them now lands on a different page than it did under `Orig`.
	assert_eq!(memo.block_misses, 1,
		"only the one edited block should miss the authoring cache, found {} misses", memo.block_misses);

	// (3) The emit cache: the cascade must actually reach several pages, and at least the leading page(s)
	// before the edit site must still hit.
	assert!(memo.page_misses >= 3,
		"the pagination cascade must re-render at least three pages, found {} misses", memo.page_misses);
	assert!(memo.page_hits >= 1,
		"at least one page before the edit site must hit the emit cache, found {} hits", memo.page_hits);

	eprintln!(
		"[ledger] {} -> {} pages: block {} miss, page {}/{} hit",
		cold_orig.len(), cold_shift.len(), memo.block_misses, memo.page_hits, memo.page_hits + memo.page_misses);
	Ok(())
}

/// Removing the first chapter's glossary segment pushes the term's first use into the second chapter's
/// rich paragraph, which must now render its bold-italic markup instead of the plain text it rendered
/// under `Orig` -- a warm recompile must reauthor that later paragraph, not serve its stale plain form.
#[test]
fn warm_recompile_reauthors_a_later_glossary_use_when_the_first_use_is_removed() -> Outcome<()> {
	let full_cites = ["scott1976", "zuboff2019a", "zuboff2019b"];

	let cold_drop = res!(compile_pages(ledger_doc(Variant::DropFirstUse), Some(res!(ledger_bib(&full_cites))), None));

	let mut memo	= Memo::new();
	let _			= res!(compile_pages(ledger_doc(Variant::Orig), Some(res!(ledger_bib(&full_cites))), Some(&mut memo)));
	let warm_drop	= res!(compile_pages(ledger_doc(Variant::DropFirstUse), Some(res!(ledger_bib(&full_cites))), Some(&mut memo)));

	assert_eq!(warm_drop, cold_drop,
		"a warm recompile that drops the first glossary use must be byte-identical to the cold compile");
	// Rich paragraph one misses (its own content changed, the segment is gone) and rich paragraph two
	// misses (its content is unchanged, but it now enters with a different glossary `seen` state, so it
	// must re-author its bold-italic first use rather than splice back its cached plain-text nodes).
	assert_eq!(memo.block_misses, 2,
		"exactly the two rich paragraphs should miss, found {} misses", memo.block_misses);
	Ok(())
}

/// Citing a new work the memo was not primed with must clear the whole cache: the cited set folds into the
/// memo's global fingerprint (`memo_fingerprint`, via the bibliography's `Debug`), so a document that has
/// not changed one block still misses everywhere once what it cites has changed.
#[test]
fn a_new_citation_clears_the_memo() -> Outcome<()> {
	let blocks = || ledger_doc(Variant::Orig);

	let cold_two_cites = res!(compile_pages(blocks(), Some(res!(ledger_bib(&["scott1976", "zuboff2019a"]))), None));

	let mut memo = Memo::new();
	let _		= res!(compile_pages(blocks(), Some(res!(ledger_bib(&["scott1976"]))), Some(&mut memo)));
	let warm	= res!(compile_pages(blocks(), Some(res!(ledger_bib(&["scott1976", "zuboff2019a"]))), Some(&mut memo)));

	assert_eq!(warm, cold_two_cites,
		"a warm recompile after a citation-set change must be byte-identical to a cold compile with that set");
	assert_eq!(memo.block_hits, 0,
		"a changed cited set must clear the global fingerprint and miss every block, found {} hits",
		memo.block_hits);
	Ok(())
}
