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

use crate::driver::{
	Document,
	FootStyle,
};
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	Dims,
	DrawOp,
	Footnote,
	Glue,
	Graphic,
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
use crate::linebreak::{
	break_paragraph,
	break_paragraph_pieces,
	Piece,
};
use crate::math::{
	self,
	Atom,
};
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
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		PathBuilder,
		Pt,
	},
};

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

/// One run of a rich paragraph: a stretch of body text, a strongly emphasised run (`*strong*`, set
/// bold), an emphasised run (`/emph/`, set italic), or a footnote whose mark falls after the run before
/// it. The note text is set at the foot of the page the mark lands on, and numbered in document order.
#[derive(Clone, Debug)]
pub enum Segment {
	Text(String),
	Strong(String),	// set in the bold face
	Emph(String),	// set in the italic face
	Footnote { note: String },
	Math(Atom),	// an inline maths expression, set within the running line
	PageRef(String),	// a cross-reference to a labelled anchor, resolving to its page number
	Code(String),	// an inline code span, set in the mono face
	Glossary { term: String, display: String },	// a glossary term: bold-italic on its first document use, plain after
}

impl Segment {
	pub fn text<S: Into<String>>(text: S) -> Self {
		Self::Text(text.into())
	}

	pub fn strong<S: Into<String>>(text: S) -> Self {
		Self::Strong(text.into())
	}

	pub fn emph<S: Into<String>>(text: S) -> Self {
		Self::Emph(text.into())
	}

	pub fn footnote<S: Into<String>>(note: S) -> Self {
		Self::Footnote { note: note.into() }
	}

	pub fn math(expr: Atom) -> Self {
		Self::Math(expr)
	}

	pub fn page_ref<S: Into<String>>(label: S) -> Self {
		Self::PageRef(label.into())
	}

	pub fn code<S: Into<String>>(text: S) -> Self {
		Self::Code(text.into())
	}

	pub fn glossary<T: Into<String>, D: Into<String>>(term: T, display: D) -> Self {
		Self::Glossary { term: term.into(), display: display.into() }
	}
}

/// One block of the authored document. The closed vocabulary the block layer sets; richer blocks
/// (lists, quotes, figures) are later variants here.
#[derive(Clone, Debug)]
pub enum Block {
	Heading { level: u8, text: String, label: Option<String> },	// label: an author anchor a `#ref` resolves to
	Paragraph { text: String },
	RichParagraph { segments: Vec<Segment> },	// a paragraph carrying footnote marks
	List { ordered: bool, items: Vec<Vec<Segment>> },	// a bullet or numbered list, each item a run sequence
	Code { lines: Vec<String> },	// a verbatim code block, set in the mono face, whitespace preserved
	Table(Table),
	Equation { expr: Atom, numbered: bool },	// a display equation on its own centred line
	Figure { graphic: Graphic, caption: Option<String> },	// a drawn figure, centred, numbered, captioned
	// A `#figure(...)` wrapping a `#table(...)`: the ruled table, then a numbered caption beneath. The
	// supplement is the caption's leading word ("Table"/"Figure"); the label anchors a cross-reference.
	TableFigure { table: Table, caption: Option<String>, supplement: String, label: Option<String> },
	// A `#figure(...)` wrapping an image: a sized placeholder box stands in for the image this increment
	// does not load, with the numbered caption beneath. The path is kept for a later image loader.
	ImageFigure { path: String, caption: Option<String>, supplement: String, label: Option<String> },
}

impl Block {
	pub fn heading<S: Into<String>>(level: u8, text: S) -> Self {
		Self::Heading { level, text: text.into(), label: None }
	}

	/// A heading carrying an author label, so a `#ref(<label>)` elsewhere resolves to its page.
	pub fn heading_labelled<S: Into<String>>(level: u8, text: S, label: Option<String>) -> Self {
		Self::Heading { level, text: text.into(), label }
	}

	pub fn paragraph<S: Into<String>>(text: S) -> Self {
		Self::Paragraph { text: text.into() }
	}

	pub fn rich(segments: Vec<Segment>) -> Self {
		Self::RichParagraph { segments }
	}

