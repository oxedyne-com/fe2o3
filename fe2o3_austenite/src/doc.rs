//! The authoring layer: blocks of prose above the box-glue-penalty stream.
//!
//! [`driver::Document`](crate::driver::Document) is the composed form -- a flat vertical stream the
//! two-pass driver paginates. This module sits above it. An author writes a [`Block`] list --
//! headings and paragraphs -- and [`author`] turns each block into the stream: a heading is shaped
//! bold and larger, its identity recorded as a [`Heading`](crate::ledger::AnchorKind::Heading)
//! anchor so a running head or a table of contents can later find its page; a paragraph is set into
//! justified lines by [`break_paragraph`](crate::linebreak::break_paragraph).
//!
//! Two facts a reader could not derive. A heading is kept with the first line of its paragraph by
//! setting the two inside one unbreakable box, so the driver's greedy page breaker never leaves a
//! heading stranded at a page foot (the widow guard). And the page furniture -- the running head and
//! the folio -- is added by [`decorate`] after the document has converged, because it lives in the
//! margins, outside the text block, and so cannot disturb the pagination it describes. The running
//! head is TeX's `\mark` reimplemented through the ledger: the section current at the top of a page
//! is the most recent heading the ledger resolved to an earlier page.

use crate::driver::Document;
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	Dims,
	Glue,
	Leaf,
	Node,
	Penalty,
	Sp,
};
use crate::ledger::{
	AnchorId,
	AnchorKind,
	Ledger,
	Ref,
};
use crate::linebreak::break_paragraph;
use crate::table::{
	self,
	Table,
};
use crate::page::{
	Page,
	PageGeometry,
	Placed,
	PlacedKind,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::Dir,
};

use std::sync::Arc;

/// One block of the authored document. The closed vocabulary the block layer sets; richer blocks
/// (lists, quotes, figures) are later variants here.
#[derive(Clone, Debug)]
pub enum Block {
	Heading { level: u8, text: String },
	Paragraph { text: String },
	Table(Table),
}

impl Block {
	pub fn heading<S: Into<String>>(level: u8, text: S) -> Self {
		Self::Heading { level, text: text.into() }
	}

	pub fn paragraph<S: Into<String>>(text: S) -> Self {
		Self::Paragraph { text: text.into() }
	}

	pub fn table(table: Table) -> Self {
		Self::Table(table)
	}
}

/// The point sizes and vertical spaces the block layer sets to. Every length is scaled points, so
/// the styling never leaves the integer domain the driver breaks on.
#[derive(Clone, Copy, Debug)]
pub struct Style {
	pub body_size:		Sp,
	pub leading:		Sp,
	pub para_skip:		Sp,	// extra space between one paragraph and the next
	pub h1_size:		Sp,
	pub h2_size:		Sp,
	pub h3_size:		Sp,
	pub header_size:	Sp,	// the running head's size
	pub folio_size:		Sp,
	pub table_skip:		Sp,	// space set above and below a table
	pub cell_pad_x:		Sp,	// horizontal padding between a cell's text and its column rules
	pub cell_pad_y:		Sp,	// vertical padding above and below a cell's lines
	pub line_gap:		Sp,	// leading between the wrapped lines within one cell
	pub rule_thin:		Sp,	// an interior grid rule
	pub rule_thick:		Sp,	// the frame and the rule beneath a header
}

impl Default for Style {
	fn default() -> Self {
		Self {
			body_size:		Sp::from_pt(11.0),
			leading:		Sp::from_pt(13.2),	// 1.2x the body
			para_skip:		Sp::from_pt(6.0),
			h1_size:		Sp::from_pt(16.0),
			h2_size:		Sp::from_pt(13.0),
			h3_size:		Sp::from_pt(12.0),
			header_size:	Sp::from_pt(9.5),
			folio_size:		Sp::from_pt(10.0),
			table_skip:		Sp::from_pt(10.0),
			cell_pad_x:		Sp::from_pt(5.0),
			cell_pad_y:		Sp::from_pt(3.0),
			line_gap:		Sp::from_pt(3.0),
			rule_thin:		Sp::from_pt(0.4),
			rule_thick:		Sp::from_pt(0.8),
		}
	}
}

