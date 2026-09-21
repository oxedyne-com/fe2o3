//! The list-fidelity gate: a RENDER-level check that a bullet or numbered list sets its items at the
//! body's own baseline pitch and seats each marker on its item's baseline, rather than tighter and
//! lower as the pre-cap-edge list layout did. It measures the placed frame of the committed
//! `samples/hierarchy.typ` -- the same file the reader's nav panel is exercised against -- so it gates
//! the geometry a reader actually sees, not the parse tree.
//!
//! Non-vacuity is the point of the exercise, so the three assertions are split across three tests, each
//! mapped to one of the three fixes in `doc.rs`:
//!
//!   * [`tight_list_item_pitch_equals_body_pitch`] reds if the inter-item glue stops being sized by the
//!     baselineskip rule (revert `push_item_gap` to a raw `Glue::fixed(item_skip)`): the items then set
//!     at `cap + item_skip`, tighter than the body `leading`.
//!   * [`bullet_marker_sits_on_the_line_baseline`] reds if the marker stops copying the line's text shift
//!     (revert `indent_item` to inserting the marker with shift zero): the bullet then drops by
//!     `ascender - cap` (~0.45 em) below the text baseline.
//!   * [`nested_list_indents_and_keeps_pitch`] reds on the same glue revert, at the nested level.
//!
//! Each test also cross-checks the measured body pitch against the theme's own `leading`, so a gate that
//! measured nothing (an empty frame, a marker-less list) fails loudly rather than passing vacuously.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::page::PlacedKind;
use oxedyne_fe2o3_austenite::theme::Theme;

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;
use std::sync::Arc;

const BULLET: &str = "\u{2022}";

// A quarter point, in scaled points: the geometry is exact integer arithmetic, so this is slack for
// nothing but rounding in the cap-edge trim and the shape metrics.
const TOL: i32 = 65536 / 4;

/// One placed run of shaped text, reduced to the numbers the gate reasons about: which page it landed on,
/// its baseline y (`top + ascent`), its left edge, its ascent, and the source string the run carries (a
/// bullet is `"\u{2022}"`, an enumerator `"1."`, a word its letters).
struct Run {
	page:	u32,
	base:	i32,	// baseline y in scaled points: the y a reader sees the glyphs sit on
	x:		i32,
	h:		i32,	// the run's ascent, uniform across a face and size
	src:	String,
}

/// Compiles `samples/hierarchy.typ` through the real assemble/author/run path and returns every placed
/// text run in the document, in page-then-baseline order.
fn runs() -> Outcome<Vec<Run>> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples").join("hierarchy.typ");
	let (assembled, _refusals, _skip) = res!(compile::assemble(
		&path,
		|| Ok(Arc::new(res!(fonts::libertinus()))),
	));
	let rendered = res!(compile::author_and_run(assembled));

	let mut out: Vec<Run> = Vec::new();
	for page in &rendered.out.pages {
		for placed in &page.frame.placed {
			if let PlacedKind::Text(shaped) = &placed.kind {
				out.push(Run {
					page:	page.number,
					base:	placed.y.raw() + placed.dims.height.raw(),
					x:		placed.x.raw(),
					h:		placed.dims.height.raw(),
					src:	shaped.source().to_string(),
				});
			}
		}
	}
	out.sort_by(|a, b| (a.page, a.base).cmp(&(b.page, b.base)));
	Ok(out)
}

/// The body ascent: the most common run ascent across the document. Body prose dominates every sample, so
/// its ascent is the mode; a heading's larger ascent is a minority and drops out. This tells a body line
/// from a heading line without reading a font metric.
fn body_ascent(runs: &[Run]) -> i32 {
	let mut counts: Vec<(i32, u32)> = Vec::new();
	for r in runs {
		match counts.iter_mut().find(|(h, _)| *h == r.h) {
			Some(entry)	=> entry.1 += 1,
			None		=> counts.push((r.h, 1)),
		}
	}
	counts.iter().max_by_key(|(_, n)| *n).map(|(h, _)| *h).unwrap_or(0)
}

/// The body baseline pitch: the most common gap between two consecutive body-size line baselines on the
/// document's first page. The opening paragraph alone wraps to seven lines all one `leading` apart, so
/// the mode is `leading` whatever the lists below do -- which is what keeps this an independent reference
/// for the list tests even when a list's own pitch is the thing under test.
fn body_pitch(runs: &[Run]) -> i32 {
	let body = body_ascent(runs);
	// The opening paragraph ends where the first list marker begins; everything above that on page 1 is
	// plain wrapped body prose. Measuring the pitch there alone keeps this reference independent of the
	// lists under test -- the intro's own leading does not move when a list's item spacing is broken, so
	// the reference stays put while the thing being compared to it fails, which is what makes the list
	// tests non-vacuous.
	let first_marker = runs.iter()
		.filter(|r| r.page == 1 && r.src == BULLET)
		.map(|r| r.base)
		.min()
		.unwrap_or(i32::MAX);
	// Distinct intro-line baselines on page 1, ascending. Same-baseline runs (the several words of a line)
	// collapse to one entry.
	let mut bases: Vec<i32> = Vec::new();
	for r in runs {
		if r.page == 1 && r.h == body && r.base < first_marker && !bases.contains(&r.base) {
			bases.push(r.base);
		}
	}
	bases.sort();
	// The mode of the exact consecutive gaps. The layout is exact-integer arithmetic, so every within-
	// paragraph pitch is the very same integer `leading`; the opening paragraph alone contributes six of
	// them, so `leading` wins the mode over the varied heading and inter-paragraph gaps -- no bucketing,
	// and the reference is the true value to the scaled point.
	let mut counts: Vec<(i32, u32)> = Vec::new();
	for w in bases.windows(2) {
		let gap = w[1] - w[0];
		match counts.iter_mut().find(|(g, _)| *g == gap) {
			Some(entry)	=> entry.1 += 1,
			None		=> counts.push((gap, 1)),
		}
	}
	counts.iter().max_by_key(|(_, n)| *n).map(|(g, _)| *g).unwrap_or(0)
}