	/// A bullet (`ordered` false) or numbered (`ordered` true) list. Each item is a run sequence, so an
	/// item may carry emphasis, a footnote or inline maths exactly as a rich paragraph does.
	pub fn list(ordered: bool, items: Vec<Vec<Segment>>) -> Self {
		Self::List { ordered, items }
	}

	/// A verbatim code block: each line set in the mono face with its whitespace preserved and no
	/// justification, the way source is shown.
	pub fn code(lines: Vec<String>) -> Self {
		Self::Code { lines }
	}

	pub fn table(table: Table) -> Self {
		Self::Table(table)
	}

	/// A display equation set centred on its own line. A numbered one takes the next equation number at
	/// the right margin and records an [`Equation`](crate::ledger::AnchorKind::Equation) anchor.
	pub fn equation(expr: Atom, numbered: bool) -> Self {
		Self::Equation { expr, numbered }
	}

	/// A drawn figure, centred on its own line and captioned "Figure N" beneath, its identity recorded
	/// as a [`Float`](crate::ledger::AnchorKind::Float) anchor so a cross-reference resolves its page.
	pub fn figure(graphic: Graphic, caption: Option<String>) -> Self {
		Self::Figure { graphic, caption }
	}

	/// A table wrapped in a figure: the ruled grid, then a "{supplement} N: {caption}" line beneath,
	/// numbered per supplement so tables and figures carry independent counts.
	pub fn table_figure(
		table:		Table,
		caption:	Option<String>,
		supplement:	String,
		label:		Option<String>,
	)
		-> Self
	{
		Self::TableFigure { table, caption, supplement, label }
	}

	/// An image wrapped in a figure, stood in for by a placeholder box this increment. The path is kept
	/// so a later increment can load the image in its place.
	pub fn image_figure(
		path:		String,
		caption:	Option<String>,
		supplement:	String,
		label:		Option<String>,
	)
		-> Self
	{
		Self::ImageFigure { path, caption, supplement, label }
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
	pub foot_size:		Sp,	// the footnote text's size, a touch below the body
	pub foot_leading:	Sp,	// leading between the wrapped lines of one footnote
	pub list_marker_gap:	Sp,	// space between a list marker and the item text it introduces
	pub list_item_skip:		Sp,	// vertical space set between one list item and the next
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
			foot_size:		Sp::from_pt(9.0),
			foot_leading:	Sp::from_pt(10.8),	// 1.2x the footnote size
				list_marker_gap:	Sp::from_pt(6.0),
				list_item_skip:		Sp::from_pt(3.0),
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
	let mut foot_no	= 0u32;	// the footnote number, a document-order fold over the marks
	let mut ref_no	= 0u32;	// a running counter giving each inline cross-reference its own anchor id
	let mut eq_no	= 0u32;	// the equation number, a document-order fold over the numbered displays
	let mut fig_no	= 0u32;	// the figure number, a document-order fold over the drawn figures
	// The number per figure supplement ("Figure", "Table"): a document-order fold, so tables and figures
	// carry independent counts, matching Typst's per-kind numbering.
	let mut counters:	HashMap<String, u32>	= HashMap::new();
	// Glossary terms already set once, in document order. The first mention of a term is set bold-italic
	// and every later mention plain; author walks the blocks in order, so the set decides first-use with
	// no second pass. Keyed by the term as written, matching the template's case-sensitive tracking.
	let mut seen:	HashSet<String>	= HashSet::new();
	while i < blocks.len() {
		match &blocks[i] {
			Block::Heading { level, text, label } => {
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
				let mut keep:	Vec<Node> = vec![Node::Anchor(id)];
				// An author label rides the same position as the heading, so a `#ref(<label>)` resolves to
				// the heading's page. It is a second anchor at one point, not a second heading.
				if let Some(l) = label {
					keep.push(Node::Anchor(AnchorId::new(AnchorKind::Label, l.clone())));
				}
				keep.push(hbox);
				keep.push(Node::Glue(Glue::fixed(style.space_below(*level))));
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
			Block::RichParagraph { segments } => {
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.para_skip)));
				}
				let pieces = res!(build_pieces(fonts.clone(), geom, style, segments, &mut foot_no, &mut ref_no, &mut seen));
				let lines = res!(break_paragraph_pieces(
					fonts.clone(), Role::Body, Dir::Ltr, style.body_size, &pieces, measure, style.leading));
				nodes.extend(lines);
				i += 1;
				first = false;
			},
			Block::List { ordered, items } => {
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.para_skip)));
				}
				res!(list(&mut nodes, fonts.clone(), geom, style, measure, *ordered, items, &mut foot_no, &mut ref_no, &mut seen));
				i += 1;
				first = false;
			},
			Block::Code { lines: src } => {
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.para_skip)));
				}
				res!(code_block(&mut nodes, fonts.clone(), style, src));
				nodes.push(Node::Glue(Glue::fixed(style.para_skip)));
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
			Block::Equation { expr, numbered } => {
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.para_skip)));
				}
				let number = if *numbered { eq_no += 1; Some(eq_no) } else { None };
				res!(equation(&mut nodes, fonts.clone(), style, measure, expr, number));
				nodes.push(Node::Glue(Glue::fixed(style.para_skip)));
				i += 1;
				first = false;
			},
			Block::Figure { graphic, caption } => {
				// Space above the figure, discarded at a page top like any other leading. The figure is
				// one keep box, so the breaker moves it whole to the next page when it will not fit.
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.table_skip)));
				}
				fig_no += 1;
				res!(figure(&mut nodes, fonts.clone(), style, measure, graphic.clone(), caption.as_deref(), fig_no));
				nodes.push(Node::Glue(Glue::fixed(style.table_skip)));
				i += 1;
				first = false;
			},
			Block::TableFigure { table, caption, supplement, label } => {
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.table_skip)));
				}
				let number = next_number(&mut counters, supplement);
				res!(table_figure(
					&mut nodes, fonts.clone(), style, measure, table,
					caption.as_deref(), supplement, number, label.as_deref()));
				nodes.push(Node::Glue(Glue::fixed(style.table_skip)));
				i += 1;
				first = false;
			},
			Block::ImageFigure { path, caption, supplement, label } => {
				if !first {
					nodes.push(Node::Glue(Glue::fixed(style.table_skip)));
				}
				let number = next_number(&mut counters, supplement);
				res!(image_figure(
					&mut nodes, fonts.clone(), style, measure, path,
					caption.as_deref(), supplement, number, label.as_deref()));
				nodes.push(Node::Glue(Glue::fixed(style.table_skip)));
				i += 1;
				first = false;
			},
		}
	}

	let mut document = Document::new(nodes, geom);
	document.foot = foot_style(style);
	Ok((document, heads))
}