impl Style {
	fn heading_size(&self, level: u8) -> Sp {
		match level {
			1 => self.h1_size,
			2 => self.h2_size,
			_ => self.h3_size,
		}
	}

	/// The space set above a heading of this level, always greater than the space below it, so a
	/// heading binds visually to the text it introduces rather than to the text it follows.
	fn space_above(&self, level: u8) -> Sp {
		match level {
			1 => Sp::from_pt(20.0),
			2 => Sp::from_pt(15.0),
			_ => Sp::from_pt(12.0),
		}
	}

	fn space_below(&self, level: u8) -> Sp {
		match level {
			1 => Sp::from_pt(8.0),
			2 => Sp::from_pt(6.0),
			_ => Sp::from_pt(5.0),
		}
	}
}

/// A recorded heading: the anchor identity the ledger resolves to a page, its level, and its display
/// title. The block layer keeps this table beside the composed stream so [`decorate`] can read a
/// title back from an anchor -- the ledger stores only the identity, not the words.
#[derive(Clone, Debug)]
pub struct Heading {
	pub id:		AnchorId,
	pub level:	u8,
	pub title:	String,
}

/// Turns an authored block list into the composed document, and the heading table the running heads
/// resolve against. The geometry fixes the measure every paragraph is set to.
pub fn author(
	fonts:	Arc<FontSet>,
	geom:	PageGeometry,
	style:	Style,
	blocks:	&[Block],
)
	-> Outcome<(Document, Vec<Heading>)>
{
	let measure	= geom.content_width();
	let mut nodes:	Vec<Node>		= Vec::new();
	let mut heads:	Vec<Heading>	= Vec::new();

	let mut i		= 0usize;
	let mut first	= true;
	while i < blocks.len() {
		match &blocks[i] {
			Block::Heading { level, text } => {
				// Space above the heading. At a page top the driver discards it, so the first heading on a
				// page still sits flush to the text block.
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.space_above(*level))));
				}

				let id = AnchorId::new(AnchorKind::Heading, fmt!("{:02}-{}", heads.len() + 1, slug(text)));
				heads.push(Heading { id: id.clone(), level: *level, title: text.clone() });

				let shaped	= res!(ShapedText::new(
					fonts.clone(), Role::Bold, Dir::Ltr, style.heading_size(*level), text));
				let hd		= shaped.dims();
				let hbox	= Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::text(shaped))], hd));

				// The heading, its space below, and the first line of the paragraph it introduces are set
				// as one box: an atomic unit the greedy page breaker moves whole, so a heading is never the
				// last thing on a page. When no paragraph follows, the box is the heading alone.
				let mut keep	= vec![Node::Anchor(id), hbox, Node::Glue(Glue::fixed(style.space_below(*level)))];
				let mut rest:	Vec<Node> = Vec::new();
				if let Some(Block::Paragraph { text: para }) = blocks.get(i + 1) {
					let mut lines = res!(break_paragraph(
						fonts.clone(), Role::Body, Dir::Ltr, style.body_size, para, measure, style.leading));
					if !lines.is_empty() {
						keep.push(lines.remove(0));	// the first line joins the heading
						rest = lines;				// its leading glue and the remaining lines follow
					}
					i += 2;
				} else {
					i += 1;
				}

				nodes.push(vbox(keep, measure));
				nodes.extend(rest);
				first = false;
			},
			Block::Paragraph { text } => {
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.para_skip)));
				}
				let lines = res!(break_paragraph(
					fonts.clone(), Role::Body, Dir::Ltr, style.body_size, text, measure, style.leading));
				nodes.extend(lines);
				i += 1;
				first = false;
			},
			Block::Table(t) => {
				// Space above the table, discarded at a page top like any other leading. The table lowers
				// to one keep box, so the driver moves it whole to the next page when it will not fit.
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.table_skip)));
				}
				nodes.push(res!(table::lower(fonts.clone(), style, measure, t)));
				nodes.push(Node::Glue(Glue::fixed(style.table_skip)));
				i += 1;
				first = false;
			},
		}
	}

	Ok((Document::new(nodes, geom), heads))
}

