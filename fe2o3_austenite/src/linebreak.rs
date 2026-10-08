// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-layout `inline/linebreak.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
//! Total-fit line breaking, after Knuth and Plass (*Breaking Paragraphs into Lines*, 1981).
//!
//! A paragraph is turned into the box-glue-penalty stream the [`ir`](crate::ir) already models: a
//! shaped word is a rigid box, the space between words is stretchable glue, and each legal break --
//! from `fe2o3_text`'s UAX #14 opportunities -- is a point the optimiser may take. The active-node
//! dynamic program picks the set of breaks minimising the sum of squared demerits, so a loose line
//! early is paid for against the whole paragraph rather than greedily.
//!
//! Two facts a reader could not derive. The last line is set flush left, not justified, by ending
//! the stream with a glue of near-infinite stretch before the forced break: that glue swallows the
//! slack, so the words keep their natural spacing. And justification is realised in the glue itself
//! -- each chosen line's inter-word glue carries its *adjusted* natural width -- so the existing
//! driver lays a line left to right with no notion of a ratio, and still fills the measure.

use crate::font::ShapedText;
use crate::hyphenate::Hyphenator;
use crate::ir::{
	BoxNode,
	Dims,
	Glue,
	Leaf,
	Node,
	Penalty,
	Sp,
};
use crate::ledger::AnchorId;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::{
		Dir,
		Feature,
	},
};
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_text::unicode::linebreak::{
	self,
	Break,
};

use std::sync::Arc;

const LINE_PENALTY:		f64 = 10.0;			// Knuth's l, the intrinsic cost of ending any line
const MAX_RATIO:		f64 = 10.0;			// tolerance: a break looser than this is infeasible
const FLAGGED_DEMERIT:	f64 = 10_000.0;		// two flagged breaks in a row (consecutive hyphens)
const HYPHEN_PENALTY:	i32 = 50;			// the cost of taking a discretionary (interior) break
const HYPHEN_MIN:		usize = 5;			// a word shorter than this is never split

/// The finishing glue's stretch: large against any real line, so a short last line sets flush left.
fn inf_stretch() -> Sp { Sp::from_pt(10_000.0) }

/// Breaks `text` into optimally-set lines at `measure`, each an [`Node::HBox`] of shaped words and
/// justified glue, joined by inter-line glue sized so baselines fall `leading` apart -- a vertical
/// list ready for the driver.
pub fn break_paragraph(
	fonts:		Arc<FontSet>,
	role:		Role,
	dir:		Dir,
	size:		Sp,
	text:		&str,
	measure:	Sp,
	leading:	Sp,
	hyphenate:	bool,
	fill:		Rgba,	// the colour every prose leaf of this paragraph draws in, from `theme.text.fill`
	cap:		Option<Sp>,	// the block-edge model; see [`set_lines`]
)
	-> Outcome<Vec<Node>>
{
	let items = res!(build_items(fonts, role, dir, size, text, hyphenate));
	if items.is_empty() {
		return Ok(Vec::new());
	}
	let breaks	= optimal_breaks(&items, measure, true);
	let lines	= res!(set_lines(&items, &breaks, measure, leading, true, fill, cap));
	Ok(lines)
}

/// One run of a segmented paragraph: a stretch of text in a named face, or a pre-built footnote mark
/// leaf. A rich paragraph is a sequence of these -- an emphasised run and the body either side of it,
/// the text around each footnote mark, and the marks themselves -- so a run shapes in its own face while
/// line breaking still flows across the boundaries. The `role` is per run: `*strong*` arrives as a
/// `Bold` piece, `/emph/` as an `Italic` one, the surrounding prose as `Body`.
pub enum Piece {
	Text { text: String, role: Role },
	// A `#smallcaps[...]` run: shaped in `role` with the font's small-capitals feature, and otherwise
	// broken exactly as a text run -- words, interword glue and hyphenation alike.
	SmallCaps { text: String, role: Role },
	Mark(Leaf),	// a footnote mark, already shaped as a raised superscript (LeafKind::Mark)
	// A zero-width anchor woven into the line at the point it was authored, recording where an identity
	// landed without occupying any horizontal space -- the marginalia anchor, whose margin note is drawn
	// post-convergence from the ledger. It never removes a break opportunity: the line breaks exactly as
	// it would with the anchor absent, so a paragraph carrying one sets byte-identically to one that does not.
	Anchor(AnchorId),
	Math {		// an inline maths box, already flattened to leaves and glue by math::layout
		nodes:	Vec<Node>,
		width:	Sp,
		height:	Sp,	// the line height the box asks for, the surrounding text ascent
		depth:	Sp,	// how far the box hangs below the baseline
		over:	Sp,	// how far it climbs above the line, so the line above can open for it
	},
}

/// Breaks a segmented paragraph the way [`break_paragraph`] breaks a plain one, but with footnote marks
/// woven into the stream. Each text piece contributes its words and interword glue; each mark piece a
/// rigid superscript box that never breaks and that the following space may break after. The result is
/// the same vertical list of HBox lines, so the driver sets it with no special case -- and a paragraph
/// of a single text piece produces exactly what [`break_paragraph`] would.
///
/// `justify` fills each line to the measure by adjusting its interword glue, the body's set; `false`
/// sets the lines ragged right, each space at its natural width -- what a table cell wants, so the
/// band's own justification to the table width cannot stretch or collapse the words within a cell.
pub fn break_paragraph_pieces(
	fonts:		Arc<FontSet>,
	role:		Role,
	dir:		Dir,
	size:		Sp,
	pieces:		&[Piece],
	measure:	Sp,
	leading:	Sp,
	justify:	bool,
	hyphenate:	bool,
	fill:		Rgba,	// the colour every prose text leaf draws in; marks and maths keep their own black
	cap:		Option<Sp>,	// the block-edge model; see [`set_lines`]
)
	-> Outcome<Vec<Node>>
{
	let (sp_w, stretch, shrink, hyphen) = res!(interword(fonts.clone(), role, dir, size));
	let hyph = Hyphenator::en_us();

	let mut items = Vec::new();
	for piece in pieces {
		match piece {
			Piece::Text { text, role: run } => {
				// The run shapes in its own face; the interword glue keeps the paragraph's role, so a space
				// beside an emphasised word stays the body space (TeX sets the space in the surrounding font).
				res!(push_text_run(
					&mut items, fonts.clone(), *run, dir, size, text, sp_w, stretch, shrink, &hyph, &hyphen, hyphenate, &[]));
			},
			Piece::SmallCaps { text, role: run } => {
				res!(push_text_run(
					&mut items, fonts.clone(), *run, dir, size, text, sp_w, stretch, shrink, &hyph, &hyphen, hyphenate,
					&[Feature::SMALL_CAPS]));
			},
			Piece::Mark(leaf) => {
				items.push(Item {
					kind: Kind::Mark(leaf.clone()), width: leaf.dims.width, stretch: Sp::ZERO,
					shrink: Sp::ZERO, penalty: Penalty::INFINITY, flagged: false, hyphen: None });
			},
			Piece::Anchor(id) => {
				// Zero of everything: no width to shift a break, an infinite penalty so it is never itself a
				// breakpoint, and `is_break` looks through it so the glue beside it breaks as if it were absent.
				items.push(Item {
					kind: Kind::Anchor(id.clone()), width: Sp::ZERO, stretch: Sp::ZERO,
					shrink: Sp::ZERO, penalty: Penalty::INFINITY, flagged: false, hyphen: None });
			},
			Piece::Math { nodes, width, height, depth, over } => {
					// A rigid cluster, like a very wide box: it never breaks, and the space after it may.
					items.push(Item {
						kind: Kind::Math { nodes: nodes.clone(), height: *height, depth: *depth, over: *over },
						width: *width, stretch: Sp::ZERO, shrink: Sp::ZERO,
						penalty: Penalty::INFINITY, flagged: false, hyphen: None });
				},
		}
	}
	push_finish(&mut items);	// the forced break that ends the paragraph, whatever the last piece was

	if items.is_empty() {
		return Ok(Vec::new());
	}
	let breaks	= optimal_breaks(&items, measure, justify);
	let lines	= res!(set_lines(&items, &breaks, measure, leading, justify, fill, cap));
	Ok(lines)
}