/// The foot spacing derived from the block style, so the separator rule and the gaps around the notes
/// match the document's other furniture. The rule runs a third of the measure, a conventional short
/// footnote rule.
fn foot_style(style: Style) -> FootStyle {
	FootStyle {
		gap_above_rule:	style.para_skip,
		rule_thick:		style.rule_thin,
		rule_width:		Sp(style.body_size.raw() * 12),
		gap_below_rule:	Sp::from_pt(4.0),
		gap_between:	Sp::from_pt(3.0),
	}
}

/// Turns a rich paragraph's segments into the pieces the line breaker weaves, assigning each footnote
/// its number from the running fold and setting its note as a small paragraph at the foot measure, and
/// each cross-reference a reserved inline slot the driver resolves in pass B. A text segment is a piece
/// as it stands; a footnote becomes a superscript mark piece carrying the set note; a page reference or
/// a total-pages call becomes a shrink-to-fit reserved leaf, unique by the running `ref_no`.
fn build_pieces(
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style:		Style,
	segments:	&[Segment],
	foot_no:	&mut u32,
	ref_no:		&mut u32,
	seen:		&mut HashSet<String>,
)
	-> Outcome<Vec<Piece>>
{
	let measure			= geom.content_width();
	let mut pieces		= Vec::with_capacity(segments.len());
	for seg in segments {
		match seg {
			Segment::Text(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: Role::Body });
			},
			Segment::Strong(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: Role::Bold });
			},
			Segment::Emph(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: Role::Italic });
			},
			Segment::Footnote { note } => {
				*foot_no += 1;
				let label			= fmt!("{}", *foot_no);
				let (mark, dims)	= res!(superscript(fonts.clone(), Role::Body, style.body_size, &label));
				let footnote		= res!(build_footnote(fonts.clone(), style, measure, *foot_no, note, mark));
				pieces.push(Piece::Mark(Leaf::mark(footnote, dims)));
			},
			Segment::Math(expr) => {
				// The inline box is flattened to leaves and glue by the maths layout; unwrap the HBox it
				// returns and weave its children into the line, so they draw as real glyphs rather than as
				// a nested rectangle. The box seats its baseline on the text baseline -- a body ascent
				// below the line top -- so the line asks for that ascent as its height; anything the maths
				// reaches above it is the overshoot the line above must open for.
				let node = res!(math::layout(fonts.clone(), &style, expr, false));
				if let Node::HBox(b) = node {
					let ascent	= res!(ShapedText::new(
						fonts.clone(), Role::Body, Dir::Ltr, style.body_size, "0")).dims().height;
					let over	= if b.dims.height > ascent { b.dims.height - ascent } else { Sp::ZERO };
					pieces.push(Piece::Math {
						nodes:	b.list,
						width:	b.dims.width,
						height:	ascent,
						depth:	b.dims.depth,
						over,
					});
				}
			},
			Segment::PageRef(label) => {
				pieces.push(Piece::Mark(res!(ref_slot(
					fonts.clone(), style, ref_no,
					Ref::PageOf(AnchorId::new(AnchorKind::Label, label.clone()))))));
			},
			Segment::Code(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: Role::Mono });
			},
			Segment::Glossary { term, display } => {
				// The first mention of a term is set bold-italic, matching the template's `*_term_*`;
				// every later mention is plain body text. Document order is the traversal order, so the
				// set alone decides, with no second pass.
				let role = if seen.insert(term.clone()) { Role::BoldItalic } else { Role::Body };
				pieces.push(Piece::Text { text: display.clone(), role });
			},
		}
	}
	Ok(pieces)
}

