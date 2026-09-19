//! The two-pass streaming driver, and its convergence loop.
//!
//! This is the heart of Phase 0. A pass composes the document: it runs the box-glue-penalty stream
//! through a greedy vertical page breaker, places each line, records every anchor it meets, and
//! resolves a forward reference against the width reserved for it. Pass A sees an empty ledger, so
//! backward-looking anchors resolve but forward references show nothing yet. Pass B re-composes with
//! Pass A's ledger loaded, so a forward reference now reads the value it points at.
//!
//! The loop terminates two ways, and only two, and it is honest about which:
//!
//! * *Converged.* From the second pass on, if the new ledger is stable against the last -- same page
//!   count, and no anchor changed page -- the document has stopped moving. The current pages are
//!   final. This is the normal outcome, and by construction it is two passes when every forward
//!   reference fits the width reserved for it.
//! * *Did not converge.* If the pass cap is reached with the ledger still moving, the driver does
//!   not guess. It differences the last two ledgers and returns an error naming the anchor that
//!   moved, the pages it moved between, and any reference whose realised value overflowed its
//!   reservation -- which is the thing that broke the two-pass guarantee.

use crate::{
	ir::{
		BoxNode,
		Dims,
		FloatNode,
		FloatPlacement,
		Footnote,
		Leaf,
		LeafKind,
		Metrics,
		Node,
		Sp,
	},
	ledger::{
		Anchor,
		Ledger,
		Position,
	},
	page::{
		Frame,
		Page,
		PageGeometry,
		Placed,
		PlacedKind,
	},
};

use oxedyne_fe2o3_core::prelude::*;

/// The spacing that frames the footnotes at the foot of a page: the gap above the separator rule, the
/// rule itself and how wide it runs, the gap below it before the first note, and the gap between one
/// note and the next. Every length is scaled points, so the foot never leaves the integer domain the
/// driver breaks on. The note lines themselves carry their own leading; this is only the furniture
/// around them.
#[derive(Clone, Copy, Debug)]
pub struct FootStyle {
	pub gap_above_rule:	Sp,
	pub rule_thick:		Sp,
	pub rule_width:		Sp,
	pub gap_below_rule:	Sp,
	pub gap_between:	Sp,
}

impl Default for FootStyle {
	fn default() -> Self {
		Self {
			gap_above_rule:	Sp::from_pt(8.0),
			rule_thick:		Sp::from_pt(0.4),
			rule_width:		Sp::from_pt(120.0),	// a short rule, about a third of a text block
			gap_below_rule:	Sp::from_pt(4.0),
			gap_between:	Sp::from_pt(3.0),
		}
	}
}

/// A trivial in-memory document: a vertical box-glue-penalty stream, the geometry every page takes, and
/// the foot spacing its footnotes are framed with. Phase 0 has one geometry for the whole document;
/// per-chapter geometry is later.
#[derive(Clone, Debug)]
pub struct Document {
	pub nodes:	Vec<Node>,
	pub geom:	PageGeometry,
	pub foot:	FootStyle,
}

impl Document {
	pub fn new(nodes: Vec<Node>, geom: PageGeometry) -> Self {
		Self { nodes, geom, foot: FootStyle::default() }
	}
}

/// How hard the driver tries to converge. `max_passes` caps the loop; when it is reached with the
/// ledger still moving, the driver reports a non-convergence rather than looping forever. Three is
/// the architecture's stated worst case (two passes, plus one when a reservation is exceeded); a
/// little headroom above that catches a genuine oscillation without hiding it.
#[derive(Clone, Copy, Debug)]
pub struct Config {
	pub max_passes:	u32,
}

impl Default for Config {
	fn default() -> Self {
		Self { max_passes: 4 }
	}
}

/// The result of a converged compile: the final pages, the ledger that fixed them, and how many
/// passes it took -- the last being the number the flat-memory claim is proved against.
#[derive(Debug)]
pub struct CompileOutput {
	pub pages:	Vec<Page>,
	pub ledger:	Ledger,
	pub passes:	u32,
}