/// Sets a table of contents from the heading table: a bold "Contents" title, then one entry per
/// heading -- the title on the left, indented by its level, and its page on the right. The page is a
/// forward reference resolved with [`Ref::PageOf`] against the incoming ledger, so it reuses the same
/// reserve-then-resolve slot the driver already runs for any forward reference. The caller prepends
/// these nodes to the document; a trailing forced break starts the body on a fresh page.
///
/// A fact a reader could not derive. Each entry reserves a fixed slot for its page number -- three
/// digits wide, so a resolved folio never outgrows it -- and its line height is the title's, whatever
/// the number turns out to be. The contents block therefore has a constant vertical extent from the
/// first pass, so the body it displaces settles once and the forward references converge in the usual
/// two passes, with no special case in the driver. The number is set left-aligned within its slot;
/// true flush-right within the slot, and a dotted rather than a blank leader, are later refinements,
/// as is a title too wide to sit on one line with its page.
pub fn contents(
	fonts:	Arc<FontSet>,
	geom:	PageGeometry,
	style:	Style,
	heads:	&[Heading],
)
	-> Outcome<Vec<Node>>
{
	let measure			= geom.content_width();
	let mut nodes:	Vec<Node> = Vec::new();

	// The block's own heading, set bold like a section but recorded as no anchor -- so it is neither a
	// running-head section nor an entry in its own list.
	let title	= res!(ShapedText::new(fonts.clone(), Role::Bold, Dir::Ltr, style.h2_size, "Contents"));
	let td		= title.dims();
	nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::text(title))], td)));
	nodes.push(Node::Glue(Glue::fixed(style.space_below(2))));

	// A fixed slot wide enough for a three-digit folio, so a resolved number never overflows its
	// reservation and every entry keeps a constant height across passes.
	let slot	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size, "000"));
	let slot_w	= slot.dims().width;

	for (i, h) in heads.iter().enumerate() {
		let indent	= style.body_size * (h.level.saturating_sub(1) as i32);
		let entry	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size, &h.title));
		let ed		= entry.dims();

		// The leader is the blank span from the title to the slot at the right edge; the slot's right
		// edge falls on the measure. A title too wide to leave a one-em minimum keeps that minimum and
		// runs under its page -- the over-wide case, left to a later refinement.
		let min_lead	= style.body_size;
		let taken		= indent + ed.width + slot_w;
		let leader_w	= if measure > taken + min_lead { measure - taken } else { min_lead };

		// The entry's own identity, distinct from the heading it points at, so recording the slot never
		// overwrites the heading's ledger row. Its reference resolves the heading's page.
		let toc_id		= AnchorId::new(AnchorKind::Label, fmt!("toc-{}", h.id.key));
		let slot_dims	= Dims::new(slot_w, ed.height, ed.depth);

		let mut children:	Vec<Node> = Vec::new();
		if indent.raw() > 0 {
			children.push(Node::Glue(Glue::fixed(indent)));
		}
		children.push(Node::Leaf(Leaf::text(entry)));
		children.push(Node::Glue(Glue::fixed(leader_w)));
		children.push(Node::Leaf(Leaf::reserved(toc_id, Ref::PageOf(h.id.clone()), slot_dims)));

		let line_dims = Dims::new(measure, ed.height, ed.depth);
		nodes.push(Node::HBox(BoxNode::new(children, line_dims)));

		// Leading between entries, but not after the last.
		if i + 1 < heads.len() {
			let vextent	= ed.height + ed.depth;
			let gap		= if style.leading > vextent { style.leading - vextent } else { Sp::ZERO };
			nodes.push(Node::Glue(Glue::fixed(gap)));
		}
	}

	// The contents stands alone at the front; the body opens on a fresh page.
	nodes.push(Node::Penalty(Penalty::eject()));
	Ok(nodes)
}