/// Builds one inline cross-reference: a reserved leaf, unique by the running `ref_no`, that reserves a
/// three-digit slot and shrinks to the value the driver resolves for `refr` in pass B. It seats on the
/// body baseline, taking a body digit's height and depth so it aligns with the prose around it.
fn ref_slot(
	fonts:	Arc<FontSet>,
	style:	Style,
	ref_no:	&mut u32,
	refr:	Ref,
)
	-> Outcome<Leaf>
{
	*ref_no += 1;
	let own		= AnchorId::new(AnchorKind::Label, fmt!("ref-{}", *ref_no));
	let slot	= res!(ShapedText::new(fonts, Role::Body, Dir::Ltr, style.body_size, "000"));
	let sd		= slot.dims();
	Ok(Leaf::reserved_inline(own, refr, Dims::new(sd.width, sd.height, sd.depth)))
}

/// Sets a bullet or numbered list into the vertical list. Each item is broken at a measure reduced by
/// the marker column and then hung under its marker: the first line carries the marker leaf and a gap
/// that together fill the indent, the rest are shifted right by it, so every line's right edge still
/// lands on the measure. The marker column is the widest marker the list uses plus
/// [`list_marker_gap`](Style), so a bullet list and a numbered list of ten items align their text
/// alike. Items are parted by [`list_item_skip`](Style); the list's space from its neighbours is the
/// caller's. Each item is a segment run, so it breaks through the same path a rich paragraph does and
/// may carry emphasis, a footnote or inline maths.
fn list(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style:		Style,
	measure:	Sp,
	ordered:	bool,
	items:		&[Vec<Segment>],
	foot_no:	&mut u32,
	ref_no:		&mut u32,
	seen:		&mut HashSet<String>,
)
	-> Outcome<()>
{
	// Shape every marker once and keep the widest, so each item's text starts at the one indent.
	let mut markers:	Vec<ShapedText>	= Vec::with_capacity(items.len());
	let mut marker_w					= Sp::ZERO;
	for idx in 0..items.len() {
		let label	= if ordered { fmt!("{}.", idx + 1) } else { "\u{2022}".to_string() };	// U+2022 bullet
		let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size, &label));
		if shaped.dims().width > marker_w { marker_w = shaped.dims().width; }
		markers.push(shaped);
	}
	let indent	= marker_w + style.list_marker_gap;
	let inner	= if measure > indent { measure - indent } else { measure };

	for (idx, item) in items.iter().enumerate() {
		if idx > 0 {
			nodes.push(Node::Glue(Glue::fixed(style.list_item_skip)));
		}
		let pieces		= res!(build_pieces(fonts.clone(), geom, style, item, foot_no, ref_no, seen));
		let mut lines	= res!(break_paragraph_pieces(
			fonts.clone(), Role::Body, Dir::Ltr, style.body_size, &pieces, inner, style.leading));
		indent_item(&mut lines, Leaf::text(markers[idx].clone()), indent);
		nodes.extend(lines);
	}
	Ok(())
}