/// One entry of the box-glue-penalty stream. A `Boxed` word (or word fragment) is rigid; a `Glued`
/// space stretches and shrinks; a `Pen` is a break carrying no space (a hyphen point, or the forced
/// end of the stream); a `Mark` is a footnote's superscript, rigid like a box but already built with
/// its raised dimensions.
enum Kind {
	Boxed(ShapedText),
	Glued,
	Pen,
	Mark(Leaf),
	Anchor(AnchorId),			// a zero-width position marker woven into the line, drawn as a `Node::Anchor`
	Math {						// an inline maths box: pre-built leaves and glue, its own extent
		nodes:	Vec<Node>,
		height:	Sp,
		depth:	Sp,
		over:	Sp,	// how far the box climbs above the line top, for the interline gap above it
	},
}

struct Item {
	kind:		Kind,
	width:		Sp,						// box advance, or glue natural length
	stretch:	Sp,
	shrink:		Sp,
	penalty:	i32,					// break cost; a box carries INFINITY, a plain space break 0
	flagged:	bool,
	hyphen:		Option<ShapedText>,		// a discretionary's hyphen glyph, drawn only if the break is taken
}

/// Shapes each word of `text` and turns UAX #14's opportunities into the box-glue-penalty stream, then
/// closes it with the forced break that ends the paragraph.
fn build_items(
	fonts:	Arc<FontSet>,
	role:	Role,
	dir:	Dir,
	size:	Sp,
	text:	&str,
	hyphenate:	bool,
)
	-> Outcome<Vec<Item>>
{
	let (sp_w, stretch, shrink, hyphen) = res!(interword(fonts.clone(), role, dir, size));
	let hyph = Hyphenator::en_us();

	let mut items = Vec::new();
	res!(push_text_run(&mut items, fonts, role, dir, size, text, sp_w, stretch, shrink, &hyph, &hyphen, hyphenate, &[]));
	push_finish(&mut items);
	Ok(items)
}

/// The interword space, measured from the face with the classic TeX-ish elasticity -- it grows by a
/// half and gives up a third -- and the hyphen glyph a taken discretionary draws, both shaped once.
fn interword(
	fonts:	Arc<FontSet>,
	role:	Role,
	dir:	Dir,
	size:	Sp,
)
	-> Outcome<(Sp, Sp, Sp, ShapedText)>
{
	let space	= res!(ShapedText::new(fonts.clone(), role, dir, size, " "));
	let sp_w	= space.dims().width;
	let stretch	= Sp(sp_w.raw() / 2);
	let shrink	= Sp(sp_w.raw() / 3);
	let hyphen	= res!(ShapedText::new(fonts, role, dir, size, "-"));
	Ok((sp_w, stretch, shrink, hyphen))
}

/// The glue and forced break that end a paragraph: a glue of near-infinite stretch swallows the last
/// line's slack so it sets flush left, then a forced break the optimiser must take.
fn push_finish(items: &mut Vec<Item>) {
	items.push(Item {
		kind: Kind::Glued, width: Sp::ZERO, stretch: inf_stretch(), shrink: Sp::ZERO,
		penalty: Penalty::INFINITY, flagged: false, hyphen: None });
	items.push(Item {
		kind: Kind::Pen, width: Sp::ZERO, stretch: Sp::ZERO, shrink: Sp::ZERO,
		penalty: Penalty::EJECT, flagged: false, hyphen: None });
}

/// Appends one run of text as words and interword glue, turning UAX #14's opportunities into the
/// stream. Punctuation stays with its word: the opportunities only fall after spaces (and the odd slash
/// or hyphen), so trimming a segment's trailing whitespace leaves the word with its clinging marks. The
/// run's terminal opportunity does not close the paragraph -- that is [`push_finish`]'s single job,
/// after every run -- so a mark piece may follow this run's last word with nothing between them.
#[allow(clippy::too_many_arguments)]
fn push_text_run(
	items:	&mut Vec<Item>,
	fonts:	Arc<FontSet>,
	role:	Role,
	dir:	Dir,
	size:	Sp,
	text:	&str,
	sp_w:	Sp,
	stretch:	Sp,
	shrink:	Sp,
	hyph:	&Hyphenator,
	hyphen:	&ShapedText,
	hyphenate:	bool,
	features:	&[Feature],	// OpenType features every word of the run is shaped with
)
	-> Outcome<()>
{
	// A run that follows a mark (or another run) may open with a space -- the interword space that parts
	// the mark from the next word. Lift it out as its own elastic, breakable glue rather than baking it
	// into the first word's box, so the space justifies and the line may break after the mark.
	let trimmed	= text.trim_start_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r'));
	let lead	= text.len() - trimmed.len();
	if lead > 0 {
		let spaces = text[..lead].chars().filter(|c| *c == ' ').count() as i32;
		if spaces > 0 {
			items.push(Item {
				kind: Kind::Glued, width: Sp(sp_w.raw() * spaces), stretch, shrink,
				penalty: 0, flagged: false, hyphen: None });
		}
	}
	let text		= trimmed;

	let opps		= linebreak::line_breaks(text);
	let n			= opps.len();
	let mut prev	= 0usize;
	for (oi, opp) in opps.iter().enumerate() {
		let seg		= &text[prev..opp.offset];
		prev		= opp.offset;
		let word	= seg.trim_end_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r'));
		let tail	= &seg[word.len()..];
		if !word.is_empty() {
			res!(push_word(items, fonts.clone(), role, dir, size, word, hyph, hyphen, hyphenate, features));
		}
		let spaces = tail.chars().filter(|c| *c == ' ').count() as i32;
		match opp.kind {
			Break::Mandatory if oi + 1 == n => {
				// The run's own end. Any trailing space becomes a breakable glue so a following mark or
				// run may part from this word; the paragraph's forced break is added once, by push_finish.
				if spaces > 0 {
					items.push(Item {
						kind: Kind::Glued, width: Sp(sp_w.raw() * spaces), stretch, shrink,
						penalty: 0, flagged: false, hyphen: None });
				}
			},
			Break::Mandatory => {
				// An interior hard break (an explicit newline within the run): flush the line and force it.
				push_finish(items);
			},
			Break::Optional => {
				if spaces > 0 {
					items.push(Item {
						kind: Kind::Glued, width: Sp(sp_w.raw() * spaces), stretch, shrink,
						penalty: 0, flagged: false, hyphen: None });
				} else {
					// A break with no space -- a slash or an already-present hyphen. Latin prose rarely
					// reaches here; it becomes a zero-width penalty so the two parts may still part.
					items.push(Item {
						kind: Kind::Pen, width: Sp::ZERO, stretch: Sp::ZERO, shrink: Sp::ZERO,
						penalty: 0, flagged: false, hyphen: None });
				}
			},
		}
	}
	Ok(())
}