/// Wraps a vertical run of nodes as a keep box, its extent the sum of its children's, so the driver
/// places it whole or moves it whole. The whole extent is carried as height; a block has no baseline
/// the page cares about, so the depth is zero.
fn vbox(list: Vec<Node>, width: Sp) -> Node {
	let mut ext = Sp::ZERO;
	for n in &list {
		ext += n.vextent();
	}
	Node::VBox(BoxNode::new(list, Dims::new(width, ext, Sp::ZERO)))
}

/// A filesystem-safe key from a heading's words: lowercase, runs of non-alphanumerics collapsed to a
/// single dash. Prefixed with an ordinal by the caller, so two headings of the same words stay
/// distinct identities.
fn slug(text: &str) -> String {
	let mut out		= String::new();
	let mut dash	= false;
	for c in text.chars() {
		if c.is_ascii_alphanumeric() {
			out.push(c.to_ascii_lowercase());
			dash = false;
		} else if !dash && !out.is_empty() {
			out.push('-');
			dash = true;
		}
	}
	while out.ends_with('-') {
		out.pop();
	}
	if out.is_empty() { "heading".to_string() } else { out }
}

/// Draws the page furniture -- a running head in the top margin and a folio in the bottom -- onto
/// every composed page. Called after the driver has converged: the furniture sits outside the text
/// block, so adding it moves nothing and cannot reopen the fixed point.
///
/// The running head is the section current at the top of each page: the most recent heading the
/// ledger resolved to an earlier page. A page onto which no earlier section runs -- the first page,
/// and any page a section opens at its very top -- omits the running head, which is the usual
/// suppression on a title or chapter-opening page. Both the head and the folio are shaped through the
/// same path as the body and drawn as glyph outlines.
pub fn decorate(
	pages:	&mut [Page],
	ledger:	&Ledger,
	heads:	&[Heading],
	fonts:	&Arc<FontSet>,
	style:	Style,
	geom:	PageGeometry,
)
	-> Outcome<()>
{
	let content_top = geom.content_top();
	for page in pages.iter_mut() {
		// The section running at the top of this page, and whether a section opens the page.
		let mut running:	Option<&str>	= None;
		let mut suppress					= false;
		for h in heads {
			if let Some(a) = ledger.get(&h.id) {
				if a.pos.page < page.number {
					running = Some(&h.title);
				} else if a.pos.page == page.number {
					if a.pos.y == content_top {
						suppress = true;	// a section opens at the very top: this is its opening page
					}
					break;
				} else {
					break;	// headings are in document order, so the rest resolve to later pages
				}
			}
		}
		if suppress {
			running = None;
		}

		if let Some(title) = running {
			let shaped	= res!(ShapedText::new(fonts.clone(), Role::Italic, Dir::Ltr, style.header_size, title));
			let d		= shaped.dims();
			let x		= centre_x(geom, d.width);
			// Seat the head above the text block, its baseline within the top margin.
			let y		= content_top - d.vextent() - Sp::from_pt(6.0);
			page.frame.push(Placed::new(x, y, d, PlacedKind::Text(shaped)));
		}

		let num		= fmt!("{}", page.number);
		let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.folio_size, &num));
		let d		= shaped.dims();
		let x		= centre_x(geom, d.width);
		let y		= content_top + geom.content_height() + Sp::from_pt(14.0);
		page.frame.push(Placed::new(x, y, d, PlacedKind::Text(shaped)));
	}
	Ok(())
}

/// The x that centres a box of width `w` in the text block. A box wider than the measure starts at
/// the left edge rather than hanging off it.
fn centre_x(geom: PageGeometry, w: Sp) -> Sp {
	let slack = (geom.content_width().raw() - w.raw()).max(0) / 2;
	geom.content_left() + Sp(slack)
}