/// Sets a verbatim code block: each source line in the mono face, its leading whitespace preserved by
/// shaping the whole line, given a one-em hanging indent, and never justified or wrapped. A blank line
/// keeps the mono line's height so the block's vertical rhythm holds. The block's space from its
/// neighbours is the caller's. A long line overflows the measure rather than wrapping -- code is not
/// reflowed; a scrolling or wrapping treatment is a later refinement, as is keeping the block whole
/// across a page break.
fn code_block(
	nodes:	&mut Vec<Node>,
	fonts:	Arc<FontSet>,
	style:	Style,
	lines:	&[String],
)
	-> Outcome<()>
{
	// Code is set a touch smaller than the body, as most templates do, so more of a wide line fits the
	// measure before it overflows.
	let size	= style.foot_size;
	let indent	= style.body_size;	// a one-em hang, so the block sits off the left margin
	let sample	= res!(ShapedText::new(fonts.clone(), Role::Mono, Dir::Ltr, size, "0"));
	let sh		= sample.dims().height;	// a mono digit fixes the height of a blank line
	let sd		= sample.dims().depth;
	for (i, line) in lines.iter().enumerate() {
		let shaped	= res!(ShapedText::new(
			fonts.clone(), Role::Mono, Dir::Ltr, size,
			if line.is_empty() { " " } else { line }));
		let d		= shaped.dims();
		let h		= if d.height > Sp::ZERO { d.height } else { sh };
		let dep		= if d.depth > Sp::ZERO { d.depth } else { sd };
		let children = vec![Node::Glue(Glue::fixed(indent)), Node::Leaf(Leaf::text(shaped))];
		nodes.push(Node::HBox(BoxNode::new(children, Dims::new(indent + d.width, h, dep))));
		if i + 1 < lines.len() {
			let gap = if style.leading > h + dep { style.leading - h - dep } else { style.line_gap };
			nodes.push(Node::Glue(Glue::fixed(gap)));
		}
	}
	Ok(())
}

/// Hangs a broken item under its marker. The first line takes the marker leaf and a gap filling the
/// rest of the indent; every line takes a leading glue that shifts it right by the indent; each line's
/// box grows to the full measure. The item was broken at `measure - indent`, so the right edge lands on
/// the measure. Only [`Node::HBox`] lines are shifted -- the interline glue between them is left alone.
fn indent_item(lines: &mut [Node], marker: Leaf, indent: Sp) {
	let mut first = true;
	for line in lines.iter_mut() {
		if let Node::HBox(b) = line {
			if first {
				let gap = if indent > marker.dims.width { indent - marker.dims.width } else { Sp::ZERO };
				b.list.insert(0, Node::Glue(Glue::fixed(gap)));
				b.list.insert(0, Node::Leaf(marker.clone()));
				first = false;
			} else {
				b.list.insert(0, Node::Glue(Glue::fixed(indent)));
			}
			b.dims = Dims::new(b.dims.width + indent, b.dims.height, b.dims.depth);
		}
	}
}

/// Builds a footnote from its already-shaped body mark and its note text. The note is set as a small
/// paragraph at the foot measure, prefixed by the number as a hanging superscript, and its stacked
/// height noted so the page breaker can reserve it.
fn build_footnote(
	fonts:		Arc<FontSet>,
	style:		Style,
	measure:	Sp,
	number:		u32,
	note:		&str,
	mark:		ShapedText,
)
	-> Outcome<Footnote>
{
	let mut lines = res!(break_paragraph(
		fonts.clone(), Role::Body, Dir::Ltr, style.foot_size, note, measure, style.foot_leading));

	// Prefix the note's first line with the number as a small superscript and a thin gap, so the note
	// reads against its mark. The prefix is shorter than the line, so it does not change the line height.
	let (pre_shaped, pre_dims) = res!(superscript(fonts.clone(), Role::Body, style.foot_size, &fmt!("{}", number)));
	if let Some(Node::HBox(b)) = lines.first_mut() {
		let gap = Sp(style.foot_size.raw() / 4);
		b.list.insert(0, Node::Glue(Glue::fixed(gap)));
		b.list.insert(0, Node::Leaf(Leaf::text_dims(pre_shaped, pre_dims)));
	}

	let mut height = Sp::ZERO;
	for n in &lines {
		height += n.vextent();
	}

	Ok(Footnote { number, mark, note: lines, height })
}

