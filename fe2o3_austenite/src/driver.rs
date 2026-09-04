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

/// A trivial in-memory document: a vertical box-glue-penalty stream, and the geometry every page
/// takes. Phase 0 has one geometry for the whole document; per-chapter geometry is later.
#[derive(Clone, Debug)]
pub struct Document {
	pub nodes:	Vec<Node>,
	pub geom:	PageGeometry,
}

impl Document {
	pub fn new(nodes: Vec<Node>, geom: PageGeometry) -> Self {
		Self { nodes, geom }
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
					finish_page(&mut pages, &mut frame, &mut page_no, &mut y, top, geom);
					at_top = true;
				}
			},
			Node::Anchor(id) => {
				ledger.record(Anchor::new(id.clone(), Position::new(page_no, geom.content_left(), y)));
			},
			Node::HBox(b) => {
				let v = b.dims.vextent();
				if !frame.is_empty() && y + v > bottom {
					finish_page(&mut pages, &mut frame, &mut page_no, &mut y, top, geom);
				}
				res!(place_line(b, y, page_no, geom, metrics, incoming, &mut frame, &mut ledger));
				y += v;
				at_top = false;
			},
			Node::VBox(b) => {
				let v = b.dims.vextent();
				if !frame.is_empty() && y + v > bottom {
					finish_page(&mut pages, &mut frame, &mut page_no, &mut y, top, geom);
				}
				frame.push(Placed::new(geom.content_left(), y, b.dims, PlacedKind::Rule));
				y += v;
				at_top = false;
			},
			Node::Leaf(l) => {
				let v = l.dims.vextent();
				if !frame.is_empty() && y + v > bottom {
					finish_page(&mut pages, &mut frame, &mut page_no, &mut y, top, geom);
				}
				res!(place_leaf(l, geom.content_left(), y, page_no, metrics, incoming, &mut frame, &mut ledger));
				y += v;
				at_top = false;
			},
		}
	}

	// The last page holds whatever is left, unless nothing is.
	if !frame.is_empty() {
		pages.push(Page::new(page_no, geom, std::mem::take(&mut frame)));
	} else if pages.is_empty() {
		// A document with no material is still one blank page, so a page count is always at least one.
		pages.push(Page::new(page_no, geom, Frame::new()));
	}

	ledger.total_pages = pages.len() as u32;
	Ok((pages, ledger))
}

/// Closes the current page: stores it, resets the frame and cursor, and advances the folio.
fn finish_page(
	pages:		&mut Vec<Page>,
	frame:		&mut Frame,
	page_no:	&mut u32,
	y:			&mut Sp,
	top:		Sp,
	geom:		PageGeometry,
) {
	pages.push(Page::new(*page_no, geom, std::mem::take(frame)));
	*page_no += 1;
	*y = top;
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
		LeafKind::Reserved(id, refr) => {
			// A forward reference. What it resolves to is the reference's own business (a total count, a
			// cross-referenced page); the driver only asks the previous pass's ledger for the value and
			// holds the declared width open until it has one.
			let reserved = leaf.dims.width;
			let realised = match refr.resolve(incoming) {
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
							w
						},
						None => {
							frame.push(Placed::new(x, y, leaf.dims, PlacedKind::Reserved));
							res!(metrics.measure(&text)).width
						},
					}
				},
				None => {
					// Pass A: no value yet. Hold the reservation open and realise nothing, so no overflow
					// is charged before there is a value that could exceed the width.
					frame.push(Placed::new(x, y, leaf.dims, PlacedKind::Reserved));
					Sp::ZERO
				},
			};

			// The slot never shrinks below the reservation, so following material stays put while the
			// value fits; a value wider than its reservation grows the slot and shifts what follows,
			// which is the honest cause of a further pass, recorded as the anchor's overflow.
			let slot = if realised > reserved { realised } else { reserved };
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