/// Runs the document to a fixed point, or reports why it would not settle.
pub fn run<M: Metrics>(
	doc:		&Document,
	metrics:	&M,
	cfg:		Config,
)
	-> Outcome<CompileOutput>
{
	if cfg.max_passes < 2 {
		return Err(err!(
			"The driver needs at least two passes to resolve a forward reference, but max_passes \
			is {}.", cfg.max_passes; Input, Invalid, Configuration));
	}
	let mut prev = Ledger::new();	// Pass A sees no resolved forward references.
	let mut pass = 0u32;
	loop {
		pass += 1;
		let (pages, ledger) = res!(compose(doc, metrics, &prev));

		// A ledger is only meaningfully stable once a second pass has had the first pass's ledger to
		// read; comparing Pass A against the empty ledger it started from would converge falsely.
		if pass >= 2 && ledger.is_stable_against(&prev) {
			return Ok(CompileOutput { pages, ledger, passes: pass });
		}

		if pass >= cfg.max_passes {
			return Err(non_convergence(pass, &ledger, &prev));
		}
		prev = ledger;
	}
}

/// One composition pass over the whole document. Greedy: material is stacked until the next box
/// would overflow the text block, then the page is broken. Each page's frame is built, kept in the
/// returned vector, and would be dropped after writing in a streaming caller; Phase 0 returns them
/// together so the harness can write them and count them.
///
/// Footnotes couple to the break: a footnote's note is set at the foot of the page its mark lands on,
/// so the effective bottom shrinks by the note's height as marks accumulate, and a line is judged
/// against a bottom already charged for its own note. This does not threaten convergence. A note's
/// height is fixed at author time, independent of the layout, so the reservation a page pays is a pure
/// function of which marks fell on it; the fill is as deterministic as it was without footnotes. What
/// footnotes can do is push a heading or an anchor to a later page than a footnote-free fill would --
/// exactly the movement the ledger already reconciles across passes, and the loop settles on it in the
/// usual way.
fn compose<M: Metrics>(
	doc:		&Document,
	metrics:	&M,
	incoming:	&Ledger,
)
	-> Outcome<(Vec<Page>, Ledger)>
{
	let geom	= doc.geom;
	let top		= geom.content_top();
	let bottom	= geom.content_top() + geom.content_height();

	let mut ledger	= Ledger::new();
	let mut pages	= Vec::new();
	let mut frame	= Frame::new();
	let mut page_no	= 1u32;
	let mut y		= top;
	let mut at_top	= true;	// just after a break, leading glue and penalties are discarded

	// The footnotes whose marks have landed on the page under construction. Each reduces the height left
	// for the body by its own height plus the separator furniture, so the effective bottom shrinks as
	// they accumulate; they are set at the foot when the page closes, then this is cleared.
	let mut notes:	Vec<Footnote> = Vec::new();

	// The floats deferred out of the flow, in document order, awaiting a page with room. A float that will
	// not fit where it stands joins this queue rather than pushing the flow down and leaving a gap; the
	// text that follows it backfills the current page, and the float is set at the top (or foot) of the
	// next page a flush finds room on -- Typst's `placement: auto` top/bottom semantics. The heights are
	// fixed at author time, exactly as a footnote's is, so the queue is a pure function of the stream and
	// the geometry and does not threaten convergence. `bot_reserve` is the height a page's foot floats
	// claimed, held back from the body for the life of that page.
	let mut pending:	Vec<FloatNode>	= Vec::new();
	let mut bands					= FloatBands::empty();

	for node in &doc.nodes {
		match node {
			Node::Glue(g) => {
				// Glue at the very top of a page is discarded, as TeX discards it, so a page does not
				// open with blank space left over from the break.
				if !at_top {
					y += g.natural;
				}
			},
			Node::Penalty(p) => {
				if p.is_forced() && !frame.is_empty() {
					res!(finish_page(
						&mut pages, &mut frame, &mut page_no, &mut y, top, geom,
						&mut notes, &doc.foot, bottom, bands.bot_reserve, metrics, incoming, &mut ledger));
					bands = FloatBands::empty();
					res!(flush_floats(
						&mut pending, &mut frame, &mut y, &mut bands, Sp::ZERO, page_no, geom, top, bottom,
						metrics, incoming, &mut ledger));
					// The body resumes at the region top below any flushed top floats, its leading collapsed.
					at_top = true;
				}
			},
			Node::Anchor(id) => {
				// The body-start and back-matter markers are noticed by `Ledger::record` as the anchor is
				// recorded, so a heading nested in a keep box (the inline-heading idiom) fixes the body start
				// exactly as a top-level chapter opener does.
				ledger.record(Anchor::new(id.clone(), Position::new(page_no, geom.content_left(), y)));
			},
			Node::Float(f) => {
				// A float never sets at the cursor -- it is inserted into the page's top or foot region and the
				// body already on the page shifts to make room (Typst's relayout). An earlier queued float holds
				// this one back so document order is kept; a float that will not fit here defers likewise.
				if !pending.is_empty() {
					pending.push(f.clone());
				} else {
					let was_empty	= frame.is_empty();
					let foot_now	= foot_reserve(&notes, &[], &doc.foot);
					// Clearance counts in the fit only when the page already carries content; an empty page
					// seats the float regardless (`force`) and overflows rather than deferring forever.
					let ok = res!(try_insert_float(
						f, !was_empty, was_empty, &mut y, &mut bands, foot_now, page_no, geom, top, bottom,
						metrics, incoming, &mut frame, &mut ledger));
					if ok {
						// A float on an otherwise-empty page keeps the region top for the body that follows, so
						// that body's leading collapses; mid-page, the body already sets the cursor.
						at_top = was_empty;
					} else {
						pending.push(f.clone());
					}
				}
			},
			Node::HBox(_) | Node::VBox(_) | Node::Leaf(_) => {
				let v = node.vextent();

				// The footnotes this node's marks introduce. The break is judged against a bottom already
				// shrunk by the notes on the page, by these, and by any foot floats the page carries -- so a
				// line and its own note never part, and neither collides with a bottom float.
				let mut marks: Vec<Footnote> = Vec::new();
				collect_marks(node, &mut marks);
				let reserve = foot_reserve(&notes, &marks, &doc.foot);
				if !frame.is_empty() && y + v > bottom - bands.bot_reserve - reserve {
					res!(finish_page(
						&mut pages, &mut frame, &mut page_no, &mut y, top, geom,
						&mut notes, &doc.foot, bottom, bands.bot_reserve, metrics, incoming, &mut ledger));
					bands = FloatBands::empty();
					res!(flush_floats(
						&mut pending, &mut frame, &mut y, &mut bands, Sp::ZERO, page_no, geom, top, bottom,
						metrics, incoming, &mut ledger));
				}
				res!(place_node(node, y, page_no, geom, metrics, incoming, &mut frame, &mut ledger));
				notes.append(&mut marks);	// its marks now belong to the page the node landed on
				y += v;
				at_top = false;
			},
		}
	}

	// The floats still queued at the end are set now: as many as fit on the current page (respecting the
	// bands and footnotes it already carries), then a fresh page per remaining batch, so the queue always
	// drains rather than a float being lost. A fresh, empty page seats at least its front float (even one
	// taller than the column, which overflows rather than spins), so `pending` shrinks by at least one per
	// fresh page and the loop terminates.
	loop {
		let foot_now = foot_reserve(&notes, &[], &doc.foot);
		res!(flush_floats(
			&mut pending, &mut frame, &mut y, &mut bands, foot_now, page_no, geom, top, bottom,
			metrics, incoming, &mut ledger));
		if pending.is_empty() {
			break;
		}
		res!(finish_page(
			&mut pages, &mut frame, &mut page_no, &mut y, top, geom,
			&mut notes, &doc.foot, bottom, bands.bot_reserve, metrics, incoming, &mut ledger));
		bands = FloatBands::empty();
	}

	// The last page holds whatever is left, unless nothing is; its footnotes are set at its foot first.
	if !frame.is_empty() {
		res!(lay_footnotes(&mut frame, &notes, page_no, geom, &doc.foot, bottom - bands.bot_reserve, metrics, incoming, &mut ledger));
		pages.push(Page::new(page_no, geom, std::mem::take(&mut frame)));
	} else if pages.is_empty() {
		// A document with no material is still one blank page, so a page count is always at least one.
		pages.push(Page::new(page_no, geom, Frame::new()));
	}

	ledger.total_pages = pages.len() as u32;
	Ok((pages, ledger))
}