/// Shapes a short run at `0.7x` the surrounding size and returns it with the box that raises its
/// baseline. The box height is the surrounding ascent less a raise of a third of that ascent; the
/// emitter draws a run's baseline at `y + height`, so a shorter box lifts the run above the line's
/// baseline. The width and depth are the small run's own, keeping the mark narrow.
fn superscript(
	fonts:	Arc<FontSet>,
	role:	Role,
	base:	Sp,
	text:	&str,
)
	-> Outcome<(ShapedText, Dims)>
{
	let small	= Sp(base.raw() * 7 / 10);
	let shaped	= res!(ShapedText::new(fonts.clone(), role, Dir::Ltr, small, text));
	let sd		= shaped.dims();

	// The surrounding line's ascent, taken from a body-size digit, and the raise off its baseline.
	let sample	= res!(ShapedText::new(fonts, role, Dir::Ltr, base, "0"));
	let ascent	= sample.dims().height;
	let raise	= Sp(ascent.raw() * 35 / 100);
	let height	= if ascent > raise { ascent - raise } else { ascent };

	Ok((shaped, Dims::new(sd.width, height, sd.depth)))
}

/// Sets a display equation as a centred line, appended to the vertical list. The maths box is laid
/// out, its returned HBox unwrapped, and its leaves centred in the measure; a numbered equation gets
/// its number flush at the right margin and an [`Equation`](crate::ledger::AnchorKind::Equation) anchor
/// recorded just before the line, so the ledger can later resolve a reference to it. The line's height
/// and depth take the greater of the maths extent and a body digit, so a short equation still leaves
/// room for its number.
fn equation(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style:		Style,
	measure:	Sp,
	expr:		&Atom,
	number:		Option<u32>,
)
	-> Outcome<()>
{
	let node = res!(math::layout(fonts.clone(), &style, expr, true));
	let (list, dims) = match node {
		Node::HBox(b)	=> (b.list, b.dims),
		_				=> return Err(err!(
			"Maths layout returned a non-HBox node for a display equation."; Bug)),
	};

	let w		= dims.width;
	let centre	= if measure > w { Sp((measure.raw() - w.raw()) / 2) } else { Sp::ZERO };
	let baseline	= dims.height;	// the maths baseline's distance below the line top

	// A body digit fixes the line's minimum height and depth, so the number is never clipped when the
	// maths sits shallow.
	let sample	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size, "0"));
	let height	= if baseline > sample.dims().height { baseline } else { sample.dims().height };
	let depth	= if dims.depth > sample.dims().depth { dims.depth } else { sample.dims().depth };

	let mut children:	Vec<Node> = Vec::new();
	if centre.raw() > 0 {
		children.push(Node::Glue(Glue::fixed(centre)));
	}
	for n in list {
		children.push(n);
	}
	let cursor = centre + w;	// where the maths ends, from the line's left

	if let Some(num) = number {
		let label	= fmt!("({})", num);
		let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size, &label));
		let nw		= shaped.dims().width;
		let target	= if measure > nw { measure - nw } else { cursor };
		if target > cursor {
			children.push(Node::Glue(Glue::fixed(target - cursor)));
		}
		// The number sits on the maths baseline; a zero-height leaf plus the baseline shift seats it there.
		let leaf = Leaf::text_dims(shaped, Dims::new(nw, Sp::ZERO, Sp::ZERO)).with_shift(baseline);
		children.push(Node::Leaf(leaf));

		let id = AnchorId::new(AnchorKind::Equation, fmt!("eq-{}", num));
		nodes.push(Node::Anchor(id));
	}

	nodes.push(Node::HBox(BoxNode::new(children, Dims::new(measure, height, depth))));
	Ok(())
}