/// Shapes one word and pushes it as boxes. A word long enough to hold a legal Liang break is split at
/// each point into fragment boxes joined by flagged hyphen penalties, so the optimiser may take one
/// and end a line inside the word; otherwise the word is a single rigid box. The hyphenation runs on
/// the word's alphabetic core, so leading and trailing punctuation stay clinging to the outer
/// fragments.
#[allow(clippy::too_many_arguments)]
fn push_word(
	items:	&mut Vec<Item>,
	fonts:	Arc<FontSet>,
	role:	Role,
	dir:	Dir,
	size:	Sp,
	word:	&str,
	hyph:	&Hyphenator,
	hyphen:	&ShapedText,
	hyphenate:	bool,
	features:	&[Feature],
)
	-> Outcome<()>
{
	// The alphabetic core, and where it starts in the word, so break points map back to word bytes. With
	// hyphenation off (`#set text(hyphenate: false)`), no point is offered and the word stays one rigid box.
	let start	= word.len() - word.trim_start_matches(|c: char| !c.is_alphabetic()).len();
	let core	= word.trim_matches(|c: char| !c.is_alphabetic());
	let points	= if hyphenate && core.chars().count() >= HYPHEN_MIN { hyph.hyphenate(core) } else { Vec::new() };

	if points.is_empty() {
		let shaped	= res!(ShapedText::new_with_features(fonts, role, dir, size, word, features));
		let w		= shaped.dims().width;
		items.push(Item {
			kind: Kind::Boxed(shaped), width: w, stretch: Sp::ZERO, shrink: Sp::ZERO,
			penalty: Penalty::INFINITY, flagged: false, hyphen: None });
		return Ok(());
	}

	// Turn each char-prefix count into a byte split within the word, then walk the fragments.
	let mut splits: Vec<usize> = Vec::with_capacity(points.len());
	for &chars in &points {
		let off = core.char_indices().nth(chars).map_or(core.len(), |(b, _)| b);
		splits.push(start + off);
	}
	let mut bounds = Vec::with_capacity(splits.len() + 2);
	bounds.push(0);
	bounds.extend_from_slice(&splits);
	bounds.push(word.len());

	for w in bounds.windows(2) {
		let frag	= &word[w[0]..w[1]];
		let shaped	= res!(ShapedText::new_with_features(fonts.clone(), role, dir, size, frag, features));
		let fw		= shaped.dims().width;
		items.push(Item {
			kind: Kind::Boxed(shaped), width: fw, stretch: Sp::ZERO, shrink: Sp::ZERO,
			penalty: Penalty::INFINITY, flagged: false, hyphen: None });
		if w[1] < word.len() {
			// A discretionary between two fragments: a flagged break whose taken cost is the hyphen's
			// width, added to the line only when the optimiser chooses it (see line_len).
			items.push(Item {
				kind: Kind::Pen, width: Sp::ZERO, stretch: Sp::ZERO, shrink: Sp::ZERO,
				penalty: HYPHEN_PENALTY, flagged: true, hyphen: Some(hyphen.clone()) });
		}
	}
	Ok(())
}

/// Is item `i` a legal breakpoint? A space breaks after a word, unless forbidden -- the finishing
/// glue carries an infinite penalty so only the forced break after it ends the last line. A penalty
/// breaks unless forbidden; a box never breaks.
fn is_break(items: &[Item], i: usize) -> bool {
	match items[i].kind {
		Kind::Glued		=> i > 0
							&& prev_is_boxlike(items, i)
							&& items[i].penalty < Penalty::INFINITY,
		Kind::Pen		=> items[i].penalty < Penalty::INFINITY,
		Kind::Boxed(_)	=> false,
		Kind::Mark(_)	=> false,		// a mark clings to its word; the space after it may break
		Kind::Anchor(_)	=> false,		// a zero-width anchor never breaks; the glue beside it may
		Kind::Math { .. }	=> false,	// a maths box is rigid; the space after it may break
	}
}

/// Is the item before glue `i` one a space can break after -- a word, a mark or a maths box? A zero-width
/// [`Kind::Anchor`] is looked through, not counted: it neither is a breakable box nor blocks the space from
/// breaking after the box before it, so a line carrying a margin anchor breaks exactly as the same line
/// without one would. Without this the anchor would sit between a word and its trailing space and forbid
/// the break there, moving the line ends of a paragraph that carries a claim code.
fn prev_is_boxlike(items: &[Item], i: usize) -> bool {
	let mut j = i;
	while j > 0 {
		j -= 1;
		match items[j].kind {
			Kind::Anchor(_)	=> continue,	// look through it to the real box before
			Kind::Boxed(_) | Kind::Mark(_) | Kind::Math { .. }	=> return true,
			_				=> return false,
		}
	}
	false
}

/// Was the break at predecessor position `pos` flagged? The sentinel start (`-1`) never was.
fn flagged_at(items: &[Item], pos: isize) -> bool {
	pos >= 0 && items[pos as usize].flagged
}

/// The natural length of the line running `[lower, b)` and ending at the break `b`, given the width
/// prefix sums `sw`. A discretionary break adds its hyphen glyph -- the width appears on the line only
/// when the break is taken, which is exactly the standard Knuth-Plass discretionary rule.
fn line_len(items: &[Item], sw: &[i64], lower: usize, b: usize) -> f64 {
	let mut l = (sw[b] - sw[lower]) as f64;
	if let Some(h) = &items[b].hyphen {
		l += h.dims().width.raw() as f64;
	}
	l
}

/// One settled breakpoint: where it broke, which node it came from, and the running demerit total.
struct Rec {
	pos:	isize,			// item index of the break, or -1 for the paragraph start
	prev:	Option<usize>,	// index into the node store of the line's start
	total:	f64,
}

/// The adjustment ratio for a line of natural length `l` set to `target`, given its total stretch
/// `y` and shrink `z`. Positive stretches, negative shrinks; the float boundary the architecture
/// permits at a ratio.
fn ratio(target: f64, l: f64, y: f64, z: f64) -> f64 {
	let diff = target - l;
	if diff > 0.0 {
		if y > 0.0 { diff / y } else { MAX_RATIO + 1.0 }	// nothing to stretch: as good as too loose
	} else if diff < 0.0 {
		if z > 0.0 { diff / z } else { -2.0 }			// nothing to shrink: overfull
	} else {
		0.0
	}
}

/// The demerits of a line ending at a break of cost `pen`, with adjustment ratio `r`, following a
/// flagged break iff `after_flagged` and being itself `flagged`.
fn demerits(r: f64, pen: i32, flagged: bool, after_flagged: bool) -> f64 {
	let bad			= 100.0 * r.abs().powi(3);
	let base		= (LINE_PENALTY + bad).powi(2);
	let mut d		= base;
	if pen >= 0 && pen < Penalty::INFINITY {
		d += (pen as f64).powi(2);
	} else if pen > Penalty::EJECT && pen < 0 {
		d -= (pen as f64).powi(2);
	}
	// A forced break (pen <= EJECT) adds nothing beyond the base.
	if flagged && after_flagged {
		d += FLAGGED_DEMERIT;
	}
	d
}

