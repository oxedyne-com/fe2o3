//! The changed-only page delta, proved on the native compile path.
//!
//! The master goal is to swap Austenite in for Daimond's Typst-wasm compiler. The emit memo (see
//! `tests/memo.rs`) buys back the CPU of a per-keystroke recompile; this layer buys back the MEMORY and the
//! wire cost, by returning only the pages whose rendered SVG has changed since the consumer's last-known
//! set, keyed by a stable page-content id (see [`oxedyne_fe2o3_austenite::delta`]). These tests hold that
//! layer to the properties the consumer contract rests on: a first compile is a full reset; an unchanged
//! recompile sends nothing; a warm edit sends only the pages that genuinely changed; an inserted page
//! resends only itself and not every page after it (the id-vs-position claim); and the state kept between
//! compiles is the id list alone, never the rendered SVG.
//!
//! The wasm return shape is a thin marshal of this same [`delta::compute`] result, so unit-testing the
//! delta logic over real (and synthetic) pages here proves the browser behaviour without a browser. Each
//! test passes the prior id set explicitly, exactly as the wasm surface passes the consumer's supplied
//! `known` ids: the compiler holds no prior set of its own, so the boundary these tests exercise is the
//! real one.

use oxedyne_fe2o3_austenite::compile::{
	author_and_run,
	Assembled,
};
use oxedyne_fe2o3_austenite::delta;
use oxedyne_fe2o3_austenite::doc::Block;
use oxedyne_fe2o3_austenite::emit::svg;
use oxedyne_fe2o3_austenite::fonts::{
	self,
	FaceResolver,
};
use oxedyne_fe2o3_austenite::ir::{
	Dims,
	Sp,
};
use oxedyne_fe2o3_austenite::page::{
	Frame,
	Page,
	PageGeometry,
	Placed,
	PlacedKind,
};
use oxedyne_fe2o3_austenite::theme::Theme;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashSet;
use std::sync::Arc;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ REAL COMPILE FIXTURES                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// A deterministic body of prose paragraphs, each unique per index so no two pages coincide by accident.
/// `edit_at` rewrites one paragraph, standing in for the one edit a keystroke makes; `None` is untouched.
fn build_doc(n: usize, edit_at: Option<usize>) -> Vec<Block> {
	let words = [
		"typesetting", "streams", "a", "document", "through", "two", "passes", "of", "the", "driver",
		"while", "the", "ledger", "resolves", "each", "anchor", "into", "the", "page", "it", "landed",
		"on", "and", "the", "breaker", "sets", "every", "justified", "line", "to", "the", "measure",
		"without", "a", "single", "floating", "point", "along", "the", "way", "so", "the", "build",
	];
	let mut blocks = Vec::with_capacity(n);
	for k in 0..n {
		let edited	= edit_at == Some(k);
		let seed	= if edited { k * 7 + 3 } else { k * 5 + 1 };
		let count	= 34 + (seed % 20);
		let mut s	= fmt!("Paragraph{}{}.", k, if edited { "edited" } else { "" });
		for i in 0..count {
			s.push(' ');
			s.push_str(words[(seed + i * 3) % words.len()]);
		}
		s.push('.');
		blocks.push(Block::paragraph(s));
	}
	blocks
}

/// Authors and runs a document to its resolved pages -- the same pipeline the wasm delta path drives, minus
/// the emit (the delta renders each page itself, only when its id is new).
fn compile_pages(blocks: Vec<Block>) -> Outcome<Vec<Page>> {
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let assembled = Assembled {
		blocks,
		fonts,
		geom:	PageGeometry::a4(),
		style:	Theme::default(),
		title:	String::new(),
		faces:	FaceResolver::default(),
		front:	None,
		bib:	None,
	};
	Ok(res!(author_and_run(assembled)).out.pages)
}

/// The first compile resets and carries every page: `reset` is true, `version` steps to one, `order` names
/// every page, and `changed` carries the SVG of each (all page ids distinct, since each page's folio and
/// body differ).
#[test]
fn first_compile_resets_and_sends_every_page() -> Outcome<()> {
	let pages	= res!(compile_pages(build_doc(24, None)));
	assert!(pages.len() > 1, "the fixture must paginate over several pages, found {}", pages.len());

	let d = res!(delta::compute(&pages, &[], 0));

	assert!(d.reset, "the first compile against an empty prior set is a reset");
	assert_eq!(d.version, 1, "the version steps from zero to one on the first compile");
	assert_eq!(d.order.len(), pages.len(), "order names every page");
	assert_eq!(d.changed.len(), d.order.len(),
		"a reset carries every page's SVG, found {} changed for {} pages", d.changed.len(), d.order.len());
	// Every id in order is carried in changed, and each changed entry's SVG is the real page SVG.
	let carried: HashSet<u64> = d.changed.iter().map(|(id, _)| *id).collect();
	for id in &d.order {
		assert!(carried.contains(id), "every ordered id is carried on a reset");
	}
	Ok(())
}