/// Closes the current page: sets its footnotes at the foot, stores it, clears the note accumulator, and
/// resets the frame and cursor before advancing the folio.
#[allow(clippy::too_many_arguments)]
fn finish_page<M: Metrics>(
	pages:		&mut Vec<Page>,
	frame:		&mut Frame,
	page_no:	&mut u32,
	y:			&mut Sp,
	top:		Sp,
	geom:		PageGeometry,
	notes:		&mut Vec<Footnote>,
	foot:		&FootStyle,
	bottom:		Sp,
	bot_reserve:	Sp,	// height a foot float claimed on this page; footnotes sit above it
	metrics:	&M,
	incoming:	&Ledger,
	ledger:		&mut Ledger,
)
	-> Outcome<()>
{
	res!(lay_footnotes(frame, notes, *page_no, geom, foot, bottom - bot_reserve, metrics, incoming, ledger));
	pages.push(Page::new(*page_no, geom, std::mem::take(frame)));
	notes.clear();
	*page_no += 1;
	*y = top;
	Ok(())
}

/// The side an `auto` float settles on: Typst's midpoint rule. `used` is the region height already
/// consumed, `need` the float's height plus its clearance, `base` the full page-text height. The float
/// goes to the top when its own midpoint (`used + need/2`) would fall in the upper half of the page were
/// it set in the flow, and to the foot otherwise. Computed in i64 so a full page's scaled points cannot
/// overflow the doubling.
fn auto_side(used: Sp, need: Sp, base: Sp) -> FloatPlacement {
	let lhs = (used.raw() as i64) * 2 + (need.raw() as i64);
	if lhs <= base.raw() as i64 {
		FloatPlacement::Top
	} else {
		FloatPlacement::Bottom
	}
}