/// Runs the active-node dynamic program and returns the chosen break positions in order, beginning
/// with the sentinel start `-1` and ending at the forced end of the stream.
///
/// The fallback for an overfull line with no feasible break: when no active node can reach a break
/// within tolerance -- a word wider than the measure, say -- and the break is forced or the active
/// set would empty, the least-bad predecessor is taken anyway, so the program never dead-ends and
/// the line is simply set overfull.
fn optimal_breaks(items: &[Item], measure: Sp, justify: bool) -> Vec<isize> {
	let n		= items.len();
	let target	= measure.raw() as f64;

	// Prefix sums to the point, in i64: a whole paragraph's width can exceed i32.
	let mut sw = vec![0i64; n + 1];
	let mut sy = vec![0i64; n + 1];
	let mut sz = vec![0i64; n + 1];
	for i in 0..n {
		sw[i + 1] = sw[i] + items[i].width.raw() as i64;
		sy[i + 1] = sy[i] + items[i].stretch.raw() as i64;
		sz[i + 1] = sz[i] + items[i].shrink.raw() as i64;
	}

	let mut nodes:	Vec<Rec>	= vec![Rec { pos: -1, prev: None, total: 0.0 }];
	let mut active:	Vec<usize>	= vec![0];

	for b in 0..n {
		if !is_break(items, b) {
			continue;
		}
		let forced		= matches!(items[b].kind, Kind::Pen) && items[b].penalty <= Penalty::EJECT;
		let pen			= items[b].penalty;
		let flagged_b	= items[b].flagged;

		let mut best_feasible:	Option<(usize, f64)> = None;
		let mut best_forced:	Option<(usize, f64)> = None;	// least-bad, feasibility ignored
		let mut dead:			Vec<usize> = Vec::new();

		for (k, &ni) in active.iter().enumerate() {
			let a		= nodes[ni].pos;
			let lower	= if a < 0 { 0usize } else { a as usize + 1 };
			let l		= line_len(items, &sw, lower, b);
			let y		= (sy[b] - sy[lower]) as f64;
			// A ragged set never shrinks a space to fit: with no shrink capacity, a line naturally wider
			// than the measure is infeasible, so the breaker takes an earlier break rather than letting the
			// line overrun -- what a table cell wants, where an overrun would cross its column rule.
			let z		= if justify { (sz[b] - sz[lower]) as f64 } else { 0.0 };
			let r		= ratio(target, l, y, z);
			let d		= demerits(r, pen, flagged_b, flagged_at(items, a));
			let total	= nodes[ni].total + d;

			if best_forced.map_or(true, |(_, t)| total < t) {
				best_forced = Some((ni, total));
			}
			let feasible = r >= -1.0 && (forced || r <= MAX_RATIO);
			if feasible && best_feasible.map_or(true, |(_, t)| total < t) {
				best_feasible = Some((ni, total));
			}
			// A node whose line to b is overfull cannot reach any later break either; a forced break
			// ends every line before it. Retire such nodes.
			if r < -1.0 || forced {
				dead.push(k);
			}
		}

		for &k in dead.iter().rev() {
			active.remove(k);
		}

		let chosen = best_feasible.or_else(|| {
			if forced || active.is_empty() { best_forced } else { None }
		});
		if let Some((prev_ni, total)) = chosen {
			nodes.push(Rec { pos: b as isize, prev: Some(prev_ni), total });
			active.push(nodes.len() - 1);
		}
	}

	// The terminal node is the one settled at the final forced break with the least total demerits.
	let mut terminal:	Option<usize> = None;
	let mut best:		f64 = f64::INFINITY;
	for (idx, rec) in nodes.iter().enumerate() {
		if rec.pos == (n as isize - 1) && rec.total <= best {
			best		= rec.total;
			terminal	= Some(idx);
		}
	}

	let mut breaks	= Vec::new();
	let mut cur		= terminal;
	while let Some(idx) = cur {
		breaks.push(nodes[idx].pos);
		cur = nodes[idx].prev;
	}
	breaks.reverse();	// from the sentinel start to the forced end
	breaks
}

/// Raises every drawn leaf of a line by `drop`, a negative shift added to whatever shift the leaf already
/// carries -- so a footnote mark or a maths script, raised by its own shift or by a shortened box, keeps
/// that raise relative to the text around it. It is how the block-edge model lifts a first line's glyphs to
/// meet the cap-height top edge without disturbing anything set on the line.
fn raise_leaves(list: &mut [Node], drop: Sp) {
	for node in list.iter_mut() {
		match node {
			Node::Leaf(l)					=> l.shift = l.shift - drop,
			Node::HBox(b) | Node::VBox(b)	=> raise_leaves(&mut b.list, drop),
			_								=> (),
		}
	}
}