/// Every bullet marker run, in page-then-baseline order -- the tight list's four, then the nested turn's
/// four, then the loose list's three, exactly the document order.
fn bullets(runs: &[Run]) -> Vec<&Run> {
	runs.iter().filter(|r| r.src == BULLET).collect()
}

fn pt(sp: i32) -> f64 { sp as f64 / 65536.0 }

#[test]
fn tight_list_item_pitch_equals_body_pitch() -> Outcome<()> {
	let runs	= res!(runs());
	let leading	= Theme::default().text.leading.raw();
	let body	= body_pitch(&runs);

	// The gate measured a real body pitch, and it is the theme's leading -- not a vacuous zero.
	assert!((body - leading).abs() < TOL,
		"measured body pitch {:.3}pt is not the theme leading {:.3}pt -- the gate measured nothing usable",
		pt(body), pt(leading));

	let bullets = bullets(&runs);
	assert!(bullets.len() >= 4,
		"expected at least four bullet markers, found {} -- the tight list did not render", bullets.len());

	// The first four bullets are the tight list under `== Bullets, Set Tight`, its items single-line, so a
	// marker-to-marker gap is one item's pitch. Each must equal the body pitch: a tight list sets at the
	// body leading, its items neither touching nor gapped.
	for pair in bullets[..4].windows(2) {
		let pitch = pair[1].base - pair[0].base;
		assert!((pitch - body).abs() < TOL,
			"tight bullet item pitch {:.3}pt != body pitch {:.3}pt (leading {:.3}pt): the list is set \
			{} the body it sits in",
			pt(pitch), pt(body), pt(leading),
			if pitch < body { "tighter than" } else { "looser than" });
	}
	Ok(())
}

#[test]
fn bullet_marker_sits_on_the_line_baseline() -> Outcome<()> {
	let runs = res!(runs());
	let bullets = bullets(&runs);
	assert!(!bullets.is_empty(), "no bullet markers rendered -- nothing to seat");

	// For the first tight bullet item, the marker and the item's own first word share the line, so they
	// must share a baseline. The item text is the next run on the same line to the marker's right. Before
	// the seating fix the marker kept shift zero while the text was raised to the cap edge, dropping the
	// bullet by `ascender - cap` (~0.45 em) below the text.
	let marker = bullets[0];
	let text = runs.iter()
		.filter(|r| r.page == marker.page && r.x > marker.x && r.src != BULLET)
		.filter(|r| (r.base - marker.base).abs() < Theme::default().text.leading.raw())
		.min_by_key(|r| r.x);
	let text = res!(text.ok_or_else(|| err!(
		"the first bullet item carried no text run to seat its marker against"; Invalid, Missing)));

	let drop = (marker.base - text.base).abs();
	assert!(drop < TOL,
		"bullet marker baseline is {:.3}pt off the item's text baseline -- it should sit on the line, not \
		~0.45 em below it",
		pt(drop));
	Ok(())
}

#[test]
fn nested_list_indents_and_keeps_pitch() -> Outcome<()> {
	let runs	= res!(runs());
	let leading	= Theme::default().text.leading.raw();
	let body	= body_pitch(&runs);
	let bullets	= bullets(&runs);
	assert!(bullets.len() >= 8,
		"expected the nested-turn bullets, found only {} markers", bullets.len());

	// The tight list's own left edge (its markers share one x), the outer indent to compare against.
	let outer_x = bullets[0].x;

	// The nested turn is bullets four through eight (0-based 4..8): outer item, its two sub-items, outer
	// item. The sub-items sit one indent to the right of the outer markers, so they are the bullets in
	// that group whose x exceeds the outer edge.
	let nested: Vec<&Run> = bullets[4..8].iter().copied().filter(|r| r.x > outer_x + TOL).collect();
	assert!(nested.len() >= 2,
		"expected two indented sub-items in the nested list, found {} -- the sub-list did not indent",
		nested.len());

	// The nesting is a real indent, not a hair.
	assert!(nested[0].x > outer_x + TOL,
		"nested sub-item left edge {:.3}pt is not indented past the outer edge {:.3}pt",
		pt(nested[0].x), pt(outer_x));

	// And the nested list keeps the body pitch between its own single-line items, like the top level.
	let pitch = nested[1].base - nested[0].base;
	assert!((pitch - body).abs() < TOL,
		"nested item pitch {:.3}pt != body pitch {:.3}pt (leading {:.3}pt)",
		pt(pitch), pt(body), pt(leading));
	Ok(())
}