/// Sets a float's material as a small vertical list from `y_top`: each child placed like a keep box's, and
/// the float's own [`Float`](crate::ledger::AnchorKind::Float) anchor recorded at the y it reaches, so a
/// cross-reference resolves the page and position it settled on. The material carries no framing glue --
/// the caller lays the clearance around it -- so this places the list as it stands.
fn place_float<M: Metrics>(
	f:			&FloatNode,
	y_top:		Sp,
	page_no:	u32,
	geom:		PageGeometry,
	metrics:	&M,
	incoming:	&Ledger,
	frame:		&mut Frame,
	ledger:		&mut Ledger,
)
	-> Outcome<()>
{
	let mut yy = y_top;
	for child in &f.list {
		match child {
			Node::Glue(g) => {
				yy += g.natural;
			},
			Node::HBox(b) => {
				res!(place_line(b, yy, page_no, geom, metrics, incoming, frame, ledger));
				yy += b.dims.vextent();
			},
			Node::VBox(b) => {
				res!(place_vbox(b, yy, page_no, geom, metrics, incoming, frame, ledger));
				yy += b.dims.vextent();
			},
			Node::Leaf(l) => {
				res!(place_leaf(l, geom.content_left(), yy, page_no, metrics, incoming, frame, ledger));
				yy += l.dims.vextent();
			},
			Node::Anchor(id) => {
				ledger.record(Anchor::new(id.clone(), Position::new(page_no, geom.content_left(), yy)));
			},
			Node::Penalty(_)	=> (),
			// A float never nests inside another float; a nested one would be a construction error, so it is
			// left unplaced rather than silently flattened.
			Node::Float(_)		=> (),
		}
	}
	Ok(())
}

/// A page's float bands: the top band already stacked (top floats plus their clearances) and the foot band
/// already reserved (foot floats plus their clearances). The body fills what is left between them.
#[derive(Clone, Copy, Debug)]
struct FloatBands {
	top_used:		Sp,
	bot_reserve:	Sp,
}

impl FloatBands {
	fn empty() -> Self { Self { top_used: Sp::ZERO, bot_reserve: Sp::ZERO } }
}