/// Sets each chosen line as an HBox of shaped words and justified glue, joined by leading glue. A
/// line ending at a forced break keeps natural spacing (flush left); every other line distributes
/// its slack by the adjustment ratio.
fn set_lines(
	items:		&[Item],
	breaks:		&[isize],
	measure:	Sp,
	leading:	Sp,
	justify:	bool,
	fill:		Rgba,	// the fill every text box (and a taken hyphen) is coloured with as it becomes a leaf
	// The block-edge model. `Some(cap)` seats the paragraph's top edge at the cap height `cap` rather than
	// the face ascender, and its bottom edge at the baseline rather than the descender -- Typst's default
	// `top-edge: "cap-height", bottom-edge: "baseline"`, which is what the inter-block glue (`par.skip`, a
	// heading's before/after, a caption's framing) then attaches to. Within-paragraph line pitch is
	// untouched: only the first line's height and the last line's depth move, and neither feeds the
	// interline glue. `None` keeps the ascender/descender edges, for a context whose box edges are its own
	// (a table cell measured to its inset, the standalone SVG preview).
	cap:		Option<Sp>,
)
	-> Outcome<Vec<Node>>
{
	let n		= items.len();
	let target	= measure.raw() as f64;

	let mut sw = vec![0i64; n + 1];
	let mut sy = vec![0i64; n + 1];
	let mut sz = vec![0i64; n + 1];
	for i in 0..n {
		sw[i + 1] = sw[i] + items[i].width.raw() as i64;
		sy[i + 1] = sy[i] + items[i].stretch.raw() as i64;
		sz[i + 1] = sz[i] + items[i].shrink.raw() as i64;
	}

	// Each set line, kept with its own height, depth and overshoot, so the interline glue between two of
	// them can be sized from the depth of the upper and the height of the lower -- TeX's baselineskip
	// rule, which a single line's own extent cannot give -- and opened further when the lower line
	// carries inline maths that climbs above its own top.
	let mut lines:	Vec<(Node, Sp, Sp, Sp, Sp)> = Vec::new();
	for w in breaks.windows(2) {
		let a		= w[0];
		let hi		= w[1] as usize;
		let lower	= if a < 0 { 0usize } else { a as usize + 1 };
		let forced	= matches!(items[hi].kind, Kind::Pen) && items[hi].penalty <= Penalty::EJECT;

		let l		= line_len(items, &sw, lower, hi);
		let y		= (sy[hi] - sy[lower]) as f64;
		let z		= (sz[hi] - sz[lower]) as f64;
		let r		= ratio(target, l, y, z);

		let mut children:	Vec<Node> = Vec::new();
		let mut height		= Sp::ZERO;
		let mut depth		= Sp::ZERO;
		let mut over		= Sp::ZERO;	// how far any inline maths on the line climbs above the line top
		let mut mdepth		= Sp::ZERO;	// depth owed to inline maths alone, kept as the block bottom edge
		for item in items.iter().take(hi).skip(lower) {
			match &item.kind {
				Kind::Boxed(shaped) => {
					// Every prose box takes the paragraph's fill as it becomes a drawn leaf; black leaves the
					// glyph emitters' bytes exactly as before.
					let leaf = Leaf::text(shaped.clone().with_colour(fill.into()));
					if leaf.dims.height > height { height = leaf.dims.height; }
					if leaf.dims.depth > depth { depth = leaf.dims.depth; }
					children.push(Node::Leaf(leaf));
				},
				Kind::Mark(leaf) => {
					// The mark carries its own raised dims; a superscript is shorter than the line, so it
					// takes the line's height from the words around it, not from itself.
					let leaf = leaf.clone();
					if leaf.dims.height > height { height = leaf.dims.height; }
					if leaf.dims.depth > depth { depth = leaf.dims.depth; }
					children.push(Node::Leaf(leaf));
				},
				Kind::Anchor(id) => {
					// A zero-width marker: it draws no ink and takes no height or depth, so it neither
					// advances the line's cursor nor raises its extent. The driver records where it landed
					// when it places the line, which is the (x, y) the margin note is drawn against.
					children.push(Node::Anchor(id.clone()));
				},
				Kind::Math { nodes, height: mh, depth: md, over: mo } => {
					// The cluster's leaves are already seated by shift. It asks for the line's own text
					// height so its baseline meets the prose; its depth may hang lower, opening the space
					// below; and its overshoot opens the space above, so a tall fraction clears the line
					// above rather than climbing into it.
					if *mh > height { height = *mh; }
					if *md > depth { depth = *md; }
					if *md > mdepth { mdepth = *md; }
					if *mo > over { over = *mo; }
					for n in nodes {
						children.push(n.clone());
					}
				},
				Kind::Glued => {
					// Justification lives in the glue: the natural space plus the ratio's share of its
					// elasticity, so the driver's plain left-to-right pass fills the measure. A ragged set
					// (a cell), or the natural-spaced last line of a paragraph, keeps the space at its
					// natural width.
					let nat = item.width;
					let adj = if !justify || forced {
						nat
					} else if r >= 0.0 {
						Sp(nat.raw() + (r * item.stretch.raw() as f64).round() as i32)
					} else {
						Sp(nat.raw() + (r * item.shrink.raw() as f64).round() as i32)
					};
					children.push(Node::Glue(Glue::new(adj, Sp::ZERO, Sp::ZERO)));
				},
				Kind::Pen => (),	// an unchosen interior break sets no ink
			}
		}

		// A taken discretionary draws its hyphen as the line's last box, in the paragraph's own fill.
		if let Some(h) = &items[hi].hyphen {
			let leaf = Leaf::text(h.clone().with_colour(fill.into()));
			if leaf.dims.height > height { height = leaf.dims.height; }
			if leaf.dims.depth > depth { depth = leaf.dims.depth; }
			children.push(Node::Leaf(leaf));
		}

		let dims = Dims::new(measure, height, depth);
		lines.push((Node::HBox(BoxNode::new(children, dims)), height, depth, over, mdepth));
	}

	// The block-edge model, applied once the lines are set. The first line's top edge drops from the face
	// ascender to the cap height, and the last line's bottom edge rises from the descender to the baseline
	// (or to the depth of inline maths, which genuinely hangs below and must keep its room). This moves the
	// box edges the inter-block glue attaches to, not the baselines: the drawer seats each glyph at the
	// line-top plus the leaf's own ascent, so lifting the first line's top to the cap height would drop the
	// glyphs unless they are raised to match -- hence `raise_leaves`, which shifts every leaf of the first
	// line up by the same amount the top moved, keeping a footnote mark's or a script's raise intact. The
	// interline glue below reads the untouched tuple metrics, and never a line's own height above it nor its
	// own depth below it, so the within-paragraph pitch set by `leading` is left exactly as it was. A
	// single-line paragraph is both first and last, and takes both edges.
	if let Some(cap) = cap {
		if let Some(first) = lines.first_mut() {
			let natural = first.1;	// the line's own ascent, from the untouched tuple
			if natural > cap {
				let drop = natural - cap;
				if let Node::HBox(b) = &mut first.0 {
					b.dims.height = cap;
					raise_leaves(&mut b.list, drop);
				}
			}
		}
		if let Some(last) = lines.last_mut() {
			let md = last.4;
			if let Node::HBox(b) = &mut last.0 {
				b.dims.depth = md;
			}
		}
	}

	// Assemble the vertical list: each line, then the glue to the next. The glue sets the baselines
	// `leading` apart when the type is loose enough, and otherwise falls to a minimum so a tall line --
	// one carrying an inline fraction, say -- opens the space it needs rather than climbing into the
	// line above. The upper line's depth and the lower line's height are what the gap is measured from,
	// so a line's own height never has to stand in for its neighbour's.
	let heights:	Vec<Sp> = lines.iter().map(|l| l.1).collect();
	let overs:		Vec<Sp> = lines.iter().map(|l| l.3).collect();
	let count				= lines.len();
	let mut out = Vec::with_capacity(count * 2);
	for (i, (node, _height, depth_above, _over, _mdepth)) in lines.into_iter().enumerate() {
		out.push(node);
		if i + 1 < count {
			// The baselineskip glue, never less than the overshoot the lower line needs to clear the one
			// above; a plain pair of prose lines wants neither, so the gap stays zero as before.
			let want	= leading - depth_above - heights[i + 1];
			let mut gap	= if want > Sp::ZERO { want } else { Sp::ZERO };
			if overs[i + 1] > gap { gap = overs[i + 1]; }
			out.push(Node::Glue(Glue::fixed(gap)));
		}
	}
	Ok(out)
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE EVALUATOR'S LINE BREAKER                                               │
// └───────────────────────────────────────────────────────────────────────────┘
//
// Everything above serves the curated reader and goes with it at cut-over. What follows is the
// evaluator's breaker, a port of Typst 0.15.1's `typst-layout/src/inline/linebreak.rs` (Apache-2.0,
// (c) the Typst project authors), so that a paragraph breaks where the oracle breaks it:
//
// - Break opportunities are UAX #14's, a mandatory one after a hard line-break class, links broken by
//   Typst's own rule, and hyphenation points inside alphabetic words (Liang's patterns, through
//   `fe2o3_text::hyphen`, the port of the `hypher` automata Typst uses).
// - `linebreaks: "simple"` is first fit. `"optimized"` is Knuth-Plass over Typst's cost, not TeX's
//   demerits: a line's cost is (1 + badness + penalty)^2, badness 100|r|^3 (none for a ragged last line
//   that need not shrink), a runt cost of 100 for a lone word before a mandatory break, a hyphenation
//   cost of 135 scaled by `text.costs` and by 15% per character short of five either side of the break,
//   and the hyphenation cost again for two dashes in a row. The ratio stretches past a line's own
//   stretchability onto its justifiable glyphs, measured in half ems.
// - A first pass over cumulative width estimates finds a likely layout whose exact cost bounds the
//   exact pass, which then skips every predecessor that cannot beat it.

use crate::flow::inline::{
	line,
	Dash,
	Item as InlineItem,
	Line,
	Linebreaks,
	Prep,
};
use crate::fonts::is_default_ignorable;

use oxedyne_fe2o3_text::hyphen;
use oxedyne_fe2o3_text::unicode::lookup::Partitioned;
use oxedyne_fe2o3_text::unicode::prop::LineBreakClass;
use oxedyne_fe2o3_text::unicode::segment::word_boundaries;

type Cost = f64;

// Typst's costs: higher than Knuth-Plass's 50 for a hyphen, which hyphenates too eagerly without glue.
const DEFAULT_HYPH_COST:	Cost	= 135.0;
const DEFAULT_RUNT_COST:	Cost	= 100.0;
const MIN_RATIO:			f64		= -1.0;
const MIN_APPROX_RATIO:		f64		= -0.5;
const BOUND_EPS:			f64		= 1e-3;
const ABS_EPS:				f64		= 1e-4 / 127.0;	// Typst's length epsilon, in points

/// A place a line may end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Breakpoint {
	Normal,				// an ordinary opportunity, after a space say
	Mandatory,			// after a line feed, or at the end of the text
	Hyphen(u8, u8),		// inside a word, with the characters of the word before and after
}

