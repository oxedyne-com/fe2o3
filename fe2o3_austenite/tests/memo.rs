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

use oxedyne_fe2o3_austenite::compile::{
	author_and_run_memo,
	Assembled,
};
use oxedyne_fe2o3_austenite::doc::Block;
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
/// is held against.
fn compile_pages(blocks: Vec<Block>, memo: Option<&mut Memo>) -> Outcome<Vec<String>> {
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
		bib:	None,
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
	let cold_orig	= res!(compile_pages(build_doc(n, None), None));
	let cold_edited	= res!(compile_pages(build_doc(n, Some(edit_at)), None));

	// Prime a memo on the original, then recompile the edited source warm against it.
	let mut memo = Memo::new();
	let warm_orig = res!(compile_pages(build_doc(n, None), Some(&mut memo)));
	assert_eq!(warm_orig, cold_orig,
		"a first, cold compile through the memo must be byte-identical to a compile with no memo");
	// After the priming compile every block was a miss (the cache was empty) and every page was rendered.
	assert_eq!(memo.block_hits, 0, "the priming compile fills the cache, so nothing hits");
	assert_eq!(memo.block_misses as usize, n, "every block is authored on the priming compile");

	let warm_edited = res!(compile_pages(build_doc(n, Some(edit_at)), Some(&mut memo)));

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
	let cold_edited	= res!(compile_pages(doc(true), None));
	let mut memo	= Memo::new();
	let _		= res!(compile_pages(doc(false), Some(&mut memo)));	// prime on the original
	let warm	= res!(compile_pages(doc(true), Some(&mut memo)));
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
	let no_memo		= res!(compile_pages(blocks(), None));
	let mut memo	= Memo::new();
	let with_memo	= res!(compile_pages(blocks(), Some(&mut memo)));
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
	let cold	= res!(compile_pages(build_doc(n, None), None));
	let cold_ms	= t0.elapsed().as_secs_f64() * 1000.0;

	// Prime the memo on the original document (the first open of the document, not the edit loop).
	let mut memo	= Memo::new();
	let _		= res!(compile_pages(build_doc(n, None), Some(&mut memo)));

	// Warm: the edit-and-re-render a keystroke triggers.
	let t1		= Instant::now();
	let warm	= res!(compile_pages(build_doc(n, Some(edit_at)), Some(&mut memo)));
	let warm_ms	= t1.elapsed().as_secs_f64() * 1000.0;

	let cold_ref = res!(compile_pages(build_doc(n, Some(edit_at)), None));
	assert_eq!(warm, cold_ref, "the measured warm recompile must still be byte-identical");

	eprintln!(
		"[memo] {} pages: cold {:.1} ms, warm {:.1} ms, speedup {:.1}x (block {}/{} hit, page {}/{} hit)",
		cold.len(), cold_ms, warm_ms, cold_ms / warm_ms.max(0.001),
		memo.block_hits, n - 1, memo.page_hits, warm.len());
	Ok(())
}