/// Sets a figure: its identity as a [`Float`](crate::ledger::AnchorKind::Float) anchor, the graphic
/// centred on its own line, and a caption centred beneath. The graphic's dimensions are its bounding
/// box, `height` the whole visual extent and `depth` zero, so the line advances by the figure's height
/// and the greedy breaker moves it whole. The anchor is recorded before the ink so a reference to the
/// figure resolves the page it lands on.
fn figure(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style:		Style,
	measure:	Sp,
	graphic:	Graphic,
	caption:	Option<&str>,
	number:		u32,
)
	-> Outcome<()>
{
	let id = AnchorId::new(AnchorKind::Float, fmt!("fig-{}", number));
	nodes.push(Node::Anchor(id));

	// The graphic centred: a fixed box with glue to its left, on a line whose height is the figure's.
	let leaf	= Leaf::graphic(graphic);
	let gw		= leaf.dims.width;
	let gh		= leaf.dims.height + leaf.dims.depth;
	let pad		= if measure > gw { Sp((measure.raw() - gw.raw()) / 2) } else { Sp::ZERO };
	let mut row:	Vec<Node> = Vec::new();
	if pad.raw() > 0 {
		row.push(Node::Glue(Glue::fixed(pad)));
	}
	row.push(Node::Leaf(leaf));
	nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, gh, Sp::ZERO))));

	// The caption, centred beneath the figure, set in the italic at the footnote size.
	let text = match caption {
		Some(c)	=> fmt!("Figure {}.  {}", number, c),
		None	=> fmt!("Figure {}.", number),
	};
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(5.0))));
	let shaped	= res!(ShapedText::new(fonts, Role::Italic, Dir::Ltr, style.foot_size, &text));
	let cd		= shaped.dims();
	let cpad	= if measure > cd.width { Sp((measure.raw() - cd.width.raw()) / 2) } else { Sp::ZERO };
	let mut crow:	Vec<Node> = Vec::new();
	if cpad.raw() > 0 {
		crow.push(Node::Glue(Glue::fixed(cpad)));
	}
	crow.push(Node::Leaf(Leaf::text(shaped)));
	nodes.push(Node::HBox(BoxNode::new(crow, Dims::new(measure, cd.height, cd.depth))));
	Ok(())
}

/// The next number for a figure supplement, incrementing its running count so tables and figures carry
/// independent sequences.
fn next_number(counters: &mut HashMap<String, u32>, supplement: &str) -> u32 {
	let n = counters.entry(supplement.to_string()).or_insert(0);
	*n += 1;
	*n
}

/// Sets a table wrapped in a figure: the figure's anchors, the ruled table as one keep box, then a
/// numbered caption beneath. The table lowers exactly as a bare [`Block::Table`] does, so it moves whole
/// to the next page when it will not fit where it stands.
#[allow(clippy::too_many_arguments)]
fn table_figure(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style:		Style,
	measure:	Sp,
	table:		&Table,
	caption:	Option<&str>,
	supplement:	&str,
	number:		u32,
	label:		Option<&str>,
)
	-> Outcome<()>
{
	figure_anchors(nodes, supplement, number, label);
	nodes.push(res!(table::lower(fonts.clone(), style, measure, table)));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(5.0))));
	res!(captioned(nodes, fonts, style, measure, supplement, number, caption));
	Ok(())
}

/// Sets an image wrapped in a figure: the figure's anchors, a sized placeholder box centred where the
/// image will go, then a numbered caption beneath. The image itself is loaded by a later increment; the
/// placeholder holds its space so the surrounding pagination is already near-right.
#[allow(clippy::too_many_arguments)]
fn image_figure(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style:		Style,
	measure:	Sp,
	_path:		&str,
	caption:	Option<&str>,
	supplement:	&str,
	number:		u32,
	label:		Option<&str>,
)
	-> Outcome<()>
{
	figure_anchors(nodes, supplement, number, label);

	let leaf	= Leaf::graphic(res!(placeholder(measure)));
	let gw		= leaf.dims.width;
	let gh		= leaf.dims.height + leaf.dims.depth;
	let pad		= if measure > gw { Sp((measure.raw() - gw.raw()) / 2) } else { Sp::ZERO };
	let mut row:	Vec<Node> = Vec::new();
	if pad.raw() > 0 {
		row.push(Node::Glue(Glue::fixed(pad)));
	}
	row.push(Node::Leaf(leaf));
	nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, gh, Sp::ZERO))));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(5.0))));
	res!(captioned(nodes, fonts, style, measure, supplement, number, caption));
	Ok(())
}