/// Where a line's text stops counting for layout and for shaping: trailing spaces are shaped (to be
/// copied) but take no width, and a line feed is neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trim {
	pub layout:		usize,
	pub shaping:	usize,
}

impl Breakpoint {
	pub fn trim(self, start: usize, line: &str) -> Trim {
		match self {
			Breakpoint::Normal => {
				let trimmed = line.trim_end_matches(|c: char| c.is_whitespace() || is_default_ignorable(c));
				Trim { layout: start + trimmed.len(), shaping: start + line.len() }
			}
			Breakpoint::Mandatory => {
				let trimmed = line.trim_end_matches(|c: char| is_hard_break(c));
				let t = start + trimmed.len();
				Trim { layout: t, shaping: t }
			}
			Breakpoint::Hyphen(..) => {
				let t = start + line.len();
				Trim { layout: t, shaping: t }
			}
		}
	}

	pub fn is_hyphen(self) -> bool { matches!(self, Breakpoint::Hyphen(..)) }
}

/// Is the character of a mandatory line-break class (BK, CR, LF, NL)?
fn is_hard_break(c: char) -> bool {
	matches!(LineBreakClass::of(c), LineBreakClass::BK | LineBreakClass::CR | LineBreakClass::LF | LineBreakClass::NL)
}

/// Breaks a prepared paragraph into lines `width` points wide.
pub fn linebreak(p: &Prep, width: f64) -> Outcome<Vec<Line>> {
	match p.config.linebreaks {
		Linebreaks::Simple		=> linebreak_simple(p, width),
		Linebreaks::Optimized	=> linebreak_optimized(p, width),
	}
}

fn fits(width: f64, other: f64) -> bool {
	width + ABS_EPS >= other
}

/// First fit: each line as long as fits.
fn linebreak_simple(p: &Prep, width: f64) -> Outcome<Vec<Line>> {
	let mut lines: Vec<Line> = Vec::with_capacity(16);
	let mut start = 0;
	let mut last: Option<(Line, usize)> = None;
	for (end, bp) in breakpoints(p) {
		let mut attempt = res!(line(p, start, end, bp, lines.last()));
		if !fits(width, attempt.width) {
			if let Some((last_attempt, last_end)) = last.take() {
				lines.push(last_attempt);
				start = last_end;
				attempt = res!(line(p, start, end, bp, lines.last()));
			}
		}
		if bp == Breakpoint::Mandatory || !fits(width, attempt.width) {
			lines.push(attempt);
			start = end;
			last = None;
		} else {
			last = Some((attempt, end));
		}
	}
	if let Some((l, _)) = last {
		lines.push(l);
	}
	Ok(lines)
}

/// Knuth-Plass over Typst's cost, bounded by the exact cost of an estimated layout.
fn linebreak_optimized(p: &Prep, width: f64) -> Outcome<Vec<Line>> {
	let metrics = CostMetrics::compute(p);
	let bound = res!(linebreak_optimized_approximate(p, width, &metrics));
	linebreak_optimized_bounded(p, width, &metrics, bound)
}

struct Entry {
	pred:	usize,
	total:	Cost,
	line:	Line,
	end:	usize,
}

fn linebreak_optimized_bounded(p: &Prep, width: f64, metrics: &CostMetrics, upper_bound: Cost) -> Outcome<Vec<Line>> {
	let mut table: Vec<Entry> = vec![Entry { pred: 0, total: 0.0, line: Line::empty(), end: 0 }];
	let mut active = 0;
	let mut prev_end = 0;
	for (end, bp) in breakpoints(p) {
		let mut best: Option<Entry> = None;
		let mut line_lower_bound: Option<Cost> = None;
		for pred_index in active..table.len() {
			let (start, pred_total) = (table[pred_index].end, table[pred_index].total);
			let unbreakable = prev_end == start;
			if line_lower_bound.map_or(false, |lower| pred_total + lower > upper_bound + BOUND_EPS) {
				continue;
			}
			let attempt = res!(line(p, start, end, bp, Some(&table[pred_index].line)));
			let (line_ratio, line_cost) = ratio_and_cost(p, metrics, width, &table[pred_index].line, &attempt, bp, unbreakable);
			if line_ratio < metrics.min_ratio && active == pred_index {
				active += 1;
			}
			let total = pred_total + line_cost;
			if line_ratio > 0.0 && line_lower_bound.is_none() && !attempt.has_negative_width_items() {
				line_lower_bound = Some(line_cost);
			}
			if total > upper_bound + BOUND_EPS {
				continue;
			}
			if best.as_ref().map_or(true, |b| b.total >= total) {
				best = Some(Entry { pred: pred_index, total, line: attempt, end });
			}
		}
		if bp == Breakpoint::Mandatory {
			active = table.len();
		}
		if let Some(b) = best {
			table.push(b);
		}
		prev_end = end;
	}

	// A bound that proves faulty is dropped, as Typst's release build does.
	let last = table.len() - 1;
	if table[last].end != p.text.len() {
		if upper_bound.is_infinite() {
			return Err(err!("The optimised line breaker found no layout for a paragraph of {} bytes.", p.text.len(); Bug));
		}
		return linebreak_optimized_bounded(p, width, metrics, Cost::INFINITY);
	}
	let mut lines = Vec::with_capacity(16);
	let mut idx = last;
	while idx != 0 {
		table.truncate(idx + 1);
		let entry = match table.pop() {
			Some(e)	=> e,
			None	=> break,
		};
		idx = entry.pred;
		lines.push(entry.line);
	}
	lines.reverse();
	Ok(lines)
}

struct ApproxEntry {
	pred:			usize,
	total:			Cost,
	end:			usize,
	unbreakable:	bool,
	bp:				Breakpoint,
}