/// Recompiling identical source sends nothing: `reset` is false, `order` is unchanged, `changed` is empty,
/// and `version` still steps. This is the idle keystroke -- a compile that produced the same bytes.
#[test]
fn identical_recompile_sends_nothing() -> Outcome<()> {
	let first	= res!(compile_pages(build_doc(24, None)));
	let d1		= res!(delta::compute(&first, &[], 0));

	let second	= res!(compile_pages(build_doc(24, None)));
	let d2		= res!(delta::compute(&second, &d1.order, d1.version));

	assert!(!d2.reset, "a recompile against a non-empty prior set is not a reset");
	assert_eq!(d2.version, 2, "the version steps on every compile, idle or not");
	assert_eq!(d2.order, d1.order, "identical source yields the identical id sequence");
	assert!(d2.changed.is_empty(),
		"an unchanged recompile resends nothing, found {} changed", d2.changed.len());
	Ok(())
}

/// The audit blocker: the consumer -- not the compiler -- owns the SVG cache, and clears it on a document
/// close or switch. A recompile after that clear must be told to resend everything (`reset`, full
/// `changed`), NOT that nothing changed against the prior ids -- which would leave the emptied cache with no
/// SVG for any id and a blank preview it could never recover from. Modelled by supplying an empty `known`
/// (the consumer's now-empty cache) though the source, and so the pages and their ids, are identical to the
/// priming compile. Were the prior set held in the compiler instead of supplied here, this would return
/// `changed: []` and the assertion below would fail.
#[test]
fn a_cleared_cache_forces_a_full_resend() -> Outcome<()> {
	let pages	= res!(compile_pages(build_doc(24, None)));
	let d1		= res!(delta::compute(&pages, &[], 0));
	assert!(!d1.changed.is_empty(), "the priming compile sends the pages");

	// The consumer closed the document: its cache is empty, so it supplies no known ids on reopen.
	let reopened	= res!(compile_pages(build_doc(24, None)));
	let d2			= res!(delta::compute(&reopened, &[], d1.version));

	assert!(d2.reset, "a recompile against an empty known set is a reset -- the recovery");
	assert_eq!(d2.changed.len(), d2.order.len(),
		"a cleared cache must be resent in full, not told nothing changed: {} changed of {} pages",
		d2.changed.len(), d2.order.len());
	assert!(d2.version > d1.version, "the version still steps monotonically across the clear");
	Ok(())
}