/// Inserts one float into the current page's top or foot region by Typst's rule, shifting the body (a top
/// float) or the existing foot band (a foot float) to make room -- Typst re-flows the region on a float
/// insertion; this shifts only what moved. `clearance_flag` is whether the clearance counts in the FIT and
/// midpoint budget: Typst sets it only when the page already carries flow content, and a re-queued float is
/// re-processed with it false. The clearance is ALWAYS laid in the band, per Typst's finalize. `force`
/// seats the float even when it will not fit (an empty page's front float, which overflows rather than
/// deferring forever). `foot_now` is the footnote furniture already reserved, so a float never lands over
/// the notes. Returns `true` when placed, `false` when it does not fit and `force` is unset.
#[allow(clippy::too_many_arguments)]
fn try_insert_float<M: Metrics>(
	f:				&FloatNode,
	clearance_flag:	bool,
	force:			bool,
	y:				&mut Sp,
	bands:			&mut FloatBands,
	foot_now:		Sp,
	page_no:		u32,
	geom:			PageGeometry,
	top:			Sp,
	bottom:			Sp,
	metrics:		&M,
	incoming:		&Ledger,
	frame:			&mut Frame,
	ledger:			&mut Ledger,
)
	-> Outcome<bool>
{
	let base		= geom.content_height();
	let band		= f.height + f.clearance;	// finalize always lays the clearance
	let need_fit	= f.height + if clearance_flag { f.clearance } else { Sp::ZERO };
	let remaining	= (bottom - bands.bot_reserve - foot_now) - *y;
	if need_fit > remaining && !force {
		return Ok(false);
	}
	let used = base - remaining;
	let side = match f.placement {
		FloatPlacement::Top		=> FloatPlacement::Top,
		FloatPlacement::Bottom	=> FloatPlacement::Bottom,
		FloatPlacement::Auto	=> auto_side(used, need_fit, base),
	};
	match side {
		FloatPlacement::Bottom => {
			// Shift the existing foot band up by this float's band and seat the new float at the very foot,
			// so document order runs top-to-bottom down the foot region (the earliest foot float highest).
			frame.shift_y(-band, bottom - bands.bot_reserve, Sp(i32::MAX));
			ledger.shift_anchors(page_no, -band, bottom - bands.bot_reserve, Sp(i32::MAX));
			res!(place_float(f, bottom - f.height, page_no, geom, metrics, incoming, frame, ledger));
			bands.bot_reserve = bands.bot_reserve + band;
		},
		// Top (auto resolved to top or bottom above, so this arm is top).
		_ => {
			// Seat the float below the top band already stacked and shift the body below it down, so document
			// order runs top-to-bottom down the top region (the earliest top float highest).
			let at = top + bands.top_used;
			frame.shift_y(band, at, bottom - bands.bot_reserve);
			ledger.shift_anchors(page_no, band, at, bottom - bands.bot_reserve);
			res!(place_float(f, at, page_no, geom, metrics, incoming, frame, ledger));
			bands.top_used = bands.top_used + band;
			*y += band;
		},
	}
	Ok(true)
}

/// Sets the queued floats that fit on the current page, in document order, updating its [`FloatBands`].
/// A queued float is re-processed with `clearance_flag` false (Typst's relayout does the same); the front
/// float of an empty page is seated even when it will not fit, so the queue always drains, and flushing
/// stops at the first float that will not fit so a float never jumps ahead of an earlier one. `foot_now` is
/// the footnote furniture already on the page (zero on a freshly opened one), so a float never lands over
/// the notes -- the end-of-document drain calls this on a part-filled page and must respect them.
#[allow(clippy::too_many_arguments)]
fn flush_floats<M: Metrics>(
	pending:	&mut Vec<FloatNode>,
	frame:		&mut Frame,
	y:			&mut Sp,
	bands:		&mut FloatBands,
	foot_now:	Sp,
	page_no:	u32,
	geom:		PageGeometry,
	top:		Sp,
	bottom:		Sp,
	metrics:	&M,
	incoming:	&Ledger,
	ledger:		&mut Ledger,
)
	-> Outcome<()>
{
	let mut placed_any = false;
	while !pending.is_empty() {
		let force = frame.is_empty() && !placed_any;
		let f = pending[0].clone();
		let ok = res!(try_insert_float(
			&f, false, force, y, bands, foot_now, page_no, geom, top, bottom, metrics, incoming, frame, ledger));
		if ok {
			pending.remove(0);
			placed_any = true;
		} else {
			break;
		}
	}
	Ok(())
}