/// Knuth-Plass over cumulative estimates, then the exact cost of the layout it finds: a sound upper
/// bound for the exact pass, or infinity when that layout overflows.
fn linebreak_optimized_approximate(p: &Prep, width: f64, metrics: &CostMetrics) -> Outcome<Cost> {
	let est = Estimates::compute(p);
	let mut table: Vec<ApproxEntry> = vec![ApproxEntry {
		pred: 0, total: 0.0, end: 0, unbreakable: false, bp: Breakpoint::Mandatory,
	}];
	let mut active = 0;
	let mut prev_end = 0;
	for (end, bp) in breakpoints(p) {
		let mut best: Option<ApproxEntry> = None;
		for pred_index in active..table.len() {
			let pred = &table[pred_index];
			let start = pred.end;
			let unbreakable = prev_end == start;
			let justify = p.config.justify && bp != Breakpoint::Mandatory;
			let consecutive_dash = pred.bp.is_hyphen() && bp.is_hyphen();
			let trimmed_end = start + p.text.get(start..end).map_or(0, |t| t.trim_end().len());
			let line_ratio = raw_ratio(
				p,
				width,
				est.widths.estimate(start, trimmed_end) + if bp.is_hyphen() { metrics.approx_hyphen_width } else { 0.0 },
				est.stretchability.estimate(start, trimmed_end),
				est.shrinkability.estimate(start, trimmed_end),
				est.justifiables.estimate(start, trimmed_end),
			);
			let line_cost = raw_cost(metrics, bp, line_ratio, justify, unbreakable, consecutive_dash, true);
			if line_ratio < metrics.min_ratio && active == pred_index {
				active += 1;
			}
			let total = pred.total + line_cost;
			if best.as_ref().map_or(true, |b| b.total >= total) {
				best = Some(ApproxEntry { pred: pred_index, total, end, unbreakable, bp });
			}
		}
		if bp == Breakpoint::Mandatory {
			active = table.len();
		}
		if let Some(b) = best {
			table.push(b);
		}
		prev_end = end;
	}

	let mut indices = Vec::with_capacity(16);
	let mut idx = table.len() - 1;
	while idx != 0 {
		indices.push(idx);
		idx = table[idx].pred;
	}
	let mut pred = Line::empty();
	let mut start = 0;
	let mut exact = 0.0;
	for idx in indices.into_iter().rev() {
		let (end, bp, unbreakable) = (table[idx].end, table[idx].bp, table[idx].unbreakable);
		let attempt = res!(line(p, start, end, bp, Some(&pred)));
		let (ratio, cost) = ratio_and_cost(p, metrics, width, &pred, &attempt, bp, unbreakable);
		if ratio < metrics.min_ratio {
			return Ok(Cost::INFINITY);
		}
		pred = attempt;
		start = end;
		exact += cost;
	}
	Ok(exact)
}

fn ratio_and_cost(
	p:				&Prep,
	metrics:		&CostMetrics,
	available:		f64,
	pred:			&Line,
	attempt:		&Line,
	bp:				Breakpoint,
	unbreakable:	bool,
)
	-> (f64, Cost)
{
	let ratio = raw_ratio(p, available, attempt.width, attempt.stretchability(), attempt.shrinkability(), attempt.justifiables());
	let consecutive = pred.dash.is_some() && attempt.dash.is_some();
	let cost = raw_cost(metrics, bp, ratio, attempt.justify, unbreakable, consecutive, false);
	(ratio, cost)
}

/// How far a line must stretch (positive) or shrink (negative) to fill `available`, in multiples of its
/// adjustability; past full stretch, the extra is spread over its justifiable glyphs in half ems. Below
/// `MIN_RATIO` is overfull.
fn raw_ratio(p: &Prep, available: f64, width: f64, stretch: f64, shrink: f64, justifiables: usize) -> f64 {
	let mut delta = available - width;
	if delta == 0.0 || delta.abs() < ABS_EPS {
		delta = 0.0;
	}
	let adjustability = if delta >= 0.0 { stretch } else { shrink };
	// A line holding no text sums its stretch over nothing, which Rust gives as -0.0, and `f64::max`
	// may hand that sign back. Dividing by -0.0 turns an underfull line into an overfull one, which
	// drops the line's start from the active set, so every break after it is chosen from the second
	// breakpoint on. Typst's `Abs::max` is `Ord::max`, which on a tie returns the zero given.
	let mut ratio = delta / if adjustability > 0.0 { adjustability } else { 0.0 };
	if ratio.is_nan() {
		ratio = 0.0;
	}
	if ratio > 1.0 {
		let extra = (delta - adjustability) / justifiables.max(1) as f64;
		ratio = 1.0 + extra / (p.config.font_size / 2.0);
	}
	ratio.clamp(MIN_RATIO - 1.0, 10.0)
}

/// Typst's cost of a line: (1 + badness + penalty)^2.
fn raw_cost(
	metrics:			&CostMetrics,
	bp:					Breakpoint,
	ratio:				f64,
	justify:			bool,
	unbreakable:		bool,
	consecutive_dash:	bool,
	approx:				bool,
)
	-> Cost
{
	let badness = if ratio < metrics.min(approx) {
		1_000_000.0
	} else if bp != Breakpoint::Mandatory || justify || ratio < 0.0 {
		100.0 * scalar(ratio.abs()).powi(3)
	} else {
		0.0
	};
	let mut penalty = 0.0;
	if unbreakable && bp == Breakpoint::Mandatory {
		penalty += metrics.runt_cost;
	}
	if let Breakpoint::Hyphen(l, r) = bp {
		const LIMIT: u8 = 5;
		let steps = LIMIT.saturating_sub(l) + LIMIT.saturating_sub(r);
		let extra = 0.15 * steps as f64;
		penalty += (1.0 + extra) * metrics.hyph_cost;
	}
	if consecutive_dash {
		penalty += metrics.hyph_cost;
	}
	scalar(1.0 + badness + penalty).powi(2)
}

/// A float with NaN read as zero, as Typst's `Scalar` reads it.
fn scalar(v: f64) -> f64 {
	if v.is_nan() { 0.0 } else { v }
}

/// The break opportunities of the paragraph's text, in order, each with its kind.
pub fn breakpoints(p: &Prep) -> Vec<(usize, Breakpoint)> {
	let text = p.text.as_str();
	let mut out = Vec::new();
	if text.is_empty() {
		out.push((0, Breakpoint::Mandatory));
		return out;
	}
	let hyphenate = p.config.hyphenate != Some(false);
	let points: Vec<usize> = oxedyne_fe2o3_text::unicode::linebreak::line_breaks(text)
		.into_iter().map(|o| o.offset).collect();
	let mut last = 0usize;
	let mut k = 0usize;
	loop {
		// Links break by Typst's own rule rather than UAX #14's.
		let (head, tail) = text.split_at(last);
		if head.ends_with("://") || tail.starts_with("www.") {
			let (link, _) = crate::syntax::lexer::link_prefix(tail);
			let base = last;
			linebreak_link(link, |i| out.push((base + i, Breakpoint::Normal)));
			last += link.len();
			while k < points.len() && points[k] < last {
				k += 1;
			}
		}
		let point = match points.get(k) {
			Some(pt)	=> *pt,
			None		=> break,
		};
		k += 1;
		let c = match text.get(..point).and_then(|t| t.chars().next_back()) {
			Some(c)	=> c,
			None	=> continue,
		};
		let bp = if point == text.len() {
			Breakpoint::Mandatory
		} else if is_hard_break(c) {
			Breakpoint::Mandatory
		} else if LineBreakClass::of(c) == LineBreakClass::CM
			&& text.get(point..).map_or(false, |t| t.starts_with('\u{FFFC}'))
			&& last + c.len_utf8() == point
		{
			continue;
		} else {
			Breakpoint::Normal
		};
		if hyphenate && last < point {
			if let Some(span) = text.get(last..point) {
				let bounds = word_boundaries(span);
				for w in bounds.windows(2) {
					let seg = &span[w[0]..w[1]];
					if !seg.is_empty() && seg.chars().all(char::is_alphabetic) {
						hyphenations(p, last + w[0], seg, &mut out);
					}
				}
			}
		}
		out.push((point, bp));
		last = point;
	}
	out
}