/// Records a figure's anchors: an author label (when the source labelled it) so a cross-reference
/// resolves the figure's page, and a [`Float`](crate::ledger::AnchorKind::Float) anchor keyed by
/// supplement and number for the figure's own identity.
fn figure_anchors(nodes: &mut Vec<Node>, supplement: &str, number: u32, label: Option<&str>) {
	if let Some(l) = label {
		nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Label, l.to_string())));
	}
	nodes.push(Node::Anchor(AnchorId::new(
		AnchorKind::Float, fmt!("{}-{}", supplement.to_lowercase(), number))));
}

/// Sets a figure caption -- "{supplement} {number}: {caption}" -- centred beneath the figure, wrapped
/// greedily into ragged centred lines at the body size. A caption with no text sets just its number.
fn captioned(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style:		Style,
	measure:	Sp,
	supplement:	&str,
	number:		u32,
	caption:	Option<&str>,
)
	-> Outcome<()>
{
	let text = match caption {
		Some(c) if !c.trim().is_empty()	=> fmt!("{} {}: {}", supplement, number, c.trim()),
		_								=> fmt!("{} {}", supplement, number),
	};

	let size	= style.body_size;
	let mut line	= String::new();
	let mut first	= true;
	for word in text.split_whitespace() {
		let trial = if line.is_empty() { word.to_string() } else { fmt!("{} {}", line, word) };
		let shaped = res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, &trial));
		if shaped.dims().width > measure && !line.is_empty() {
			res!(emit_caption_line(nodes, fonts.clone(), style, measure, size, &line, &mut first));
			line = word.to_string();
		} else {
			line = trial;
		}
	}
	if !line.is_empty() {
		res!(emit_caption_line(nodes, fonts.clone(), style, measure, size, &line, &mut first));
	}
	Ok(())
}

/// Sets one centred caption line, with interline leading before every line but the first.
fn emit_caption_line(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style:		Style,
	measure:	Sp,
	size:		Sp,
	line:		&str,
	first:		&mut bool,
)
	-> Outcome<()>
{
	let shaped	= res!(ShapedText::new(fonts, Role::Body, Dir::Ltr, size, line));
	let d		= shaped.dims();
	if !*first {
		let vext	= d.height + d.depth;
		let gap		= if style.leading > vext { style.leading - vext } else { style.line_gap };
		nodes.push(Node::Glue(Glue::fixed(gap)));
	}
	*first = false;
	let pad		= if measure > d.width { Sp((measure.raw() - d.width.raw()) / 2) } else { Sp::ZERO };
	let mut row:	Vec<Node> = Vec::new();
	if pad.raw() > 0 {
		row.push(Node::Glue(Glue::fixed(pad)));
	}
	row.push(Node::Leaf(Leaf::text(shaped)));
	nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, d.height, d.depth))));
	Ok(())
}

/// Builds the placeholder box that stands in for an image this increment does not load: a light-filled,
/// lightly-stroked rectangle the width of the measure and half as tall, capped so a wide page does not
/// leave a giant void. The caption beneath still names the figure.
fn placeholder(measure: Sp) -> Outcome<Graphic> {
	let w	= measure.to_pt() as f32;
	let h	= (w * 0.5).clamp(120.0, 360.0);
	let mut pb = PathBuilder::new();
	pb.move_to(Pt::new(0.0, 0.0));
	pb.line_to(Pt::new(w, 0.0));
	pb.line_to(Pt::new(w, h));
	pb.line_to(Pt::new(0.0, h));
	pb.close();
	let path	= res!(pb.finish());
	let ops		= vec![
		DrawOp::Fill { path: path.clone(), colour: Rgba::opaque(238, 238, 240) },
		DrawOp::Stroke { path, colour: Rgba::opaque(150, 150, 150), width: 0.8 },
	];
	Ok(Graphic::new(ops, Dims::new(Sp::from_pt(w as f64), Sp::from_pt(h as f64), Sp::ZERO)))
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