/// Dispatches a node to the placement helper for its shape. The break decision is the caller's; this
/// only lays the node's ink at `y` on `page_no`.
fn place_node<M: Metrics>(
	node:		&Node,
	y:			Sp,
	page_no:	u32,
	geom:		PageGeometry,
	metrics:	&M,
	incoming:	&Ledger,
	frame:		&mut Frame,
	ledger:		&mut Ledger,
)
	-> Outcome<()>
{
	match node {
		// A keep box (a heading bound to the first line of its paragraph) is placed whole, so the greedy
		// breaker moves it entire rather than splitting it.
		Node::VBox(b)	=> place_vbox(b, y, page_no, geom, metrics, incoming, frame, ledger),
		Node::HBox(b)	=> place_line(b, y, page_no, geom, metrics, incoming, frame, ledger),
		Node::Leaf(l)	=> place_leaf(l, geom.content_left(), y, page_no, metrics, incoming, frame, ledger).map(|_| ()),
		_				=> Ok(()),
	}
}

/// Gathers the footnotes whose marks fall anywhere within `node`, in the document order they were set,
/// by walking its boxes. A mark is a [`LeafKind::Mark`] leaf; the note it carries is what the page
/// breaker reserves foot space for and what the closing page sets at its foot.
fn collect_marks(node: &Node, out: &mut Vec<Footnote>) {
	match node {
		Node::HBox(b) | Node::VBox(b)	=> for child in &b.list { collect_marks(child, out); },
		Node::Leaf(l)					=> if let LeafKind::Mark(f) = &l.kind { out.push(f.clone()); },
		_								=> (),
	}
}

/// The height a set of footnotes takes at the foot: the separator furniture, the notes' own stacked
/// heights, and the gaps between them. Zero when there are none, so a page with no footnote keeps the
/// whole body height. `existing` are the page's notes already; `extra` are a candidate line's, weighed
/// in so a line and its own note are judged against the same page together.
fn foot_reserve(existing: &[Footnote], extra: &[Footnote], foot: &FootStyle) -> Sp {
	let n = existing.len() + extra.len();
	if n == 0 {
		return Sp::ZERO;
	}
	let mut h = Sp::ZERO;
	for f in existing.iter().chain(extra.iter()) {
		h += f.height;
	}
	foot.gap_above_rule + foot.rule_thick + foot.gap_below_rule + h + foot.gap_between * (n as i32 - 1)
}

/// Sets a page's accumulated footnotes at its foot: a short separator rule, then each note as the small
/// paragraph it was set into, seated so the whole block's foot meets the bottom of the text block, above
/// the folio. The block's height is exactly [`foot_reserve`]'s, so the body above it -- placed against a
/// bottom shrunk by that same amount -- never collides with it.
#[allow(clippy::too_many_arguments)]
fn lay_footnotes<M: Metrics>(
	frame:		&mut Frame,
	notes:		&[Footnote],
	page_no:	u32,
	geom:		PageGeometry,
	foot:		&FootStyle,
	bottom:		Sp,
	metrics:	&M,
	incoming:	&Ledger,
	ledger:		&mut Ledger,
)
	-> Outcome<()>
{
	if notes.is_empty() {
		return Ok(());
	}
	let total	= foot_reserve(notes, &[], foot);
	let mut yy	= bottom - total;

	yy += foot.gap_above_rule;
	frame.push(Placed::new(
		geom.content_left(), yy, Dims::new(foot.rule_width, foot.rule_thick, Sp::ZERO), PlacedKind::Rule));
	yy = yy + foot.rule_thick + foot.gap_below_rule;

	for (i, f) in notes.iter().enumerate() {
		for child in &f.note {
			match child {
				Node::HBox(b) => {
					res!(place_line(b, yy, page_no, geom, metrics, incoming, frame, ledger));
					yy += b.dims.vextent();
				},
				Node::Glue(g) => {
					yy += g.natural;
				},
				_ => (),
			}
		}
		if i + 1 < notes.len() {
			yy += foot.gap_between;
		}
	}
	Ok(())
}

