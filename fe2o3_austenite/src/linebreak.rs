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

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::Dir,
};
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
)
	-> Outcome<Vec<Node>>
{
	let items = res!(build_items(fonts, role, dir, size, text));
	if items.is_empty() {
		return Ok(Vec::new());
	}
	let breaks	= optimal_breaks(&items, measure);
	let lines	= res!(set_lines(&items, &breaks, measure, leading));
	Ok(lines)
}

/// One run of a segmented paragraph: a stretch of plain text, or a pre-built footnote mark leaf. A
/// paragraph carrying footnotes is a sequence of these -- the text either side of each mark, and the
/// mark itself -- so a mark clings to the word before it and line breaking still flows around it.
pub enum Piece {
	Text(String),
	Mark(Leaf),	// a footnote mark, already shaped as a raised superscript (LeafKind::Mark)
}

/// Breaks a segmented paragraph the way [`break_paragraph`] breaks a plain one, but with footnote marks
/// woven into the stream. Each text piece contributes its words and interword glue; each mark piece a
/// rigid superscript box that never breaks and that the following space may break after. The result is
/// the same vertical list of HBox lines, so the driver sets it with no special case -- and a paragraph
/// of a single text piece produces exactly what [`break_paragraph`] would.
pub fn break_paragraph_pieces(
	fonts:		Arc<FontSet>,
	role:		Role,
	dir:		Dir,
	size:		Sp,
	pieces:		&[Piece],
	measure:	Sp,
	leading:	Sp,
)
	-> Outcome<Vec<Node>>
{
	let (sp_w, stretch, shrink, hyphen) = res!(interword(fonts.clone(), role, dir, size));
	let hyph = Hyphenator::en_us();

	let mut items = Vec::new();
	for piece in pieces {
		match piece {
			Piece::Text(text) => {
				res!(push_text_run(
					&mut items, fonts.clone(), role, dir, size, text, sp_w, stretch, shrink, &hyph, &hyphen));
			},
			Piece::Mark(leaf) => {
				items.push(Item {
					kind: Kind::Mark(leaf.clone()), width: leaf.dims.width, stretch: Sp::ZERO,
					shrink: Sp::ZERO, penalty: Penalty::INFINITY, flagged: false, hyphen: None });
			},
		}
	}
	push_finish(&mut items);	// the forced break that ends the paragraph, whatever the last piece was

	if items.is_empty() {
		return Ok(Vec::new());
	}
	let breaks	= optimal_breaks(&items, measure);
	let lines	= res!(set_lines(&items, &breaks, measure, leading));
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
)
	-> Outcome<Vec<Item>>
{
	let (sp_w, stretch, shrink, hyphen) = res!(interword(fonts.clone(), role, dir, size));
	let hyph = Hyphenator::en_us();

	let mut items = Vec::new();
	res!(push_text_run(&mut items, fonts, role, dir, size, text, sp_w, stretch, shrink, &hyph, &hyphen));
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
			res!(push_word(items, fonts.clone(), role, dir, size, word, hyph, hyphen));
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
fn push_word(
	items:	&mut Vec<Item>,
	fonts:	Arc<FontSet>,
	role:	Role,
	dir:	Dir,
	size:	Sp,
	word:	&str,
	hyph:	&Hyphenator,
	hyphen:	&ShapedText,
)
	-> Outcome<()>
{
	// The alphabetic core, and where it starts in the word, so break points map back to word bytes.
	let start	= word.len() - word.trim_start_matches(|c: char| !c.is_alphabetic()).len();
	let core	= word.trim_matches(|c: char| !c.is_alphabetic());
	let points	= if core.chars().count() >= HYPHEN_MIN { hyph.hyphenate(core) } else { Vec::new() };

	if points.is_empty() {
		let shaped	= res!(ShapedText::new(fonts, role, dir, size, word));
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
		let shaped	= res!(ShapedText::new(fonts.clone(), role, dir, size, frag));
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
		Kind::Glued		=> i > 0 && matches!(items[i - 1].kind, Kind::Boxed(_) | Kind::Mark(_))
							&& items[i].penalty < Penalty::INFINITY,
		Kind::Pen		=> items[i].penalty < Penalty::INFINITY,
		Kind::Boxed(_)	=> false,
		Kind::Mark(_)	=> false,	// a mark clings to its word; the space after it may break
	}
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
fn optimal_breaks(items: &[Item], measure: Sp) -> Vec<isize> {
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
			let z		= (sz[b] - sz[lower]) as f64;
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

/// Sets each chosen line as an HBox of shaped words and justified glue, joined by leading glue. A
/// line ending at a forced break keeps natural spacing (flush left); every other line distributes
/// its slack by the adjustment ratio.
fn set_lines(
	items:		&[Item],
	breaks:		&[isize],
	measure:	Sp,
	leading:	Sp,
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

	let mut out		= Vec::new();
	let windows		= breaks.windows(2).count();
	for (li, w) in breaks.windows(2).enumerate() {
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
		for item in items.iter().take(hi).skip(lower) {
			match &item.kind {
				Kind::Boxed(shaped) => {
					let leaf = Leaf::text(shaped.clone());
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
				Kind::Glued => {
					// Justification lives in the glue: the natural space plus the ratio's share of its
					// elasticity, so the driver's plain left-to-right pass fills the measure.
					let nat = item.width;
					let adj = if forced {
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

		// A taken discretionary draws its hyphen as the line's last box.
		if let Some(h) = &items[hi].hyphen {
			let leaf = Leaf::text(h.clone());
			if leaf.dims.height > height { height = leaf.dims.height; }
			if leaf.dims.depth > depth { depth = leaf.dims.depth; }
			children.push(Node::Leaf(leaf));
		}

		let dims = Dims::new(measure, height, depth);
		out.push(Node::HBox(BoxNode::new(children, dims)));

		// Leading between baselines, but not after the final line.
		if li + 1 < windows {
			let vextent	= height + depth;
			let gap		= if leading > vextent { leading - vextent } else { Sp::ZERO };
			out.push(Node::Glue(Glue::fixed(gap)));
		}
	}
	Ok(out)
}