/// The hyphenation points inside `word`, which starts at byte `offset`.
fn hyphenations(p: &Prep, offset: usize, word: &str, out: &mut Vec<(usize, Breakpoint)>) {
	let lang = match lang_at(p, offset) {
		Some(l)	=> l,
		None	=> return,
	};
	let count = word.chars().count();
	for at in hyphen::hyphenate(word, lang) {
		let pos = offset + at;
		if !hyphenate_at(p, pos) {
			continue;
		}
		// Not after a glue, a word joiner or a zero-width joiner.
		if let Some(prev) = word.get(..at).and_then(|t| t.chars().next_back()) {
			if matches!(LineBreakClass::of(prev), LineBreakClass::GL | LineBreakClass::WJ | LineBreakClass::ZWJ) {
				continue;
			}
		}
		let chars = word.get(..at).map_or(0, |t| t.chars().count());
		let l = chars.min(255) as u8;
		let r = (count - chars).min(255) as u8;
		out.push((pos, Breakpoint::Hyphen(l, r)));
	}
}

/// Is hyphenation on at `offset`: the paragraph's uniform setting, else that of the text there, which is
/// `justify` when `auto`.
fn hyphenate_at(p: &Prep, offset: usize) -> bool {
	match p.config.hyphenate {
		Some(h)	=> h,
		None	=> match &p.get(offset).1 {
			InlineItem::Text(t)	=> t.props.hyphenate.unwrap_or(p.config.justify),
			_				=> false,
		},
	}
}

/// The patterns for the language at `offset`: the paragraph's uniform language, else the text's there,
/// when it is a two-letter code patterns exist for.
fn lang_at(p: &Prep, offset: usize) -> Option<hyphen::Lang> {
	let lang = match &p.config.lang {
		Some(l)	=> l.clone(),
		None	=> match &p.get(offset).1 {
			InlineItem::Text(t)	=> t.props.lang.clone(),
			_				=> return None,
		},
	};
	if lang.len() != 2 {
		return None;
	}
	hyphen::Lang::from_iso(&lang)
}

/// Break opportunities inside a URL: between two non-alphanumerics, and at a change between letters and
/// digits, never after an opening bracket; a very long piece breaks anywhere.
fn linebreak_link<F: FnMut(usize)>(link: &str, mut f: F) {
	#[derive(PartialEq)]
	enum Class {
		Alphabetic,
		Digit,
		Open,
		Other,
	}
	fn class_of(c: char) -> Class {
		if c.is_alphabetic() {
			Class::Alphabetic
		} else if c.is_numeric() {
			Class::Digit
		} else if matches!(c, '(' | '[') {
			Class::Open
		} else {
			Class::Other
		}
	}
	let mut offset = 0;
	let mut prev = Class::Other;
	for (end, c) in link.char_indices() {
		let class = class_of(c);
		if end > 0
			&& prev != Class::Open
			&& if class == Class::Other { prev == Class::Other } else { class != prev }
		{
			let piece = &link[offset..end];
			if piece.len() < 16 {
				offset = end;
				f(offset);
			} else {
				for ch in piece.chars() {
					offset += ch.len_utf8();
					f(offset);
				}
			}
		}
		prev = class;
	}
}

struct CostMetrics {
	min_ratio:				f64,
	min_approx_ratio:		f64,
	approx_hyphen_width:	f64,
	hyph_cost:				Cost,
	runt_cost:				Cost,
}

impl CostMetrics {
	fn compute(p: &Prep) -> Self {
		Self {
			min_ratio:				if p.config.justify { MIN_RATIO } else { 0.0 },
			min_approx_ratio:		if p.config.justify { MIN_APPROX_RATIO } else { 0.0 },
			approx_hyphen_width:	0.33 * p.config.font_size,
			hyph_cost:				DEFAULT_HYPH_COST * p.config.costs.hyphenation,
			runt_cost:				DEFAULT_RUNT_COST * p.config.costs.runt,
		}
	}

	fn min(&self, approx: bool) -> f64 {
		if approx { self.min_approx_ratio } else { self.min_ratio }
	}
}

/// Cumulative per-byte sums of the width, stretch, shrink and justifiable count, for estimates.
struct Estimates {
	widths:			Cumulative<f64>,
	stretchability:	Cumulative<f64>,
	shrinkability:	Cumulative<f64>,
	justifiables:	Cumulative<usize>,
}

impl Estimates {
	fn compute(p: &Prep) -> Self {
		let cap = p.text.len();
		let mut widths = Cumulative::with_capacity(cap);
		let mut stretchability = Cumulative::with_capacity(cap);
		let mut shrinkability = Cumulative::with_capacity(cap);
		let mut justifiables = Cumulative::with_capacity(cap);
		for (range, item) in &p.items {
			match item {
				InlineItem::Text(t) => for g in t.kept() {
					let n = g.range.1 - g.range.0;
					widths.push(n, g.x_advance * g.size);
					stretchability.push(n, (g.stretch.0 + g.stretch.1) * g.size);
					shrinkability.push(n, (g.shrink.0 + g.shrink.1) * g.size);
					justifiables.push(n, g.justifiable as usize);
				},
				other => widths.push(range.1 - range.0, other.natural_width()),
			}
			widths.adjust(range.1);
			stretchability.adjust(range.1);
			shrinkability.adjust(range.1);
			justifiables.adjust(range.1);
		}
		Self { widths, stretchability, shrinkability, justifiables }
	}
}

struct Cumulative<T> {
	total:	T,
	summed:	Vec<T>,
}

impl<T> Cumulative<T>
	where T: Default + Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T>
{
	fn with_capacity(cap: usize) -> Self {
		let total = T::default();
		let mut summed = Vec::with_capacity(cap);
		summed.push(total);
		Self { total, summed }
	}

	fn adjust(&mut self, len: usize) {
		self.summed.resize(len, self.total);
	}

	fn push(&mut self, byte_len: usize, metric: T) {
		self.total = self.total + metric;
		for _ in 0..byte_len {
			self.summed.push(self.total);
		}
	}

	fn estimate(&self, start: usize, end: usize) -> T {
		self.get(end) - self.get(start)
	}

	fn get(&self, index: usize) -> T {
		match index.checked_sub(1) {
			None	=> T::default(),
			Some(i)	=> self.summed.get(i).copied().unwrap_or(self.total),
		}
	}
}

/// Does a line ending in this text end with a dash, and which?
pub fn dash_of(bp: Breakpoint, full: &str) -> Option<Dash> {
	if bp.is_hyphen() || full.ends_with('\u{00AD}') {
		Some(Dash::Soft)
	} else if full.ends_with('-') {
		Some(Dash::Hard)
	} else if full.ends_with(['\u{2013}', '\u{2014}']) {
		Some(Dash::Other)
	} else {
		None
	}
}