/// Lays one horizontal box -- a line -- left to right, placing each child and recording any anchor
/// or forward reference it carries. Nested boxes are placed as their own rectangle in Phase 0;
/// shaping their contents is Phase 1.
fn place_line<M: Metrics>(
	line:		&BoxNode,
	y:			Sp,
	page_no:	u32,
	geom:		PageGeometry,
	metrics:	&M,
	incoming:	&Ledger,
	frame:		&mut Frame,
	ledger:		&mut Ledger,
)
	-> Outcome<()>
{
	let mut x = geom.content_left();
	for child in &line.list {
		match child {
			Node::Leaf(l) => {
				x = res!(place_leaf(l, x, y, page_no, metrics, incoming, frame, ledger));
			},
			Node::Glue(g) => {
				x += g.natural;
			},
			Node::Anchor(id) => {
				ledger.record(Anchor::new(id.clone(), Position::new(page_no, x, y)));
			},
			Node::Penalty(_) => {
				// A line arrives here already broken: `linebreak::break_paragraph` runs the Knuth-Plass
				// optimiser upstream and hands the driver finished HBox lines of words and justified
				// glue. A penalty inside such a line would be a later intra-line refinement (a kept
				// discretionary break), which Phase 1 does not yet place, so there is nothing to weigh.
			},
			Node::HBox(b) | Node::VBox(b) => {
				frame.push(Placed::new(x, y, b.dims, PlacedKind::Rule));
				x += b.dims.width;
			},
			// A float is a block-level node the driver handles before it ever reaches a line; one woven into a
			// line would be a construction error, so it draws nothing rather than being flattened here.
			Node::Float(_) => (),
		}
	}
	Ok(())
}

/// Sets a vertical keep box: its children stacked from the box top, each at the content left. Lines
/// are placed, glue advances the cursor, and an anchor is recorded at the y it reaches -- so a
/// heading's anchor takes the page and position the box settled on, never a provisional one from
/// before the box was moved to fit.
fn place_vbox<M: Metrics>(
	vbox:		&BoxNode,
	y_top:		Sp,
	page_no:	u32,
	geom:		PageGeometry,
	metrics:	&M,
	incoming:	&Ledger,
	frame:		&mut Frame,
	ledger:		&mut Ledger,
)
	-> Outcome<()>
{
	let mut yy = y_top;
	for child in &vbox.list {
		match child {
			Node::HBox(b) => {
				res!(place_line(b, yy, page_no, geom, metrics, incoming, frame, ledger));
				yy += b.dims.vextent();
			},
			Node::VBox(b) => {
				res!(place_vbox(b, yy, page_no, geom, metrics, incoming, frame, ledger));
				yy += b.dims.vextent();
			},
			Node::Leaf(l) => {
				res!(place_leaf(l, geom.content_left(), yy, page_no, metrics, incoming, frame, ledger));
				yy += l.dims.vextent();
			},
			Node::Glue(g) => {
				yy += g.natural;
			},
			Node::Anchor(id) => {
				ledger.record(Anchor::new(id.clone(), Position::new(page_no, geom.content_left(), yy)));
			},
			Node::Penalty(_) => (),
			// A float never nests inside a keep box; one that did would be a construction error, so it draws
			// nothing rather than being flattened into the box.
			Node::Float(_) => (),
		}
	}
	Ok(())
}