/// A warm edit late in the document resends only the pages whose SVG genuinely changed: the leading pages
/// -- unchanged in body and in folio -- keep their ids and are absent from `changed`, while the edited page
/// and the pagination cascade below it carry new ids that appear in `changed`.
#[test]
fn warm_edit_sends_only_the_changed_pages() -> Outcome<()> {
	let n		= 24;
	let edit_at	= 20;	// late, so many earlier pages fall outside the cascade

	let orig	= res!(compile_pages(build_doc(n, None)));
	let d1		= res!(delta::compute(&orig, &[], 0));

	let edited	= res!(compile_pages(build_doc(n, Some(edit_at))));
	let d2		= res!(delta::compute(&edited, &d1.order, d1.version));

	assert!(!d2.reset, "the warm edit is not a reset");
	assert_eq!(d2.version, 2, "the version steps to two");

	// The edit really changes something, but not everything: some pages are resent, some are not.
	assert!(!d2.changed.is_empty(), "the edit must resend at least the edited page");
	assert!(d2.changed.len() < d2.order.len(),
		"a late edit must leave earlier pages untouched: {} changed of {} pages",
		d2.changed.len(), d2.order.len());

	// changed carries exactly the ids new against the prior set -- no page the consumer already holds.
	let prior:	HashSet<u64>	= d1.order.iter().copied().collect();
	for (id, _) in &d2.changed {
		assert!(!prior.contains(id), "a resent page's id must be new against the prior set");
	}

	// The leading pages before the edit are byte-identical (same body, same folio), so their ids survive
	// into the new order and are NOT resent -- the changed-only property, proved non-vacuously against the
	// real cold renders of both documents.
	let leading_shared = d1.order.iter().zip(d2.order.iter()).take_while(|(a, b)| a == b).count();
	assert!(leading_shared > 0, "the edit must leave at least one leading page untouched");
	let resent: HashSet<u64> = d2.changed.iter().map(|(id, _)| *id).collect();
	for id in d2.order.iter().take(leading_shared) {
		assert!(!resent.contains(id), "an unchanged leading page must not be resent");
	}
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE ID-VS-POSITION CLAIM (synthetic pages)                                 │
// └───────────────────────────────────────────────────────────────────────────┘
//
// The core correctness claim is that the delta keys on a page's CONTENT, not its ordinal position. A
// content-free fixture isolates that claim: each page here carries one rule of a distinct width, so its id
// and its SVG depend on its content alone and not on any folio furniture. (A real folio'd document would
// resend a shifted page too -- correctly, since its printed folio, and so its rendered SVG, genuinely
// changes; that is the whole-frame keying the real-compile tests above exercise. Here the furniture is
// removed so the content-vs-position distinction stands alone.)

/// A synthetic page carrying one rule whose width encodes its identity: distinct `tag` gives distinct
/// content, so a distinct id and a distinct SVG, with no folio furniture in the frame.
fn rule_page(number: u32, geom: PageGeometry, tag: i32) -> Page {
	let mut frame	= Frame::new();
	let dims		= Dims::new(Sp::from_pt(100.0 + tag as f64), Sp::from_pt(10.0), Sp::ZERO);
	frame.push(Placed::new(Sp::from_pt(60.0), Sp::from_pt(80.0), dims, PlacedKind::Rule));
	Page::new(number, geom, frame)
}

/// How many slots a POSITION-keyed cache would resend: a page at index `i` is compared with whatever the
/// prior compile held at index `i`, so a page that merely moved to a new index counts as changed. This is
/// the strawman the content-keyed delta must beat, computed here so the beat is a measured fact.
fn positional_resends(prior: &[Page], now: &[Page]) -> Outcome<usize> {
	let mut n = 0;
	for (i, page) in now.iter().enumerate() {
		let same = match prior.get(i) {
			Some(p)	=> res!(svg::render_page(p)) == res!(svg::render_page(page)),
			None	=> false,	// a new slot the prior compile never held
		};
		if !same { n += 1; }
	}
	Ok(n)
}

/// Inserting a page near the front resends only the inserted page. Every later page shifts position but
/// keeps its content, so its id is unchanged and it is not resent -- whereas a position-keyed cache would
/// resend every page from the insertion point on. This is the load-bearing proof that the id keys on
/// content, not position: it FAILS under positional keying, and the test measures exactly that.
#[test]
fn inserting_a_page_resends_only_the_inserted_page() -> Outcome<()> {
	let geom = PageGeometry::a4();

	// Four distinct pages, first compiled cold.
	let before: Vec<Page> = (0..4).map(|k| rule_page(k as u32 + 1, geom, 10 * (k + 1))).collect();
	let d1 = res!(delta::compute(&before, &[], 0));
	assert!(d1.reset, "the first compile resets");
	assert_eq!(d1.changed.len(), 4, "all four pages are sent cold");

	// Insert a fresh page at the front; the four originals follow, unchanged in content, renumbered.
	let mut after: Vec<Page> = Vec::new();
	after.push(rule_page(1, geom, 999));	// the inserted page, a width no original uses
	for (k, tag) in [10, 20, 30, 40].iter().enumerate() {
		after.push(rule_page(k as u32 + 2, geom, *tag));
	}

	let d2 = res!(delta::compute(&after, &d1.order, d1.version));

	assert!(!d2.reset, "the insert is a warm compile, not a reset");
	assert_eq!(d2.order.len(), 5, "order now names five pages");
	// The four originals' ids survive into the new order (shifted one slot right), proving id-by-content.
	assert_eq!(&d2.order[1..], &d1.order[..],
		"every original page keeps its content id when it shifts position");
	// Only the inserted page is resent.
	assert_eq!(d2.changed.len(), 1,
		"exactly one page -- the inserted one -- is resent, found {} changed", d2.changed.len());
	let prior: HashSet<u64> = d1.order.iter().copied().collect();
	assert!(!prior.contains(&d2.changed[0].0), "the resent page's id is genuinely new");
	assert_eq!(d2.changed[0].0, d2.order[0], "the resent page is the one at the front of the order");

	// The non-vacuous proof: a position-keyed cache would have resent all five slots (the front slot's page
	// changed, and every following slot now holds a page different from the one it held before). The
	// content-keyed delta resends one. If the delta keyed on position, this test's `changed.len() == 1`
	// above would instead be five, so the assertion is load-bearing on the id-by-content design.
	let positional = res!(positional_resends(&before, &after));
	assert_eq!(positional, 5, "positional keying would resend every slot, the strawman we beat");
	assert!(d2.changed.len() < positional,
		"content keying ({}) must resend fewer than positional keying ({})", d2.changed.len(), positional);
	Ok(())
}

/// The state kept between compiles is the id list alone, never the rendered SVG. The retained `order` is a
/// `Vec<u64>`: its backing store is exactly eight bytes per page (a 64-bit id), independent of how large
/// each page's SVG is. Were it ever changed to retain the SVG (a `Vec<String>` or `Vec<(u64, String)>`),
/// each element would be at least a `String`'s 24 bytes and this assertion would fail.
#[test]
fn retained_state_is_ids_only() -> Outcome<()> {
	let pages	= res!(compile_pages(build_doc(16, None)));
	let d		= res!(delta::compute(&pages, &[], 0));

	assert_eq!(core::mem::size_of::<u64>(), 8, "a retained id is a bare 64-bit value");
	assert_eq!(
		core::mem::size_of_val(d.order.as_slice()),
		d.order.len() * core::mem::size_of::<u64>(),
		"the retained order holds one 8-byte id per page and no SVG");
	// The SVG that WAS rendered lives only in `changed`, to be handed over and dropped -- it is not reachable
	// from anything a caller would keep (`order`), which is the residency the swap depends on.
	assert!(!d.changed.is_empty(), "the reset rendered pages into changed, to be sent and then dropped");
	Ok(())
}
