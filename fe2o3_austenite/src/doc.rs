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
	Sp,
};
use crate::ledger::{
	AnchorId,
	AnchorKind,
	Ledger,
};
use crate::linebreak::break_paragraph;
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
}

impl Block {
	pub fn heading<S: Into<String>>(level: u8, text: S) -> Self {
		Self::Heading { level, text: text.into() }
	}

	pub fn paragraph<S: Into<String>>(text: S) -> Self {
		Self::Paragraph { text: text.into() }
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
		}
	}

	Ok((Document::new(nodes, geom), heads))
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