/// Places one leaf at `(x, y)` and returns the x the next child starts at. A rule is drawn as it
/// stands. A forward reference reserves a slot: the width it needs for the value resolved from the
/// previous pass, never less than the width the author declared. When the resolved value outgrows
/// the declared reservation the slot grows to fit it, which shifts everything after it -- the honest
/// cause of a further pass, recorded on the anchor as an overflow.
fn place_leaf<M: Metrics>(
	leaf:		&Leaf,
	x:			Sp,
	y:			Sp,
	page_no:	u32,
	metrics:	&M,
	incoming:	&Ledger,
	frame:		&mut Frame,
	ledger:		&mut Ledger,
)
	-> Outcome<Sp>
{
	// The leaf's own vertical shift moves its ink off the line's baseline without a nested box -- a
	// maths script raised, a fraction's numerator lifted and its bar seated on the axis. The x advance
	// is unaffected, so the horizontal cursor the caller tracks is untouched.
	let y = y + leaf.shift;
	match &leaf.kind {
		LeafKind::Rule => {
			frame.push(Placed::new(x, y, leaf.dims, PlacedKind::Rule));
			Ok(x + leaf.dims.width)
		},
		LeafKind::Text(shaped) => {
			// Already shaped and measured; place it and advance by its width. The writer reads the run
			// back out of the frame to draw the glyphs.
			frame.push(Placed::new(x, y, leaf.dims, PlacedKind::Text(shaped.clone())));
			Ok(x + leaf.dims.width)
		},
		LeafKind::Mark(footnote) => {
			// The superscript number is drawn like any run; its raised dims put the baseline above the
			// line's. The note it carries is set at the page foot by the breaker, not here.
			frame.push(Placed::new(x, y, leaf.dims, PlacedKind::Text(footnote.mark.clone())));
			Ok(x + leaf.dims.width)
		},
		LeafKind::Graphic(g) => {
			// A figure placed whole: its ops are translated to this position and drawn by the emitter.
			frame.push(Placed::new(x, y, leaf.dims, PlacedKind::Graphic(g.clone())));
			Ok(x + leaf.dims.width)
		},
		LeafKind::Reserved(id, refr, hold) => {
			// A forward reference. What it resolves to is the reference's own business (a total count, a
			// cross-referenced page); the driver only asks the previous pass's ledger for the value and
			// holds the declared width open until it has one.
			let reserved = leaf.dims.width;
			let (realised, resolved) = match refr.resolve(incoming) {
				Some(value) => {
					// The previous pass fixed the value. Shape it as real text when a font backs the
					// metric, or keep the reservation box under the fontless stub; either way its realised
					// width is recorded so the overflow logic still governs a further pass.
					let text = fmt!("{}", value);
					match res!(metrics.shape(&text)) {
						Some(shaped) => {
							let w		= shaped.dims().width;
							let dims	= Dims::new(w, leaf.dims.height, leaf.dims.depth);
							frame.push(Placed::new(x, y, dims, PlacedKind::Text(shaped)));
							(w, true)
						},
						None => {
							frame.push(Placed::new(x, y, leaf.dims, PlacedKind::Reserved));
							(res!(metrics.measure(&text)).width, true)
						},
					}
				},
				None => {
					// Pass A: no value yet. Hold the reservation open and realise nothing, so no overflow
					// is charged before there is a value that could exceed the width.
					frame.push(Placed::new(x, y, leaf.dims, PlacedKind::Reserved));
					(Sp::ZERO, false)
				},
			};

			// A value wider than its reservation always grows the slot -- the honest cause of a further
			// pass, charged as the anchor's overflow. Within the reservation, furniture (`hold`) keeps the
			// declared width so a right-aligned column stays put, while an inline reference shrinks to the
			// resolved value so it reads without a gap; a still-unresolved slot keeps its reservation.
			let slot = if realised > reserved {
				realised
			} else if *hold || !resolved {
				reserved
			} else {
				realised
			};
			let mut anchor = Anchor::new(id.clone(), Position::new(page_no, x, y));
			anchor.reserved = reserved;
			anchor.realised = realised;
			ledger.record(anchor);
			Ok(x + slot)
		},
	}
}

/// Builds the non-convergence error: the ledger difference the architecture promises, naming the
/// anchor that moved and the pages it moved between, plus any reference that overflowed its
/// reservation.
fn non_convergence(
	pass:	u32,
	ledger:	&Ledger,
	prev:	&Ledger,
)
	-> Error<ErrTag>
{
	let deltas		= ledger.diff(prev);
	let overflows	= ledger.overflowed();

	let mut moved = String::new();
	for d in &deltas {
		moved.push_str(&fmt!(" [{:?} {} moved p{}->p{}]", d.id.kind, d.id.key, d.from, d.to));
	}
	let mut over = String::new();
	for id in &overflows {
		over.push_str(&fmt!(" [{:?} {} overflowed its reservation]", id.kind, id.key));
	}
	err!(
		"Composition did not converge after {} passes; the ledger is still moving. Moved anchors:{}. \
		Reservations exceeded:{}.", pass, moved, over;
		Data, Excessive, LimitReached)
}
