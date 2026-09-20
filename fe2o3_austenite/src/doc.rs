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

use crate::bib::Bibliography;
use crate::driver::{
	Document,
	FootStyle,
};
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	ColumnsNode,
	Dims,
	DrawOp,
	FloatNode,
	FloatPlacement,
	Footnote,
	Glue,
	Graphic,
	Leaf,
	LeafKind,
	Length,
	Node,
	Penalty,
	RasterImage,
	Sp,
};
use crate::ledger::{
	AnchorId,
	AnchorKind,
	Ledger,
	Ref,
};
use crate::memo::{
	BlockEntry,
	BlockState,
	Fnv,
	Memo,
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
	Align,
	Cell,
	Row,
	Table,
};
use crate::page::{
	Frame,
	Page,
	PageGeometry,
	Placed,
	PlacedKind,
};
use crate::theme::{
	Theme,
	ThemePatch,
};

use oxedyne_fe2o3_core::prelude::*;
use crate::fonts::FaceResolver;

use oxedyne_fe2o3_font::{
	face::Role,
	font::Font,
	set::FontSet,
	shape::Dir,
};
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
		PathBuilder,
		Pt,
	},
	svg_doc::{
		Anchor,
		SvgOp,
		SvgPicture,
	},
	transform::Transform,
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
	BoldItalic(String),	// `*_x_*`/`_*x*_`, set in the bold-italic face
	Super(String),	// #super[...], set raised and smaller, its baseline lifted above the line's
	Sub(String),	// #sub[...], set dropped and smaller, its baseline lowered below the line's
	Footnote { note: Vec<Segment> },
	Math(Atom),	// an inline maths expression, set within the running line
	PageRef(String),	// a cross-reference to a labelled anchor, resolving to its page number
	Code(String),	// an inline code span, set in the mono face
	Glossary { term: String, display: String },	// a glossary term: bold-italic on its first document use, plain after
	Cite(Vec<String>),	// a citation, resolved to "(Author Year)" against the bibliography
	// A `#claim-label(...)` or `#claim-refs(...)`: a zero-width margin anchor setting nothing in the body
	// column. `display` is the compressed code a label draws in the outside margin (empty for a metadata-only
	// reference); `codes` are the raw codes a reference registers for the reverse claim index (empty for a label).
	MarginNote { display: String, codes: Vec<String> },
	// An index marker: records the term's occurrence for the back-matter index and sets nothing in the body.
	// `term` is the sort key (markup flattened, e.g. "March, James"); `display` is the styled text the index
	// page sets (e.g. "James March", or an italicised case name), so the index prints the display and never
	// the sort key. `sub` carries a nested entry's child term. `main` marks a primary reference (`#idx-main`),
	// whose folio the index page sets bold, as in-dexter's `index-main = index.with(fmt: strong)` does. The
	// back-matter index reads every occurrence's page back from the ledger post-convergence.
	Index { term: String, sub: Option<String>, display: Vec<Segment>, main: bool },
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

	pub fn bold_italic<S: Into<String>>(text: S) -> Self {
		Self::BoldItalic(text.into())
	}

	pub fn superscript<S: Into<String>>(text: S) -> Self {
		Self::Super(text.into())
	}

	pub fn subscript<S: Into<String>>(text: S) -> Self {
		Self::Sub(text.into())
	}

	pub fn footnote(note: Vec<Segment>) -> Self {
		Self::Footnote { note }
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

	pub fn cite(keys: Vec<String>) -> Self {
		Self::Cite(keys)
	}

	pub fn margin_note<S: Into<String>>(display: S, codes: Vec<String>) -> Self {
		Self::MarginNote { display: display.into(), codes }
	}

	pub fn index<T: Into<String>>(term: T, sub: Option<String>, main: bool, display: Vec<Segment>) -> Self {
		Self::Index { term: term.into(), sub, display, main }
	}
}

/// One entry of a [`Block::List`]: its own rich runs and any lists nested beneath it, so a step carrying
/// indented sub-bullets keeps them under the step. A nested list is itself a [`Block::List`], set at an
/// increased left indent when the parent renders.
#[derive(Clone, Debug)]
pub struct ListEntry {
	pub segments:	Vec<Segment>,
	pub children:	Vec<Block>,
}

/// One block of the authored document. The closed vocabulary the block layer sets; richer blocks
/// (lists, quotes, figures) are later variants here.
#[derive(Clone, Debug)]
pub enum Block {
	Heading { level: u8, segments: Vec<Segment>, label: Option<String> },	// segments: the title's rich runs; label: an author anchor a `#ref` resolves to
	Paragraph { text: String },
	RichParagraph { segments: Vec<Segment> },	// a paragraph carrying footnote marks
	List { ordered: bool, items: Vec<ListEntry> },	// a bullet or numbered list; an entry may nest sub-lists
	Code { lines: Vec<String> },	// a verbatim code block, set in the mono face, whitespace preserved
	Table(Table),
	Equation { expr: Atom, numbered: bool, label: Option<String> },	// a display equation on its own centred line; label anchors an @-reference
	// A drawn figure, centred, numbered, captioned. `placement` is `Some` when the source floated it
	// (`figure(placement: auto | top | bottom)`): the driver then sets it at the top or foot of the next
	// page it fits on rather than in the flow. `None` (the Typst default, and `placement: none`) sets it
	// where it stands.
	Figure { graphic: Graphic, caption: Option<String>, placement: Option<FloatPlacement> },
	// A `#figure(...)` wrapping a `#table(...)`: the ruled table, then a numbered caption beneath. The
	// supplement is the caption's leading word ("Table"/"Figure"); the label anchors a cross-reference.
	TableFigure { table: Table, caption: Option<Vec<Segment>>, supplement: String, label: Option<String>, placement: Option<FloatPlacement> },
	// A `#figure(...)` wrapping an image: the loaded raster centred in the measure with the numbered
	// caption beneath, or -- when the path resolves to nothing or is a vector SVG with no raster beside
	// it -- a sized placeholder box in its place. The sizing hints size the drawn image.
	ImageFigure {
		path:		String,
		width:		Option<Length>,
		height:		Option<Length>,
		scale:		Option<f64>,
		caption:	Option<Vec<Segment>>,
		supplement:	String,
		label:		Option<String>,
		placement:	Option<FloatPlacement>,
	},
	// A `#figure(...)` whose body is drawn by code -- a CeTZ/Fletcher diagram, a bar chart or a line plot.
	// The graphic is built at render time from the document's font set and placed like an image figure,
	// with the numbered caption beneath.
	CodeFigure {
		figure:		crate::lang::codefig::CodeFigure,
		caption:	Option<Vec<Segment>>,
		supplement:	String,
		label:		Option<String>,
		placement:	Option<FloatPlacement>,
	},
	// A back-matter section title (the Bibliography) on its own page, set left in the display face and
	// unnumbered. It records a heading anchor so the contents lists it, and a back-matter marker so the
	// running head is dropped and the folio centres from here on.
	BackMatterHeading { title: String },
	// One bibliography reference: its styled runs, each carrying whether it sets in italic. Set small,
	// as a paragraph the reader reads as one entry.
	Reference { runs: Vec<(String, bool)> },
	// A standalone `#line(...)` horizontal divider: a stroked rule of the given width (a fraction of the
	// measure or an absolute length), thickness in points, and grey level, with a paragraph skip either side.
	Rule { width: Length, thickness: f64, grey: u8 },
	// A line-leading `#padded-image(...)`/`#image(...)`: the loaded image centred in the measure with a
	// little space either side, carrying no figure number or caption -- a section opener's logo, not a float.
	Image { path: String, width: Option<Length>, height: Option<Length>, scale: Option<f64> },
	// A line-leading `#section-banner("logo")`: a fresh page, then the template's full-width grey bar hanging
	// into the top and side margins, carrying the section's logo right-aligned on the band's vertical middle.
	SectionBanner { path: String },
	// A line-leading `#print-glossary()` before the book layer resolves it: a placeholder the assembler
	// replaces in place with a [`Table`] of the document's glossary terms and their definitions. It never
	// survives to layout -- `book::resolve_glossary` walks the assembled blocks and swaps it out -- so the
	// layout and word-count passes treat a stray one as empty rather than setting anything for it.
	Glossary,
	// The back-matter index placeholder: a marker the assembler appends after the bibliography and glossary
	// when the root asks for an index (`meta-data.index: true`) and the body carries index markers. It sets
	// nothing itself; [`author`] builds the alphabetical entry list from the index-marker occurrences it
	// gathered walking the body, each entry's page list read back from the ledger post-convergence.
	Index,
	// The reverse claim-reference index placeholder: a marker a line-leading `#context { ... collect-claim-refs()
	// ... }` in the Logic appendix lowers to. It sets nothing itself; [`author`] groups the claim references
	// gathered walking the body by code (byte order, matching Typst's `.sorted()`) and sets one wrapped paragraph
	// per code -- the bold code, a colon, and one page per reference in document order (NOT deduplicated: a code
	// referenced twice on a page lists that page twice, as `claims.typ`'s `pages.join(", ")` does), each page read
	// back from the ledger post-convergence.
	ClaimIndex,
	// A `#styled-box[...]` callout: its inner blocks set inside a padded box that runs the full measure,
	// washed the template's `colours.veronica.lighten(90%)` (a pale violet) with a 4 pt corner radius. The
	// callout is laid out as one keep box, so it moves whole to the next page rather than splitting the wash
	// from its words. `patch` is the theme overlay the box body's own `#set` declarations lower to, applied
	// to the box's subtree at render (H3) so a `#set` inside a callout scopes to it, not the document.
	// `placement` is `Some` when the callout is a float (an `#aside-box(float: true)` re-wrapped in
	// `figure(placement: auto)`): the driver then defers it to the next page it fits on rather than pushing
	// the flow down. `None` sets it where it stands.
	Box { blocks: Vec<Block>, patch: ThemePatch, placement: Option<FloatPlacement> },
	// A theme scope: `patch` is overlaid on the effective theme for the nested `blocks`, and lifts again
	// when they end. Nesting the governed blocks rather than bracketing them with a separate open/close
	// marker makes an unmatched or missing close structurally impossible, and every pass that recurses over
	// `blocks` scopes for free -- an included chapter's (or any selected subtree's) `#set` declarations
	// style only that subtree, not the document. This is the general mechanism the rule engine's set-fields
	// transform reuses to patch a subtree.
	Scoped { patch: ThemePatch, blocks: Vec<Block> },
	// A vertical space a `#show` template's `v(<len>)` lowers to: a fixed leading emitted as a sibling
	// before or after the element the template wraps. It carries no words and anchors no reference.
	Space(Sp),
	// A line-leading `#pagebreak()` (or `#pagebreak(weak: true)`): a forced page eject at this point in the
	// flow. Emitted as a forced break penalty, which the driver drops when the page is already fresh, so a
	// break that lands at a page top never opens a blank page -- the WEAK semantics, the same the section
	// furniture turns the page with (see the `SectionBanner` arm). Both markup forms map here: a strong
	// `#pagebreak()` on an already-empty page (Typst's default WOULD open a blank one) is not distinguished,
	// a documented weak-only limitation, not exercised by any corpus.
	PageBreak,
}

impl Block {
	pub fn heading<S: Into<String>>(level: u8, text: S) -> Self {
		Self::Heading { level, segments: vec![Segment::text(text.into())], label: None }
	}

	/// A heading carrying an author label, so a `#ref(<label>)` elsewhere resolves to its page.
	pub fn heading_labelled<S: Into<String>>(level: u8, text: S, label: Option<String>) -> Self {
		Self::Heading { level, segments: vec![Segment::text(text.into())], label }
	}

	/// A heading whose title carries rich inline runs -- emphasis, a glossary term, an index call or a
	/// maths span -- so each sets its display text in the head and the table of contents rather than
	/// leaking its raw source.
	pub fn heading_rich(level: u8, segments: Vec<Segment>, label: Option<String>) -> Self {
		Self::Heading { level, segments, label }
	}

	pub fn paragraph<S: Into<String>>(text: S) -> Self {
		Self::Paragraph { text: text.into() }
	}

	pub fn rich(segments: Vec<Segment>) -> Self {
		Self::RichParagraph { segments }
	}

	/// A bullet (`ordered` false) or numbered (`ordered` true) list. Each entry carries its run sequence --
	/// emphasis, a footnote or inline maths, as a rich paragraph does -- and any sub-lists nested beneath it.
	pub fn list(ordered: bool, items: Vec<ListEntry>) -> Self {
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

	pub fn rule(width: Length, thickness: f64, grey: u8) -> Self {
		Self::Rule { width, thickness, grey }
	}

	pub fn space(height: Sp) -> Self { Self::Space(height) }

	pub fn page_break() -> Self { Self::PageBreak }

	/// A `#styled-box[...]` callout: the inner blocks set in a padded box washed the template's pale
	/// violet. The wash is the theme's `callout.fill` at render (its default that pale violet), so a
	/// `#set`/rule that lowers a callout fill reaches it; the caller supplies the body and the theme patch
	/// the box's own `#set` declarations lowered to (empty when it declared none).
	pub fn box_callout(blocks: Vec<Block>, patch: ThemePatch) -> Self {
		Self::Box { blocks, patch, placement: None }
	}

	/// A `#styled-box`/`#aside-box` callout the source floated (`float: true`): the driver defers it to the
	/// top or foot of the next page it fits on, its wash and words kept together.
	pub fn box_callout_float(blocks: Vec<Block>, patch: ThemePatch, placement: FloatPlacement) -> Self {
		Self::Box { blocks, patch, placement: Some(placement) }
	}

	/// A display equation set centred on its own line. A numbered one takes the next equation number at
	/// the right margin and records an [`Equation`](crate::ledger::AnchorKind::Equation) anchor; a trailing
	/// `<label>` lets an `@`-reference resolve to "Equation N".
	pub fn equation(expr: Atom, numbered: bool, label: Option<String>) -> Self {
		Self::Equation { expr, numbered, label }
	}

	/// A drawn figure, centred on its own line and captioned "Figure N" beneath, its identity recorded
	/// as a [`Float`](crate::ledger::AnchorKind::Float) anchor so a cross-reference resolves its page.
	pub fn figure(graphic: Graphic, caption: Option<String>, placement: Option<FloatPlacement>) -> Self {
		Self::Figure { graphic, caption, placement }
	}

	/// A table wrapped in a figure: the ruled grid, then a "{supplement} N: {caption}" line beneath,
	/// numbered per supplement so tables and figures carry independent counts.
	pub fn table_figure(
		table:		Table,
		caption:	Option<Vec<Segment>>,
		supplement:	String,
		label:		Option<String>,
		placement:	Option<FloatPlacement>,
	)
		-> Self
	{
		Self::TableFigure { table, caption, supplement, label, placement }
	}

	/// An image wrapped in a figure: the raster at `path`, sized by the declared hints, centred in the
	/// measure with its numbered caption beneath. A path that resolves to nothing, or a vector SVG with no
	/// raster beside it, falls back to a placeholder box at render time.
	#[allow(clippy::too_many_arguments)]
	pub fn image_figure(
		path:		String,
		width:		Option<Length>,
		height:		Option<Length>,
		scale:		Option<f64>,
		caption:	Option<Vec<Segment>>,
		supplement:	String,
		label:		Option<String>,
		placement:	Option<FloatPlacement>,
	)
		-> Self
	{
		Self::ImageFigure { path, width, height, scale, caption, supplement, label, placement }
	}

	/// A figure drawn by code (a diagram, bar chart or line plot): its builder, numbered caption, and the
	/// label a cross-reference resolves to. The graphic is built at render time from the font set.
	pub fn code_figure(
		figure:		crate::lang::codefig::CodeFigure,
		caption:	Option<Vec<Segment>>,
		supplement:	String,
		label:		Option<String>,
		placement:	Option<FloatPlacement>,
	)
		-> Self
	{
		Self::CodeFigure { figure, caption, supplement, label, placement }
	}

	/// A back-matter section heading (the Bibliography), on its own page, unnumbered.
	pub fn back_matter_heading<S: Into<String>>(title: S) -> Self {
		Self::BackMatterHeading { title: title.into() }
	}

	/// One bibliography reference, a sequence of runs each flagged for italic.
	pub fn reference(runs: Vec<(String, bool)>) -> Self {
		Self::Reference { runs }
	}

	/// A plain centred image (a `#padded-image`/`#image` section logo), with any declared sizing.
	pub fn image(path: String, width: Option<Length>, height: Option<Length>, scale: Option<f64>) -> Self {
		Self::Image { path, width, height, scale }
	}

	/// A documentation section's opening banner: a fresh page carrying the template's full-width grey bar
	/// with the logo at `path` right-aligned on it.
	pub fn section_banner(path: String) -> Self {
		Self::SectionBanner { path }
	}
}

/// The point sizes and vertical spaces the block layer sets to. Every length is scaled points, so
/// How a top-level heading opens. A book chapter opens with the giant grey number and dotted numbering
/// the manuscripts use ([`BookOpener`](HeadingStyle::BookOpener)); a documentation tree that opens each
/// chapter with the template's full-width grey banner bar and no numbering takes
/// ([`DocBanner`](HeadingStyle::DocBanner)); a documentation tree whose sections carry their own
/// `#section-banner` logo bar (the Hematite guide) sets its level-1 headings inline instead, with no
/// banner and no numbering ([`DocInline`](HeadingStyle::DocInline)) -- the template's `chapter-banners:
/// false`. The block layer reads this to pick the opener and to decide whether a heading carries a
/// number, so one authoring path serves every idiom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadingStyle {
	BookOpener,
	DocBanner,
	DocInline,
	DocGrid,	// a doc tree whose template opens level 1 with a fixed logo/title grid, no number -- oxeweb's template.typ
}

/// A recorded heading: the anchor identity the ledger resolves to a page, its level, and its display
/// title. The block layer keeps this table beside the composed stream so [`decorate`] can read a
/// title back from an anchor -- the ledger stores only the identity, not the words.
#[derive(Clone, Debug)]
pub struct Heading {
	pub id:			AnchorId,
	pub level:		u8,
	pub title:		String,	// the display words, markup removed, for the anchor slug and a plain fallback
	pub segments:	Vec<Segment>,	// the title's rich runs, so a running head or contents entry renders its maths and emphasis
	pub number:		String,	// the dotted number a numbered heading shows ("2.3.1"); empty for a part divider
	pub banner:		bool,	// set inline beneath a `#section-banner`, so the page suppresses its running head like a chapter opener
}

/// The book's front matter, read from the root's template call: the title, subtitle and author the
/// title page sets, the cover raster a development build carries, and the imprint the meta page prints.
/// A field a book omits is `None` and its line is not set. The whole struct is `None` for a lone
/// manuscript, which carries no front matter at all.
#[derive(Clone, Debug, Default)]
pub struct FrontMatter {
	pub title:			String,
	pub subtitle:		Option<String>,
	pub author:			String,
	pub cover_image:	Option<String>,	// a `/assets/...` raster path, set only in a development build
	pub logo_image:		Option<String>,	// the publisher logo under the title, often an SVG (then not set)
	pub publisher:		Option<String>,
	pub edition:		Option<String>,
	pub isbn:			Option<String>,
	pub copyright:		Option<String>,	// the already-composed "Copyright © 2026 ..." line
	pub rights:			Option<String>,
	pub ai_declaration:	Option<String>,
	pub website:		Option<String>,
	pub toolchain:		bool,			// whether to print the "Created using ..." toolchain line
	pub dedication:		Option<String>,
	pub about_author:	Option<String>,	// the author biography, set on its own page
	// The display sizes the title page and back-matter titles set, read from the config's type scale.
	pub title_size:		Sp,
	pub subtitle_size:	Sp,
	pub author_size:	Sp,
	pub back_title_size:	Sp,	// the "About the Author"/"Bibliography" heading size
	// The documentation template's two-column title page (`template.typ`'s `title-page`): a full-height
	// coloured sidebar down the left carrying a logo near its top and one near its foot, with the title and
	// subtitle centred on the white right. `sidebar_grey` marks this idiom -- `Some(luma)` draws it and
	// `None` keeps the book's plain centred title page. The rest are read from the root's `doc.with` call.
	pub sidebar_grey:		Option<u8>,	// the sidebar fill as a grey level; None keeps the plain title page
	pub sidebar_frac:		f64,		// the sidebar width as a fraction of the page width (`margins.title_page`)
	pub title_smallcaps:	bool,		// whether the title sets in small caps rather than italic
	pub top_logo:			Option<String>,	// the logo near the sidebar's top
	pub top_logo_width:		Sp,
	pub bottom_logo:		Option<String>,	// the logo near the sidebar's foot
	pub bottom_logo_width:	Sp,
	pub footer_logo:		Option<String>,	// the logo the template seats at the left of the page footer
	// The documentation template's meta/colophon page (`template.typ`'s `meta-page`): a bordered
	// Ver/Date/Author(s)/Notes table over an acknowledgement, a copyright line and a toolchain line at the
	// page foot. `meta_rows` carries the revision rows, newest first; a non-empty list (or a named author)
	// marks the idiom, so the doc meta page is composed only for a doc tree, never over the book imprint.
	pub meta_rows:			Vec<MetaRow>,	// the revision rows the version table sets, in source order
	pub reading_min:		Option<u32>,	// the whole-document reading time in minutes, appended to the last row's notes
	pub acknowledgement:	Option<String>,	// the acknowledgement paragraph set near the page foot
}

/// One revision row of the documentation meta/colophon table: its version, date, author(s), notes, and
/// the AI-declaration mark the row carries beneath its author. A field the row omits is `None`, and its
/// column is left out of the table when every row omits it (matching the template's `filled` test).
#[derive(Clone, Debug)]
pub struct MetaRow {
	pub version:		Option<String>,
	pub date:			Option<String>,
	pub authors:		String,
	pub notes:			Option<String>,
	pub ai_mark_path:	Option<String>,	// the declaration mark image, resolved from the row's slug
	pub ai_mark_words:	Option<String>,	// the mark's caption, the row's own words when it rescopes them
	pub ai_mark_url:	Option<String>,	// the scheme page the mark links to, <scheme>/<slug>/<medium>
}

/// The index markers gathered walking the body: a document-order counter making each occurrence's anchor
/// identity unique, and the occurrences themselves (the term, a nested child term, and the anchor keyed by
/// the counter). The back-matter index groups these by term, and each entry reads its occurrences' pages
/// back from the ledger after convergence. A throwaway one is handed to a measurement flow, whose markers
/// never reach the document.
#[derive(Default)]
pub(crate) struct IndexGather {
	pub(crate) no:	u32,
	// Each occurrence: the sort key, a nested child term, the styled display the index page sets, the anchor
	// keyed by the counter, and whether it is a primary (`#idx-main`) reference whose folio sets bold. The
	// display is carried per occurrence and read from the first of a group.
	pub(crate) occ:	Vec<(String, Option<String>, Vec<Segment>, AnchorId, bool)>,
}

/// The claim references gathered walking the body: one `(code, anchor)` pair per code named in a
/// `#claim-refs(...)` call, the anchor the zero-width margin anchor recorded at the reference point. The
/// reverse claim index groups these by code, and each entry reads its references' pages back from the ledger
/// after convergence, exactly as the back-matter index reads its folios. A throwaway one is handed to a
/// measurement flow, whose references never reach the document.
#[derive(Default)]
pub(crate) struct ClaimGather {
	pub(crate) occ:	Vec<(String, AnchorId)>,
}

/// The mutable authoring state and immutable context of one document render, so the block walk can
/// recurse into a [`Block::Scoped`] subtree -- setting its blocks under the scoped theme while every
/// document-order counter (headings, footnotes, figures, the glossary first-use set) keeps counting
/// across the boundary. The counters and accumulators are shared; only the theme changes per scope.
struct Authoring<'a> {
	// Immutable render context.
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	faces:		&'a FaceResolver,
	measure:	Sp,
	bib:		Option<&'a Bibliography>,
	refs:		HashMap<String, String>,
	// The composed body nodes and the heading table, both grown in document order.
	nodes:		Vec<Node>,
	heads:		Vec<Heading>,
	// Running document-order state.
	first:			bool,
	sec:			[u32; 6],
	prev_para:		bool,
	pending_banner:	bool,
	part_no:		u32,
	foot_no:		u32,
	ref_no:			u32,
	margin_no:		u32,	// a document-order counter making each margin note's anchor identity unique
	eq_no:			u32,
	fig_no:			u32,
	counters:		HashMap<String, u32>,
	seen:			HashSet<String>,
	index_gather:	IndexGather,	// the back-matter index's markers, gathered in document order
	want_index:		bool,			// a `Block::Index` placeholder was met, so the index is built after the walk
	claim_gather:	ClaimGather,	// the reverse claim index's references, gathered in document order
	want_claim_index:	bool,		// a `Block::ClaimIndex` placeholder was met, so the claim index is built after the walk
	claim_index_at:	Option<usize>,	// the body-node position the `Block::ClaimIndex` placeholder sat at, where the listing is spliced in flow
	global_fp:		u64,			// the compile-wide fingerprint (theme, geometry, cross-reference targets) every block memo key folds in
}

/// A continuation handed to [`Authoring::walk`]: the block that follows the walked slice at its parent's
/// level, and the theme it is set under. A heading that is the last block of a [`Block::Scoped`] reads this
/// so its "keep with the next paragraph" pairing sees through the scope's closing edge to the sibling
/// beyond it, rather than stranding the heading and setting the gap's leading twice. The continuation is
/// carried with its own (parent) theme, since it lies outside the scope the heading sits in. `None` at the
/// top level, where the slice has no parent to continue from.
#[derive(Clone, Copy)]
struct Cont<'a> {
	block:	&'a Block,
	theme:	&'a Theme,
}

/// The paragraph a heading keeps with, seen through any single-block `#show par` scope, and the effective
/// theme its kept line is broken under. A bare [`Block::Paragraph`] returns its text and `theme` unchanged;
/// a [`Block::Scoped`] a `par` set-fields rule produced -- always the one-block shape [`rules::wrap_matching`]
/// makes -- is descended, folding each `patch` onto the theme, so the kept line takes the rule's styling.
/// Only a single-block scope is seen through: a multi-block chapter scope (from the book assembler) is left
/// opaque, so a heading never reaches across a chapter boundary to strand its own keep. `None` when the
/// lookahead is not a paragraph (a figure, a list, another heading), so the heading keeps with nothing.
fn keep_with_next_para<'a>(look: &'a Block, theme: &Theme) -> Option<(&'a str, Theme)> {
	match look {
		Block::Paragraph { text }	=> Some((text.as_str(), theme.clone())),
		// The exact single-block wrap a `par` rule makes: descend it under the folded theme. A multi-block
		// scope is not a rule wrap and is left opaque, so the heading does not see through a chapter boundary.
		Block::Scoped { patch, blocks } if blocks.len() == 1 => {
			let scoped = { let mut t = theme.clone(); t.apply(patch); t };
			keep_with_next_para(&blocks[0], &scoped)
		},
		_							=> None,
	}
}

impl<'a> Authoring<'a> {
	/// Sets a block slice under `style`, the theme in force for it. A [`Block::Scoped`] overlays its patch
	/// on `style` and recurses over its own blocks under that scoped theme, so a `#set` inside an included
	/// chapter (or any bracketed subtree) styles only that subtree; the shared counters count on across the
	/// boundary. With no scope -- every corpus document today -- `style` is the document theme throughout, so
	/// the render is byte-identical.
	///
	/// `cont` is the block that follows this slice at the parent's level (`None` at the top). Returns whether
	/// the slice's final heading pulled that continuation paragraph into its keep box -- the caller then skips
	/// the paragraph rather than setting it a second time, so a heading alone in a scope still keeps with the
	/// sibling paragraph beyond the scope edge.
	fn walk(
		&mut self,
		blocks:	&[Block],
		style:	&Theme,
		cont:	Option<Cont<'_>>,
		memo:	&mut Option<&mut Memo>,
	)
		-> Outcome<bool>
	{
		// Set when the final heading of this slice keeps with the parent's continuation paragraph, so the
		// caller skips that paragraph rather than setting it twice.
		let mut consumed_cont = false;
		let mut i = 0usize;
		// The block-authoring memo runs only at the top level, where `style` is the document theme every
		// key is fingerprinted under and there is no parent continuation to reach across; a scoped subtree
		// (an included chapter that declares its own styling -- rare) recurses with the memo switched off,
		// so it is always authored fresh and stays byte-identical. `pending` holds the markers of a block
		// currently being authored on a cache miss: its delta is captured and stored at the top of the next
		// iteration (or after the loop), which is where a `continue`-ing arm -- a chapter opener -- lands.
		let use_memo = cont.is_none() && memo.is_some();
		let mut pending: Option<PendingBlock> = None;
		while i < blocks.len() {
			// Close out the block authored on the previous miss, now that `i` has advanced past it.
			if let Some(p) = pending.take() {
				self.memo_capture(memo, p, i);
			}
			if let Block::Scoped { patch, blocks: inner } = &blocks[i] {
				let scoped = { let mut t = style.clone(); t.apply(patch); t };
				// The scoped slice's continuation is the block that follows the scope at THIS level, set under
				// THIS theme -- so a heading ending the scope keeps with the sibling paragraph beyond it. When
				// the scope is this slice's last block, the continuation is instead this walk's own (the
				// parent's block beyond every enclosing scope, under its own theme), threaded inward so a
				// heading ending a *nested* scope -- an authored rule stacked on the default one -- still keeps
				// with the paragraph past the scopes' closing edges rather than stranding it.
				let has_sibling	= i + 1 < blocks.len();
				let inner_cont	= if has_sibling {
					Some(Cont { block: &blocks[i + 1], theme: style })
				} else {
					cont
				};
				// A scoped subtree is authored fresh (the memo is switched off inside it), so its render stays
				// byte-identical whether or not the memo is present.
				let ate = res!(self.walk(inner, &scoped, inner_cont, &mut None));
				if ate {
					if has_sibling {
						// The inner walk pulled this slice's next sibling into its keep box: skip it here.
						i += 2;
					} else {
						// The inner walk pulled THIS walk's own continuation (the parent's block): tell the
						// caller to skip it, exactly as a bare final heading of this slice would.
						consumed_cont	= true;
						i += 1;
					}
				} else {
					i += 1;
				}
				continue;
			}

			// The block-authoring memo: at the top level, before authoring the block, look it up by its
			// content and the counter state it enters under. A hit splices the previously authored nodes,
			// heads and index/claim occurrences straight back and restores the exit state, skipping the
			// shaping and line breaking entirely; a miss records the markers and captures the delta once the
			// block has been authored (at the next iteration's top). A block whose output is position- or
			// flag-dependent -- the index and claim-index placeholders -- is never memoised.
			if use_memo && memoisable(&blocks[i]) {
				let look = if matches!(&blocks[i], Block::Heading { .. }) {
					blocks.get(i + 1)
				} else {
					None
				};
				let key = self.block_key(&blocks[i], look);
				if let Some(m) = memo.as_deref_mut() {
					if let Some(entry) = m.block_lookup(key) {
						let consume = entry.consume;
						self.apply_block_entry(entry);
						i += consume;
						continue;
					}
				}
				// A miss: remember where authoring this block begins, so its delta can be captured after.
				pending = Some(PendingBlock {
					key,
					i_before:		i,
					nodes_before:	self.nodes.len(),
					heads_before:	self.heads.len(),
					index_before:	self.index_gather.occ.len(),
					claim_before:	self.claim_gather.occ.len(),
					seen_before:	self.seen.clone(),
					counters_before:	self.counters.clone(),
				});
			}
			match &blocks[i] {
				Block::Heading { level, segments, label } => {
					// Step the counters for a numbered level (1..); a part divider (level 0) steps none.
					if *level >= 1 {
						let l = (*level as usize).min(6);
						self.sec[l - 1] += 1;
						for k in l..6 { self.sec[k] = 0; }
					}
					// A documentation tree sets `numbering: none`: its headings carry no dotted number, on the
					// heading line, in the contents, or before a sub-heading. A book keeps the document-order number.
					let number = match style.heading.kind {
						HeadingStyle::DocBanner | HeadingStyle::DocInline | HeadingStyle::DocGrid	=> String::new(),
						HeadingStyle::BookOpener													=> heading_number_themed(*level, &self.sec, style),
					};

					// The rendered title, its markup reduced to display words: it keys the anchor slug and is the
					// title the contents list and the running head read back. The heading itself is set from the
					// rich runs below, so a glossary term or emphasis in a heading renders rather than leaking.
					let title = flatten_segments(segments);
					let id = AnchorId::new(AnchorKind::Heading, fmt!("{:02}-{}", self.heads.len() + 1, slug(&title)));
					// A level-1 heading that follows a `#section-banner` opens its section beneath the banner, so
					// its page suppresses the running head like a chapter opener; the flag is one-shot.
					let banner = self.pending_banner && *level == 1;
					self.pending_banner = false;
					self.heads.push(Heading {
						id:			id.clone(),
						level:		*level,
						title:		title.clone(),
						segments:	segments.clone(),
						number:		number.clone(),
						banner,
					});

					// A chapter (level 1) or a part divider (level 0) opens a fresh page and stands alone; a
					// deeper heading binds to the first line of the paragraph it introduces, so the greedy page
					// breaker never strands it at a page foot. A level-1 heading that carries its own
					// `#section-banner` is the exception: it is set inline beneath the banner the section drew, so
					// it takes the sub-heading path with no page break of its own -- the banner already turned the
					// page. This holds whether the tree sets every section that way (`DocInline`, the Hematite
					// guide) or opts one chapter in with an explicit `#section-banner` while defaulting to the
					// grey title bar (`DocBanner`): an explicit banner always owns its chapter's header, so the
					// duplicate title bar is suppressed regardless of the doc's default mode.
					let opens = *level == 0
						|| (*level == 1 && style.heading.kind != HeadingStyle::DocInline && !banner);
					if opens {
						if !self.first {
							self.nodes.push(Node::Penalty(Penalty::eject()));
						}
						// A part divider (level 0) carries a "Part N" run-in label above its title; a chapter carries
						// none. The ordinal is a Roman numeral, the template's `smallcaps(part-counter.display("I"))`.
						let part_label = if *level == 0 {
							self.part_no += 1;
							fmt!("Part {}", roman(self.part_no))
						} else {
							String::new()
						};
						res!(chapter_opener(
							&mut self.nodes, &self.fonts, self.faces, style, self.geom, self.measure, *level, &number, &title,
							&part_label, &id, label.as_deref()));
						i += 1;
						self.first = false;
						self.prev_para = false;	// the opener is not a paragraph, so the first body line takes no indent
						continue;
					}

					// Space above the heading. At a page top the driver discards it, so the first heading on a
					// page still sits flush to the text block. A heading following a non-consuming heading omits
					// it, so the gap between the two is the upper heading's `space_below` alone.
					if !self.first {
						self.nodes.push(Node::Glue(Glue::fixed(style.space_above(*level))));
					}

					let hbox = res!(subheading_hbox(
						self.fonts.clone(), self.faces, style, *level, &number, segments, &mut self.seen));

					let mut keep:	Vec<Node> = vec![Node::Anchor(id)];
					if let Some(l) = label {
						keep.push(Node::Anchor(AnchorId::new(AnchorKind::Label, l.clone())));
					}
					keep.push(hbox);
					keep.push(Node::Glue(Glue::fixed(style.space_below(*level))));
					let mut rest:	Vec<Node> = Vec::new();
					let mut consumed_para = false;
					// The paragraph the heading keeps with: its in-slice next sibling, or -- when the heading is
					// the last block of a scope -- the parent's continuation beyond the scope's closing edge. The
					// continuation is broken under its own (parent) theme, since it lies outside this scope.
					let (look, look_theme): (Option<&Block>, &Theme) = if i + 1 < blocks.len() {
						(Some(&blocks[i + 1]), style)
					} else {
						match cont {
							Some(c)	=> (Some(c.block), c.theme),
							None	=> (None, style),
						}
					};
					// The lookahead is seen through a single-block `#show par` scope, so a rule-wrapped paragraph
					// keeps with the heading just as a bare one does; `eff_theme` folds any such scope's patch so
					// the kept line is broken at the scoped size. A bare paragraph returns the theme unchanged, so
					// the render is byte-identical where no `par` rule wraps it.
					let kept = look.and_then(|block| keep_with_next_para(block, look_theme));
					if let Some((para, eff_theme)) = kept {
						// The first paragraph after a heading opens the section, so it takes no first-line indent.
						let mut lines = res!(break_paragraph(
							self.fonts.clone(), Role::Body, Dir::Ltr, eff_theme.text.body_size, para, self.measure, eff_theme.text.leading, eff_theme.text.hyphenate, eff_theme.text.fill,
							Some(cap_edge(&eff_theme, eff_theme.text.body_size))));
						if !lines.is_empty() {
							keep.push(lines.remove(0));			// the first line joins the heading
							rest = guard_widows(lines);			// its leading glue and the remaining lines follow
						}
						consumed_para = true;
						if i + 1 < blocks.len() {
							i += 2;
						} else {
							// The paragraph was the parent's continuation, in the parent's slice: leave it there
							// for the caller to skip, since this walk cannot advance past its own slice's end.
							consumed_cont = true;
							i += 1;
						}
					} else {
						i += 1;
					}

					self.nodes.push(vbox(keep, self.measure));
					self.nodes.extend(rest);
					self.first = false;
					// A heading opens a section: the paragraph it swallowed took no indent, but the NEXT paragraph
					// follows a paragraph and so is indented.
					self.prev_para = consumed_para;
				},
				Block::Paragraph { text } => {
					if !self.first {
						self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					}
					// A plain paragraph is set through the piece breaker so a leading indent box can ride the
					// front of its first line; without an indent it produces exactly what `break_paragraph` does.
					let mut pieces = Vec::new();
					if self.prev_para && style.par.indent.raw() > 0 {
						pieces.push(indent_piece(style.par.indent));
					}
					pieces.push(Piece::Text { text: text.clone(), role: Role::Body });
					let lines = res!(break_paragraph_pieces(
						self.fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &pieces, self.measure, style.text.leading, style.text.justify, style.text.hyphenate, style.text.fill,
						Some(cap_edge(style, style.text.body_size))));
					self.nodes.extend(guard_widows(lines));
					i += 1;
					self.first = false;
					self.prev_para = true;
				},
				Block::RichParagraph { segments } => {
					if !self.first {
						self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					}
					let mut pieces = Vec::new();
					if self.prev_para && style.par.indent.raw() > 0 {
						pieces.push(indent_piece(style.par.indent));
					}
					pieces.extend(res!(build_pieces(
						self.fonts.clone(), self.geom, style, segments, Role::Body, &mut self.foot_no, &mut self.ref_no, &mut self.margin_no, &mut self.seen, &mut self.index_gather, &mut self.claim_gather, self.bib, &self.refs)));
					let lines = res!(break_paragraph_pieces(
						self.fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &pieces, self.measure, style.text.leading, style.text.justify, style.text.hyphenate, style.text.fill,
						Some(cap_edge(style, style.text.body_size))));
					self.nodes.extend(guard_widows(lines));
					i += 1;
					self.first = false;
					self.prev_para = true;
				},
				Block::List { ordered, items } => {
					if !self.first {
						self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					}
					res!(list(&mut self.nodes, self.fonts.clone(), self.geom, style, self.measure, *ordered, items, &mut self.foot_no, &mut self.ref_no, &mut self.margin_no, &mut self.seen, &mut self.index_gather, &mut self.claim_gather, self.bib, &self.refs));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::Code { lines: src } => {
					if !self.first {
						self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					}
					res!(code_block(&mut self.nodes, self.fonts.clone(), style, src));
					self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::Table(t) => {
					// Space above the table, discarded at a page top like any other leading. A plain table lowers
					// to one keep box, moved whole to the next page when it will not fit; a breakable table (the
					// glossary) sets one keep box per row, so the driver paginates between its rows.
					if !self.first {
						self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
					}
					if t.breakable {
						self.nodes.extend(res!(table::lower_rows(
							self.fonts.clone(), self.geom, style, self.measure, t,
							&mut self.foot_no, &mut self.ref_no, &mut self.margin_no, &mut self.seen,
							&mut self.index_gather, &mut self.claim_gather, self.bib, &self.refs)));
					} else {
						self.nodes.push(res!(table::lower(
							self.fonts.clone(), self.geom, style, self.measure, t,
							&mut self.foot_no, &mut self.ref_no, &mut self.margin_no, &mut self.seen,
							&mut self.index_gather, &mut self.claim_gather, self.bib, &self.refs)));
					}
					self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::Equation { expr, numbered, .. } => {
					if !self.first {
						self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					}
					let number = if *numbered { self.eq_no += 1; Some(self.eq_no) } else { None };
					res!(equation(&mut self.nodes, self.fonts.clone(), style, self.measure, expr, number));
					self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::Figure { graphic, caption, placement } => {
					// Space above the figure, discarded at a page top like any other leading. The figure is
					// one keep box, so the breaker moves it whole to the next page when it will not fit; a
					// floated one leaves the flow entirely (see [`push_float`]).
					self.fig_no += 1;
					match placement {
						Some(p) => {
							let mut mid = Vec::new();
							res!(figure(&mut mid, self.fonts.clone(), style, self.measure, graphic.clone(), caption.as_deref(), self.fig_no));
							push_float(&mut self.nodes, mid, float_clearance(style), *p);
						},
						None => {
							if !self.first {
								self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
							}
							res!(figure(&mut self.nodes, self.fonts.clone(), style, self.measure, graphic.clone(), caption.as_deref(), self.fig_no));
							self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
						},
					}
					i += 1;
					self.first = false;
				},
				Block::TableFigure { table, caption, supplement, label, placement } => {
					let number = next_number(&mut self.counters, supplement);
					match placement {
						Some(p) => {
							let mut mid = Vec::new();
							res!(table_figure(
								&mut mid, self.fonts.clone(), self.geom, style, self.measure, table,
								caption.as_deref(), supplement, number, label.as_deref(),
								&mut self.foot_no, &mut self.ref_no, &mut self.margin_no, &mut self.seen,
								&mut self.index_gather, &mut self.claim_gather, self.bib, &self.refs));
							push_float(&mut self.nodes, mid, float_clearance(style), *p);
						},
						None => {
							if !self.first {
								self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
							}
							res!(table_figure(
								&mut self.nodes, self.fonts.clone(), self.geom, style, self.measure, table,
								caption.as_deref(), supplement, number, label.as_deref(),
								&mut self.foot_no, &mut self.ref_no, &mut self.margin_no, &mut self.seen,
								&mut self.index_gather, &mut self.claim_gather, self.bib, &self.refs));
							self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
						},
					}
					i += 1;
					self.first = false;
				},
				Block::ImageFigure { path, width, height, scale, caption, supplement, label, placement } => {
					let number = next_number(&mut self.counters, supplement);
					match placement {
						Some(p) => {
							let mut mid = Vec::new();
							res!(image_figure(
								&mut mid, self.fonts.clone(), style, self.measure, path, *width, *height, *scale,
								caption.as_deref(), supplement, number, label.as_deref()));
							push_float(&mut self.nodes, mid, float_clearance(style), *p);
						},
						None => {
							if !self.first {
								self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
							}
							res!(image_figure(
								&mut self.nodes, self.fonts.clone(), style, self.measure, path, *width, *height, *scale,
								caption.as_deref(), supplement, number, label.as_deref()));
							self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
						},
					}
					i += 1;
					self.first = false;
				},
				Block::CodeFigure { figure, caption, supplement, label, placement } => {
					let number = next_number(&mut self.counters, supplement);
					match placement {
						Some(p) => {
							let mut mid = Vec::new();
							res!(code_figure(
								&mut mid, self.fonts.clone(), style, self.measure, figure,
								caption.as_deref(), supplement, number, label.as_deref()));
							push_float(&mut self.nodes, mid, float_clearance(style), *p);
						},
						None => {
							if !self.first {
								self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
							}
							res!(code_figure(
								&mut self.nodes, self.fonts.clone(), style, self.measure, figure,
								caption.as_deref(), supplement, number, label.as_deref()));
							self.nodes.push(Node::Glue(Glue::fixed(style.table.skip)));
						},
					}
					i += 1;
					self.first = false;
				},
				Block::BackMatterHeading { title } => {
					if !self.first {
						self.nodes.push(Node::Penalty(Penalty::eject()));
					}
					// The back-matter marker (a Citation anchor) fixes where the running head drops and the
					// folio centres; a heading anchor lists it in the contents. Both sit at the page top.
					self.nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Citation, slug(title))));
					let id = AnchorId::new(AnchorKind::Heading, fmt!("{:02}-{}", self.heads.len() + 1, slug(title)));
					self.heads.push(Heading {
					id:			id.clone(),
					level:		0,
					title:		title.clone(),
					segments:	vec![Segment::text(title.clone())],
					number:		String::new(),
					banner:		false,
				});
					self.nodes.push(Node::Anchor(id));
					// The title left in the display face at the chapter-title size (the template's
					// glossary-index-title size, equal to it in these books' scales).
					let sh	= res!(head_shape(&self.fonts, &resolved_head_face(1, style, self.faces, is_doc_heading(style)), style.heading.levels[0].size, title));
					let d	= sh.dims();
					self.nodes.push(Node::HBox(BoxNode::new(
						vec![Node::Leaf(Leaf::text(sh))], Dims::new(self.measure, d.height, d.depth))));
					self.nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(20.0))));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::Reference { runs } => {
					// Typst joins bibliography entries with a paragraph break, so they part by the bibliography's
					// own paragraph spacing, not by a footnote interline gap. The book template sets the reference
					// list at `text(size: 0.85em)`, and paragraph spacing scales with the text size, so the gap is
					// the body paragraph skip (`par.skip`, 1.2 em at the body size) scaled by the same 0.85 -- the
					// earlier fix sized the entry text and leading this way but left this gap on the footnote metric,
					// which set the entries far too tight.
					if !self.first {
						let gap = Sp(style.par.skip.raw() * 85 / 100);
						self.nodes.push(Node::Glue(Glue::fixed(gap)));
					}
					res!(reference_block(&mut self.nodes, self.fonts.clone(), style, self.measure, runs));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::Rule { width, thickness, grey } => {
					if !self.first {
						self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					}
					rule_divider(&mut self.nodes, self.measure, *width, *thickness, *grey);
					self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::Image { path, width, height, scale } => {
					res!(plain_image(&mut self.nodes, self.fonts.clone(), self.measure, path, *width, *height, *scale));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::SectionBanner { path } => {
					// The template's `#section-banner` turns the page first (`pagebreak(weak: true)`); a forced
					// eject the driver drops when the page is already fresh, so it never opens a blank one.
					self.nodes.push(Node::Penalty(Penalty::eject()));
					res!(section_banner(&mut self.nodes, self.fonts.clone(), self.geom, self.measure, path));
					self.pending_banner = true;	// the section's level-1 heading follows and opens beneath this banner
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::Box { blocks: inner, patch, placement } => {
					// Space above the callout, discarded at a page top like any other leading. It lowers to one keep
					// box, so the breaker moves it whole to the next page when it will not fit; a floated callout
					// (an `#aside-box(float: true)`) leaves the flow and defers (see [`push_float`]).
					// The box body is set with the document theme overlaid by the box's own `#set` declarations,
					// scoped to the box (H3). An empty patch leaves the document theme, so a callout that declares
					// nothing renders byte-identically. The wash is the scoped theme's `callout.fill`.
					let scoped = { let mut t = style.clone(); t.apply(patch); t };
					let fill = scoped.callout.fill;
					match placement {
						Some(p) => {
							// A floated callout is a `figure(placement: ...)` under the bonnet, so it records a
							// zero-extent [`Float`](crate::ledger::AnchorKind::Float) anchor -- keyed by a running
							// aside count -- as Typst counts it among its figures. The anchor rides inside the float,
							// so it takes the page and position the callout settles on.
							let mut mid = Vec::new();
							let n = next_number(&mut self.counters, "aside");
							mid.push(Node::Anchor(AnchorId::new(AnchorKind::Float, fmt!("aside-{}", n))));
							res!(styled_box(
								&mut mid, self.fonts.clone(), self.geom, &scoped, self.measure, inner, fill,
								&mut self.foot_no, &mut self.ref_no, &mut self.margin_no, &mut self.seen, &mut self.index_gather, &mut self.claim_gather, self.bib, &self.refs));
							push_float(&mut self.nodes, mid, float_clearance(style), *p);
						},
						None => {
							if !self.first {
								self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
							}
							res!(styled_box(
								&mut self.nodes, self.fonts.clone(), self.geom, &scoped, self.measure, inner, fill,
								&mut self.foot_no, &mut self.ref_no, &mut self.margin_no, &mut self.seen, &mut self.index_gather, &mut self.claim_gather, self.bib, &self.refs));
							self.nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
						},
					}
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				// The book layer resolves every `#print-glossary()` placeholder into a table before layout, so one
				// reaching here (a lone-file compile that never ran the resolver) sets nothing rather than failing.
				Block::Glossary => { i += 1; },
				// The back-matter index placeholder: it sets nothing here, only marks that the index is wanted,
				// so `author` builds the entry list from the markers gathered walking the body once the walk
				// ends. The heading above it (a `Block::BackMatterHeading`) opens the section and lists it.
				Block::Index => { self.want_index = true; i += 1; },
				// The reverse claim-reference index placeholder: it sets nothing here, but unlike `Block::Index`
				// the listing is set IN FLOW at this source position (the `#context { ... collect-claim-refs() }`
				// block sits in the Logic appendix, before the bibliography), not appended as back matter. So the
				// body-node position is recorded now and the listing spliced in once the walk has gathered every
				// reference, so its heading and intro are never stranded on a page ahead of a headless listing.
				Block::ClaimIndex => { self.want_claim_index = true; self.claim_index_at = Some(self.nodes.len()); i += 1; },
				// A scope is handled by the recursion at the loop top; named here only for exhaustiveness.
				Block::Scoped { .. } => { i += 1; },
				Block::Space(sp) => {
					// A template's `v(<len>)`, set as a fixed leading between its siblings. Not discarded at a
					// page top: the author asked for it, so it holds like any authored space.
					self.nodes.push(Node::Glue(Glue::fixed(*sp)));
					i += 1;
					self.first = false;
					self.prev_para = false;
				},
				Block::PageBreak => {
					// A line-leading `#pagebreak()`: a forced eject, exactly as the section banner turns the
					// page. The driver drops the break when the frame is already empty, so one landing at a page
					// top opens no blank page. `first`/`prev_para` are left untouched -- the break sets no ink,
					// so the block that follows leads against the page top, not against a paragraph.
					self.nodes.push(Node::Penalty(Penalty::eject()));
					i += 1;
				},
			}
		}
		// Capture the last authored block's delta, which has no next iteration to close it.
		if let Some(p) = pending.take() {
			self.memo_capture(memo, p, blocks.len());
		}
		Ok(consumed_cont)
	}

	/// Snapshots the scalar authoring counters as they stand: the state a block enters under (folded into
	/// its memo key) or leaves (stored in its memo value and restored on a hit).
	fn block_state(&self) -> BlockState {
		BlockState {
			first:			self.first,
			prev_para:		self.prev_para,
			pending_banner:	self.pending_banner,
			sec:			self.sec,
			part_no:		self.part_no,
			foot_no:		self.foot_no,
			ref_no:			self.ref_no,
			margin_no:		self.margin_no,
			eq_no:			self.eq_no,
			fig_no:			self.fig_no,
			heads_len:		self.heads.len() as u32,
			index_no:		self.index_gather.no,
		}
	}

	/// Restores the scalar counters to a cached block's exit state on a hit. The heading and index/claim
	/// occurrence vectors are extended separately, so `heads.len()` and the index counter already stand
	/// where the exit state records them; the rest are set here.
	fn restore_state(&mut self, s: &BlockState) {
		self.first			= s.first;
		self.prev_para		= s.prev_para;
		self.pending_banner	= s.pending_banner;
		self.sec			= s.sec;
		self.part_no		= s.part_no;
		self.foot_no		= s.foot_no;
		self.ref_no			= s.ref_no;
		self.margin_no		= s.margin_no;
		self.eq_no			= s.eq_no;
		self.fig_no			= s.fig_no;
		self.index_gather.no	= s.index_no;
	}

	/// An order-independent fingerprint of the glossary first-use set: a block that sets a term bold-italic
	/// on its first use and plain after depends on exactly which terms have already been seen, so the set is
	/// in every key. The fold is by XOR of each term's hash, which needs no sort and updates in step with a
	/// growing set without ever caring about insertion order.
	fn seen_hash(&self) -> u64 {
		let mut acc = 0u64;
		for term in &self.seen {
			let mut h = Fnv::new();
			h.write_str(term);
			acc ^= h.finish();
		}
		acc
	}

	/// An order-independent fingerprint of the per-supplement counters (Figure, Table, aside), each folded
	/// with its current value, so a block that stamps the next figure or table number keys on the number it
	/// will actually stamp.
	fn counters_hash(&self) -> u64 {
		let mut acc = 0u64;
		for (k, v) in &self.counters {
			let mut h = Fnv::new();
			h.write_str(k);
			h.write_u32(*v);
			acc ^= h.finish();
		}
		acc
	}

	/// The memo key of a block at the current position: the compile-wide fingerprint, the measure, the
	/// block's own content (and, for a heading, the following block it may keep the first line of), and the
	/// counter state it enters under. Two compiles that reach a block with the same content and the same
	/// entering state produce byte-identical nodes, so they must share a key; an edit that shifts any of
	/// those must not.
	fn block_key(&self, block: &Block, look: Option<&Block>) -> u64 {
		let mut h = Fnv::new();
		h.write(b"block");
		h.write_u64(self.global_fp);
		h.write_i32(self.measure.raw());
		// The block's full content. The derived `Debug` is a faithful, total structural rendering -- it can
		// never silently drop a field the way a hand-written walker can -- and no type reachable from a
		// `Block` carries a lossy `Debug` (the one that summarises, `ShapedText`, appears only after
		// authoring, in `Node`). Formatting a block's `Debug` costs a fraction of shaping and breaking it.
		h.write_str(&fmt!("{:?}", block));
		if let Some(la) = look {
			h.write_str(&fmt!("{:?}", la));
		}
		self.block_state().hash_into(&mut h);
		h.write_u64(self.seen_hash());
		h.write_u64(self.counters_hash());
		h.finish()
	}

	/// Applies a cached block result on a hit: splices its authored nodes, heading records and index/claim
	/// occurrences back in, advances the glossary and supplement accumulators by the deltas the block made,
	/// and restores the exit counter state -- so the authoring state stands exactly where a fresh authoring
	/// of the same block would have left it.
	fn apply_block_entry(&mut self, entry: BlockEntry) {
		self.nodes.extend(entry.nodes);
		self.heads.extend(entry.heads);
		self.index_gather.occ.extend(entry.index_occ);
		self.claim_gather.occ.extend(entry.claim_occ);
		for term in entry.seen_add {
			self.seen.insert(term);
		}
		for (k, v) in entry.counters_set {
			self.counters.insert(k, v);
		}
		self.restore_state(&entry.exit);
	}

	/// Captures the delta a just-authored block produced and stores it under its key: the nodes, heads and
	/// occurrences it appended, the source blocks it consumed, the glossary terms it newly marked seen, the
	/// supplement counters it changed, and its exit counter state. `i_end` is the position after the block,
	/// so the consumed count carries a chapter heading's swallowed paragraph.
	fn memo_capture(&mut self, memo: &mut Option<&mut Memo>, p: PendingBlock, i_end: usize) {
		let m = match memo.as_deref_mut() {
			Some(m)	=> m,
			None	=> return,
		};
		let seen_add: Vec<String> = self.seen.iter()
			.filter(|t| !p.seen_before.contains(*t))
			.cloned()
			.collect();
		let counters_set: Vec<(String, u32)> = self.counters.iter()
			.filter(|(k, v)| p.counters_before.get(*k) != Some(*v))
			.map(|(k, v)| (k.clone(), *v))
			.collect();
		let entry = BlockEntry::new(
			i_end - p.i_before,
			self.nodes[p.nodes_before..].to_vec(),
			self.heads[p.heads_before..].to_vec(),
			self.index_gather.occ[p.index_before..].to_vec(),
			self.claim_gather.occ[p.claim_before..].to_vec(),
			seen_add,
			counters_set,
			self.block_state(),
		);
		m.block_store(p.key, entry);
	}
}

/// Is a block one the authoring memo caches? A scope recurses (and is authored fresh), and the index and
/// claim-index placeholders set nothing but a document-position or a flag the post-walk assembly reads, so
/// neither is cached; every other block is a self-contained authoring unit keyed on its content and state.
fn memoisable(block: &Block) -> bool {
	!matches!(block, Block::Scoped { .. } | Block::Index | Block::ClaimIndex | Block::Glossary)
}

/// The markers of a block being authored on a cache miss: its key, where it starts in each accumulator,
/// and the glossary and supplement state before it ran, so the delta it makes can be captured once it has.
struct PendingBlock {
	key:				u64,
	i_before:			usize,
	nodes_before:		usize,
	heads_before:		usize,
	index_before:		usize,
	claim_before:		usize,
	seen_before:		HashSet<String>,
	counters_before:	HashMap<String, u32>,
}

/// Turns an authored block list into the composed document, and the heading table the running heads
/// resolve against. The geometry fixes the measure every paragraph is set to.
///
/// When `front` is set the front matter -- cover, title page, imprint, dedication and author note --
/// is composed ahead of the body, so the body's first heading fixes where the printed folio restarts
/// at one; a lone manuscript passes `None` and carries no front matter.
pub fn author(
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style: &Theme,
	faces:		&FaceResolver,
	blocks:		&[Block],
	front:		Option<&FrontMatter>,
	bib:		Option<&Bibliography>,
)
	-> Outcome<(Document, Vec<Heading>)>
{
	author_memo(fonts, geom, style, faces, blocks, front, bib, None)
}

/// [`author`] with the incremental block-authoring memo threaded through: a live edit-and-re-render loop
/// hands the same [`Memo`] each compile so an unchanged block splices its previously authored nodes back
/// rather than re-shaping and re-breaking them. Passing `None` is exactly [`author`], byte for byte -- the
/// memo path never runs -- which is why every other caller keeps calling `author`.
///
/// The memo's fingerprint scopes it to one (fonts, faces, geometry, theme, cross-reference) configuration;
/// the caller opens each compile with [`Memo::begin`] carrying that fingerprint (see [`memo_fingerprint`]),
/// so a change to any of those clears the stale cache rather than serving it.
#[allow(clippy::too_many_arguments)]
pub fn author_memo(
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style: &Theme,
	faces:		&FaceResolver,
	blocks:		&[Block],
	front:		Option<&FrontMatter>,
	bib:		Option<&Bibliography>,
	mut memo:	Option<&mut Memo>,
)
	-> Outcome<(Document, Vec<Heading>)>
{
	// The text every labelled cross-reference resolves to, settled once from document order so a forward
	// reference reads its referent's supplement and number without a layout round-trip.
	let refs = ref_targets(blocks, style);
	// The compile-wide fingerprint every block key folds in, so a change to the theme, the geometry, the
	// cross-reference targets (a renumbered chapter a `@ref` points at) or the bibliography (the text a
	// `#cite` resolves to) invalidates the whole cache.
	let global_fp = memo_fingerprint(style, geom, &refs, bib);
	if let Some(m) = memo.as_deref_mut() {
		m.begin(global_fp);
	}
	let mut authoring = Authoring {
		fonts:		fonts.clone(),
		geom,
		faces,
		measure:	geom.content_width(),
		bib,
		refs,
		nodes:		Vec::new(),
		heads:		Vec::new(),
		first:		true,
		sec:		[0; 6],
		prev_para:	false,
		pending_banner:	false,
		part_no:	0,
		foot_no:	0,
		ref_no:		0,
		margin_no:	0,
		eq_no:		0,
		fig_no:		0,
		counters:	HashMap::new(),
		seen:		HashSet::new(),
		index_gather:	IndexGather::default(),
		want_index:		false,
		claim_gather:	ClaimGather::default(),
		want_claim_index:	false,
		claim_index_at:	None,
		global_fp,
	};
	// The top level has no parent continuation; the returned "consumed" flag is meaningless here and dropped.
	res!(authoring.walk(blocks, style, None, &mut memo));
	// The back-matter index, built from the markers gathered walking the body once the `Block::Index`
	// placeholder has been met and every occurrence's anchor is woven in. Its entries read their pages back
	// from the ledger after convergence, so they are set after the body they point into, as back matter.
	if authoring.want_index && !authoring.index_gather.occ.is_empty() {
		let occ			= std::mem::take(&mut authoring.index_gather.occ);
		// The back-matter index sets in two columns, as Typst's `print-index` wraps `make-index` in
		// `columns(2)`. The gutter is 4% of the page measure; each entry is set to the resulting column
		// width, so a folio list fills its own column rather than the whole page. The driver's multi-column
		// pass flows the entries down the first column, then the second, breaking to a fresh page as needed.
		let gutter		= Sp(geom.content_width().raw() * 4 / 100);
		let col_measure	= geom.column_slice(0, 2, gutter).content_width();
		let entries		= res!(index_nodes(&fonts, style, col_measure, &occ));
		authoring.nodes.push(Node::Columns(ColumnsNode::new(entries, 2, gutter)));
	}
	// The reverse claim-reference index, built from the references gathered walking the body once the
	// `Block::ClaimIndex` placeholder has been met. It sets in a single column (Typst's appendix wraps it in
	// no `columns`), one wrapped paragraph per code, each code's pages read back from the ledger as forward
	// references. Unlike the back-matter index it is spliced in flow at the placeholder's own source position
	// -- the `#context` block sits in the Logic appendix before the bibliography, so the §heading and intro
	// above it must be followed immediately by the listing, not by a headless run of entries pages later.
	if let Some(at) = authoring.claim_index_at {
		if !authoring.claim_gather.occ.is_empty() {
			let occ			= std::mem::take(&mut authoring.claim_gather.occ);
			let entries		= res!(claim_index_nodes(&fonts, style, authoring.measure, &occ));
			// The parbreak gap Typst leaves between the appendix intro paragraph and the first listing entry,
			// the same block skip an authored paragraph takes above it.
			let mut spliced: Vec<Node> = Vec::with_capacity(entries.len() + 1);
			spliced.push(Node::Glue(Glue::fixed(style.par.skip)));
			spliced.extend(entries);
			let at = at.min(authoring.nodes.len());
			authoring.nodes.splice(at..at, spliced);
		}
	}
	let heads = authoring.heads;

	// The front matter is composed ahead of the body so its cover, title, imprint and note leaves take
	// the physical pages before the body opens; the body then carries no heading anchor of the front
	// matter's, so the driver fixes the folio restart at the first body heading.
	let mut stream: Vec<Node> = Vec::new();
	if let Some(fm) = front {
		res!(front_matter(&mut stream, &fonts, faces, geom, style, fm));
		// The contents follows the front matter and precedes the body, resolving each entry's folio as a
		// forward reference into the body the driver has not composed yet.
		stream.extend(res!(contents(
			fonts.clone(), faces, geom, style, fm.back_title_size, &heads)));
	}
	stream.extend(authoring.nodes);

	let mut document = Document::new(stream, geom);
	document.foot = foot_style(style);
	// The two-generation sweep is deferred to the caller, after the emit stage: the page-emit cache is
	// touched during emit, which runs after this returns, so sweeping here would drop last compile's page
	// entries before this compile's emit could reuse them. `Memo::begin` (called above) opened the
	// generation; `Memo::sweep`, called once the pages are emitted, closes it.
	let _ = memo;
	Ok((document, heads))
}

/// The compile-wide fingerprint the block-authoring memo scopes every key to: the theme, the page
/// geometry, the settled cross-reference targets and the bibliography. A change to any of them changes
/// what every block authors -- the theme decides sizes and faces; the geometry decides the measure; a
/// `@ref`'s resolved text changes when the thing it points at is renumbered; a `#cite` resolves against
/// the bibliography -- so it must invalidate the cache wholesale. The document's fonts and faces are held
/// constant by the memo's per-document contract: a font or face change takes a fresh [`Memo`], not this one.
pub fn memo_fingerprint(
	style:	&Theme,
	geom:	PageGeometry,
	refs:	&HashMap<String, String>,
	bib:	Option<&Bibliography>,
)
	-> u64
{
	let mut h = Fnv::new();
	h.write(b"global");
	// The theme as data. Its derived `Debug` renders every styled value in a canonical field order, so
	// two identical themes fingerprint alike and any change to one shows.
	h.write_str(&fmt!("{:?}", style));
	h.write_i32(geom.width.raw());
	h.write_i32(geom.height.raw());
	h.write_i32(geom.inside.raw());
	h.write_i32(geom.outside.raw());
	h.write_i32(geom.top.raw());
	h.write_i32(geom.bottom.raw());
	// The cross-reference targets, folded order-independently: a label maps to its resolved "Chapter 4"
	// text, and that text changing (a renumber) must miss every block that sets a reference.
	let mut rf = 0u64;
	for (k, v) in refs {
		let mut e = Fnv::new();
		e.write_str(k);
		e.write_str(v);
		rf ^= e.finish();
	}
	h.write_u64(rf);
	// The bibliography as data, so editing a `.bib` (which changes the text a `#cite` resolves to without
	// touching any block's own content) misses every citation-bearing block rather than serving it stale.
	h.write_bool(bib.is_some());
	if let Some(b) = bib {
		h.write_str(&fmt!("{:?}", b));
	}
	h.finish()
}

/// The foot spacing derived from the block style, so the separator rule and the gaps around the notes
/// match the document's other furniture. The rule runs a third of the measure, a conventional short
/// footnote rule.
fn foot_style(style: &Theme) -> FootStyle {
	FootStyle {
		gap_above_rule:	style.par.skip,
		rule_thick:		style.table.rule_thin,
		rule_width:		Sp(style.text.body_size.raw() * 12),
		gap_below_rule:	Sp::from_pt(4.0),
		gap_between:	Sp::from_pt(3.0),
	}
}

/// A first-line indent as a rigid leading piece: an empty box of the indent width that the optimiser
/// counts against the first line and that never breaks, so the first word sits one indent in and the
/// line still fills the measure. Modelled as a maths piece of zero height carrying a single fixed glue,
/// which is how the piece breaker already threads a pre-built inline cluster into the line.
fn indent_piece(indent: Sp) -> Piece {
	Piece::Math {
		nodes:	vec![Node::Glue(Glue::fixed(indent))],
		width:	indent,
		height:	Sp::ZERO,
		depth:	Sp::ZERO,
		over:	Sp::ZERO,
	}
}

/// The block top edge for prose set at `size`: the cap height, as the fraction of the em the theme
/// calibrates ([`ThemeCalibration::line_box_em`](crate::theme::ThemeCalibration)). Passed to the
/// paragraph breakers as the block-edge model's top, so a paragraph's first line seats its top at the cap
/// height and its inter-block glue attaches where Typst's does.
pub(crate) fn cap_edge(style: &Theme, size: Sp) -> Sp {
	Sp((size.raw() as f64 * style.calibration.line_box_em).round() as i32)
}

/// Guards a flow paragraph's set lines against a widow or an orphan across a page break. The page breaker
/// may break at any interline glue; a forbidden penalty set immediately after the first line and
/// immediately before the last line stops it stranding a single line either side of a break -- an orphan at
/// the page foot or a widow at the page head. The lines then move as a unit rather than splitting one off,
/// which is the page-bottom slack Typst leaves. A paragraph of one or two lines becomes wholly unbreakable;
/// three or more keep at least two lines on each side of any break they do take. `lines` is the breaker's
/// output -- HBoxes joined by interline glue -- and the return is the same list with the two penalties woven
/// in; a paragraph of a single line is returned unchanged.
fn guard_widows(lines: Vec<Node>) -> Vec<Node> {
	let n = lines.iter().filter(|node| matches!(node, Node::HBox(_))).count();
	if n < 2 {
		return lines;
	}
	let forbid		= Node::Penalty(Penalty::new(Penalty::INFINITY, false));
	let mut out		= Vec::with_capacity(lines.len() + 2);
	let mut seen	= 0usize;	// HBoxes emitted so far
	for node in lines {
		let is_line = matches!(node, Node::HBox(_));
		out.push(node);
		if is_line {
			seen += 1;
			// After the first line: forbid the break that would orphan it at a page foot. After the
			// last-but-one line: forbid the break that would widow the last line at a page head. The two
			// coincide for a two-line paragraph, forbidding its only interior break.
			if seen == 1 || seen == n - 1 {
				out.push(forbid.clone());
			}
		}
	}
	out
}

/// The text each labelled cross-reference resolves to, fixed in a document-order pre-pass. A reference's
/// supplement word and number depend only on document order, not on layout, so they are settled once here
/// and set as static text -- Typst's own "Chapter 4" for a chapter, "Section 7.7" for a section, and
/// "{supplement} {number}" for a figure or table -- matching the oracle's own output. The heading and
/// figure and equation counters are stepped exactly as [`author`] steps them, so a label's number here is
/// the number the block itself sets -- a chapter, section, figure, table or "Equation N". A label the
/// pre-pass never records is left for the caller's page-number fallback.
fn ref_targets(blocks: &[Block], style: &Theme) -> HashMap<String, String> {
	let mut out:		HashMap<String, String>	= HashMap::new();
	let mut sec:		[u32; 6]				= [0; 6];
	let mut counters:	HashMap<String, u32>	= HashMap::new();
	let mut eq_no		= 0u32;	// the equation counter, stepped exactly as `author` steps it
	ref_targets_walk(blocks, style, &mut out, &mut sec, &mut counters, &mut eq_no);
	out
}

/// The document-order counting walk behind [`ref_targets`], recursing into a [`Block::Scoped`] under its
/// overlaid theme with the counters shared across the boundary -- exactly as [`Authoring::walk`] numbers
/// the same tree, so a heading, figure or equation inside a scope takes the number it will actually be set
/// with, and a cross-reference after or into a scope resolves to the right one. The heading number is read
/// through [`heading_number_themed`], honouring any per-level numbering pattern the scope's theme carries,
/// so a numbered pattern matches the rendered heading rather than the plain dotted arabic.
fn ref_targets_walk(
	blocks:		&[Block],
	style: &Theme,
	out:		&mut HashMap<String, String>,
	sec:		&mut [u32; 6],
	counters:	&mut HashMap<String, u32>,
	eq_no:		&mut u32,
) {
	for block in blocks {
		match block {
			Block::Scoped { patch, blocks: inner } => {
				let scoped = { let mut t = style.clone(); t.apply(patch); t };
				ref_targets_walk(inner, &scoped, out, sec, counters, eq_no);
			},
			Block::Heading { level, label, .. } => {
				if *level >= 1 {
					let l = (*level as usize).min(6);
					sec[l - 1] += 1;
					for k in l..6 { sec[k] = 0; }
				}
				if let Some(l) = label {
					let number = heading_number_themed(*level, sec, style);
					// A chapter (level 1) takes the "Chapter" supplement the template sets; a deeper heading
					// takes "Section" with its full dotted number, as Typst's default heading reference does. A
					// part divider (level 0) carries no number and is no reference target.
					let text = match *level {
						0			=> continue,
						1			=> fmt!("Chapter {}", number),
						_			=> fmt!("Section {}", number),
					};
					out.insert(l.clone(), text);
				}
			},
			Block::TableFigure { supplement, label, .. }
			| Block::ImageFigure { supplement, label, .. }
			| Block::CodeFigure { supplement, label, .. } => {
				let n = next_number(counters, supplement);
				if let Some(l) = label {
					out.insert(l.clone(), fmt!("{} {}", supplement, n));
				}
			},
			Block::Equation { numbered, label, .. } => {
				// A numbered equation steps the counter; a labelled one anchors "Equation N", Typst's
				// default equation reference. An unnumbered equation carries no number, so its label is
				// left to the caller's page-number fallback.
				if *numbered {
					*eq_no += 1;
					if let Some(l) = label {
						out.insert(l.clone(), fmt!("Equation {}", *eq_no));
					}
				}
			},
			// A `#styled-box` body sets running prose only -- `Authoring::walk` numbers no heading, figure or
			// equation inside it -- so a label there is no numbered target and the box is not descended, exactly
			// as the render leaves it.
			_ => {},
		}
	}
}

/// Turns a rich paragraph's segments into the pieces the line breaker weaves, assigning each footnote
/// its number from the running fold and setting its note as a small paragraph at the foot measure, and
/// each cross-reference a reserved inline slot the driver resolves in pass B. A text segment is a piece
/// as it stands; a footnote becomes a superscript mark piece carrying the set note; a page reference or
/// a total-pages call becomes a shrink-to-fit reserved leaf, unique by the running `ref_no`. `base` is the
/// face plain text and a resolved reference take: `Role::Body` in the running flow, a header row's `Role::Bold`
/// when a table cell is built through this same path, so a cell renders, cites, gathers its index markers and
/// records its claim anchors exactly as a body paragraph does.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_pieces(
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style: &Theme,
	segments:	&[Segment],
	base:		Role,
	foot_no:	&mut u32,
	ref_no:		&mut u32,
	margin_no:	&mut u32,
	seen:		&mut HashSet<String>,
	idx:		&mut IndexGather,
	claim:		&mut ClaimGather,
	bib:		Option<&Bibliography>,
	refs:		&HashMap<String, String>,
)
	-> Outcome<Vec<Piece>>
{
	let measure			= geom.content_width();
	let mut pieces		= Vec::with_capacity(segments.len());
	for seg in segments {
		match seg {
			Segment::Text(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: base });
			},
			Segment::Strong(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: Role::Bold });
			},
			Segment::Emph(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: crate::table::emph_role(base) });
			},
			Segment::BoldItalic(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: Role::BoldItalic });
			},
			Segment::Super(text) => {
				// The same raise the footnote mark rides: a run shaped at 0.7x, its box shortened so the
				// emitter seats its baseline above the line's. It is rigid and never breaks -- the space
				// after it may -- exactly as a mark piece behaves.
				let (shaped, dims)	= res!(superscript(fonts.clone(), Role::Body, style.text.body_size, text));
				pieces.push(Piece::Mark(Leaf::text_dims(shaped, dims)));
			},
			Segment::Sub(text) => {
				// The mirror of the superscript arm just above: a run shaped at 0.7x, its box lengthened so
				// the emitter seats its baseline below the line's.
				let (shaped, dims)	= res!(subscript(fonts.clone(), Role::Body, style.text.body_size, text));
				pieces.push(Piece::Mark(Leaf::text_dims(shaped, dims)));
			},
			Segment::Footnote { note } => {
				*foot_no += 1;
				let label			= fmt!("{}", *foot_no);
				let (mark, dims)	= res!(superscript(fonts.clone(), Role::Body, style.text.body_size, &label));
				let footnote		= res!(build_footnote(fonts.clone(), style, measure, *foot_no, note, mark));
				pieces.push(Piece::Mark(Leaf::mark(footnote, dims)));
			},
			Segment::Math(expr) => {
				// The inline box is flattened to leaves and glue by the maths layout; unwrap the HBox it
				// returns and weave its children into the line, so they draw as real glyphs rather than as
				// a nested rectangle. The box seats its baseline on the text baseline -- a body ascent
				// below the line top -- so the line asks for that ascent as its height; anything the maths
				// reaches above it is the overshoot the line above must open for.
				let node = res!(math::layout(fonts.clone(), style, expr, false));
				if let Node::HBox(b) = node {
					let ascent	= res!(ShapedText::new(
						fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, "0")).dims().height;
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
				// A cross-reference resolves to Typst's own supplement-and-number text -- "Chapter 4",
				// "Section 7.7", "Figure 2", "Table 1", "Equation 9" -- fixed by the document-order pre-pass
				// and set as body text. A label the pre-pass did not record falls back to the reserved
				// page-number slot the driver resolves in pass B, so the reference still reads rather than
				// vanishing.
				match refs.get(label) {
					Some(text)	=> pieces.push(Piece::Text { text: text.clone(), role: base }),
					None		=> pieces.push(Piece::Mark(res!(ref_slot(
						fonts.clone(), style, ref_no,
						Ref::PageOf(AnchorId::new(AnchorKind::Label, label.clone())))))),
				}
			},
			Segment::Code(text) => {
				pieces.push(Piece::Text { text: text.clone(), role: Role::Mono });
			},
			Segment::Glossary { term, display } => {
				// The first mention of a term is set bold-italic, matching the template's `*_term_*`;
				// every later mention is plain body text. Document order is the traversal order, so the
				// set alone decides, with no second pass.
				let role = if seen.insert(term.clone()) { Role::BoldItalic } else { base };
				pieces.push(Piece::Text { text: display.clone(), role });
			},
			Segment::Cite(keys) => {
				// Resolve the citation to "(Author Year)" against the bibliography, set as body text. A
				// key the bibliography does not hold, or a run with no bibliography loaded, falls back to
				// the bracketed keys so the citation still reads rather than vanishing.
				let text = match bib {
					Some(b) => {
						let refs: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();
						b.format_citation(&refs).unwrap_or_else(|_| fmt!("({})", keys.join("; ")))
					},
					None => fmt!("({})", keys.join("; ")),
				};
				pieces.push(Piece::Text { text, role: base });
			},
			Segment::MarginNote { display, codes } => {
				// A margin note sets nothing in the body column: it weaves a zero-width anchor into the line
				// at this point, recording where it landed so `decorate` draws the compressed code in the
				// outside margin after convergence. The identity carries a document-order ordinal, so two
				// identical codes stay distinct in the ledger, and the display text itself, which `decorate`
				// reads back from the key -- no side table has to be threaded out of the layout. A metadata-only
				// `#claim-refs` carries an empty display, so `decorate` draws nothing for it, but the anchor is
				// still recorded so the reverse claim index can read the page each of its codes was referenced on.
				*margin_no += 1;
				let key = fmt!("{}\u{1f}{}", *margin_no, display);
				let id	= AnchorId::new(AnchorKind::MarginNote, key);
				// Each referenced code is remembered against this anchor, so the reverse claim index groups the
				// references by code and reads their pages back from the ledger, exactly as the index does its folios.
				for code in codes {
					claim.occ.push((code.clone(), id.clone()));
				}
				pieces.push(Piece::Anchor(id));
			},
			Segment::Index { term, sub, display, main } => {
				// An index marker sets nothing in the body column: it weaves a zero-width anchor into the line
				// at this point, so the driver records the folio it lands on, and remembers the occurrence so
				// the back-matter index lists the term with the page. The ordinal makes each occurrence's
				// identity unique, so two mentions of one term on one page stay distinct until the entry
				// deduplicates their resolved folios. The styled display travels with the occurrence so the
				// index page sets the display words, not the sort key; `main` travels too, so a primary
				// reference's folio sets bold on the index page.
				idx.no += 1;
				let key = fmt!("{}\u{1f}{}\u{1f}{}", idx.no, term, sub.as_deref().unwrap_or(""));
				let id	= AnchorId::new(AnchorKind::IndexEntry, key);
				idx.occ.push((term.clone(), sub.clone(), display.clone(), id.clone(), *main));
				pieces.push(Piece::Anchor(id));
			},
		}
	}
	Ok(pieces)
}

/// The compressed code a margin note's anchor key carries, recovered for [`decorate`] to draw: the key is
/// `<ordinal>\u{1f}<display>`, the ordinal making the identity unique and the display the words. An
/// unexpected key with no separator yields the empty string, so a stray anchor draws nothing rather than
/// its own bookkeeping.
fn margin_display(key: &str) -> &str {
	match key.split_once('\u{1f}') {
		Some((_, display))	=> display,
		None				=> "",
	}
}

/// Builds one inline cross-reference: a reserved leaf, unique by the running `ref_no`, that reserves a
/// three-digit slot and shrinks to the value the driver resolves for `refr` in pass B. It seats on the
/// body baseline, taking a body digit's height and depth so it aligns with the prose around it.
fn ref_slot(
	fonts:	Arc<FontSet>,
	style: &Theme,
	ref_no:	&mut u32,
	refr:	Ref,
)
	-> Outcome<Leaf>
{
	*ref_no += 1;
	let own		= AnchorId::new(AnchorKind::Label, fmt!("ref-{}", *ref_no));
	let slot	= res!(ShapedText::new(fonts, Role::Body, Dir::Ltr, style.text.body_size, "000"));
	let sd		= slot.dims();
	Ok(Leaf::reserved_inline(own, refr, Dims::new(sd.width, sd.height, sd.depth)))
}

/// Sets a bullet or numbered list into the vertical list. Each item is broken at a measure reduced by
/// the marker column and then hung under its marker: the first line carries the marker leaf and a gap
/// that together fill the indent, the rest are shifted right by it, so every line's right edge still
/// lands on the measure. The marker column is the widest marker the list uses plus
/// [`marker_gap`](crate::theme::ThemeList::marker_gap), so a bullet list and a numbered list of ten
/// items align their text alike. Items are parted by [`item_skip`](crate::theme::ThemeList::item_skip);
/// the list's space from its neighbours is the
/// caller's. Each item is a segment run, so it breaks through the same path a rich paragraph does and
/// may carry emphasis, a footnote or inline maths.
#[allow(clippy::too_many_arguments)]
fn list(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style: &Theme,
	measure:	Sp,
	ordered:	bool,
	items:		&[ListEntry],
	foot_no:	&mut u32,
	ref_no:		&mut u32,
	margin_no:	&mut u32,
	seen:		&mut HashSet<String>,
	idx:		&mut IndexGather,
	claim:		&mut ClaimGather,
	bib:		Option<&Bibliography>,
	refs:		&HashMap<String, String>,
)
	-> Outcome<()>
{
	// An ordered list takes its metrics and marker pattern from the `enumeration` group, a bulleted list
	// from `list`. The two groups' defaults match, so an untouched theme sets either alike; a
	// `#set enum(...)` reaches the ordered branch alone.
	let (marker_gap, item_skip) = if ordered {
		(style.enumeration.marker_gap, style.enumeration.item_skip)
	} else {
		(style.list.marker_gap, style.list.item_skip)
	};
	// Shape every marker once and keep the widest, so each item's text starts at the one indent. The
	// number counts across every entry regardless of any sub-list, so an ordered list stays 1..N.
	let mut markers:	Vec<ShapedText>	= Vec::with_capacity(items.len());
	let mut marker_w					= Sp::ZERO;
	for idx in 0..items.len() {
		// The ordered marker follows the theme's `enumeration.numbering` pattern where set (Typst's
		// `#set enum(numbering: ...)`), else the plain `N.` the template sets; a bulleted item is a bullet.
		let label	= if ordered {
			match &style.enumeration.numbering {
				Some(pattern)	=> format_numbering(pattern, &[idx as u32 + 1]),
				None			=> fmt!("{}.", idx + 1),
			}
		} else {
			"\u{2022}".to_string()	// U+2022 bullet
		};
		let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &label));
		if shaped.dims().width > marker_w { marker_w = shaped.dims().width; }
		markers.push(shaped);
	}
	let indent	= marker_w + marker_gap;
	let inner	= if measure > indent { measure - indent } else { measure };

	for (ei, entry) in items.iter().enumerate() {
		if ei > 0 {
			nodes.push(Node::Glue(Glue::fixed(item_skip)));
		}
		let pieces		= res!(build_pieces(fonts.clone(), geom, style, &entry.segments, Role::Body, foot_no, ref_no, margin_no, seen, idx, claim, bib, refs));
		let mut lines	= res!(break_paragraph_pieces(
			fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &pieces, inner, style.text.leading, style.text.justify, style.text.hyphenate, Rgba::BLACK,
			Some(cap_edge(style, style.text.body_size))));
		indent_item(&mut lines, Leaf::text(markers[ei].clone()), indent);
		nodes.extend(guard_widows(lines));
		// A list nested under this item sets at an increased left indent, with its own kind and numbering:
		// it is laid out within the item's inner measure and then shifted right by this list's indent.
		for child in &entry.children {
			if let Block::List { ordered: cord, items: citems } = child {
				nodes.push(Node::Glue(Glue::fixed(item_skip)));
				let mut sub: Vec<Node> = Vec::new();
				res!(list(&mut sub, fonts.clone(), geom, style, inner, *cord, citems,
					foot_no, ref_no, margin_no, seen, idx, claim, bib, refs));
				shift_nodes(&mut sub, indent);
				nodes.extend(sub);
			}
		}
	}
	Ok(())
}

/// Shifts every line box in `nodes` right by `by`, inserting a leading glue and growing the box width, so
/// a nested list sets indented under its parent item. The interline glue between the boxes is left alone.
fn shift_nodes(nodes: &mut [Node], by: Sp) {
	for node in nodes.iter_mut() {
		if let Node::HBox(b) = node {
			b.list.insert(0, Node::Glue(Glue::fixed(by)));
			b.dims = Dims::new(b.dims.width + by, b.dims.height, b.dims.depth);
		}
	}
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
	style: &Theme,
	lines:	&[String],
)
	-> Outcome<()>
{
	// Code is set a touch smaller than the body, as most templates do, so more of a wide line fits the
	// measure before it overflows. The size is the theme's own `code` group, so a `#set raw(...)` unit can
	// lower into it; its default matches the footnote size the block has always used.
	let size	= style.code.size;
	let indent	= style.text.body_size;	// a one-em hang, so the block sits off the left margin
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
			let gap = if style.text.leading > h + dep { style.text.leading - h - dep } else { style.table.line_gap };
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
	style: &Theme,
	measure:	Sp,
	number:		u32,
	note:		&[Segment],
	mark:		ShapedText,
)
	-> Outcome<Footnote>
{
	// The note's own inline runs, so a `*strong*` or `_emph_` term in the note sets with its own face
	// rather than flattening to upright text. A nested footnote or a cross-reference in a note -- rare --
	// sets nothing here, as a footnote carries no counter or reserved slot of its own.
	let pieces = res!(footnote_pieces(fonts.clone(), style, note));

	// The number sets as a small superscript that hangs to the left of the note: the note breaks at a
	// measure reduced by the mark's hang, its first line carries the mark and a gap that together fill the
	// hang, and every continuation line is shifted right by it, so the note's text block sits proud of its
	// mark exactly as Typst hangs a footnote.
	let (pre_shaped, pre_dims)	= res!(superscript(fonts.clone(), Role::Body, style.furniture.foot_size, &fmt!("{}", number)));
	let gap		= Sp(style.furniture.foot_size.raw() / 4);
	let hang	= pre_dims.width + gap;
	let inner	= if measure > hang { measure - hang } else { measure };

	let mut lines = res!(break_paragraph_pieces(
		fonts.clone(), Role::Body, Dir::Ltr, style.furniture.foot_size, &pieces, inner, style.furniture.foot_leading, true, true, Rgba::BLACK,
		Some(cap_edge(style, style.furniture.foot_size))));
	indent_item(&mut lines, Leaf::text_dims(pre_shaped, pre_dims), hang);

	let mut height = Sp::ZERO;
	for n in &lines {
		height += n.vextent();
	}

	Ok(Footnote { number, mark, note: lines, height })
}

/// Turns a footnote's inline runs into the pieces the line breaker weaves: a text run keeps its face, a
/// `*strong*` sets bold, an `_emph_` italic, a superscript rides raised, a code span sets mono, an in-note
/// maths span is flattened to leaves, and a glossary term sets its display text. A nested footnote, a
/// cross-reference and a citation are set as plain text or dropped, since a footnote carries no counter,
/// reserved page slot or bibliography of its own at this increment.
fn footnote_pieces(
	fonts:		Arc<FontSet>,
	style: &Theme,
	segments:	&[Segment],
)
	-> Outcome<Vec<Piece>>
{
	let size = style.furniture.foot_size;
	let mut pieces = Vec::with_capacity(segments.len());
	for seg in segments {
		match seg {
			Segment::Text(t)		=> pieces.push(Piece::Text { text: t.clone(), role: Role::Body }),
			Segment::Strong(t)		=> pieces.push(Piece::Text { text: t.clone(), role: Role::Bold }),
			Segment::Emph(t)		=> pieces.push(Piece::Text { text: t.clone(), role: Role::Italic }),
			Segment::BoldItalic(t)	=> pieces.push(Piece::Text { text: t.clone(), role: Role::BoldItalic }),
			Segment::Code(t)		=> pieces.push(Piece::Text { text: t.clone(), role: Role::Mono }),
			Segment::Glossary { display, .. }
									=> pieces.push(Piece::Text { text: display.clone(), role: Role::Body }),
			Segment::Cite(keys)		=> pieces.push(Piece::Text { text: fmt!("({})", keys.join("; ")), role: Role::Body }),
			Segment::PageRef(_)		=> {},	// a cross-reference in a note carries no reserved slot here
			Segment::Footnote { .. }	=> {},	// a nested footnote is not set within a footnote
			Segment::MarginNote { .. }	=> {},	// a margin note is not set within a footnote's own body
			Segment::Index { .. }	=> {},	// an index marker in a note is not recorded here
			Segment::Super(t) => {
				let (shaped, dims) = res!(superscript(fonts.clone(), Role::Body, size, t));
				pieces.push(Piece::Mark(Leaf::text_dims(shaped, dims)));
			},
			Segment::Sub(t) => {
				let (shaped, dims) = res!(subscript(fonts.clone(), Role::Body, size, t));
				pieces.push(Piece::Mark(Leaf::text_dims(shaped, dims)));
			},
			Segment::Math(expr) => {
				let node = res!(math::layout(fonts.clone(), style, expr, false));
				if let Node::HBox(b) = node {
					let ascent	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, "0")).dims().height;
					let over	= if b.dims.height > ascent { b.dims.height - ascent } else { Sp::ZERO };
					pieces.push(Piece::Math { nodes: b.list, width: b.dims.width, height: ascent, depth: b.dims.depth, over });
				}
			},
		}
	}
	Ok(pieces)
}

/// Shapes a short run at `0.7x` the surrounding size and returns it with the box that raises its
/// baseline. The box height is the surrounding ascent less a raise of a third of that ascent; the
/// emitter draws a run's baseline at `y + height`, so a shorter box lifts the run above the line's
/// baseline. The width and depth are the small run's own, keeping the mark narrow.
pub(crate) fn superscript(
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

/// Shapes a short run at `0.7x` the surrounding size and returns it with the box that drops its
/// baseline, the mirror of [`superscript`]. The box height is the surrounding ascent plus a drop of a
/// fifth of that ascent; since the emitter draws a run's baseline at `y + height`, a taller box seats
/// the run below the line's baseline. The width and depth are the small run's own, keeping the mark
/// narrow.
pub(crate) fn subscript(
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

	// The surrounding line's ascent, taken from a body-size digit, and the drop below its baseline.
	let sample	= res!(ShapedText::new(fonts, role, Dir::Ltr, base, "0"));
	let ascent	= sample.dims().height;
	let drop	= Sp(ascent.raw() * 20 / 100);
	let height	= ascent + drop;

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
	style: &Theme,
	measure:	Sp,
	expr:		&Atom,
	number:		Option<u32>,
)
	-> Outcome<()>
{
	let node = res!(math::layout(fonts.clone(), style, expr, true));
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
	let sample	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, "0"));
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
		let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &label));
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

/// Wraps a float's already-lowered material `mid` (a figure and its caption, or an aside box) as a
/// [`Node::Float`]. No block spacing is added around it: Typst frames a float with `clearance` (default
/// 1.5em of the float's font size), which the driver lays as the gap between the float and the body, so
/// the committed height the break weighs is `mid` alone.
fn push_float(nodes: &mut Vec<Node>, mid: Vec<Node>, clearance: Sp, placement: FloatPlacement) {
	let mut h = Sp::ZERO;
	for n in &mid {
		h += n.vextent();
	}
	nodes.push(Node::Float(FloatNode::new(mid, h, clearance, placement)));
}

/// The clearance a float is framed with -- Typst's `place.clearance` default, 1.5em of the float's font
/// size, resolved here against the body text size in force where the float is set.
fn float_clearance(style: &Theme) -> Sp {
	Sp::from_pt(style.text.body_size.to_pt() * 1.5)
}

/// Sets a figure: its identity as a [`Float`](crate::ledger::AnchorKind::Float) anchor, the graphic
/// centred on its own line, and a caption centred beneath. The graphic's dimensions are its bounding
/// box, `height` the whole visual extent and `depth` zero, so the line advances by the figure's height
/// and the greedy breaker moves it whole. The anchor is recorded before the ink so a reference to the
/// figure resolves the page it lands on.
fn figure(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style: &Theme,
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
	// The caption sets in the italic at the theme's figure caption size (its default the footnote size).
	let shaped	= res!(ShapedText::new(fonts, Role::Italic, Dir::Ltr, style.figure.caption_size, &text));
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
	geom:		PageGeometry,
	style: &Theme,
	measure:	Sp,
	table:		&Table,
	caption:	Option<&[Segment]>,
	supplement:	&str,
	number:		u32,
	label:		Option<&str>,
	foot_no:	&mut u32,
	ref_no:		&mut u32,
	margin_no:	&mut u32,
	seen:		&mut HashSet<String>,
	idx:		&mut IndexGather,
	claim:		&mut ClaimGather,
	bib:		Option<&Bibliography>,
	refs:		&HashMap<String, String>,
)
	-> Outcome<()>
{
	figure_anchors(nodes, supplement, number, label);
	nodes.push(res!(table::lower(
		fonts.clone(), geom, style, measure, table,
		foot_no, ref_no, margin_no, seen, idx, claim, bib, refs)));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(5.0))));
	res!(captioned(nodes, fonts, style, measure, supplement, number, caption));
	Ok(())
}

/// Sets an image wrapped in a figure: the figure's anchors, the loaded raster centred in the measure,
/// then a numbered caption beneath. A path that resolves to nothing, or a vector SVG with no raster
/// beside it, falls back to the placeholder box, which holds the same space so pagination is unchanged.
#[allow(clippy::too_many_arguments)]
fn image_figure(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style: &Theme,
	measure:	Sp,
	path:		&str,
	width:		Option<Length>,
	height:		Option<Length>,
	scale:		Option<f64>,
	caption:	Option<&[Segment]>,
	supplement:	&str,
	number:		u32,
	label:		Option<&str>,
)
	-> Outcome<()>
{
	figure_anchors(nodes, supplement, number, label);

	// The loaded figure sized to the measure, or the placeholder box when nothing loads. A load failure
	// is not fatal: the figure keeps its space and its caption, and the missing ink is a reported gap. A
	// raster fills a rectangle; an SVG is drawn as its own scaled paths.
	let graphic = match crate::image::load_figure(path) {
		Ok(crate::image::Figure::Raster(img))	=> res!(image_graphic(measure, img, width, height, scale)),
		Ok(crate::image::Figure::Vector(pic))	=> res!(svg_graphic(fonts.clone(), measure, pic, width, height, scale)),
		Err(_)									=> res!(placeholder(measure)),
	};
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
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(5.0))));
	res!(captioned(nodes, fonts, style, measure, supplement, number, caption));
	Ok(())
}

/// Sets a plain centred image with no figure number or caption -- a `#padded-image`/`#image` section
/// opener's logo. The image is sized and loaded exactly as a figure's is (an SVG drawn as its own scaled
/// paths, a raster to fill its box, a failed load standing in with the placeholder), then centred in the
/// measure with the template's 10 pt of padding above and below, so the words after it keep their air.
fn plain_image(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	measure:	Sp,
	path:		&str,
	width:		Option<Length>,
	height:		Option<Length>,
	scale:		Option<f64>,
)
	-> Outcome<()>
{
	let graphic = match crate::image::load_figure(path) {
		Ok(crate::image::Figure::Raster(img))	=> res!(image_graphic(measure, img, width, height, scale)),
		Ok(crate::image::Figure::Vector(pic))	=> res!(svg_graphic(fonts.clone(), measure, pic, width, height, scale)),
		Err(_)									=> res!(placeholder(measure)),
	};
	let pad = Sp::from_pt(10.0);	// the template's `padded-image` padding, above and below
	nodes.push(Node::Glue(Glue::fixed(pad)));
	let leaf	= Leaf::graphic(graphic);
	let gw		= leaf.dims.width;
	let gh		= leaf.dims.height + leaf.dims.depth;
	let lpad	= if measure > gw { Sp((measure.raw() - gw.raw()) / 2) } else { Sp::ZERO };
	let mut row:	Vec<Node> = Vec::new();
	if lpad.raw() > 0 {
		row.push(Node::Glue(Glue::fixed(lpad)));
	}
	row.push(Node::Leaf(leaf));
	nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, gh, Sp::ZERO))));
	nodes.push(Node::Glue(Glue::fixed(pad)));
	Ok(())
}

/// Sets a figure drawn by code: the figure's anchors, the built graphic centred in the measure (scaled
/// down uniformly if it is wider than the measure), then a numbered caption beneath. Building can fail --
/// a malformed diagram -- in which case the placeholder holds the space so pagination is unchanged.
#[allow(clippy::too_many_arguments)]
fn code_figure(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style: &Theme,
	measure:	Sp,
	figure:		&crate::lang::codefig::CodeFigure,
	caption:	Option<&[Segment]>,
	supplement:	&str,
	number:		u32,
	label:		Option<&str>,
)
	-> Outcome<()>
{
	figure_anchors(nodes, supplement, number, label);

	let graphic = match figure.build(fonts.clone()) {
		Ok(g)	=> res!(fit_graphic(g, measure)),
		Err(_)	=> res!(placeholder(measure)),
	};
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
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(5.0))));
	res!(captioned(nodes, fonts, style, measure, supplement, number, caption));
	Ok(())
}

/// Scales a built graphic down uniformly if it is wider than the measure, so a wide diagram fits the text
/// block; a graphic already within the measure is returned unchanged. Every path is carried through the
/// same factor, and the dimensions follow.
fn fit_graphic(g: Graphic, measure: Sp) -> Outcome<Graphic> {
	let w = g.dims.width;
	if w <= measure || w.raw() <= 0 {
		return Ok(g);
	}
	let s = measure.to_pt() as f32 / w.to_pt() as f32;
	let t = Transform::scale(s, s);
	let mut ops: Vec<DrawOp> = Vec::with_capacity(g.ops.len());
	for op in g.ops {
		ops.push(match op {
			DrawOp::Fill { path, colour } => DrawOp::Fill { path: res!(path.transform(&t)), colour },
			DrawOp::Stroke { path, colour, width } => DrawOp::Stroke {
				path:	res!(path.transform(&t)),
				colour,
				width:	width * s,
			},
			DrawOp::Image { image, x, y, w, h } => DrawOp::Image {
				image, x: x * s, y: y * s, w: w * s, h: h * s,
			},
		});
	}
	let dims = Dims::new(
		Sp::from_pt(g.dims.width.to_pt() * s as f64),
		Sp::from_pt(g.dims.height.to_pt() * s as f64),
		Sp::from_pt(g.dims.depth.to_pt() * s as f64),
	);
	Ok(Graphic::new(ops, dims))
}

/// Builds a graphic that draws a loaded raster to fill a box sized from the declared hints and the
/// image's own aspect. With no hint the image fills the measure; a `width`/`height` in the source sets
/// that axis and the other follows the aspect; a hint that would overflow the measure is clamped to it.
/// A single [`DrawOp::Image`] carries the pixels, so the emitters place one raster per figure.
fn image_graphic(
	measure:	Sp,
	img:		RasterImage,
	width:		Option<Length>,
	height:		Option<Length>,
	scale:		Option<f64>,
)
	-> Outcome<Graphic>
{
	let m		= measure.to_pt();
	let iw		= img.width.max(1) as f64;
	let ih		= img.height.max(1) as f64;
	let aspect	= ih / iw;

	// Resolve the declared width and height to points; a percentage is of the measure, a length absolute.
	let resolve = |len: Length| -> f64 {
		match len {
			Length::Rel(f)	=> m * f,
			Length::Abs(pt)	=> pt,
		}
	};

	// A width wins the sizing; else a height sets it through the aspect; else the image fills the
	// measure. `scale` on a `padded-image` multiplies a filled measure, so a 100% scale is the measure.
	let mut w = match (width, height) {
		(Some(wl), _)		=> resolve(wl),
		(None, Some(hl))	=> resolve(hl) / aspect,
		(None, None)		=> m * scale.unwrap_or(1.0),
	};
	if w > m || w <= 0.0 {
		w = m;
	}
	let h = match height {
		Some(hl) if width.is_none() && scale.is_none()	=> resolve(hl),
		_												=> w * aspect,
	};

	let wf	= w as f32;
	let hf	= h as f32;
	let ops	= vec![DrawOp::Image { image: Arc::new(img), x: 0.0, y: 0.0, w: wf, h: hf }];
	Ok(Graphic::new(ops, Dims::new(Sp::from_pt(w), Sp::from_pt(h), Sp::ZERO)))
}

/// Builds a graphic from a read SVG, scaled to fit the box the sizing hints and the picture's own aspect
/// ask for -- the same sizing a raster gets -- and its paths mapped to fill and stroke ops.
///
/// The picture comes out of the reader in its viewBox units, which for a typesetter's SVG are points, so
/// the intrinsic size stands in for a raster's pixel dimensions. One uniform factor scales every path;
/// a dashed or a capped stroke is baked to a filled outline first, since a plain [`DrawOp::Stroke`]
/// carries only a width, and the emitter would otherwise draw it solid. An illustrator's live `<text>`
/// arrives unshaped, so it is shaped here with the book's font set and baked to glyph outlines, and an
/// embedded raster is placed as a scaled [`DrawOp::Image`].
fn svg_graphic(
	fonts:		Arc<FontSet>,
	measure:	Sp,
	pic:		SvgPicture,
	width:		Option<Length>,
	height:		Option<Length>,
	scale:		Option<f64>,
)
	-> Outcome<Graphic>
{
	let m		= measure.to_pt();
	let iw		= (pic.width as f64).max(1.0);
	let ih		= (pic.height as f64).max(1.0);
	let aspect	= ih / iw;

	let resolve = |len: Length| -> f64 {
		match len {
			Length::Rel(f)	=> m * f,
			Length::Abs(pt)	=> pt,
		}
	};
	let mut w = match (width, height) {
		(Some(wl), _)		=> resolve(wl),
		(None, Some(hl))	=> resolve(hl) / aspect,
		(None, None)		=> m * scale.unwrap_or(1.0),
	};
	if w > m || w <= 0.0 {
		w = m;
	}
	let h = match height {
		Some(hl) if width.is_none() && scale.is_none()	=> resolve(hl),
		_												=> w * aspect,
	};

	// A uniform factor from the picture's intrinsic width to the drawn width; the height follows the
	// same factor, since the aspect was preserved above.
	let s	= (w / iw) as f32;
	let t	= Transform::scale(s, s);
	let mut ops: Vec<DrawOp> = Vec::with_capacity(pic.ops.len());
	for op in pic.ops {
		match op {
			SvgOp::Fill { path, colour } => {
				ops.push(DrawOp::Fill { path: res!(path.transform(&t)), colour });
			},
			SvgOp::Stroke { path, colour, stroke } => {
				if stroke.dash.is_some() {
					// Bake the dashes into an outline in the picture's frame, then scale that with the rest.
					let outline = res!(path.stroke(&stroke));
					ops.push(DrawOp::Fill { path: res!(outline.transform(&t)), colour });
				} else {
					ops.push(DrawOp::Stroke {
						path:	res!(path.transform(&t)),
						colour,
						width:	stroke.width * s,
					});
				}
			},
			SvgOp::Text { text, local, x, y, size, anchor, italic, bold, colour } => {
				res!(bake_svg_text(
					&mut ops, fonts.clone(), &text, &local, &t, x, y, size, anchor, italic, bold, colour));
			},
			SvgOp::Image { rgba, iw, ih, x, y, w: iwd, h: ihd } => {
				// The raster's placement rectangle is in the picture frame; the same factor scales it.
				let img = RasterImage { width: iw, height: ih, rgba };
				ops.push(DrawOp::Image {
					image:	Arc::new(img),
					x:		x * s,
					y:		y * s,
					w:		iwd * s,
					h:		ihd * s,
				});
			},
		}
	}
	Ok(Graphic::new(ops, Dims::new(Sp::from_pt(w), Sp::from_pt(h), Sp::ZERO)))
}

/// Shapes one live SVG text run with the book's font set and bakes it to filled glyph outlines. The run
/// is shaped at its own font-size in the picture's units; `local` maps that frame to the picture frame
/// and `t` the picture frame to the drawn frame. The anchor slides the pen from the run's start once the
/// advance is known, and each glyph's y-up outline is flipped onto the SVG's y-down baseline before the
/// two frame transforms carry it home -- the same bake the diagram and plot labels use.
#[allow(clippy::too_many_arguments)]
fn bake_svg_text(
	ops:	&mut Vec<DrawOp>,
	fonts:	Arc<FontSet>,
	text:	&str,
	local:	&Transform,
	t:		&Transform,
	x:		f32,
	y:		f32,
	size:	f32,
	anchor:	Anchor,
	italic:	bool,
	bold:	bool,
	colour:	Rgba,
)
	-> Outcome<()>
{
	if size <= 0.0 {
		return Ok(());
	}
	let role = match (bold, italic) {
		(true, true)	=> Role::BoldItalic,
		(true, false)	=> Role::Bold,
		(false, true)	=> Role::Italic,
		(false, false)	=> Role::Body,
	};
	let shaped	= res!(ShapedText::new(fonts, role, Dir::Ltr, Sp::from_pt(size as f64), text));
	let advance	= shaped.dims().width.to_pt() as f32;
	let pen_x	= match anchor {
		Anchor::Start	=> x,
		Anchor::Middle	=> x - advance / 2.0,
		Anchor::End		=> x - advance,
	};
	for glyph in &shaped.run().glyphs {
		let outline = res!(shaped.outline(glyph));
		if outline.is_empty() {
			continue;	// a space carries an advance but no ink
		}
		let place = Transform::scale(1.0, -1.0)
			.then(&Transform::translate(pen_x + glyph.x, y - glyph.y))
			.then(local)
			.then(t);
		ops.push(DrawOp::Fill { path: res!(outline.transform(&place)), colour });
	}
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

/// One typeset unit of a caption: an unbreakable cluster of one or more boxes (a word, or a word with an
/// attached superscript, or a maths cluster) with its extent, or a breakable interword space.
enum CapTok {
	Unit { nodes: Vec<Node>, width: Sp, height: Sp, depth: Sp },
	Space,
}

/// Sets a figure caption -- "{supplement} {number}: {caption}" -- centred beneath the figure, wrapped
/// greedily into ragged centred lines at the body size. The caption's own runs are set with their faces,
/// so an emphasised word, a superscript or an in-caption maths span renders rather than flattening to
/// upright text or vanishing. A caption with no text sets just its number.
fn captioned(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style: &Theme,
	measure:	Sp,
	supplement:	&str,
	number:		u32,
	caption:	Option<&[Segment]>,
)
	-> Outcome<()>
{
	let size	= style.text.body_size;
	let space_w	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, " ")).dims().width;

	// The leading "{supplement} {number}: " (or just the number when the caption has no text), then the
	// caption's segments, tokenised into words, spaces, superscripts and maths clusters. A shared
	// pending-space flag carries a run's trailing space to the next token, so spacing follows the source.
	let has_text	= caption.map(|c| segments_have_text(c)).unwrap_or(false);
	let prefix		= if has_text { fmt!("{} {}: ", supplement, number) } else { fmt!("{} {}", supplement, number) };

	let mut toks:	Vec<CapTok>	= Vec::new();
	let mut pending				= false;
	res!(push_caption_text(&mut toks, &mut pending, fonts.clone(), Role::Body, size, &prefix));
	if let Some(segs) = caption {
		for seg in segs {
			match seg {
				Segment::Text(t)	=> res!(push_caption_text(&mut toks, &mut pending, fonts.clone(), Role::Body, size, t)),
				Segment::Strong(t)		=> res!(push_caption_text(&mut toks, &mut pending, fonts.clone(), Role::Bold, size, t)),
				Segment::Emph(t)		=> res!(push_caption_text(&mut toks, &mut pending, fonts.clone(), Role::Italic, size, t)),
				Segment::BoldItalic(t)	=> res!(push_caption_text(&mut toks, &mut pending, fonts.clone(), Role::BoldItalic, size, t)),
				Segment::Code(t)	=> res!(push_caption_text(&mut toks, &mut pending, fonts.clone(), Role::Mono, size, t)),
				Segment::Glossary { display, .. }
									=> res!(push_caption_text(&mut toks, &mut pending, fonts.clone(), Role::Body, size, display)),
				Segment::Cite(keys)	=> res!(push_caption_text(
										&mut toks, &mut pending, fonts.clone(), Role::Body, size, &fmt!("({})", keys.join("; ")))),
				Segment::PageRef(_)		=> {},	// a cross-reference in a caption is not resolved here
				Segment::Footnote { .. }	=> {},	// a footnote in a caption is not set here
				Segment::MarginNote { .. }	=> {},	// a margin note in a caption sets nothing here
				Segment::Index { .. }	=> {},	// an index marker in a caption sets nothing here
				Segment::Super(t) => {
					let (shaped, dims) = res!(superscript(fonts.clone(), Role::Body, size, t));
					push_caption_box(&mut toks, &mut pending,
						vec![Node::Leaf(Leaf::text_dims(shaped, dims))], dims.width, dims.height, dims.depth);
				},
				Segment::Sub(t) => {
					let (shaped, dims) = res!(subscript(fonts.clone(), Role::Body, size, t));
					push_caption_box(&mut toks, &mut pending,
						vec![Node::Leaf(Leaf::text_dims(shaped, dims))], dims.width, dims.height, dims.depth);
				},
				Segment::Math(expr) => {
					let node = res!(math::layout(fonts.clone(), style, expr, false));
					if let Node::HBox(b) = node {
						push_caption_box(&mut toks, &mut pending, b.list, b.dims.width, b.dims.height, b.dims.depth);
					}
				},
			}
		}
	}

	// Greedy line fill: units joined by single spaces, broken before the unit that would overrun the
	// measure. Each finished line is centred by a left glue of half its slack.
	let mut line:	Vec<&CapTok>	= Vec::new();
	let mut line_w					= Sp::ZERO;
	let mut first					= true;
	for tok in &toks {
		if let CapTok::Unit { width, .. } = tok {
			let add = if line.is_empty() { *width } else { space_w + *width };
			if !line.is_empty() && line_w + add > measure {
				res!(emit_caption_units(nodes, style, measure, space_w, &line, line_w, &mut first));
				line.clear();
				line_w = Sp::ZERO;
			}
			line_w += if line.is_empty() { *width } else { space_w + *width };
			line.push(tok);
		}
	}
	if !line.is_empty() {
		res!(emit_caption_units(nodes, style, measure, space_w, &line, line_w, &mut first));
	}
	Ok(())
}

/// Whether any caption segment carries visible text, so the colon prefix is set only for a real caption.
fn segments_have_text(segs: &[Segment]) -> bool {
	segs.iter().any(|s| match s {
		Segment::Text(t) | Segment::Strong(t) | Segment::Emph(t) | Segment::BoldItalic(t) | Segment::Code(t) | Segment::Super(t) | Segment::Sub(t)
							=> !t.trim().is_empty(),
		Segment::Glossary { display, .. }	=> !display.trim().is_empty(),
		Segment::Math(_) | Segment::Cite(_)	=> true,
		_									=> false,
	})
}

/// Tokenises a text run into word units and interword spaces, in the given face, appending to `toks`. A
/// leading or run-crossing space is carried in `pending` and emitted only before the next word, so the
/// source's spacing survives and a trailing space attaches to whatever segment follows.
fn push_caption_text(
	toks:		&mut Vec<CapTok>,
	pending:	&mut bool,
	fonts:		Arc<FontSet>,
	role:		Role,
	size:		Sp,
	text:		&str,
)
	-> Outcome<()>
{
	let mut word = String::new();
	for c in text.chars() {
		if c.is_whitespace() {
			if !word.is_empty() {
				res!(flush_caption_word(toks, pending, fonts.clone(), role, size, &mut word));
			}
			*pending = true;
		} else {
			word.push(c);
		}
	}
	if !word.is_empty() {
		res!(flush_caption_word(toks, pending, fonts.clone(), role, size, &mut word));
	}
	Ok(())
}

/// Shapes one word and pushes it as a unit, emitting a pending space before it when one is due.
fn flush_caption_word(
	toks:		&mut Vec<CapTok>,
	pending:	&mut bool,
	fonts:		Arc<FontSet>,
	role:		Role,
	size:		Sp,
	word:		&mut String,
)
	-> Outcome<()>
{
	let shaped	= res!(ShapedText::new(fonts, role, Dir::Ltr, size, word));
	let d		= shaped.dims();
	push_caption_box(toks, pending, vec![Node::Leaf(Leaf::text(shaped))], d.width, d.height, d.depth);
	word.clear();
	Ok(())
}

/// Pushes a pre-built box as a caption unit, emitting a pending interword space before it first. Adjacent
/// boxes with no pending space between them (a word and its attached superscript) become one unit.
fn push_caption_box(
	toks:		&mut Vec<CapTok>,
	pending:	&mut bool,
	mut boxes:	Vec<Node>,
	width:		Sp,
	height:		Sp,
	depth:		Sp,
)
{
	if *pending {
		toks.push(CapTok::Space);
		*pending = false;
	} else if let Some(CapTok::Unit { nodes, width: w, height: h, depth: dp }) = toks.last_mut() {
		// No space since the previous unit: attach to it, so a word and its superscript stay unbreakable.
		nodes.append(&mut boxes);
		*w		= *w + width;
		*h		= (*h).max(height);
		*dp		= (*dp).max(depth);
		return;
	}
	toks.push(CapTok::Unit { nodes: boxes, width, height, depth });
}

/// Sets one centred caption line from its units, with interline leading before every line but the first.
fn emit_caption_units(
	nodes:		&mut Vec<Node>,
	style: &Theme,
	measure:	Sp,
	space_w:	Sp,
	line:		&[&CapTok],
	line_w:		Sp,
	first:		&mut bool,
)
	-> Outcome<()>
{
	let mut height	= Sp::ZERO;
	let mut depth	= Sp::ZERO;
	for tok in line {
		if let CapTok::Unit { height: h, depth: d, .. } = tok {
			height	= height.max(*h);
			depth	= depth.max(*d);
		}
	}
	if !*first {
		let vext	= height + depth;
		let gap		= if style.text.leading > vext { style.text.leading - vext } else { style.table.line_gap };
		nodes.push(Node::Glue(Glue::fixed(gap)));
	}
	*first = false;

	let pad = if measure > line_w { Sp((measure.raw() - line_w.raw()) / 2) } else { Sp::ZERO };
	let mut row:	Vec<Node> = Vec::new();
	if pad.raw() > 0 {
		row.push(Node::Glue(Glue::fixed(pad)));
	}
	for (k, tok) in line.iter().enumerate() {
		if let CapTok::Unit { nodes: ns, .. } = tok {
			if k > 0 {
				row.push(Node::Glue(Glue::fixed(space_w)));	// the single interword space between units
			}
			for n in ns { row.push(n.clone()); }
		}
	}
	nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, height, depth))));
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

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FRONT MATTER                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// Composes the front matter ahead of the body: the cover raster (a development build only), the title
/// page, the imprint, an optional dedication, and an optional author biography, each on its own page
/// closed by a forced break. None of these leaves sets a heading anchor, so the body's first heading
/// still fixes where the printed folio restarts at one.
fn front_matter(
	nodes:		&mut Vec<Node>,
	fonts:		&Arc<FontSet>,
	faces:		&FaceResolver,
	geom:		PageGeometry,
	style: &Theme,
	fm:			&FrontMatter,
)
	-> Outcome<()>
{
	// The title-page display font: the level-1 heading face resolved once, or `None` when the tree ships no
	// display face, in which case the title helpers set in the body role exactly as before.
	let display = head_solo(&resolved_head_face(1, style, faces, is_doc_heading(style)));
	// Cover: the raster filling the content box, a development build only. A path that will not load
	// (an SVG, or a missing file) sets no cover page rather than a placeholder.
	if let Some(path) = &fm.cover_image {
		if let Ok(node) = fm_cover_node(geom, path) {
			nodes.push(node);
			nodes.push(Node::Penalty(Penalty::eject()));
		}
	}

	// A documentation tree draws the template's two-column title page (a coloured sidebar with its logos and
	// the title on the right); a book draws its plain centred title page. The sidebar grey marks the idiom.
	// A `Label` anchor at the leaf's top records its page for the PDF outline; it sets no heading, so it
	// stays out of the running heads and the contents.
	nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Label, "frontmatter:title")));
	if fm.sidebar_grey.is_some() {
		res!(fm_doc_title_page(nodes, fonts, geom, fm));
	} else {
		res!(fm_title_page(nodes, fonts, geom, style, fm));
	}
	nodes.push(Node::Penalty(Penalty::eject()));

	// The meta page: a doc tree draws the template's Ver/Date/Author(s)/Notes colophon, a book its plain
	// imprint page. Both push the `frontmatter:meta` anchor so the outline lists a Meta entry at this leaf.
	if fm.sidebar_grey.is_some() {
		if fm_has_doc_meta(fm) {
			nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Label, "frontmatter:meta")));
			res!(fm_doc_meta_page(nodes, fonts, geom, style, fm));
			nodes.push(Node::Penalty(Penalty::eject()));
		}
	} else if fm_has_imprint(fm) {
		nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Label, "frontmatter:meta")));
		res!(fm_meta_page(nodes, fonts, geom, style, fm));
		nodes.push(Node::Penalty(Penalty::eject()));
	}

	if let Some(ded) = &fm.dedication {
		res!(fm_dedication_page(nodes, fonts, geom, style, ded));
		nodes.push(Node::Penalty(Penalty::eject()));
	}

	if let Some(bio) = &fm.about_author {
		res!(fm_about_author_page(nodes, fonts, display, geom, style, fm.back_title_size, bio));
		nodes.push(Node::Penalty(Penalty::eject()));
	}

	Ok(())
}

/// Does the book set any imprint field, so a meta page is worth composing?
fn fm_has_imprint(fm: &FrontMatter) -> bool {
	fm.publisher.is_some() || fm.edition.is_some() || fm.isbn.is_some() || fm.copyright.is_some()
		|| fm.rights.is_some() || fm.ai_declaration.is_some() || fm.website.is_some() || fm.toolchain
}

/// Does the doc tree state a revision, so the template's meta/colophon page is worth composing? A doc
/// root always sets `meta-data` with at least one row, or names an author, so this holds for every doc.
fn fm_has_doc_meta(fm: &FrontMatter) -> bool {
	!fm.meta_rows.is_empty() || !fm.author.is_empty()
}

/// A rigid vertical spacer that a page top does not discard, so front-matter elements sit at fixed
/// fractions of the page down from the top. Modelled as an empty horizontal box of the wanted height,
/// which the greedy breaker advances the cursor by without placing any ink.
fn fm_spacer(height: Sp) -> Node {
	Node::HBox(BoxNode::new(Vec::new(), Dims::new(Sp::ZERO, height, Sp::ZERO)))
}

/// Shapes one line and pushes it centred in the measure, returning its vertical extent so the caller can
/// track the cursor down the page.
fn fm_centred_line(
	nodes:		&mut Vec<Node>,
	fonts:		&Arc<FontSet>,
	display:	Option<&Arc<Font>>,
	role:		Role,
	size:		Sp,
	text:		&str,
	measure:	Sp,
)
	-> Outcome<Sp>
{
	let shaped = match display {
		Some(f)	=> res!(ShapedText::new_with_font((*f).clone(), Dir::Ltr, size, text)),
		None	=> res!(ShapedText::new(fonts.clone(), role, Dir::Ltr, size, text)),
	};
	let d	= shaped.dims();
	let pad	= if measure > d.width { Sp((measure.raw() - d.width.raw()) / 2) } else { Sp::ZERO };
	let mut row: Vec<Node> = Vec::new();
	if pad.raw() > 0 {
		row.push(Node::Glue(Glue::fixed(pad)));
	}
	row.push(Node::Leaf(Leaf::text(shaped)));
	nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, d.height, d.depth))));
	Ok(d.height + d.depth)
}

/// Sets a run of text as greedily-wrapped centred lines (the title, which may not fit one line),
/// returning the total vertical extent set.
fn fm_centred_wrap(
	nodes:		&mut Vec<Node>,
	fonts:		&Arc<FontSet>,
	role:		Role,
	size:		Sp,
	text:		&str,
	measure:	Sp,
	leading:	Sp,
)
	-> Outcome<Sp>
{
	let mut line	= String::new();
	let mut total	= Sp::ZERO;
	let mut first	= true;
	let mut flush = |nodes: &mut Vec<Node>, line: &str, first: &mut bool, total: &mut Sp| -> Outcome<()> {
		let shaped	= res!(ShapedText::new(fonts.clone(), role, Dir::Ltr, size, line));
		let d		= shaped.dims();
		if !*first {
			let vext	= d.height + d.depth;
			let gap		= if leading > vext { leading - vext } else { Sp::ZERO };
			nodes.push(Node::Glue(Glue::fixed(gap)));
			*total += gap;
		}
		*first = false;
		let pad	= if measure > d.width { Sp((measure.raw() - d.width.raw()) / 2) } else { Sp::ZERO };
		let mut row: Vec<Node> = Vec::new();
		if pad.raw() > 0 {
			row.push(Node::Glue(Glue::fixed(pad)));
		}
		row.push(Node::Leaf(Leaf::text(shaped)));
		nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, d.height, d.depth))));
		*total += d.height + d.depth;
		Ok(())
	};
	for word in text.split_whitespace() {
		let trial	= if line.is_empty() { word.to_string() } else { fmt!("{} {}", line, word) };
		let shaped	= res!(ShapedText::new(fonts.clone(), role, Dir::Ltr, size, &trial));
		if shaped.dims().width > measure && !line.is_empty() {
			res!(flush(nodes, &line, &mut first, &mut total));
			line = word.to_string();
		} else {
			line = trial;
		}
	}
	if !line.is_empty() {
		res!(flush(nodes, &line, &mut first, &mut total));
	}
	Ok(total)
}

/// Builds the cover page: the raster at `path` bleeding to the trim edge on all four sides. The box the
/// breaker measures is the content area, so the cover paginates as a single leaf closed by the caller's
/// eject; the image inside is offset back to the physical page origin and sized to the whole trim, so it
/// paints under the margins to the paper edge. The emitter does not clip a graphic to its box, so the
/// overpaint lands. Page one is a recto -- the inside margin is the left one and no mirror shift applies
/// -- so the offset is simply the top and inside margins.
fn fm_cover_node(geom: PageGeometry, path: &str) -> Outcome<Node> {
	let img	= res!(crate::image::load(path));
	let cw	= geom.content_width();
	let ch	= geom.content_height();
	let ox	= -(geom.content_left().to_pt() as f32);
	let oy	= -(geom.content_top().to_pt() as f32);
	let pw	= geom.width.to_pt() as f32;
	let ph	= geom.height.to_pt() as f32;
	let ops	= vec![DrawOp::Image { image: Arc::new(img), x: ox, y: oy, w: pw, h: ph }];
	let graphic	= Graphic::new(ops, Dims::new(cw, ch, Sp::ZERO));
	Ok(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::graphic(graphic))], Dims::new(cw, ch, Sp::ZERO))))
}

/// Sets the title page: the author name in the upper band, the title and subtitle about the centre, and
/// the publisher logo near the foot -- the template's three-band grid, approximated with fixed fractions
/// of the page height.
fn fm_title_page(
	nodes:	&mut Vec<Node>,
	fonts:	&Arc<FontSet>,
	geom:	PageGeometry,
	style: &Theme,
	fm:		&FrontMatter,
)
	-> Outcome<()>
{
	let measure	= geom.content_width();
	let h		= geom.content_height();
	let mut y	= Sp::ZERO;

	// The author name, in the upper fifth.
	nodes.push(fm_spacer(Sp(h.raw() * 17 / 100)));
	y += Sp(h.raw() * 17 / 100);
	y += res!(fm_centred_line(nodes, fonts, None, Role::Body, fm.author_size, &fm.author, measure));

	// The title about the vertical centre, wrapped when it will not fit one line, then the subtitle.
	let target = Sp(h.raw() * 38 / 100);
	if target > y {
		nodes.push(fm_spacer(target - y));
		y = target;
	}
	let title_lead = Sp(fm.title_size.raw() * 6 / 5);
	y += res!(fm_centred_wrap(nodes, fonts, Role::Bold, fm.title_size, &fm.title, measure, title_lead));
	if let Some(sub) = &fm.subtitle {
		let gap = Sp(fm.title_size.raw() * 3 / 5);
		nodes.push(fm_spacer(gap));
		y += gap;
		y += res!(fm_centred_line(nodes, fonts, None, Role::Italic, fm.subtitle_size, sub, measure));
	}

	// The publisher logo near the foot, when it loads (an SVG logo does not, and is simply omitted).
	if let Some(logo) = &fm.logo_image {
		if let Ok(node) = fm_logo_node(fonts, geom, style, logo) {
			let target = Sp(h.raw() * 84 / 100);
			if target > y {
				nodes.push(fm_spacer(target - y));
			}
			nodes.push(node);
		}
	}
	Ok(())
}

/// Builds the logo line: the raster at `path` centred at a modest width. An SVG or missing file errors,
/// and the title page omits the logo.
fn fm_logo_node(_fonts: &Arc<FontSet>, geom: PageGeometry, _style: &Theme, path: &str) -> Outcome<Node> {
	let img	= res!(crate::image::load(path));
	let measure	= geom.content_width();
	let w	= Sp::from_pt(110.0);	// the type scale's logo width, about 110 pt
	let aspect	= (img.height.max(1) as f64) / (img.width.max(1) as f64);
	let hh	= Sp::from_pt(110.0 * aspect);
	let ops	= vec![DrawOp::Image {
		image: Arc::new(img), x: 0.0, y: 0.0, w: w.to_pt() as f32, h: hh.to_pt() as f32 }];
	let graphic	= Graphic::new(ops, Dims::new(w, hh, Sp::ZERO));
	let pad	= if measure > w { Sp((measure.raw() - w.raw()) / 2) } else { Sp::ZERO };
	let mut row: Vec<Node> = Vec::new();
	if pad.raw() > 0 {
		row.push(Node::Glue(Glue::fixed(pad)));
	}
	row.push(Node::Leaf(Leaf::graphic(graphic)));
	Ok(Node::HBox(BoxNode::new(row, Dims::new(measure, hh, Sp::ZERO))))
}

/// Sets the documentation template's two-column title page (`template.typ`'s `title-page`): a full-height
/// coloured sidebar down the left carrying a logo near its top and one near its foot, and the title (large,
/// in small caps or italic) with its subtitle centred on the white right. The whole page is one box whose
/// graphic ops bleed past the box bounds to the paper edges -- the emitter clips nothing -- exactly as
/// `doc_banner` draws its full-bleed bar. Its box origin is the content top-left (y down), so the page
/// origin is `(-inside, -top)` and the paper corner `(page_w - inside, page_h - top)`.
fn fm_doc_title_page(
	nodes:	&mut Vec<Node>,
	fonts:	&Arc<FontSet>,
	geom:	PageGeometry,
	fm:		&FrontMatter,
)
	-> Outcome<()>
{
	let measure	= geom.content_width();
	let box_h	= geom.content_height();
	let il		= geom.content_left().to_pt() as f32;	// left margin, and the sidebar logos' `margins.a4` pad
	let it		= geom.content_top().to_pt() as f32;	// top margin, equal to the template's `margins.a4`
	let pw		= geom.width.to_pt() as f32;
	let ph		= geom.height.to_pt() as f32;
	let frac	= fm.sidebar_frac as f32;
	let side_w	= frac * pw;	// the sidebar width, `margins.title_page` of the page

	let mut ops:	Vec<DrawOp> = Vec::new();

	// The sidebar: a solid rectangle from the page's top-left corner, `side_w` wide and the full page tall.
	let grey	= fm.sidebar_grey.unwrap_or(240);
	let fill	= Rgba::opaque(grey, grey, grey);
	ops.push(DrawOp::Fill {
		path:	res!(Path::rect(Bounds::new(-il, -it, -il + side_w, -it + ph))),
		colour:	fill,
	});

	// The top logo, centred across the sidebar, its top edge one `margins.a4` down from the page top -- which
	// equals the top margin, so its box-frame top is zero. The bottom logo sits one `margins.a4` up from the
	// page foot. Both are drawn at the width the `doc.with` call declared; a logo that will not load is left
	// out, as the template's own missing-image path would leave a gap.
	let side_mid_box	= -il + side_w / 2.0;	// the sidebar's horizontal centre, in the box frame
	if let Some(path) = &fm.top_logo {
		let w = fm.top_logo_width.to_pt() as f32;
		if let Ok((logo, _)) = logo_ops(fonts, path, w, side_mid_box - w / 2.0, 0.0) {
			ops.extend(logo);
		}
	}
	if let Some(path) = &fm.bottom_logo {
		let w = fm.bottom_logo_width.to_pt() as f32;
		if let Ok((logo, lh)) = logo_ops(fonts, path, w, 0.0, 0.0) {
			// Re-place now the height is known: bottom edge one `margins.a4` up from the page foot.
			let dy = (ph - it) - it - lh;
			let placed = res!(translate_ops(logo, side_mid_box - w / 2.0, dy));
			ops.extend(placed);
		}
	}

	// The title and subtitle centred on the right column: from the sidebar's right edge plus the template's
	// 20 pt, running to the page's right margin less 20 pt. The title rides the column's vertical centre
	// (the template's 40%/10%/50% grid seats it at the half), the subtitle two lines below. The title wraps
	// within `col_w`, exactly as the template's `rect(width: size.width - margins.title_page - 40pt)` wraps
	// `#text(size: 35pt)[#emph(title)]` -- a long title (e.g. "Oxegen Technical Specification") otherwise
	// shapes as one run and overruns the rail.
	let col_l		= side_w + 20.0;
	let col_w		= pw - side_w - 40.0;
	let centre_box	= -il + col_l + col_w / 2.0;
	let title_size	= 35.0f32;	// the template's fixed title size, independent of the config type scale
	let sub_size	= 20.0f32;
	let sample		= res!(head_shape(fonts, &HeadFace::Role(Role::Body), Sp::from_pt(title_size as f64), "Ag"));
	let asc			= sample.dims().height.to_pt() as f32;
	let dep			= sample.dims().depth.to_pt() as f32;
	let leading		= (asc + dep) * 1.2;	// title line height, leading proportioned as the body text is

	let lines		= res!(wrap_title_lines(fonts, &fm.title, title_size, col_w, fm.title_smallcaps));
	let extra		= (lines.len().saturating_sub(1)) as f32 * leading;
	// The column centre, in the box frame, shifted up by half the extra lines' height so a wrapped title
	// still balances about the same point a single line would occupy.
	let title_top	= ph / 2.0 - it - extra / 2.0;
	let mut title_base = title_top + asc;
	for (line, fit_size) in &lines {
		res!(title_run_ops(&mut ops, fonts, line, *fit_size, centre_box, title_base, fm.title_smallcaps));
		title_base += leading;
	}
	if let Some(sub) = &fm.subtitle {
		// Two blank lines below the title (the template's `\ \`), then the subtitle in italic. `title_base`
		// has already stepped past the last title line, so back off one `leading` to its baseline.
		let sub_base = title_base - leading + dep + 28.0 + sub_size;
		res!(title_run_ops(&mut ops, fonts, sub, sub_size, centre_box, sub_base, false));
	}

	let graphic = Graphic::new(ops, Dims::new(measure, box_h, Sp::ZERO));
	nodes.push(Node::HBox(BoxNode::new(
		vec![Node::Leaf(Leaf::graphic(graphic))], Dims::new(measure, box_h, Sp::ZERO))));
	Ok(())
}

/// Loads a logo (an SVG drawn as its own scaled paths, or a raster) at the drawn width `w`, translates its
/// ops to `(dx, dy)` in the caller's frame, and returns them with the drawn height. The picture comes out
/// sized to `w` with its aspect kept, so the height stands for where a bottom-aligned logo's top sits.
fn logo_ops(
	fonts:	&Arc<FontSet>,
	path:	&str,
	w:		f32,
	dx:		f32,
	dy:		f32,
)
	-> Outcome<(Vec<DrawOp>, f32)>
{
	let width	= Some(Length::Abs(w as f64));
	let graphic	= match crate::image::load_figure(path) {
		Ok(crate::image::Figure::Raster(img))	=> res!(image_graphic(Sp::from_pt(w as f64), img, width, None, None)),
		Ok(crate::image::Figure::Vector(pic))	=> res!(svg_graphic(fonts.clone(), Sp::from_pt(w as f64), pic, width, None, None)),
		Err(e)									=> return Err(e),
	};
	let h	= (graphic.dims.height + graphic.dims.depth).to_pt() as f32;
	let ops	= res!(translate_ops(graphic.ops, dx, dy));
	Ok((ops, h))
}

/// Translates every op of a graphic by `(dx, dy)` -- the fill and stroke paths through a translation, an
/// embedded raster by shifting its placement corner. Used to seat a logo built at the origin where it belongs.
fn translate_ops(src: Vec<DrawOp>, dx: f32, dy: f32) -> Outcome<Vec<DrawOp>> {
	let t = Transform::translate(dx, dy);
	let mut out: Vec<DrawOp> = Vec::with_capacity(src.len());
	for op in src {
		out.push(match op {
			DrawOp::Fill { path, colour }			=> DrawOp::Fill { path: res!(path.transform(&t)), colour },
			DrawOp::Stroke { path, colour, width }	=> DrawOp::Stroke { path: res!(path.transform(&t)), colour, width },
			DrawOp::Image { image, x, y, w, h }		=> DrawOp::Image { image, x: x + dx, y: y + dy, w, h },
		});
	}
	Ok(out)
}

/// Measures a title run's total advance exactly as `title_run_ops` shapes it (the same small-caps
/// splitting, so a wrap decided from this width breaks where the baked glyphs will actually fall).
fn title_run_width(fonts: &Arc<FontSet>, text: &str, size_pt: f32, smallcaps: bool) -> Outcome<f32> {
	let size		= Sp::from_pt(size_pt as f64);
	let small_size	= Sp(size.raw() * 3 / 4);
	let face		= if smallcaps { HeadFace::Role(Role::Body) } else { HeadFace::Role(Role::Italic) };
	let runs		= if smallcaps { smallcaps_runs(text) } else { vec![(text.to_string(), false)] };
	let mut total = 0.0f32;
	for (run, is_small) in &runs {
		let rs		= if *is_small { small_size } else { size };
		let shaped	= res!(head_shape(fonts, &face, rs, run));
		total += shaped.dims().width.to_pt() as f32;
	}
	Ok(total)
}

/// Greedily word-wraps a title to `col_w`, returning each line with the size it draws at. A line is
/// normally `size_pt`; the one exception is a single word that is still wider than `col_w` on its own
/// (an unbreakable overflow), which is kept alone on its line and scaled down to fit rather than left to
/// overrun the rail.
fn wrap_title_lines(
	fonts:		&Arc<FontSet>,
	text:		&str,
	size_pt:	f32,
	col_w:		f32,
	smallcaps:	bool,
)
	-> Outcome<Vec<(String, f32)>>
{
	let mut lines: Vec<String> = Vec::new();
	let mut line = String::new();
	for word in text.split_whitespace() {
		let trial	= if line.is_empty() { word.to_string() } else { fmt!("{} {}", line, word) };
		let w		= res!(title_run_width(fonts, &trial, size_pt, smallcaps));
		if w > col_w && !line.is_empty() {
			lines.push(line.clone());
			line = word.to_string();
		} else {
			line = trial;
		}
	}
	if !line.is_empty() {
		lines.push(line);
	}
	if lines.is_empty() {
		lines.push(String::new());
	}

	let mut out: Vec<(String, f32)> = Vec::with_capacity(lines.len());
	for line in lines {
		let mut fit = size_pt;
		let w = res!(title_run_width(fonts, &line, fit, smallcaps));
		if w > col_w && w > 0.0 {
			fit = fit * col_w / w;	// shrink-to-fit: an unbreakable word wider than the rail
		}
		out.push((line, fit));
	}
	Ok(out)
}

/// Bakes a title or subtitle run to filled glyph outlines centred on `centre_x` at baseline `base_y`, in
/// the box frame (y down). Small caps are synthesised run by run as the banner sets them (was-lowercase
/// letters uppercased at 0.75 of the size); a plain run sets italic, matching the template's `emph`. The
/// advance is measured first so the run seats on its centre, then each glyph's y-up outline is flipped
/// onto the y-down baseline.
fn title_run_ops(
	ops:		&mut Vec<DrawOp>,
	fonts:		&Arc<FontSet>,
	text:		&str,
	size_pt:	f32,
	centre_x:	f32,
	base_y:		f32,
	smallcaps:	bool,
)
	-> Outcome<()>
{
	let size		= Sp::from_pt(size_pt as f64);
	let small_size	= Sp(size.raw() * 3 / 4);
	// Small caps sets upright (the template's `smallcaps`); a plain title sets italic (its `emph`).
	let face		= if smallcaps { HeadFace::Role(Role::Body) } else { HeadFace::Role(Role::Italic) };
	let runs		= if smallcaps { smallcaps_runs(text) } else { vec![(text.to_string(), false)] };

	// Total advance, so the run seats centred on `centre_x`.
	let total = res!(title_run_width(fonts, text, size_pt, smallcaps));

	let mut x = centre_x - total / 2.0;
	for (run, is_small) in &runs {
		let rs		= if *is_small { small_size } else { size };
		let shaped	= res!(head_shape(fonts, &face, rs, run));
		for glyph in &shaped.run().glyphs {
			let outline = res!(shaped.outline(glyph));
			if outline.is_empty() {
				continue;	// a space carries an advance but no ink
			}
			let t = Transform::scale(1.0, -1.0)
				.then(&Transform::translate(x + glyph.x, base_y - glyph.y));
			ops.push(DrawOp::Fill { path: res!(outline.transform(&t)), colour: Rgba::BLACK });
		}
		x += shaped.dims().width.to_pt() as f32;
	}
	Ok(())
}

/// Sets the imprint (meta) page: the publisher, edition, copyright, rights, AI declaration, website and
/// toolchain lines, set small in the lower half of the page as the template bottom-aligns them.
fn fm_meta_page(
	nodes:	&mut Vec<Node>,
	fonts:	&Arc<FontSet>,
	geom:	PageGeometry,
	style: &Theme,
	fm:		&FrontMatter,
)
	-> Outcome<()>
{
	let measure	= geom.content_width();
	let h		= geom.content_height();
	let size	= Sp(style.text.body_size.raw() * 4 / 5);	// the template's 0.8em imprint

	// Drop to the lower part of the page; the template bottom-aligns, approximated here by a top spacer.
	nodes.push(fm_spacer(Sp(h.raw() * 48 / 100)));

	let mut lines: Vec<String> = Vec::new();
	if let Some(p) = &fm.publisher		{ lines.push(p.clone()); }
	if let Some(e) = &fm.edition		{ lines.push(e.clone()); }
	if let Some(i) = &fm.isbn			{ lines.push(fmt!("ISBN {}", i)); }
	if let Some(c) = &fm.copyright		{ lines.push(c.clone()); }
	if let Some(r) = &fm.rights			{ lines.push(r.clone()); }
	if let Some(a) = &fm.ai_declaration	{ lines.push(a.clone()); }
	if let Some(w) = &fm.website		{ lines.push(w.clone()); }
	if fm.toolchain {
		lines.push("Created using Austenite (built using Rust) and Inkscape.".to_string());
	}

	let mut first = true;
	for line in &lines {
		if !first {
			nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
		}
		first = false;
		let broken = res!(break_paragraph(fonts.clone(), Role::Body, Dir::Ltr, size, line, measure, Sp(size.raw() * 6 / 5), true, Rgba::BLACK, None));
		nodes.extend(broken);
	}
	Ok(())
}

/// Sets the documentation template's meta/colophon page (`template.typ`'s `meta-page`): a bordered
/// Ver/Date/Author(s)/Notes table at the top carrying the one revision row -- its version, date, author
/// with the "Made with AI" declaration mark beneath the name, and its notes with the reading time
/// appended -- then, seated at the page foot, the acknowledgement paragraph, the copyright line, the
/// "created using" line and the footer logo. The template `place`s the foot block against the page
/// bottom; here the foot block is measured and a rigid spacer drops it there, so the whole page sets as
/// one flow without a second leaf. The footer logo is drawn into the page here rather than by `decorate`,
/// which seats the folio footer on body pages only and leaves the front matter clean.
fn fm_doc_meta_page(
	nodes:	&mut Vec<Node>,
	fonts:	&Arc<FontSet>,
	geom:	PageGeometry,
	style: &Theme,
	fm:		&FrontMatter,
)
	-> Outcome<()>
{
	let measure	= geom.content_width();
	let h		= geom.content_height();

	// The version table, at the very top of the content box, exactly as the template sets it flush under
	// the top margin.
	let table	= res!(build_meta_table(fm));
	let refs:	HashMap<String, String>	= HashMap::new();
	// The colophon table carries only plain text (version, date, author, notes), so its cells raise no
	// footnote, cross-reference, citation, index marker or claim anchor; throwaway counters and gathers
	// absorb what the shared cell path would record and are discarded with the front matter.
	let mut foot_no		= 0u32;
	let mut ref_no		= 0u32;
	let mut margin_no	= 0u32;
	let mut seen:	HashSet<String>	= HashSet::new();
	let mut idx			= IndexGather::default();
	let mut claim		= ClaimGather::default();
	let tnode	= res!(table::lower(
		fonts.clone(), geom, style, measure, &table,
		&mut foot_no, &mut ref_no, &mut margin_no, &mut seen, &mut idx, &mut claim, None, &refs));
	let table_h	= node_vext(&tnode);
	nodes.push(tnode);

	// The foot block: the acknowledgement, the copyright line, the toolchain line and the footer logo,
	// built into a buffer so its height is known and a spacer can drop it to the page foot. The gaps
	// between the four elements approximate the template's `place(bottom, dy: ..)` offsets.
	let mut foot:	Vec<Node>	= Vec::new();
	let mut foot_h				= Sp::ZERO;
	let gap						= Sp(style.text.body_size.raw() * 3 / 4);

	if let Some(ack) = &fm.acknowledgement {
		let size	= Sp(style.text.body_size.raw() * 85 / 100);
		let broken	= res!(break_paragraph(fonts.clone(), Role::Body, Dir::Ltr, size, ack, measure, Sp(size.raw() * 6 / 5), true, Rgba::BLACK, None));
		for n in &broken { foot_h += node_vext(n); }
		foot.extend(broken);
	}
	if let Some(cr) = &fm.copyright {
		foot.push(Node::Glue(Glue::fixed(gap)));
		foot_h += gap;
		let size	= style.text.body_size;
		let broken	= res!(break_paragraph(fonts.clone(), Role::Body, Dir::Ltr, size, cr, measure, Sp(size.raw() * 6 / 5), true, Rgba::BLACK, None));
		for n in &broken { foot_h += node_vext(n); }
		foot.extend(broken);
	}
	// The toolchain line, the template's fixed "created using" credit for the doc idiom.
	{
		foot.push(Node::Glue(Glue::fixed(gap)));
		foot_h += gap;
		let size	= Sp(style.text.body_size.raw() * 3 / 4);
		let line	= "This document was created using Austenite (built using Rust).";
		let broken	= res!(break_paragraph(fonts.clone(), Role::Body, Dir::Ltr, size, line, measure, Sp(size.raw() * 6 / 5), true, Rgba::BLACK, None));
		for n in &broken { foot_h += node_vext(n); }
		foot.extend(broken);
	}
	if let Some(path) = &fm.footer_logo {
		if let Ok(graphic) = image_at_height(fonts, path, 18.0) {
			let logo = Leaf::graphic(graphic);
			let lh	 = logo.dims.height + logo.dims.depth;
			let big	 = Sp(style.text.body_size.raw() * 3 / 2);	// a little more air above the logo
			foot.push(Node::Glue(Glue::fixed(big)));
			foot_h += big + lh;
			foot.push(Node::HBox(BoxNode::new(vec![Node::Leaf(logo)], Dims::new(measure, lh, Sp::ZERO))));
		}
	}

	// Drop the foot block to the page bottom: a rigid spacer taking up the slack between the table and the
	// foot. A page too short for both simply sets them adjacent rather than overflowing to a second leaf.
	let used = table_h + foot_h;
	if h > used {
		nodes.push(fm_spacer(h - used));
	}
	nodes.extend(foot);
	Ok(())
}

/// Builds the meta page's Ver/Date/Author(s)/Notes table from the doc's revision rows. A column every row
/// leaves blank is dropped (the template's `filled` test): Author and Notes always stand, Ver and Date
/// only when some row sets them. Each row's author cell carries the declaration mark stacked beneath the
/// name, and the last row's notes take the reading time appended -- matching the template's `meta-page`
/// table with its `2fr, 2fr, 4fr, 6fr` columns.
fn build_meta_table(fm: &FrontMatter) -> Outcome<Table> {
	let has_ver	= fm.meta_rows.iter().any(|r| r.version.as_deref().unwrap_or("") != "");
	let has_date	= fm.meta_rows.iter().any(|r| r.date.as_deref().unwrap_or("") != "");

	let mut weights:	Vec<f64>		= Vec::new();
	let mut header:		Vec<Cell>	= Vec::new();
	if has_ver {
		weights.push(2.0);
		header.push(Cell::rich(vec![Segment::strong("Ver")], Align::Centre));
	}
	if has_date {
		weights.push(2.0);
		header.push(Cell::rich(vec![Segment::strong("Date")], Align::Centre));
	}
	weights.push(4.0);
	header.push(Cell::rich(vec![Segment::strong("Author(s)")], Align::Left));
	weights.push(6.0);
	header.push(Cell::rich(vec![Segment::strong("Notes")], Align::Left));

	let mut rows = vec![Row::new(header)];
	let last = fm.meta_rows.len().saturating_sub(1);
	for (i, mr) in fm.meta_rows.iter().enumerate() {
		let mut cells: Vec<Cell> = Vec::new();
		if has_ver {
			cells.push(Cell::rich(vec![Segment::text(mr.version.clone().unwrap_or_default())], Align::Centre));
		}
		if has_date {
			cells.push(Cell::rich(vec![Segment::text(mr.date.clone().unwrap_or_default())], Align::Centre));
		}
		// The author cell carries the name and, where the row declares one, the AI mark beneath it.
		let author_cell = match (&mr.ai_mark_path, &mr.ai_mark_words) {
			(Some(path), Some(words)) => {
				let mark = crate::table::CellMark {
					path:	path.clone(),
					height:	Sp::from_pt(36.0),	// the template's `image(.., height: 36pt)`
					words:	words.clone(),
					url:	mr.ai_mark_url.clone(),
				};
				Cell::rich_with_mark(vec![Segment::text(mr.authors.clone())], Align::Left, mark)
			},
			_ => Cell::rich(vec![Segment::text(mr.authors.clone())], Align::Left),
		};
		cells.push(author_cell);
		// The reading time is appended to the last row's notes only, as the template does.
		let notes = mr.notes.clone().unwrap_or_default();
		let notes = match (i == last, fm.reading_min) {
			(true, Some(m)) => if notes.is_empty() {
				fmt!("Reading time: {} [min]", m)
			} else {
				fmt!("{} Reading time: {} [min]", notes, m)
			},
			_ => notes,
		};
		cells.push(Cell::rich(vec![Segment::text(notes)], Align::Left));
		rows.push(Row::new(cells));
	}

	Ok(Table::with_weights(true, rows, weights))
}

/// Counts the words in a block stream, matching the template's reading-time counter, which steps once per
/// maximal run of letters (`\p{L}+`) as the body renders. Every text-bearing block contributes -- prose,
/// headings, list items, table cells, figure captions, code and references -- so the tally tracks Typst's
/// own `words.final()` closely; the reading time is that count over the average reading speed.
pub(crate) fn count_words(blocks: &[Block]) -> usize {
	fn count_str(s: &str, n: &mut usize) {
		let mut in_word = false;
		for ch in s.chars() {
			if ch.is_alphabetic() {
				if !in_word { *n += 1; in_word = true; }
			} else {
				in_word = false;
			}
		}
	}
	fn count_segs(segs: &[Segment], n: &mut usize) {
		for seg in segs {
			match seg {
				Segment::Text(t) | Segment::Strong(t) | Segment::Emph(t) | Segment::BoldItalic(t)
				| Segment::Super(t) | Segment::Sub(t) | Segment::Code(t)	=> count_str(t, n),
				Segment::Glossary { display, .. }		=> count_str(display, n),
				Segment::Footnote { note }				=> count_segs(note, n),
				Segment::Cite(keys)						=> for k in keys { count_str(k, n); },
				Segment::PageRef(_) | Segment::Math(_) | Segment::MarginNote { .. } | Segment::Index { .. }	=> {},
			}
		}
	}
	fn count_cells(table: &Table, n: &mut usize) {
		for row in &table.rows {
			for cell in &row.cells {
				count_segs(&cell.content, n);
			}
		}
	}
	let mut n = 0usize;
	for b in blocks {
		match b {
			Block::Heading { segments, .. }		=> count_segs(segments, &mut n),
			Block::Paragraph { text }			=> count_str(text, &mut n),
			Block::RichParagraph { segments }	=> count_segs(segments, &mut n),
			Block::List { items, .. }			=> for it in items {
				count_segs(&it.segments, &mut n);
				n += count_words(&it.children);
			},
			Block::Code { lines }				=> for l in lines { count_str(l, &mut n); },
			Block::Table(t)						=> count_cells(t, &mut n),
			Block::Figure { caption, .. }		=> if let Some(c) = caption { count_str(c, &mut n); },
			Block::TableFigure { table, caption, .. } => {
				count_cells(table, &mut n);
				if let Some(c) = caption { count_segs(c, &mut n); }
			},
			Block::ImageFigure { caption, .. } | Block::CodeFigure { caption, .. }
												=> if let Some(c) = caption { count_segs(c, &mut n); },
			Block::BackMatterHeading { title }	=> count_str(title, &mut n),
			Block::Reference { runs }			=> for (t, _) in runs { count_str(t, &mut n); },
			Block::Box { blocks, .. }			=> n += count_words(blocks),
			// A scope carries its words in its own nested blocks, counted here rather than as flat siblings.
			Block::Scoped { blocks, .. }		=> n += count_words(blocks),
			Block::Equation { .. } | Block::Rule { .. } | Block::Image { .. }
			| Block::SectionBanner { .. } | Block::Glossary | Block::Index | Block::ClaimIndex
			| Block::Space(_) | Block::PageBreak	=> {},
		}
	}
	n
}

/// The vertical extent a node occupies in a flow: a box's height plus depth, a glue's natural size, a
/// leaf's height plus depth. Anchors and penalties take no space.
fn node_vext(n: &Node) -> Sp {
	match n {
		Node::HBox(b) | Node::VBox(b)	=> b.dims.height + b.dims.depth,
		Node::Leaf(l)					=> l.dims.height + l.dims.depth,
		Node::Glue(g)					=> g.natural,
		_								=> Sp::ZERO,
	}
}

/// Sets the dedication page: the dedication centred, in italic, about the vertical centre.
fn fm_dedication_page(
	nodes:	&mut Vec<Node>,
	fonts:	&Arc<FontSet>,
	geom:	PageGeometry,
	style: &Theme,
	text:	&str,
)
	-> Outcome<()>
{
	let measure	= geom.content_width();
	let h		= geom.content_height();
	nodes.push(fm_spacer(Sp(h.raw() * 40 / 100)));
	let size	= Sp(style.text.body_size.raw() * 11 / 10);
	res!(fm_centred_wrap(nodes, fonts, Role::Italic, size, text, measure, Sp(size.raw() * 6 / 5)));
	Ok(())
}

/// Sets the "About the Author" page: the title in the display face, then the biography justified below.
fn fm_about_author_page(
	nodes:		&mut Vec<Node>,
	fonts:		&Arc<FontSet>,
	display:	Option<&Arc<Font>>,
	geom:		PageGeometry,
	style: &Theme,
	title_size:	Sp,
	bio:		&str,
)
	-> Outcome<()>
{
	let measure	= geom.content_width();
	nodes.push(fm_spacer(Sp::from_pt(24.0)));
	let title_face	= display.map(HeadFace::Solo).unwrap_or(HeadFace::Role(Role::Bold));
	let title = res!(head_shape(fonts, &title_face, title_size, "About the Author"));
	let td = title.dims();
	nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::text(title))], td)));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(18.0))));
	let size	= Sp(style.text.body_size.raw() * 9 / 10);
	let broken	= res!(break_paragraph(fonts.clone(), Role::Body, Dir::Ltr, size, bio, measure, Sp(size.raw() * 7 / 5), true, Rgba::BLACK, None));
	nodes.extend(broken);
	Ok(())
}

/// The deepest heading level the contents lists, matching the template's `outline(depth: 3)`.
const TOC_DEPTH: u8 = 3;

/// Sets a table of contents from the heading table: the "Contents" title in the display face, then one
/// entry per heading -- its number in a column indented by level, its title, a dotted leader, and its
/// printed folio flush at the right. The folio is a forward reference resolved with [`Ref::FolioOf`]
/// against the incoming ledger, so it reads the body folio (which restarts at one) rather than the
/// physical page, reusing the same reserve-then-resolve slot the driver runs for any forward reference.
/// The caller prepends these nodes after the front matter; a trailing forced break opens the body.
///
/// A fact a reader could not derive. Each entry reserves a fixed slot for its folio -- three digits
/// wide, so a resolved number never outgrows it -- and its line height is the entry's, whatever the
/// folio turns out to be, and the dotted leader takes the width left over. The contents block therefore
/// has a constant vertical extent from the first pass, so the body it displaces settles once and the
/// forward references converge in the usual two passes, with no special case in the driver.
pub fn contents(
	fonts:		Arc<FontSet>,
	faces:		&FaceResolver,
	geom:		PageGeometry,
	style: &Theme,
	title_size:	Sp,
	heads:		&[Heading],
)
	-> Outcome<Vec<Node>>
{
	let measure			= geom.content_width();
	let mut nodes:	Vec<Node> = Vec::new();

	// A `Label` anchor at the top of the contents leaf records its page for the PDF outline. It sets no
	// heading, so the contents is neither a running-head section nor an entry in its own list.
	nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Label, "frontmatter:contents")));

	// The block's own heading, in the display face at the back-matter title size, recorded as no anchor --
	// so it is neither a running-head section nor an entry in its own list.
	let title	= res!(head_shape(&fonts, &resolved_head_face(1, style, faces, is_doc_heading(style)), title_size, "Contents"));
	let td		= title.dims();
	nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::text(title))], td)));
	nodes.push(Node::Glue(Glue::fixed(style.space_below(1))));

	// A fixed slot wide enough for a three-digit folio, so a resolved number never overflows its
	// reservation and every entry keeps a constant height across passes.
	let slot	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, "000"));
	let slot_w	= slot.dims().width;
	// A dot-and-space leader unit, measured once, so a leader is filled with a whole number of dots.
	let dot		= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, ". "));
	let dot_w	= dot.dims().width.raw().max(1);
	// The step a level indents the number column by, and the gap between a number and its title.
	let step	= Sp(style.text.body_size.raw() * 3 / 2);
	let gap		= Sp(style.text.body_size.raw() * 3 / 5);

	for (i, h) in heads.iter().enumerate() {
		// The template sets `outline(depth: 3)`, so the contents stops at level 3 (a `===` subsection,
		// dotted number x.y.z); a level-4 `====` heading is listed in no contents and is skipped here.
		if h.level > TOC_DEPTH {
			continue;
		}
		// The number column is indented per level: a part (level 0) and a chapter (level 1) sit at the
		// margin, deeper levels step right. The number is empty for a part, which then shows title alone.
		let depth	= (h.level.max(1) - 1) as i32;
		let indent	= step * depth;
		let numw	= if h.number.is_empty() {
			Sp::ZERO
		} else {
			let n = res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &h.number));
			n.dims().width
		};
		// The entry's title set from its rich runs, so a maths span or emphasis in a heading renders here
		// rather than dropping to a gap. The height is the sample's, constant across passes.
		let (entry, ed)	= res!(inline_segments(&fonts, style, &h.segments, Role::Body, style.text.body_size));

		// The leader span from the title's end to the folio slot; a title too wide to leave a one-em
		// minimum keeps that minimum and runs under its folio -- the over-wide case, left as it falls.
		let num_col	= if numw.raw() > 0 { numw + gap } else { Sp::ZERO };
		let taken	= indent + num_col + ed.width + slot_w;
		let min_lead	= style.text.body_size;
		let leader_w	= if measure > taken + min_lead { measure - taken } else { min_lead };

		// Fill the leader with a whole number of dots, padded to the folio slot on the right so the slot's
		// right edge falls on the measure.
		let lead_margin	= Sp(style.text.body_size.raw() / 2);
		let usable		= (leader_w.raw() - lead_margin.raw()).max(0);
		let n_dots		= (usable / dot_w).max(0) as usize;
		let dots		= res!(ShapedText::new(
			fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &". ".repeat(n_dots)));
		let dots_w		= dots.dims().width;
		let trailing	= if leader_w > lead_margin + dots_w { leader_w - lead_margin - dots_w } else { Sp::ZERO };

		// The entry's own identity, distinct from the heading it points at, so recording the slot never
		// overwrites the heading's ledger row. Its reference resolves the heading's folio.
		let toc_id		= AnchorId::new(AnchorKind::Label, fmt!("toc-{}", h.id.key));
		let slot_dims	= Dims::new(slot_w, ed.height, ed.depth);

		let mut children:	Vec<Node> = Vec::new();
		if indent.raw() > 0 {
			children.push(Node::Glue(Glue::fixed(indent)));
		}
		if numw.raw() > 0 {
			children.push(Node::Leaf(Leaf::text(res!(
				ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &h.number)))));
			children.push(Node::Glue(Glue::fixed(gap)));
		}
		children.extend(entry);
		children.push(Node::Glue(Glue::fixed(lead_margin)));
		children.push(Node::Leaf(Leaf::text(dots)));
		if trailing.raw() > 0 {
			children.push(Node::Glue(Glue::fixed(trailing)));
		}
		children.push(Node::Leaf(Leaf::reserved(toc_id, Ref::FolioOf(h.id.clone()), slot_dims)));

		let line_dims = Dims::new(measure, ed.height, ed.depth);
		nodes.push(Node::HBox(BoxNode::new(children, line_dims)));

		// Leading between entries, but not after the last.
		if i + 1 < heads.len() {
			let vextent	= ed.height + ed.depth;
			let lead	= if style.text.leading > vextent { style.text.leading - vextent } else { Sp::ZERO };
			nodes.push(Node::Glue(Glue::fixed(lead)));
		}
	}

	// The contents stands alone at the front; the body opens on a fresh page.
	nodes.push(Node::Penalty(Penalty::eject()));
	Ok(nodes)
}

/// Sets the back-matter index from the markers gathered walking the body: one entry per index term,
/// alphabetical and case-insensitive, its display followed by the folio list its occurrences resolved to.
/// A nested marker (`#idx-nested("extraction", "Roman")`) sets its child term as an indented sub-entry
/// under its parent. Each entry's page list is a forward reference -- an [`IndexFolios`](Ref::IndexFolios)
/// slot the driver resolves against the previous pass's ledger, deduplicating and run-compressing the
/// folios -- so the index reads the pages its terms fell on without a layout query.
///
/// A fact a reader could not derive: each entry reserves a fixed slot wide enough for its occurrences'
/// folios set uncompressed ("999, " apiece), so a resolved (compressed) list never outgrows it and the
/// section's extent is settled from the first pass. An over-long single entry runs past the column rather
/// than wrapping, the same over-wide case the table of contents leaves as it falls. `measure` is the width
/// of one column: the caller wraps the returned entries in a [`Node::Columns`], and the driver flows them
/// down each column in turn, so this sets every entry to the column width, matching the Typst template's
/// two-column `print-index`.
fn index_nodes(
	fonts:	&Arc<FontSet>,
	style: &Theme,
	measure:	Sp,
	occ:	&[(String, Option<String>, Vec<Segment>, AnchorId, bool)],
)
	-> Outcome<Vec<Node>>
{
	// Group by term (case-insensitive), each term carrying its own direct occurrences and its nested
	// children, so a term with sub-entries lists them indented beneath it. Each occurrence carries whether it
	// is a primary (`#idx-main`) reference, so its folio sets bold. The folios are deduplicated and sorted at
	// resolution, so document order within a group need not be kept here. `display` is the styled text the
	// index page sets -- the first occurrence's, since every mention of one term carries the same -- so a
	// `#idx-as[March, James][James March]` entry prints "James March" and an emphasised case name sets italic,
	// rather than the sort key leaking to the page.
	struct Group {
		display:	Vec<Segment>,
		direct:		Vec<(AnchorId, bool)>,
		subs:		std::collections::BTreeMap<String, (String, Vec<(AnchorId, bool)>)>,
	}
	let mut groups: std::collections::BTreeMap<String, Group> = std::collections::BTreeMap::new();
	for (term, sub, display, id, main) in occ {
		let g = groups.entry(term.to_lowercase()).or_insert_with(|| Group {
			display:	display.clone(),
			direct:		Vec::new(),
			subs:		std::collections::BTreeMap::new(),
		});
		match sub {
			None		=> g.direct.push((id.clone(), *main)),
			Some(child)	=> {
				let e = g.subs.entry(child.to_lowercase()).or_insert_with(|| (child.clone(), Vec::new()));
				e.1.push((id.clone(), *main));
			},
		}
	}

	// The index sets at 9pt, as Typst's `print-index` does with `set text(size: 9pt)`, a little below the
	// body so more entries fit a column. Its leading keeps the body's line-to-size ratio at the smaller
	// size; a 1.5em gap parts one alphabetic section from the next (Typst's `spaced-section` `v(1.5em)`),
	// and entries within a section are parted by that ordinary leading. The single-line entry HBoxes are
	// already ragged (no justification), matching the template's `set par(justify: false)`.
	let body		= Sp::from_pt(9.0);
	// The index leading keeps the body's line-to-size ratio at the smaller size. Computed in i64 so the
	// scaled-point product does not overflow i32 before the divide brings it back into range.
	let idx_lead	= if style.text.body_size.raw() > 0 {
		Sp(((body.raw() as i64 * style.text.leading.raw() as i64) / style.text.body_size.raw() as i64) as i32)
	} else {
		body
	};
	let entry_lead	= if idx_lead > body { idx_lead - body } else { Sp::ZERO };
	let section_gap	= Sp(body.raw() * 3 / 2);	// 1.5em at the index size: Typst's per-section v(1.5em)
	let step		= Sp(body.raw() * 3 / 2);	// the indent a nested sub-entry sets in by
	// One occurrence's worst-case folio width ("999, "), so a compressed list never outgrows its slot.
	let unit		= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, body, "999, ")).dims().width;

	let mut nodes:	Vec<Node>	= Vec::new();
	let mut ref_no				= 0u32;
	let mut prev_letter: Option<char> = None;
	for (key, g) in groups.iter() {
		let letter = key.chars().next().map(|c| c.to_ascii_uppercase());
		if !nodes.is_empty() {
			// Part alphabetic sections by 1.5em, entries within a section by the ordinary index leading.
			let lead = if letter != prev_letter { section_gap } else { entry_lead };
			nodes.push(Node::Glue(Glue::fixed(lead)));
		}
		prev_letter = letter;
		res!(index_entry_line(fonts, body, measure, &g.display, &g.direct, 0, unit, step, &mut ref_no, &mut nodes));
		for (_, (disp, ids)) in &g.subs {
			nodes.push(Node::Glue(Glue::fixed(entry_lead)));	// a sub-entry sits one leading below its fellow
			let sub_display = vec![Segment::text(disp.clone())];
			res!(index_entry_line(fonts, body, measure, &sub_display, ids, 1, unit, step, &mut ref_no, &mut nodes));
		}
	}
	Ok(nodes)
}

/// The index text size's face and text for one display segment: an emphasised run sets italic, a strong
/// run bold, a bold-italic run both, and everything else (plain text, a code span whose mono face the index
/// does not reproduce, any richer segment flattened to its words) sets in the body face. This is what lets
/// `_Browder v. Gayle_` reach the index page as an italic case name rather than literal underscores.
fn index_display_run(seg: &Segment) -> (String, Role) {
	match seg {
		Segment::Text(t)		=> (t.clone(), Role::Body),
		Segment::Emph(t)		=> (t.clone(), Role::Italic),
		Segment::Strong(t)		=> (t.clone(), Role::Bold),
		Segment::BoldItalic(t)	=> (t.clone(), Role::BoldItalic),
		other					=> (flatten_segments(std::slice::from_ref(other)), Role::Body),
	}
}

/// Sets one index entry line: the styled display, indented by `depth`, then -- when the term has any page --
/// a comma, a space and the folio list the driver resolves. A term with only nested children (no direct
/// page) sets its name alone, a heading for the indented sub-entries beneath it. The comma before the folios
/// is the in-dexter separator (`entry, page`), and the display sets in its own faces, so an emphasised entry
/// italicises. `ref_no` makes each slot's own ledger identity unique.
///
/// The folios split into non-main (set in the body face) and main (`#idx-main`, set bold, reproducing
/// in-dexter's `index-main = index.with(fmt: strong)`); each group resolves as its own
/// [`IndexFolios`](Ref::IndexFolios) slot -- deduplicated, sorted and run-compressed independently -- and
/// the two are parted by `", "` when both are present. in-dexter interleaves a term's main and plain folios
/// by document order rather than grouping them; the two coincide for an entry whose references are all main
/// or all plain (the common case, and every entry in the oracle corpus), and differ only in the order of a
/// single entry that mixes the two, which no fixture yet exercises.
#[allow(clippy::too_many_arguments)]
fn index_entry_line(
	fonts:		&Arc<FontSet>,
	body:		Sp,	// the index text size (9pt), the term set and the line box sized at it
	measure:	Sp,
	display:	&[Segment],
	ids:		&[(AnchorId, bool)],
	depth:		i32,
	unit:		Sp,
	step:		Sp,
	ref_no:		&mut u32,
	nodes:		&mut Vec<Node>,
)
	-> Outcome<()>
{
	let indent	= step * depth;

	let mut children: Vec<Node> = Vec::new();
	if indent.raw() > 0 {
		children.push(Node::Glue(Glue::fixed(indent)));
	}
	// The display runs, each shaped in its own face; the line's extent is the tallest run's.
	let mut height	= Sp::ZERO;
	let mut depth_	= Sp::ZERO;
	for seg in display {
		let (text, role) = index_display_run(seg);
		if text.is_empty() {
			continue;
		}
		let shaped	= res!(ShapedText::new(fonts.clone(), role, Dir::Ltr, body, &text));
		let sd		= shaped.dims();
		if sd.height > height { height = sd.height; }
		if sd.depth > depth_ { depth_ = sd.depth; }
		children.push(Node::Leaf(Leaf::text(shaped)));
	}
	if !ids.is_empty() {
		// Split into non-main and main folios, keeping each group's document order (the sort happens at
		// resolution). A main folio sets bold, a non-main folio in the body face.
		let plain: Vec<AnchorId>	= ids.iter().filter(|(_, m)| !*m).map(|(id, _)| id.clone()).collect();
		let main:  Vec<AnchorId>	= ids.iter().filter(|(_, m)|  *m).map(|(id, _)| id.clone()).collect();
		// The bold folio's worst-case width ("999, " in the bold face), so a bold slot never outgrows it.
		let bold_unit	= res!(ShapedText::new(fonts.clone(), Role::Bold, Dir::Ltr, body, "999, ")).dims().width;

		// The in-dexter separator between an entry and its folios is a comma and a space, not a bare gap.
		let sep		= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, body, ", "));
		let sepd	= sep.dims();
		if sepd.height > height { height = sepd.height; }
		if sepd.depth > depth_ { depth_ = sepd.depth; }
		children.push(Node::Leaf(Leaf::text(sep.clone())));

		// The non-main folios, set in the body face.
		if !plain.is_empty() {
			*ref_no += 1;
			let slot_w	= Sp(unit.raw() * plain.len() as i32);
			let id		= AnchorId::new(AnchorKind::Label, fmt!("index-slot-{}", *ref_no));
			let dims	= Dims::new(slot_w, height, depth_);
			children.push(Node::Leaf(Leaf::reserved_inline(id, Ref::IndexFolios(plain), dims)));
		}
		// The main folios, set bold. When both groups are present, a `", "` parts them.
		if !main.is_empty() {
			if children.last().map_or(false, |n| matches!(n, Node::Leaf(l) if matches!(l.kind, LeafKind::Reserved(..)))) {
				children.push(Node::Leaf(Leaf::text(sep)));
			}
			*ref_no += 1;
			let slot_w	= Sp(bold_unit.raw() * main.len() as i32);
			let id		= AnchorId::new(AnchorKind::Label, fmt!("index-slot-{}", *ref_no));
			let dims	= Dims::new(slot_w, height, depth_);
			children.push(Node::Leaf(Leaf::reserved_inline_bold(id, Ref::IndexFolios(main), dims)));
		}
	}
	let line_dims = Dims::new(measure, height, depth_);
	nodes.push(Node::HBox(BoxNode::new(children, line_dims)));
	Ok(())
}

/// Sets the reverse claim-reference index: one wrapped paragraph per code, the codes in byte order (Typst's
/// `.sorted()`), each the bold code, a colon and space, then the pages the code was referenced on -- one
/// reserved [`FolioOf`](Ref::FolioOf) slot per reference in document order, parted by `", "`, and a closing
/// full stop. This is the Logic appendix's `[#strong(code): #pages.join(", ").]` line for line, and the
/// per-reference slots (rather than one range-compressed slot) are what let a long page list wrap across
/// lines the way Typst's does, so the section runs to the same length. Entries stack at the body's own
/// interline pitch, matching Typst's `linebreak()` between them. `measure` is the full text measure, since
/// the appendix sets the index in a single column.
fn claim_index_nodes(
	fonts:	&Arc<FontSet>,
	style: &Theme,
	measure:	Sp,
	occ:	&[(String, AnchorId)],
)
	-> Outcome<Vec<Node>>
{
	// Group by code, keeping each code's references in the order they were gathered (document order), so the
	// page list reads in the order Typst's query returns them. Byte order over the keys matches `.sorted()`.
	let mut groups: std::collections::BTreeMap<String, Vec<AnchorId>> = std::collections::BTreeMap::new();
	for (code, id) in occ {
		groups.entry(code.clone()).or_default().push(id.clone());
	}

	let size	= style.text.body_size;
	let leading	= style.text.leading;

	// One folio slot's reserved width: three digits at the index size, so a resolved page never outgrows it.
	// Keyed `claim-slot-{n}` -- its OWN prefix, distinct from `ref_slot`'s `ref-{n}` (a body cross-reference)
	// and `index_nodes`'s `index-slot-{n}` -- so a book carrying a `#pageref`-style slot AND a claim index does
	// not have one silently overwrite the other in the ledger's by-id map.
	let slot_probe	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, "000"));
	let slot_dims	= slot_probe.dims();
	let mut nodes:	Vec<Node>	= Vec::new();
	let mut slot_no				= 0u32;
	// The depth of the previous entry's last set line, so the glue to the next entry seats its baseline
	// `leading` below -- the body's own interline pitch, exactly the rule `set_lines` applies within a
	// paragraph. Reading the real box metrics (not a probe glyph's) is what keeps the entries at body leading:
	// the block-edge model caps a single-line entry's own height and depth, so a fixed probe-derived gap would
	// stack them far tighter than the body sets its lines.
	let mut prev_depth: Option<Sp>	= None;
	for (code, ids) in groups.iter() {
		let mut pieces: Vec<Piece> = Vec::with_capacity(ids.len() * 2 + 3);
		pieces.push(Piece::Text { text: code.clone(), role: Role::Bold });
		pieces.push(Piece::Text { text: ": ".to_string(), role: Role::Body });
		for (n, id) in ids.iter().enumerate() {
			if n > 0 {
				pieces.push(Piece::Text { text: ", ".to_string(), role: Role::Body });
			}
			slot_no += 1;
			let slot_id	= AnchorId::new(AnchorKind::Label, fmt!("claim-slot-{}", slot_no));
			let leaf	= Leaf::reserved_inline(slot_id, Ref::FolioOf(id.clone()),
				Dims::new(slot_dims.width, slot_dims.height, slot_dims.depth));
			pieces.push(Piece::Mark(leaf));
		}
		pieces.push(Piece::Text { text: ".".to_string(), role: Role::Body });
		let lines = res!(break_paragraph_pieces(
			fonts.clone(), Role::Body, Dir::Ltr, size, &pieces, measure, leading,
			style.text.justify, style.text.hyphenate, style.text.fill, Some(cap_edge(style, size))));
		// This entry's first set line's height and last set line's depth, from the boxes as they will draw.
		let first_h	= lines.iter().find_map(|n| if let Node::HBox(b) = n { Some(b.dims.height) } else { None }).unwrap_or(Sp::ZERO);
		let last_d	= lines.iter().rev().find_map(|n| if let Node::HBox(b) = n { Some(b.dims.depth) } else { None }).unwrap_or(Sp::ZERO);
		if let Some(pd) = prev_depth {
			// Seat this entry's baseline `leading` below the previous entry's: gap = leading - depth above -
			// height below, the same measure `set_lines` uses between two lines of one paragraph.
			let want	= leading - pd - first_h;
			let gap		= if want > Sp::ZERO { want } else { Sp::ZERO };
			nodes.push(Node::Glue(Glue::fixed(gap)));
		}
		nodes.extend(lines);
		prev_depth = Some(last_d);
	}
	Ok(nodes)
}

/// Sets one bibliography reference: its runs woven into justified lines at the book template's
/// body-relative reference size, with a hanging indent -- the first line flush left, every continuation
/// line indented, as a Chicago reference list sets. The runs' italic flag chooses the face, so a book or
/// journal title sets italic. The template's `set text(size: 0.85em)` on the bibliography (see the book
/// template) sizes it a touch below the body, not at the footnote furniture size, and its leading follows
/// the body's proportionally: 0.85 of the body baseline, since both the line box and the paragraph gap
/// scale with the font size.
fn reference_block(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	style: &Theme,
	measure:	Sp,
	runs:		&[(String, bool)],
)
	-> Outcome<()>
{
	let ref_size	= Sp(style.text.body_size.raw() * 85 / 100);	// the template's 0.85em bibliography size
	let ref_leading	= Sp(style.text.leading.raw() * 85 / 100);	// the body baseline scaled by the same 0.85
	let hang	= Sp(ref_size.raw() * 3 / 2);	// the 1.5 em hang the continuation lines take, at the reference size
	let inner	= if measure > hang { measure - hang } else { measure };

	let mut pieces: Vec<Piece> = Vec::with_capacity(runs.len());
	for (text, italic) in runs {
		let role = if *italic { Role::Italic } else { Role::Body };
		pieces.push(Piece::Text { text: text.clone(), role });
	}
	let mut lines = res!(break_paragraph_pieces(
		fonts.clone(), Role::Body, Dir::Ltr, ref_size, &pieces, inner, ref_leading, true, true, Rgba::BLACK,
		Some(cap_edge(style, ref_size))));

	// Indent every line but the first by the hang, so the entry hangs under its first line.
	let mut first = true;
	for line in lines.iter_mut() {
		if let Node::HBox(b) = line {
			if first {
				first = false;
			} else {
				b.list.insert(0, Node::Glue(Glue::fixed(hang)));
				b.dims = Dims::new(b.dims.width + hang, b.dims.height, b.dims.depth);
			}
		}
	}
	nodes.extend(lines);
	Ok(())
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

/// The plain display words of a heading's rich runs: the text a reader sees with the markup removed, so
/// the anchor slug, the table-of-contents entry and the running head read the rendered title rather than
/// its raw source. A glossary term contributes its display, emphasis and code their inner words; a maths
/// span, a cross-reference, a footnote and a citation have no plain form here and contribute nothing.
fn flatten_segments(segments: &[Segment]) -> String {
	let mut out = String::new();
	for seg in segments {
		match seg {
			Segment::Text(t)				=> out.push_str(t),
			Segment::Strong(t)				=> out.push_str(t),
			Segment::Emph(t)				=> out.push_str(t),
			Segment::BoldItalic(t)			=> out.push_str(t),
			Segment::Super(t)				=> out.push_str(t),
			Segment::Sub(t)					=> out.push_str(t),
			Segment::Code(t)				=> out.push_str(t),
			Segment::Glossary { display, .. }	=> out.push_str(display),
			Segment::Math(_)				=> {},
			Segment::PageRef(_)				=> {},
			Segment::Footnote { .. }		=> {},
			Segment::Cite(_)				=> {},
			Segment::MarginNote { .. }			=> {},	// the margin code is not part of the flattened body text
			Segment::Index { .. }	=> {},	// an index marker is not part of the flattened body text
		}
	}
	out
}

/// Sets a title's rich runs into one horizontal line at `size` in `role`, so a running head or a
/// table-of-contents entry renders the title's maths, emphasis and glossary term rather than flattening
/// them to plain words or dropping the maths to a gap. Every run seats on a common baseline taken from a
/// full-size sample; a maths span is set at `size` and its glyphs woven into the line as the body sets
/// inline maths. A cross-reference, footnote or citation in a title has no form here and is dropped. The
/// returned dims carry the line's total width and the sample's ascent and depth, so a caller lays it out
/// with a constant height whatever the runs turn out to be.
fn inline_segments(
	fonts:		&Arc<FontSet>,
	style: &Theme,
	segments:	&[Segment],
	role:		Role,
	size:		Sp,
)
	-> Outcome<(Vec<Node>, Dims)>
{
	let sample	= res!(ShapedText::new(fonts.clone(), role, Dir::Ltr, size, "Ag"));
	let asc		= sample.dims().height;
	let dep		= sample.dims().depth;
	let italic	= role == Role::Italic;

	let mut children:	Vec<Node> = Vec::new();
	let mut width		= Sp::ZERO;
	for seg in segments {
		// A maths span is unwrapped and its leaves woven straight into the line; every other run resolves to
		// a text string set in a face chosen against the base role, so an emphasis in an italic running head
		// toggles upright as Typst sets it.
		let (text, r): (&str, Role) = match seg {
			Segment::Text(t)		=> (t, role),
			Segment::Strong(t)		=> (t, if italic { Role::BoldItalic } else { Role::Bold }),
			Segment::Emph(t)		=> (t, if italic { Role::Body } else { Role::Italic }),
			Segment::BoldItalic(t)	=> (t, if italic { Role::Bold } else { Role::BoldItalic }),
			Segment::Super(t)		=> (t, role),
			Segment::Sub(t)			=> (t, role),
			Segment::Code(t)		=> (t, Role::Mono),
			Segment::Glossary { display, .. }	=> (display, role),
			Segment::Math(atom)	=> {
				let mut hs = style.clone();
				hs.text.body_size = size;
				if let Node::HBox(b) = res!(math::layout(fonts.clone(), &hs, atom, false)) {
					width += b.dims.width;
					children.extend(b.list);
				}
				continue;
			},
			Segment::PageRef(_) | Segment::Footnote { .. } | Segment::Cite(_) | Segment::MarginNote { .. } | Segment::Index { .. }	=> continue,
		};
		let sh	= res!(ShapedText::new(fonts.clone(), r, Dir::Ltr, size, text));
		let w	= sh.dims().width;
		children.push(Node::Leaf(Leaf::text_dims(sh, Dims::new(w, asc, dep))));
		width += w;
	}
	Ok((children, Dims::new(width, asc, dep)))
}

/// Places a horizontal run of leaves (a rich running head from [`inline_segments`]) into a page frame,
/// starting at `x0` with `top` the run's box top. It mirrors the driver's own line placement: a text or
/// graphic leaf lands at the running x plus its own shift, glue advances the cursor, and a reserved or
/// rule leaf -- neither of which a heading run holds -- is skipped.
fn place_run(frame: &mut Frame, nodes: &[Node], x0: Sp, top: Sp) {
	let mut x = x0;
	for n in nodes {
		match n {
			Node::Leaf(l) => {
				let y = top + l.shift;
				match &l.kind {
					LeafKind::Text(sh)		=> frame.push(Placed::new(x, y, l.dims, PlacedKind::Text(sh.clone()))),
					LeafKind::Graphic(g)	=> frame.push(Placed::new(x, y, l.dims, PlacedKind::Graphic(g.clone()))),
					_						=> {},
				}
				x += l.dims.width;
			},
			Node::Glue(g)	=> x += g.natural,
			_				=> {},
		}
	}
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

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ HEADINGS                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// The number shown before a heading: the chapter number alone for a chapter (level 1), the dotted path
/// for a deeper level (`2.3.1`), and nothing for a part divider (level 0).
fn heading_number(level: u8, sec: &[u32; 6]) -> String {
	match level {
		0 => String::new(),
		1 => fmt!("{}", sec[0]),
		_ => {
			let l = (level as usize).min(6);
			let parts: Vec<String> = sec[..l].iter().map(|n| fmt!("{}", n)).collect();
			parts.join(".")
		},
	}
}

/// The number shown before a heading of `level`, honouring a per-level Typst numbering pattern the theme
/// carries (what `#set heading(numbering: "1.1")` lowers into every level, or a per-level override); with
/// no pattern -- the default -- it falls to the plain dotted arabic of [`heading_number`], so an untouched
/// theme renders unchanged. A part divider (level 0) carries no number.
fn heading_number_themed(level: u8, sec: &[u32; 6], style: &Theme) -> String {
	if level == 0 {
		return String::new();
	}
	let idx = (level as usize).saturating_sub(1).min(style.heading.levels.len().saturating_sub(1));
	if let Some(pattern) = style.heading.levels.get(idx).and_then(|l| l.numbering.as_deref()) {
		let l = (level as usize).min(6);
		return format_numbering(pattern, &sec[..l]);
	}
	heading_number(level, sec)
}

/// Renders a Typst numbering pattern against a list of counter values, as `numbering(pattern, ..nums)`
/// does: each counting symbol (`1`, `a`, `A`, `i`, `I`) consumes one number and renders it in that
/// system, the literal text between symbols is kept, and when there are more numbers than symbols the
/// last symbol and its leading literal repeat -- so `"1.1"` over `[1, 2, 3]` gives `1.2.3`. A pattern with
/// no counting symbol is a fixed literal, returned unchanged.
fn format_numbering(pattern: &str, nums: &[u32]) -> String {
	// Each piece is the literal text leading up to one counting symbol; `suffix` is the literal after the
	// last symbol.
	let mut pieces:	Vec<(String, char)>	= Vec::new();
	let mut prefix						= String::new();
	for c in pattern.chars() {
		if matches!(c, '1' | 'a' | 'A' | 'i' | 'I') {
			pieces.push((std::mem::take(&mut prefix), c));
		} else {
			prefix.push(c);
		}
	}
	let suffix = prefix;
	if pieces.is_empty() {
		return pattern.to_string();
	}
	let mut out = String::new();
	for (k, n) in nums.iter().enumerate() {
		let (pre, sym) = &pieces[k.min(pieces.len() - 1)];
		out.push_str(pre);
		out.push_str(&numbering_symbol(*sym, *n));
	}
	out.push_str(&suffix);
	out
}

/// One counter value rendered in the system a Typst counting symbol names.
fn numbering_symbol(symbol: char, n: u32) -> String {
	match symbol {
		'1'	=> fmt!("{}", n),
		'a'	=> alpha_label(n, false),
		'A'	=> alpha_label(n, true),
		'i'	=> roman(n).to_lowercase(),
		'I'	=> roman(n),
		_	=> fmt!("{}", n),
	}
}

/// A bijective base-26 letter label: 1 -> `a`, 26 -> `z`, 27 -> `aa`, uppercased when `upper` -- Typst's
/// `"a"`/`"A"` counting symbols. Zero renders empty, as an unstarted counter does.
fn alpha_label(mut n: u32, upper: bool) -> String {
	if n == 0 {
		return String::new();
	}
	let base = if upper { b'A' } else { b'a' };
	let mut chars: Vec<char> = Vec::new();
	while n > 0 {
		let rem = ((n - 1) % 26) as u8;
		chars.push((base + rem) as char);
		n = (n - 1) / 26;
	}
	chars.iter().rev().collect()
}

/// A heading run's face: the display face (Radley) a book supplies for its chapters and level-2
/// sections, or a reading-set role for the finer levels.
#[derive(Clone, Copy)]
enum HeadFace<'a> {
	Solo(&'a Arc<Font>),
	Role(Role),
}

/// The face a heading level sets in. Levels 0-2 take the display face when the book supplies one, else
/// the body bold; level 3 is Libertinus italic and level 4+ Libertinus upright -- the template's
/// `if it.level <= 2 { "Radley" } else { "Libertinus Serif" }` with its level-3 italic.
/// The face a heading of `level` sets in: for levels 1 and 2, the theme's per-level display face -- or the
/// role-default heading face -- resolved through the loaded `faces`; a level 3 or deeper, or a named face
/// the book ships no file for, falls to the idiom's role behaviour (`doc_head_face` for a documentation
/// tree, `head_face_role`'s body bold/italic/upright for a book). This is what carries a document's
/// `heading-font` onto the page: a resolvable name renders in that face, an unresolvable one (the body
/// family, or a face the tree does not ship) renders exactly as the body role did before.
fn resolved_head_face<'a>(level: u8, style: &'a Theme, faces: &'a FaceResolver, doc: bool) -> HeadFace<'a> {
	if level <= 2 {
		let idx = (level.max(1) as usize) - 1;
		let lvl			= style.heading.levels.get(idx);
		let per_level	= lvl.and_then(|l| l.face.as_deref());
		let role		= style.heading.face.as_deref();
		// The level's own weight and slant choose the variant; the default (no weight, upright) resolves to
		// the Regular face, so a document naming a plain display face renders exactly as before.
		let bold		= lvl.and_then(|l| l.weight).map_or(false, |w| w >= 600);
		let italic		= lvl.map_or(false, |l| l.italic);
		for name in [per_level, role].into_iter().flatten() {
			if let Some(font) = faces.resolve_weighted(name, bold, italic) {
				return HeadFace::Solo(font);
			}
		}
	}
	if doc { doc_head_face(level) } else { head_face_role(level) }
}

/// The body-role face a heading level falls to when no display face is set or resolves: the body bold for
/// levels 1 and 2, italic for level 3, upright for deeper -- the book idiom's role fallback, split out of
/// the former `head_face` now that the display face comes from the resolver rather than a passed handle.
fn head_face_role(level: u8) -> HeadFace<'static> {
	match level {
		3				=> HeadFace::Role(Role::Italic),
		_ if level <= 2	=> HeadFace::Role(Role::Bold),	// no display face: the body bold stands in
		_				=> HeadFace::Role(Role::Body),
	}
}

/// Is this theme's heading kind a documentation tree's (banner or inline), whose role fallback differs
/// from a book's? A book takes the display face or body bold; a doc small-caps and italicises by level.
fn is_doc_heading(style: &Theme) -> bool {
	matches!(style.heading.kind, HeadingStyle::DocBanner | HeadingStyle::DocInline | HeadingStyle::DocGrid)
}

/// The concrete display font a resolved head face names, or `None` when it falls to a role face -- for the
/// front-matter title helpers, which set from a font handle rather than a `HeadFace`.
fn head_solo<'a>(face: &HeadFace<'a>) -> Option<&'a Arc<Font>> {
	match face {
		HeadFace::Solo(f)	=> Some(f),
		HeadFace::Role(_)	=> None,
	}
}

/// Shapes one heading run in its face.
fn head_shape(
	fonts:	&Arc<FontSet>,
	face:	&HeadFace,
	size:	Sp,
	text:	&str,
)
	-> Outcome<ShapedText>
{
	match face {
		HeadFace::Solo(f)	=> ShapedText::new_with_font((*f).clone(), Dir::Ltr, size, text),
		HeadFace::Role(r)	=> ShapedText::new(fonts.clone(), *r, Dir::Ltr, size, text),
	}
}

/// Splits a title into runs for synthetic small caps: a run of originally-lowercase letters, uppercased
/// and to be set at the small size, alternates with runs of everything else (capitals, digits, spaces,
/// punctuation) kept at the full size. The bool is true for the small (was-lowercase) runs. Synthetic
/// because the shaper applies no OpenType `smcp`; used only where the template's face (Libertinus, levels
/// 3-4) really carries small caps -- Radley does not, so the level-1/2 titles keep their case.
fn smallcaps_runs(text: &str) -> Vec<(String, bool)> {
	let mut runs:	Vec<(String, bool)> = Vec::new();
	let mut cur		= String::new();
	let mut small	= false;
	for ch in text.chars() {
		let is_small = ch.is_lowercase();
		if !cur.is_empty() && is_small != small {
			runs.push((std::mem::take(&mut cur), small));
		}
		small = is_small;
		if is_small {
			for u in ch.to_uppercase() { cur.push(u); }
		} else {
			cur.push(ch);
		}
	}
	if !cur.is_empty() {
		runs.push((cur, small));
	}
	runs
}

/// Builds a sub-heading line (levels 2-4): the number in the heading face, a thin gap, then the title
/// set from its rich runs, small-capped from level 3 down. Runs of differing size seat on one baseline
/// by taking a common ascent and depth from a full-size sample, so the small caps and the full caps sit
/// level. A glossary term keeps its own first-use bold-italic (recorded in `seen`, shared with the body
/// so document order decides), emphasis its face, and a maths span is set at the heading size and its
/// glyphs woven into the line -- so a call in a heading renders rather than leaking its raw source.
fn subheading_hbox(
	fonts:		Arc<FontSet>,
	faces:		&FaceResolver,
	style: &Theme,
	level:		u8,
	number:		&str,
	segments:	&[Segment],
	seen:		&mut HashSet<String>,
)
	-> Outcome<Node>
{
	// A documentation tree sets its sub-headings from the template's show rule -- an inline level-1 heading
	// (a `DocInline` tree) bold small-caps, level 2 bold-italic, level 3 italic, deeper levels upright; a
	// book takes the display face (or body bold) and small-caps the finer levels.
	let doc			= is_doc_heading(style);
	let face		= resolved_head_face(level, style, faces, doc);
	let size		= style.heading_size(level);
	let small_size	= Sp(size.raw() * 3 / 4);	// small caps at 0.75 of the heading size
	let smallcaps	= if doc { level == 1 } else { level >= 3 };
	let sample		= res!(head_shape(&fonts, &face, size, "Ag"));
	let asc			= sample.dims().height;
	let dep			= sample.dims().depth;

	let mut children:	Vec<Node> = Vec::new();
	let mut width		= Sp::ZERO;

	if !number.is_empty() {
		let sh	= res!(head_shape(&fonts, &face, size, number));
		let w	= sh.dims().width;
		children.push(Node::Leaf(Leaf::text_dims(sh, Dims::new(w, asc, dep))));
		width += w;
		let gap = Sp(size.raw() / 5);	// ~0.2 em, the template's `h(0.2em)`
		children.push(Node::Glue(Glue::fixed(gap)));
		width += gap;
	}

	for seg in segments {
		match seg {
			Segment::Text(t)	=> res!(push_head_text(
				&mut children, &mut width, &fonts, &face, size, small_size, smallcaps, t, asc, dep)),
			Segment::Strong(t)	=> res!(push_head_text(
				&mut children, &mut width, &fonts, &head_run_face(&face, HeadRun::Strong), size, small_size, smallcaps, t, asc, dep)),
			Segment::Emph(t)	=> res!(push_head_text(
				&mut children, &mut width, &fonts, &head_run_face(&face, HeadRun::Emph), size, small_size, smallcaps, t, asc, dep)),
			Segment::BoldItalic(t)	=> res!(push_head_text(
				&mut children, &mut width, &fonts, &head_run_face(&face, HeadRun::BoldItalic), size, small_size, smallcaps, t, asc, dep)),
			// A superscript or subscript in a heading is vanishingly rare; set its text in the heading face
			// rather than raising or dropping it, so the words are kept without a scripted run in display type.
			Segment::Super(t)	=> res!(push_head_text(
				&mut children, &mut width, &fonts, &face, size, small_size, smallcaps, t, asc, dep)),
			Segment::Sub(t)		=> res!(push_head_text(
				&mut children, &mut width, &fonts, &face, size, small_size, smallcaps, t, asc, dep)),
			Segment::Code(t)	=> res!(push_head_text(
				&mut children, &mut width, &fonts, &face, size, small_size, smallcaps, t, asc, dep)),
			Segment::Glossary { term, display: disp }	=> {
				// First use is set bold-italic, matching the template's `*_term_*`; a later use takes the
				// heading's own face. The set is the body's, so a term first seen in a heading is plain in
				// the prose after it, exactly as document order dictates.
				let f = if seen.insert(term.clone()) { head_run_face(&face, HeadRun::Gloss) } else { face };
				res!(push_head_text(
					&mut children, &mut width, &fonts, &f, size, small_size, smallcaps, disp, asc, dep));
			},
			Segment::Math(atom)	=> {
				// The span is set at the heading size and unwrapped, its leaves woven into the line as the
				// body sets inline maths, so a subscripted variable in a heading draws as real glyphs.
				let mut hs = style.clone();
				hs.text.body_size = size;
				if let Node::HBox(b) = res!(math::layout(fonts.clone(), &hs, atom, false)) {
					width += b.dims.width;
					children.extend(b.list);
				}
			},
			// A footnote, cross-reference, citation or margin note in a heading is vanishingly rare and has no
			// display form here; it is dropped rather than set, leaving the heading its words.
			Segment::Footnote { .. }	=> {},
			Segment::PageRef(_)			=> {},
			Segment::Cite(_)			=> {},
			Segment::MarginNote { .. }		=> {},
			Segment::Index { .. }	=> {},	// an index marker in a heading is not recorded
		}
	}

	Ok(Node::HBox(BoxNode::new(children, Dims::new(width, asc, dep))))
}

/// The face a documentation heading level sets in, matching `template.typ`'s heading show rule: an inline
/// level-1 heading bold (and small-capped by its caller), level 2 bold-italic, level 3 italic, level 4 and
/// deeper upright. All in the body family (Libertinus), which is the doc heading family too, so no display
/// face is consulted.
fn doc_head_face(level: u8) -> HeadFace<'static> {
	match level {
		1	=> HeadFace::Role(Role::Bold),
		2	=> HeadFace::Role(Role::BoldItalic),
		3	=> HeadFace::Role(Role::Italic),
		_	=> HeadFace::Role(Role::Body),
	}
}

/// A heading run marked for emphasis: strong (`*..*`), emph (`_.._`), or a glossary term's first-use
/// bold-italic.
enum HeadRun {
	Strong,
	Emph,
	BoldItalic,
	Gloss,
}

/// The face one emphasised heading run sets in. A display face (Radley, levels 1-2) has no role variants
/// loaded, so every run keeps it; a role face (levels 3-4) takes the run's own role, an emphasis inside
/// an italic heading toggling upright as Typst sets it, a strong one going bold-italic.
fn head_run_face<'a>(base: &HeadFace<'a>, run: HeadRun) -> HeadFace<'a> {
	match base {
		HeadFace::Solo(f)	=> HeadFace::Solo(f),
		HeadFace::Role(r)	=> {
			let italic = *r == Role::Italic;
			let role = match run {
				HeadRun::Strong		=> if italic { Role::BoldItalic } else { Role::Bold },
				HeadRun::BoldItalic	=> if italic { Role::Bold } else { Role::BoldItalic },	// nested emphasis toggles against an italic heading
				HeadRun::Gloss		=> Role::BoldItalic,
				HeadRun::Emph		=> if italic { Role::Body } else { Role::Italic },	// emph toggles against an italic heading
			};
			HeadFace::Role(role)
		},
	}
}

/// Sets one heading text run into the line, small-capping it (levels 3-4) run by run so a was-lowercase
/// stretch sets uppercase at the small size while capitals keep the full size, both seated on the common
/// baseline. A run with no small caps sets whole at the full size.
#[allow(clippy::too_many_arguments)]
fn push_head_text(
	children:	&mut Vec<Node>,
	width:		&mut Sp,
	fonts:		&Arc<FontSet>,
	face:		&HeadFace,
	size:		Sp,
	small_size:	Sp,
	smallcaps:	bool,
	text:		&str,
	asc:		Sp,
	dep:		Sp,
)
	-> Outcome<()>
{
	if smallcaps {
		for (run, is_small) in smallcaps_runs(text) {
			let rs	= if is_small { small_size } else { size };
			let sh	= res!(head_shape(fonts, face, rs, &run));
			let w	= sh.dims().width;
			children.push(Node::Leaf(Leaf::text_dims(sh, Dims::new(w, asc, dep))));
			*width += w;
		}
	} else {
		let sh	= res!(head_shape(fonts, face, size, text));
		let w	= sh.dims().width;
		children.push(Node::Leaf(Leaf::text_dims(sh, Dims::new(w, asc, dep))));
		*width += w;
	}
	Ok(())
}

/// Renders a shaped run as a coloured graphic: each glyph outline filled in `colour`, so a heading can
/// take a fill the text emitter (which draws every run black) does not carry. The outline is font-frame,
/// y up; it is flipped and seated on the run's baseline, `height` below the box top.
fn coloured_run(shaped: &ShapedText, colour: Rgba) -> Outcome<Graphic> {
	let base_y = shaped.dims().height.to_pt() as f32;
	let mut ops = Vec::new();
	for glyph in &shaped.run().glyphs {
		let path = res!(shaped.outline(glyph));
		if path.is_empty() {
			continue;
		}
		let t = Transform::scale(1.0, -1.0)
			.then(&Transform::translate(glyph.x, base_y - glyph.y));
		ops.push(DrawOp::Fill { path: res!(path.transform(&t)), colour });
	}
	Ok(Graphic::new(ops, shaped.dims()))
}

/// Pushes a shaped run centred within `measure` on its own line.
fn push_centred_shape(nodes: &mut Vec<Node>, sh: ShapedText, measure: Sp) -> Outcome<()> {
	let d	= sh.dims();
	let pad	= if measure > d.width { Sp((measure.raw() - d.width.raw()) / 2) } else { Sp::ZERO };
	let mut row: Vec<Node> = Vec::new();
	if pad.raw() > 0 {
		row.push(Node::Glue(Glue::fixed(pad)));
	}
	row.push(Node::Leaf(Leaf::text(sh)));
	nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, d.height, d.depth))));
	Ok(())
}

/// The upper-case Roman numeral for `n`, covering the part range a book uses.
fn roman(mut n: u32) -> String {
	let table = [
		(100u32, "C"), (90, "XC"), (50, "L"), (40, "XL"),
		(10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
	];
	let mut out = String::new();
	for (v, s) in table {
		while n >= v {
			out.push_str(s);
			n -= v;
		}
	}
	out
}

/// Sets a chapter opener (level 1) or a part divider (level 0) on a fresh page. A chapter shows its
/// number as a giant grey display numeral centred near the page top, then its title beneath in the
/// display face at the chapter-title size. A part divider carries `part_label` ("Part I") in the
/// display face above its title, both centred and set about the vertical middle of the page, matching
/// the template's `align(center + horizon)` part page. The anchor (and any label) is recorded at the
/// opener, so a running head or a cross-reference finds its page.
#[allow(clippy::too_many_arguments)]
fn chapter_opener(
	nodes:		&mut Vec<Node>,
	fonts:		&Arc<FontSet>,
	faces:		&FaceResolver,
	style: &Theme,
	geom:		PageGeometry,
	measure:	Sp,
	level:		u8,
	number:		&str,
	title:		&str,
	part_label:	&str,
	id:			&AnchorId,
	label:		Option<&str>,
)
	-> Outcome<()>
{
	nodes.push(Node::Anchor(id.clone()));
	if let Some(l) = label {
		nodes.push(Node::Anchor(AnchorId::new(AnchorKind::Label, l.to_string())));
	}

	let face = resolved_head_face(level, style, faces, is_doc_heading(style));

	// A part divider fills its own page: a small display label, a gap, then the title, the block set
	// about the vertical centre. The label is upper-cased, the template's tracked small caps rendered as
	// plain caps here (the shaper carries no tracking); the gap is the template's `#v(2em)`, two body em.
	if level == 0 {
		let label_size	= Sp::from_pt(style.heading.levels[0].size.to_pt() * 0.55);	// the template's part-label, ~13/24 of the title
		let lab			= res!(head_shape(fonts, &face, label_size, &part_label.to_uppercase()));
		let ttl			= res!(head_shape(fonts, &face, style.heading.levels[0].size, title));
		let gap			= Sp::from_pt(style.text.body_size.to_pt() * 2.0);	// #v(2em)
		let lab_v		= lab.dims().height + lab.dims().depth;
		let ttl_v		= ttl.dims().height + ttl.dims().depth;
		let block_v		= lab_v + gap + ttl_v;
		let content_h	= geom.content_height();
		// Drop the block so its middle sits at the page's vertical centre; a box spacer, which a page top
		// keeps where glue would be discarded.
		if content_h > block_v {
			let top = Sp((content_h.raw() - block_v.raw()) / 2);
			nodes.push(Node::HBox(BoxNode::new(vec![], Dims::new(Sp::ZERO, top, Sp::ZERO))));
		}
		res!(push_centred_shape(nodes, lab, measure));
		nodes.push(Node::Glue(Glue::fixed(gap)));
		res!(push_centred_shape(nodes, ttl, measure));
		return Ok(());
	}

	// A documentation tree opens a level-1 heading with the template's full-width grey banner bar carrying
	// the title in small caps, rather than a numbered chapter opener.
	if level == 1 && style.heading.kind == HeadingStyle::DocBanner {
		res!(doc_banner(nodes, fonts, faces, style, geom, measure, title));
		return Ok(());
	}

	if level == 1 && (!number.is_empty() || style.heading.kind == HeadingStyle::DocGrid) {
		// The opener reproduces the template's four-row grid (`chapter-grid-rows`): a tall band holding the
		// number centred on its middle, a gap, a shorter band holding the title on its foot, and a gap down
		// to the body. Every row is a box, not glue -- a page top discards leading glue, and the opener sits
		// at the page top -- so the bands hold their heights and the body lands on the grid's foot. A grid
		// template (oxeweb) carries no number: the first band is then the reserved logo band, an empty box.
		if !number.is_empty() {
			let sh		= res!(head_shape(fonts, &face, style.opener.chap_num_size, number));
			let d		= sh.dims();
			let num_v	= d.height + d.depth;
			let band	= style.opener.chap_grid[0];
			// The number rides the middle of its band (Typst's `center + horizon`): the slack splits above and
			// below. A band shorter than the number leaves no slack and the number simply fills it.
			let above	= if band > num_v { Sp((band.raw() - num_v.raw()) / 2) } else { Sp::ZERO };
			let below	= if band > num_v + above { band - num_v - above } else { Sp::ZERO };
			nodes.push(vspacer(above));

			let graphic	= res!(coloured_run(&sh, style.colours.chap_num_grey));
			let pad		= if measure > d.width { Sp((measure.raw() - d.width.raw()) / 2) } else { Sp::ZERO };
			let mut row:	Vec<Node> = Vec::new();
			if pad.raw() > 0 {
				row.push(Node::Glue(Glue::fixed(pad)));
			}
			row.push(Node::Leaf(Leaf::graphic(graphic)));
			nodes.push(Node::HBox(BoxNode::new(row, Dims::new(measure, num_v, Sp::ZERO))));
			nodes.push(vspacer(below));
		} else {
			// The reserved logo/title band the grid template lays down at row 0 (oxeweb's 240pt), no number.
			nodes.push(vspacer(style.opener.chap_grid[0]));
		}
		nodes.push(vspacer(style.opener.chap_grid[1]));	// the gap row between number and title

		// The title rides the foot of its band (Typst's `left + bottom`): all the slack sits above it.
		let sh_t	= res!(head_shape(fonts, &face, style.heading.levels[0].size, title));
		let dt		= sh_t.dims();
		let title_v	= dt.height + dt.depth;
		let band2	= style.opener.chap_grid[2];
		let top2	= if band2 > title_v { band2 - title_v } else { Sp::ZERO };
		nodes.push(vspacer(top2));
		nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::text(sh_t))], Dims::new(measure, dt.height, dt.depth))));
		nodes.push(vspacer(style.opener.chap_grid[3]));	// the gap row down to the body
		// A grid opener has no number band, so Typst's block-below glue (par.spacing) between the opener grid
		// and the first paragraph is added explicitly; the numbered book path stays byte-identical.
		if number.is_empty() {
			nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
		}
		return Ok(());
	}

	// An unnumbered level-1 opener (no grid number): the title set left in the display face, then a gap.
	let sh	= res!(head_shape(fonts, &face, style.heading.levels[0].size, title));
	let d	= sh.dims();
	nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::text(sh))], Dims::new(measure, d.height, d.depth))));
	nodes.push(Node::Glue(Glue::fixed(Sp::from_pt(20.0))));
	Ok(())
}

/// Draws the documentation template's chapter banner: a full-width grey bar hanging into the page's top
/// and side margins (`template.typ`'s `chapter-banner`, a `place`d rect 150 pt tall from the page top,
/// `100% + 2*margin` wide), carrying the title left-aligned in small-caps bold, seated on the band's
/// vertical middle. The bar is drawn from one box whose ops bleed past its bounds -- the emitter clips
/// nothing -- and the box holds the template's `#v(95pt)` of following space, so the body lands where the
/// oracle sets it. No number is drawn: a doc tree sets `numbering: none`.
fn doc_banner(
	nodes:		&mut Vec<Node>,
	fonts:		&Arc<FontSet>,
	faces:		&FaceResolver,
	style: &Theme,
	geom:		PageGeometry,
	measure:	Sp,
	title:		&str,
)
	-> Outcome<()>
{
	let grey		= Rgba::opaque(240, 240, 240);	// the template's `colours.lightgrey`, luma(240)
	let banner_h	= 150.0f32;						// the template's rect height
	let follow		= Sp::from_pt(95.0);			// the template's `#v(95pt)` down to the body
	// The box origin is the content top-left; the graphic's ops are in that frame, y down. The bar reaches
	// the page's left edge (x = -inside) and top edge (y = -top), and runs the full page width and 150 pt
	// deep, so it hangs into both margins exactly as the placed rect does.
	let inside_pt	= geom.content_left().to_pt() as f32;
	let top_pt		= geom.content_top().to_pt() as f32;
	let page_w_pt	= geom.width.to_pt() as f32;
	let x0			= -inside_pt;
	let y0			= -top_pt;
	let x1			= page_w_pt - inside_pt;
	let y1			= banner_h - top_pt;

	let mut ops:	Vec<DrawOp>	= Vec::new();
	ops.push(DrawOp::Fill { path: res!(Path::rect(Bounds::new(x0, y0, x1, y1))), colour: grey });

	// The title in the resolved heading face (the template's `heading-font`, e.g. Graystroke), falling to
	// the body bold when the tree ships no display face, at the template's 26 pt, small-capped run by run
	// (the shaper carries no `smcp`, so the case is synthesised: was-lowercase letters uppercased at 0.75
	// of the size).
	let face		= resolved_head_face(1, style, faces, true);
	let size		= Sp::from_pt(26.0);
	let small_size	= Sp(size.raw() * 3 / 4);
	let sample		= res!(head_shape(fonts, &face, size, "Ag"));
	let asc			= sample.dims().height.to_pt() as f32;
	let dep			= sample.dims().depth.to_pt() as f32;
	// The band's vertical middle in the box frame, then the baseline that centres the run's box on it.
	let band_mid	= (banner_h / 2.0) - top_pt;
	let base_y		= band_mid + (asc - dep) / 2.0;

	let mut x_off	= 0.0f32;	// the title's left edge sits at the content left (box origin)
	for (run, is_small) in smallcaps_runs(title) {
		let rs		= if is_small { small_size } else { size };
		let shaped	= res!(head_shape(fonts, &face, rs, &run));
		for glyph in &shaped.run().glyphs {
			let path = res!(shaped.outline(glyph));
			if path.is_empty() {
				continue;
			}
			let t = Transform::scale(1.0, -1.0)
				.then(&Transform::translate(x_off + glyph.x, base_y - glyph.y));
			ops.push(DrawOp::Fill { path: res!(path.transform(&t)), colour: Rgba::BLACK });
		}
		x_off += shaped.dims().width.to_pt() as f32;
	}

	let graphic = Graphic::new(ops, Dims::new(measure, follow, Sp::ZERO));
	// The box holds the `#v(95pt)` of flow space; the bar draws past its top edge into the margins.
	nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::graphic(graphic))], Dims::new(measure, follow, Sp::ZERO))));
	Ok(())
}

/// Draws the documentation template's section banner (`template.typ`'s `section-banner`): the same
/// full-width grey bar `doc_banner` hangs into the page's top and side margins, but carrying the section's
/// logo right-aligned on the band's vertical middle rather than a title -- the Hematite guide's per-section
/// mark. The logo is loaded at the template's 30 pt height and its right edge seated one page margin
/// (2.5 cm) in from the page's right edge (the template's `pad(right: 2.5cm)`), which lands on the content's
/// right edge. The bar is drawn from one box whose ops bleed past its bounds -- the emitter clips nothing --
/// and the box holds the template's `#v(95pt)` of following space, so the inline heading beneath lands where
/// the oracle sets it. A logo that will not load leaves the bar alone.
fn section_banner(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	measure:	Sp,
	path:		&str,
)
	-> Outcome<()>
{
	let grey		= Rgba::opaque(240, 240, 240);	// the template's `colours.lightgrey`, luma(240)
	let banner_h	= 150.0f32;						// the template's rect height
	let follow		= Sp::from_pt(95.0);			// the template's `#v(95pt)` down to the heading
	let logo_h		= 30.0f32;						// the template's `image(logo_path, height: 30pt)`
	// The box origin is the content top-left, y down; the bar reaches the page's left edge (x = -inside) and
	// top edge (y = -top), runs the full page width and 150 pt deep, so it hangs into both margins.
	let inside_pt	= geom.content_left().to_pt() as f32;
	let top_pt		= geom.content_top().to_pt() as f32;
	let page_w_pt	= geom.width.to_pt() as f32;
	let x0			= -inside_pt;
	let y0			= -top_pt;
	let x1			= page_w_pt - inside_pt;
	let y1			= banner_h - top_pt;

	let mut ops:	Vec<DrawOp>	= Vec::new();
	ops.push(DrawOp::Fill { path: res!(Path::rect(Bounds::new(x0, y0, x1, y1))), colour: grey });

	// The logo, loaded 30 pt tall, its right edge one page margin in from the page's right edge (the content
	// right edge) and its box centred on the band's vertical middle. Its own ops are in a top-left frame,
	// y down; a plain translation seats them. A logo that will not load draws the bar alone.
	if let Ok(logo) = image_at_height(&fonts, path, logo_h as f64) {
		let lw			= logo.dims.width.to_pt() as f32;
		let lh			= (logo.dims.height + logo.dims.depth).to_pt() as f32;
		let right		= x1 - inside_pt;			// 2.5 cm in from the page right edge = the content right edge
		let band_mid	= (banner_h / 2.0) - top_pt;	// the band's vertical middle, box frame, y down
		let tx			= right - lw;
		let ty			= band_mid - lh / 2.0;
		let t			= Transform::translate(tx, ty);
		for op in logo.ops {
			ops.push(match op {
				DrawOp::Fill { path, colour }			=> DrawOp::Fill { path: res!(path.transform(&t)), colour },
				DrawOp::Stroke { path, colour, width }	=> DrawOp::Stroke { path: res!(path.transform(&t)), colour, width },
				DrawOp::Image { image, x, y, w, h }		=> DrawOp::Image { image, x: x + tx, y: y + ty, w, h },
			});
		}
	}

	let graphic = Graphic::new(ops, Dims::new(measure, follow, Sp::ZERO));
	// The box holds the `#v(95pt)` of flow space; the bar draws past its top edge into the margins.
	nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::graphic(graphic))], Dims::new(measure, follow, Sp::ZERO))));
	Ok(())
}

/// A rigid vertical spacer: a zero-width box of the given height, so it holds its space at a page top
/// where leading glue would be discarded.
fn vspacer(height: Sp) -> Node {
	Node::HBox(BoxNode::new(vec![], Dims::new(Sp::ZERO, height, Sp::ZERO)))
}

/// Sets a `#styled-box[...]` callout: its inner blocks laid out at the measure less the horizontal insets,
/// seated one inset in from the left and top, over a filled rounded rectangle that runs the full measure.
/// The template's own box takes `inset: (x: 1em, y: 1em, bottom: 1.2em)`, so the sides and top pad one body
/// em and the foot 1.2 em, and `radius: 4pt` rounds the corners; the wash is `colours.veronica.lighten(90%)`,
/// a pale violet. A `#show` rule's `block.with(fill:, inset:, radius:)` can override any of the four on
/// `style.callout` (each `None` until a rule names it), so this reads them off `style` and falls back to
/// the template's own constants precisely where a rule left them unset -- the bare `#styled-box[...]` path,
/// which sets no such rule, always takes every fallback and renders exactly as before. The wash draws first
/// with no vertical extent of its own, so the words overlay it, and the whole callout is one keep box -- the
/// breaker moves it entire rather than splitting the wash from its text.
#[allow(clippy::too_many_arguments)]
fn styled_box(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style: &Theme,
	measure:	Sp,
	blocks:		&[Block],
	fill:		Rgba,
	foot_no:	&mut u32,
	ref_no:		&mut u32,
	margin_no:	&mut u32,
	seen:		&mut HashSet<String>,
	idx:		&mut IndexGather,
	claim:		&mut ClaimGather,
	bib:		Option<&Bibliography>,
	refs:		&HashMap<String, String>,
)
	-> Outcome<()>
{
	let em			= style.text.body_size;
	// Each falls back to the template's own constant precisely where a rule left it unset, so the bare
	// `#styled-box[...]` path -- which sets no such rule -- takes every fallback and is unchanged. The left
	// and right pads take an asymmetric `inset.left`/`inset.right` override first (a `#let` template block's
	// `inset: (left:, right:)`), then the symmetric `inset.x`, then one body em -- so a callout that names
	// neither is exactly as before.
	let inset_left	= style.callout.inset_left.or(style.callout.inset_x).unwrap_or(em);	// `inset.left`, default one body em
	let inset_right	= style.callout.inset_right.or(style.callout.inset_x).unwrap_or(em);	// `inset.right`, default one body em
	let inset_top	= style.callout.inset_top.unwrap_or(em);							// `inset.y`, default one body em
	let inset_bot	= style.callout.inset_bot.unwrap_or(Sp::from_pt(em.to_pt() * 1.2));	// `inset.bottom`, default 1.2 em
	let radius		= style.callout.radius.map_or(4.0f32, |sp| sp.to_pt() as f32);		// `radius`, default 4pt
	let two_x		= inset_left + inset_right;
	let inner_w		= if measure > two_x { measure - two_x } else { measure };

	// The inner blocks laid out at the reduced measure, then each line shifted one left inset in by a leading
	// glue: `place_vbox` seats every child at the content left, so the horizontal inset rides inside the line
	// rather than on the box.
	let mut inner:	Vec<Node>	= Vec::new();
	res!(box_flow(&mut inner, fonts.clone(), geom, style, inner_w, blocks, foot_no, ref_no, margin_no, seen, idx, claim, bib, refs));
	for node in inner.iter_mut() {
		if let Node::HBox(b) = node {
			b.list.insert(0, Node::Glue(Glue::fixed(inset_left)));
			b.dims = Dims::new(b.dims.width + inset_left, b.dims.height, b.dims.depth);
		}
	}

	// The stacked height of the inner content, so the wash encloses it plus the top and bottom insets.
	let mut content_h = Sp::ZERO;
	for node in &inner {
		content_h += node.vextent();
	}
	let total = inset_top + content_h + inset_bot;

	let mut children:	Vec<Node>	= Vec::new();
	// The wash and the left rule, both drawn behind the words as one zero-extent graphic: a rounded rectangle
	// the full measure wide and the whole box tall, then -- when a `stroke: (left: <w> + <colour>)` names one
	// -- a vertical bar of that width and colour seated at the left edge. A fully transparent fill (a `#let`
	// template block with no `fill:` -- a plain indented block, not a washed callout) draws no rectangle, so
	// the inset positions the text with no panel behind it; a left rule with no fill still draws.
	let mut ops: Vec<DrawOp> = Vec::new();
	if fill.a != 0 {
		let rect	= res!(Path::round_rect(
			Bounds::new(0.0, 0.0, measure.to_pt() as f32, total.to_pt() as f32), radius));
		ops.push(DrawOp::Fill { path: rect, colour: fill });
	}
	if let (Some(w), Some(col)) = (style.callout.stroke_left_w, style.callout.stroke_left_col) {
		if w.to_pt() > 0.0 && col.a != 0 {
			let bar	= res!(Path::rect(Bounds::new(0.0, 0.0, w.to_pt() as f32, total.to_pt() as f32)));
			ops.push(DrawOp::Fill { path: bar, colour: col });
		}
	}
	if !ops.is_empty() {
		let graphic	= Graphic::new(ops, Dims::new(measure, Sp::ZERO, Sp::ZERO));
		children.push(Node::Leaf(Leaf::graphic(graphic)));
	}
	children.push(Node::Glue(Glue::fixed(inset_top)));
	children.append(&mut inner);
	children.push(Node::Glue(Glue::fixed(inset_bot)));
	nodes.push(vbox(children, measure));
	Ok(())
}

/// Measures a block flow without placing it: the blocks are set at `measure` exactly as [`box_flow`] sets
/// them, and the stacked vertical extent is returned as [`Dims`] -- `width` the measure, `height` the sum of
/// the flow's node extents, `depth` zero. The overlay pass sizes a note this way before drawing it, the same
/// measure Typst's own `measure(content)` gives. The document-order counters a full render threads are
/// throwaway here (a measure numbers nothing), so a footnote or reference inside the measured blocks counts
/// only within this scratch flow and never reaches the document.
pub fn measure_blocks(
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style: &Theme,
	measure:	Sp,
	blocks:		&[Block],
	bib:		Option<&Bibliography>,
	refs:		&HashMap<String, String>,
)
	-> Outcome<Dims>
{
	let mut nodes:		Vec<Node>		= Vec::new();
	let mut foot_no						= 0u32;
	let mut ref_no						= 0u32;
	let mut margin_no					= 0u32;
	let mut seen:		HashSet<String>	= HashSet::new();
	// A scratch measurement flow numbers nothing that reaches the document, so its index markers and claim
	// references are gathered into throwaways that are dropped -- they must not join the real back matter.
	let mut idx			= IndexGather::default();
	let mut claim		= ClaimGather::default();
	res!(box_flow(&mut nodes, fonts, geom, style, measure, blocks,
		&mut foot_no, &mut ref_no, &mut margin_no, &mut seen, &mut idx, &mut claim, bib, refs));
	let mut height = Sp::ZERO;
	for n in &nodes {
		height += n.vextent();
	}
	Ok(Dims::new(measure, height, Sp::ZERO))
}

/// Lays a callout's inner blocks into a flow of line nodes at `measure`: a plain or rich paragraph is
/// woven into justified lines and a list set as its bullets, blocks parted by a paragraph skip. Only the
/// block kinds a callout body carries are set -- a `#styled-box` wraps running prose, not a heading, a
/// figure or a table -- so any other block is passed over.
#[allow(clippy::too_many_arguments)]
fn box_flow(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style: &Theme,
	measure:	Sp,
	blocks:		&[Block],
	foot_no:	&mut u32,
	ref_no:		&mut u32,
	margin_no:	&mut u32,
	seen:		&mut HashSet<String>,
	idx:		&mut IndexGather,
	claim:		&mut ClaimGather,
	bib:		Option<&Bibliography>,
	refs:		&HashMap<String, String>,
)
	-> Outcome<()>
{
	let mut first = true;
	res!(box_flow_scoped(nodes, fonts, geom, style, measure, blocks, foot_no, ref_no, margin_no, seen, idx, claim, bib, refs, &mut first));
	Ok(())
}

/// The recursive core of [`box_flow`]: sets a callout's blocks under `style`, descending into a
/// [`Block::Scoped`] under its overlaid theme so a `#set` inside a callout body styles only its subtree
/// rather than being dropped. `first` is shared across the recursion so the inter-block paragraph skip is
/// placed on document order, not reset at a scope boundary.
#[allow(clippy::too_many_arguments)]
fn box_flow_scoped(
	nodes:		&mut Vec<Node>,
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style: &Theme,
	measure:	Sp,
	blocks:		&[Block],
	foot_no:	&mut u32,
	ref_no:		&mut u32,
	margin_no:	&mut u32,
	seen:		&mut HashSet<String>,
	idx:		&mut IndexGather,
	claim:		&mut ClaimGather,
	bib:		Option<&Bibliography>,
	refs:		&HashMap<String, String>,
	first:		&mut bool,
)
	-> Outcome<()>
{
	for block in blocks {
		if let Block::Scoped { patch, blocks: inner } = block {
			let scoped = { let mut t = style.clone(); t.apply(patch); t };
			res!(box_flow_scoped(nodes, fonts.clone(), geom, &scoped, measure, inner,
				foot_no, ref_no, margin_no, seen, idx, claim, bib, refs, first));
			continue;
		}
		if !*first {
			nodes.push(Node::Glue(Glue::fixed(style.par.skip)));
		}
		match block {
			Block::Paragraph { text } => {
				let pieces = vec![Piece::Text { text: text.clone(), role: Role::Body }];
				let lines = res!(break_paragraph_pieces(
					fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &pieces, measure, style.text.leading, style.text.justify, style.text.hyphenate, style.text.fill,
						Some(cap_edge(style, style.text.body_size))));
				nodes.extend(lines);
			},
			Block::RichParagraph { segments } => {
				let pieces = res!(build_pieces(
					fonts.clone(), geom, style, segments, Role::Body, foot_no, ref_no, margin_no, seen, idx, claim, bib, refs));
				let lines = res!(break_paragraph_pieces(
					fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, &pieces, measure, style.text.leading, style.text.justify, style.text.hyphenate, style.text.fill,
						Some(cap_edge(style, style.text.body_size))));
				nodes.extend(lines);
			},
			Block::List { ordered, items } => {
				res!(list(
					nodes, fonts.clone(), geom, style, measure, *ordered, items,
					foot_no, ref_no, margin_no, seen, idx, claim, bib, refs));
			},
			// A verbatim code block a template moved into a washed box (`#show raw: block.with(fill: ...)`):
			// set in the mono face at the scoped `code.size`, the same as a top-level code block. Without this
			// arm the box body dropped its code silently.
			Block::Code { lines } => {
				res!(code_block(nodes, fonts.clone(), style, lines));
			},
			// A nested space a template placed inside a boxed body.
			Block::Space(sp) => {
				nodes.push(Node::Glue(Glue::fixed(*sp)));
			},
			// A `#pagebreak()` inside a callout body has no page to turn, so it is refused visibly at parse
			// time ([`crate::lang::parse::refuse_nested_page_breaks`]) and never reaches here. This explicit
			// arm keeps it out of the silent catch-all below, so a future path that did route one here would
			// surface as a compile-time non-exhaustiveness rather than a silent drop.
			Block::PageBreak => {},
			_ => {},
		}
		*first = false;
	}
	Ok(())
}

/// Appends a horizontal rule -- a standalone `#line(...)` divider -- as a filled grey bar of the given
/// width (a fraction of the measure or an absolute length), thickness and grey level, seated flush left.
/// A degenerate rule (zero width or thickness) adds nothing rather than an empty box.
fn rule_divider(nodes: &mut Vec<Node>, measure: Sp, width: Length, thickness: f64, grey: u8) {
	let w = match width {
		Length::Rel(f)	=> Sp::from_pt(measure.to_pt() * f),
		Length::Abs(pt)	=> Sp::from_pt(pt),
	};
	let wf = w.to_pt() as f32;
	let hf = thickness as f32;
	if wf <= 0.0 || hf <= 0.0 {
		return;
	}
	let h		= Sp::from_pt(thickness);
	let colour	= Rgba::opaque(grey, grey, grey);
	let rect = match Path::rect(Bounds::new(0.0, 0.0, wf, hf)) {
		Ok(r)	=> r,
		Err(_)	=> return,
	};
	let graphic	= Graphic::new(vec![DrawOp::Fill { path: rect, colour }], Dims::new(w, h, Sp::ZERO));
	nodes.push(Node::HBox(BoxNode::new(vec![Node::Leaf(Leaf::graphic(graphic))], Dims::new(measure, h, Sp::ZERO))));
}

/// Loads an image at a fixed drawn height: the image at `path` read (an SVG as its own scaled paths, a
/// raster to fill its box) at height `h`, its ops seated at the origin so a caller can place it. The
/// height fixes the size and the width follows the aspect, matching the template's `image(.., height: Npt)`.
/// Used for the page footer logo, and for the meta page's declaration mark within a table cell.
pub(crate) fn image_at_height(fonts: &Arc<FontSet>, path: &str, h: f64) -> Outcome<Graphic> {
	let height = Some(Length::Abs(h));
	// A wide box so the height hint, not the measure, governs the size; the picture keeps its aspect.
	let box_w	= Sp::from_pt(1000.0);
	match crate::image::load_figure(path) {
		Ok(crate::image::Figure::Raster(img))	=> image_graphic(box_w, img, None, height, None),
		Ok(crate::image::Figure::Vector(pic))	=> svg_graphic(fonts.clone(), box_w, pic, None, height, None),
		Err(e)									=> Err(e),
	}
}

/// Draws the page furniture -- a running head in the top margin and a folio -- onto every composed
/// page. Called after the driver has converged: the furniture sits outside the text block, so adding
/// it moves nothing and cannot reopen the fixed point.
///
/// The running head follows the book's own scheme, the even/odd split the template sets. A verso (even)
/// page carries the folio at the outer edge and the book title, in italic, at the inner; a recto (odd)
/// page carries the current chapter title, in italic, at the inner edge and the folio at the outer. The
/// current chapter is the most recent level-1 heading the ledger resolved to an earlier page. A page a
/// chapter opens at its very top -- and the first page, before any chapter runs -- omits the running
/// head and sets a centred folio at the foot instead, the usual chapter-opening treatment. The frame is
/// laid at the recto (binding-left) split; `ingot` mirrors a verso page's whole frame to the fore-edge
/// afterwards, so placing the folio at the block's left on a verso page lands it at the outer margin.
/// Both the head and the folio are shaped through the same path as the body and drawn as glyph outlines.
pub fn decorate(
	pages:			&mut [Page],
	ledger:			&Ledger,
	heads:			&[Heading],
	fonts:			&Arc<FontSet>,
	style: &Theme,
	geom:			PageGeometry,
	book_title:		&str,
	footer_logo:	Option<&str>,
)
	-> Outcome<()>
{
	let content_top		= geom.content_top();
	let content_left	= geom.content_left();
	let content_width	= geom.content_width();
	// The documentation template seats a logo at the left of every page footer. It is loaded once and
	// placed on each body page; a logo that will not load leaves the footer to the folio alone.
	let footer = footer_logo.and_then(|p| image_at_height(fonts, p, 18.0).ok().map(Arc::new));
	// The body opens on this physical page; the printed folio restarts at one here, so a body page's
	// folio is its physical page less the front matter before it. A run with no headings (a lone
	// manuscript) leaves `body_start_page` zero, so the whole document is body and the folio is physical.
	let body_start		= ledger.body_start_page.max(1);
	for page in pages.iter_mut() {
		// Front matter -- the cover, title, imprint and contents leaves before the body -- carries no
		// running head and no folio, exactly as the template sets `numbering: none` there.
		if page.number < body_start {
			continue;
		}
		let folio = page.number - (body_start - 1);

		// The footer logo, seated at the left of the foot on every body page, its top at the folio's foot line.
		if let Some(g) = &footer {
			let foot_top = content_top + geom.content_height() + Sp::from_pt(14.0);
			page.frame.push(Placed::new(content_left, foot_top, g.dims, PlacedKind::Graphic(g.clone())));
		}

		// The back matter -- the bibliography and beyond -- drops the running head and centres the folio at
		// the foot, as the template sets it.
		let back_start = ledger.back_matter_start_page;
		if back_start != 0 && page.number >= back_start {
			let foot_top	= content_top + geom.content_height() + Sp::from_pt(14.0);
			let shaped		= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.furniture.folio_size, &fmt!("{}", folio)));
			let d			= shaped.dims();
			let x			= centre_x(geom, d.width);
			page.frame.push(Placed::new(x, foot_top, d, PlacedKind::Text(shaped)));
			continue;
		}

		// The chapter running at the top of this page (the most recent level-1 heading resolved to an
		// earlier page), whether a chapter opens at the very top of this one, and whether a part divider
		// does -- a part page carries no folio at all.
		let mut chapter:	Option<&Heading>	= None;
		let mut opens					= false;
		let mut opens_part				= false;
		for h in heads {
			if let Some(a) = ledger.get(&h.id) {
				if a.pos.page < page.number {
					if h.level == 1 { chapter = Some(h); }
				} else if a.pos.page == page.number {
					// A chapter (or part divider) that resolves to this page opens it: a book/doc-banner chapter
					// and a part divider force a fresh page, so a level-1/level-0 heading present here is this
					// page's opener, whether it sits at the very top or has been shifted down by a top float set
					// above it -- detecting it by presence rather than by `y == content_top` is what keeps a
					// running head and folio off a float-shifted opener. A DocInline section (`= Section` mid-file,
					// no banner) does NOT force a page and sits mid-page, so it must NOT be read as an opener or
					// the previous section's running head would wrongly drop; it is admitted only when it carries
					// a banner, matching the opener test the chapter-banner path uses.
					if h.level == 1 && (h.banner || style.heading.kind != HeadingStyle::DocInline) { opens = true; }
					if h.level == 0 { opens_part = true; }
				} else {
					break;	// headings are in document order, so the rest resolve to later pages
				}
			}
		}

		// A part divider stands alone with no folio and no head, as the template's part page sets.
		if opens_part {
			continue;
		}

		// The head baseline sits a fixed step above the text block; a folio at the foot sits a step below.
		let head_base	= content_top - Sp::from_pt(8.0);
		let foot_top	= content_top + geom.content_height() + Sp::from_pt(14.0);
		let num			= fmt!("{}", folio);

		if opens || chapter.is_none() {
			// A chapter-opening page: no running head, a centred folio at the foot.
			let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.furniture.folio_size, &num));
			let d		= shaped.dims();
			let x		= centre_x(geom, d.width);
			page.frame.push(Placed::new(x, foot_top, d, PlacedKind::Text(shaped)));
			continue;
		}

		// The folio, at the outer margin of the running head. On a recto (odd) page the outer edge is the
		// block's right; on a verso (even) page it is the block's left, which the mirror shift carries to
		// the fore-edge.
		let folio	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.furniture.folio_size, &num));
		let fd		= folio.dims();
		let folio_x	= if page.number % 2 == 0 {
			content_left
		} else {
			content_left + content_width - fd.width
		};
		page.frame.push(Placed::new(folio_x, head_base - fd.height, fd, PlacedKind::Text(folio)));

		// The title side: the book title on a verso page, set against the folio at the inner edge; the
		// chapter title on a recto, set from its rich runs so a maths span or emphasis in the title renders
		// rather than dropping to a gap. Both italic.
		if page.number % 2 == 0 {
			if !book_title.is_empty() {
				let shaped	= res!(ShapedText::new(fonts.clone(), Role::Italic, Dir::Ltr, style.furniture.header_size, book_title));
				let d		= shaped.dims();
				let x		= content_left + content_width - d.width;	// verso: title at the inner (spine) edge
				page.frame.push(Placed::new(x, head_base - d.height, d, PlacedKind::Text(shaped)));
			}
		} else if let Some(ch) = chapter {
			let (rnodes, rd) = res!(inline_segments(fonts, style, &ch.segments, Role::Italic, style.furniture.header_size));
			// Recto: title at the inner (spine) edge, its box top a full ascent above the head baseline.
			place_run(&mut page.frame, &rnodes, content_left, head_base - rd.height);
		}
	}

	// The overlay's second job: the marginalia the composition recorded, drawn on every page from the same
	// converged ledger the running head reads. A run over all pages, separate from the running-head loop's
	// front-matter and part-page early-outs, since a margin note belongs to its own body page regardless of
	// that page's running-head treatment.
	for page in pages.iter_mut() {
		res!(draw_marginalia(page, ledger, fonts, style, geom));
	}
	Ok(())
}

/// Draws the marginalia the composition recorded: each `#claim-label(...)`'s compressed code, set at the
/// corpus's 6.5 pt `luma(90)` grey in the outside margin at the vertical position its zero-width anchor
/// landed. It is the overlay pass's second job beside the running head -- both are zero-flow ink drawn from
/// the converged ledger, so neither can reopen the fixed point. The frame is laid at the recto split, so a
/// recto note seats flush against the block's right (its outer edge) and a verso note flush against the
/// block's left, which `ingot`'s mirror shift then carries out to the fore-edge -- the same mirror the folio
/// rides. The code's baseline is seated on the body baseline of the line its anchor sits in, so it lines up
/// with the prose it annotates rather than floating at the line top.
fn draw_marginalia(
	page:	&mut Page,
	ledger:	&Ledger,
	fonts:	&Arc<FontSet>,
	style: &Theme,
	geom:	PageGeometry,
)
	-> Outcome<()>
{
	let content_left	= geom.content_left();
	let content_width	= geom.content_width();
	let size	= Sp::from_pt(6.5);
	let colour	= Rgba::opaque(90, 90, 90);	// Typst's `luma(90)`
	// The body ascent, so the small code's baseline meets the prose baseline of the line it annotates.
	let body_asc = res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size, "Ag")).dims().height;
	for anchor in ledger.anchors() {
		if anchor.id.kind != AnchorKind::MarginNote || anchor.pos.page != page.number {
			continue;
		}
		let display = margin_display(&anchor.id.key);
		if display.is_empty() {
			continue;
		}
		let shaped	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, display)).with_colour(colour);
		let d		= shaped.dims();
		// Horizontal, in recto (binding-left) coordinates: a recto page seats the code's left edge at the
		// block's right edge (the outer margin); a verso page seats its right edge at the block's left edge,
		// which the verso mirror shift `ingot` applies afterwards carries out to the fore-edge.
		let x = if page.number % 2 == 0 {
			content_left - d.width
		} else {
			content_left + content_width
		};
		// Vertical: the anchor's y is the top of the line it landed in; the line's baseline is a body ascent
		// below that, and the code is lifted by its own height so its baseline -- not its top -- meets it.
		let y = anchor.pos.y + body_asc - d.height;
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn count_words_counts_letter_runs_across_blocks() {
		// Letter runs, as the template's `\p{L}+` counter steps: "don't" is two runs, a bare number none.
		let blocks = vec![
			Block::Heading { level: 1, segments: vec![Segment::text("The Purpose")], label: None },
			Block::Paragraph { text: "It reads a document and writes 42 pages.".to_string() },
			Block::List { ordered: false, items: vec![
				ListEntry { segments: vec![Segment::strong("one two")], children: vec![] }] },
		];
		// Heading: 2; paragraph: "It reads a document and writes pages" = 7 (the "42" counts none);
		// list item: 2. Total 11.
		assert_eq!(count_words(&blocks), 11);
	}

	#[test]
	fn build_meta_table_appends_reading_time_and_mark() {
		let fm = FrontMatter {
			title:			"Austenite".to_string(),
			subtitle:		None,
			author:			"J. D. Hoogland".to_string(),
			cover_image:	None,
			logo_image:		None,
			publisher:		None,
			edition:		None,
			isbn:			None,
			copyright:		Some("Copyright © 12025 Oxedyne. All rights reserved.".to_string()),
			rights:			None,
			ai_declaration:	None,
			website:		None,
			toolchain:		false,
			dedication:		None,
			about_author:	None,
			title_size:		Sp::from_pt(28.0),
			subtitle_size:	Sp::from_pt(16.0),
			author_size:	Sp::from_pt(17.0),
			back_title_size:	Sp::from_pt(14.0),
			sidebar_grey:	Some(240),
			sidebar_frac:	0.45,
			title_smallcaps:	true,
			top_logo:		None,
			top_logo_width:		Sp::ZERO,
			bottom_logo:	None,
			bottom_logo_width:	Sp::ZERO,
			footer_logo:	None,
			meta_rows:		vec![MetaRow {
				version:		Some("0.1.0".to_string()),
				date:			Some("12026-08-08".to_string()),
				authors:		"J. D. Hoogland".to_string(),
				notes:			Some("Created.".to_string()),
				ai_mark_path:	Some("assets/svg/doc_made_with_ai_opt.svg".to_string()),
				ai_mark_words:	Some("Made with AI".to_string()),
				ai_mark_url:	Some("https://need2know.ai/with-ai/doc".to_string()),
			}],
			reading_min:	Some(51),
			acknowledgement:	Some("We acknowledge...".to_string()),
		};
		let table = build_meta_table(&fm).expect("meta table builds");
		assert_eq!(table.rows.len(), 2, "one header row and one revision row");
		assert!(table.header, "the first row is the header");
		// The author cell carries the declaration mark stacked beneath the name (column index 2: Ver, Date, Author).
		assert!(table.rows[1].cells[2].mark.is_some(), "the author cell carries the AI mark");
		// The notes cell has the reading time appended to the authored notes.
		let notes = match &table.rows[1].cells[3].content[0] {
			Segment::Text(t)	=> t.clone(),
			_					=> String::new(),
		};
		assert!(notes.contains("Created.") && notes.contains("Reading time: 51 [min]"),
			"the notes cell appends the reading time: {:?}", notes);
	}

	#[test]
	fn docbanner_section_banner_sets_its_heading_inline_not_a_second_opener() -> Outcome<()> {
		// A `DocBanner` tree -- its default chapter opener is the grey title bar -- that opts one chapter in
		// with an explicit `#section-banner` must set that chapter's title inline beneath the banner, not
		// open a second grey bar of its own. The section banner forces one page eject; the heading that
		// follows it must add none. Before the fix the heading opened as a chapter and the forced-eject
		// count was two, which is the duplicate bar this guards against.
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let mut style	= Theme::default();
		style.heading.kind = HeadingStyle::DocBanner;
		let blocks = vec![
			Block::Paragraph { text: "Intro before the section.".to_string() },
			Block::section_banner("assets/svg/pearlite_logo_text_right.svg".to_string()),
			Block::Heading { level: 1, segments: vec![Segment::text("Pearlite")], label: None },
			Block::Paragraph { text: "Pearlite is the format.".to_string() },
		];
		let (doc, heads) = res!(author(fonts, geom, &style, &FaceResolver::default(), &blocks, None, None));
		assert!(heads.iter().any(|h| h.level == 1 && h.title == "Pearlite" && h.banner),
			"the level-1 heading after a #section-banner carries the banner flag");
		let forced = doc.nodes.iter()
			.filter(|n| matches!(n, Node::Penalty(p) if p.is_forced()))
			.count();
		assert_eq!(forced, 1,
			"only the section banner forces a page break; its heading opens inline, adding none (got {})", forced);
		Ok(())
	}

	/// A [`Block::Scoped`] carrying `#set text(size: 20pt)` sets its own paragraph at 20 pt while a sibling
	/// paragraph outside the scope keeps the document's 11 pt -- proving the scope machinery end to end
	/// through a real render: the patch reaches the renderer (the scoped line is taller) and does not leak
	/// past the scope boundary (the sibling matches an unscoped control exactly). Before this, no test
	/// rendered through a scope at all.
	#[test]
	fn a_scoped_set_styles_its_own_subtree_and_no_further() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let style	= Theme::default();
		let para	= || Block::Paragraph { text:
			"A paragraph long enough to set at least one full line of body text on the page.".to_string() };

		// The tallest and shortest paragraph-line heights in a rendered document.
		fn line_heights(doc: &Document) -> (Sp, Sp) {
			let hs: Vec<Sp> = doc.nodes.iter().filter_map(|n| match n {
				Node::HBox(b) if b.dims.height > Sp::ZERO	=> Some(b.dims.height),
				_										=> None,
			}).collect();
			let max = hs.iter().copied().fold(Sp::ZERO, |a, h| if h > a { h } else { a });
			let min = hs.iter().copied().fold(max, |a, h| if h < a { h } else { a });
			(min, max)
		}

		// Chapter A carries `#set text(size: 20pt)`; chapter B (the flat sibling) carries nothing.
		let mut scope_patch = ThemePatch::default();
		scope_patch.text.body_size = Some(Sp::from_pt(20.0));
		let scoped = vec![
			Block::Scoped { patch: scope_patch, blocks: vec![para()] },
			para(),
		];
		let (doc, _)	= res!(author(fonts.clone(), geom, &style, &FaceResolver::default(), &scoped, None, None));
		let (min_s, max_s) = line_heights(&doc);

		// A control with no scope: both paragraphs at the document's 11 pt, so every line is the same height.
		let plain = vec![para(), para()];
		let (doc_p, _)	= res!(author(fonts, geom, &style, &FaceResolver::default(), &plain, None, None));
		let (min_p, max_p) = line_heights(&doc_p);

		assert_eq!(min_p, max_p, "the unscoped control must set every paragraph at one size");
		assert!(max_s > min_s, "the scoped 20pt paragraph must set taller lines than the 11pt sibling");
		// The sibling outside the scope matches the control exactly: the scope did not leak past its blocks.
		assert_eq!(min_s, min_p, "the paragraph outside the scope must keep the document's own size");
		// And the scoped paragraph really rose above the document size, so the patch reached the renderer.
		assert!(max_s > max_p, "the scoped paragraph must exceed the unscoped document size");
		Ok(())
	}

	// Every Fill colour drawn anywhere in a rendered node tree, for a callout-fill assertion.
	fn collect_fills(nodes: &[Node], out: &mut Vec<Rgba>) {
		for n in nodes {
			match n {
				Node::HBox(b) | Node::VBox(b)	=> collect_fills(&b.list, out),
				Node::Leaf(l)					=> if let LeafKind::Graphic(g) = &l.kind {
					for op in &g.ops {
						if let DrawOp::Fill { colour, .. } = op { out.push(*colour); }
					}
				},
				_								=> {},
			}
		}
	}

	// The ink width of a set line -- the sum of its children's advances -- which fills to the measure on a
	// justified interior line and falls short of it on a ragged one, though the line box is measure-wide
	// either way.
	fn line_ink_width(line: &[Node]) -> Sp {
		let mut w = Sp::ZERO;
		for n in line {
			w = w + match n {
				Node::Leaf(l)					=> l.dims.width,
				Node::Glue(g)					=> g.natural,
				Node::HBox(b) | Node::VBox(b)	=> b.dims.width,
				_								=> Sp::ZERO,
			};
		}
		w
	}

	/// `format_numbering` renders each Typst counting system, keeps literal text, and repeats the last
	/// symbol for a hierarchical number -- and a pattern with no counting symbol is a fixed literal.
	#[test]
	fn numbering_pattern_renders_each_system() {
		assert_eq!(format_numbering("1.1", &[2, 3]), "2.3");	// hierarchical: the last symbol repeats
		assert_eq!(format_numbering("1.", &[3]), "3.");
		assert_eq!(format_numbering("(a)", &[2]), "(b)");
		assert_eq!(format_numbering("A", &[27]), "AA");
		assert_eq!(format_numbering("I", &[4]), "IV");
		assert_eq!(format_numbering("i.", &[9]), "ix.");
		assert_eq!(format_numbering("Q", &[3]), "Q");		// no counting symbol: a fixed literal
	}

	/// A `#set heading(numbering: ...)` pattern reaches the rendered heading number: a level-1 heading set
	/// to pattern "A" carries "A", where the untouched default carries the plain arabic "1".
	#[test]
	fn heading_numbering_pattern_reaches_the_number() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let blocks	= vec![Block::Heading { level: 1, segments: vec![Segment::text("Alpha")], label: None }];

		let (_, heads_d) = res!(author(fonts.clone(), geom, &Theme::default(), &FaceResolver::default(), &blocks, None, None));
		assert_eq!(heads_d[0].number, "1", "the default heading number is the plain arabic count");

		let mut alpha = Theme::default();
		for l in &mut alpha.heading.levels { l.numbering = Some("A".to_string()); }
		let (_, heads_a) = res!(author(fonts, geom, &alpha, &FaceResolver::default(), &blocks, None, None));
		assert_eq!(heads_a[0].number, "A", "a heading numbering pattern must reach the rendered number");
		Ok(())
	}

	/// A heading inside a scope is counted in document order, and a cross-reference into that scope resolves
	/// its number: `[Scoped{[H1 "A" <a>]}, H1 "B", @a]` numbers the headings 1 and 2 and resolves `@a` to
	/// "Chapter 1". Before the reference pre-pass recursed into a `Block::Scoped`, the scoped heading was
	/// invisible to it -- `@a` fell back to a page number and the top-level "B" would have taken "Chapter 1"
	/// -- so this fixture would have caught that blindness.
	#[test]
	fn cross_reference_into_a_scope_resolves_the_scoped_heading_number() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let style	= Theme::default();	// BookOpener: headings carry a document-order number
		let heading = |t: &str, label: Option<&str>| Block::Heading {
			level:		1,
			segments:	vec![Segment::text(t)],
			label:		label.map(|s| s.to_string()),
		};
		let blocks = vec![
			Block::Scoped { patch: ThemePatch::default(), blocks: vec![heading("A", Some("a"))] },
			heading("B", None),
			Block::RichParagraph { segments: vec![Segment::page_ref("a".to_string())] },
		];

		// The reference pre-pass sees the heading inside the scope: `@a` resolves to "Chapter 1", the number
		// that heading actually takes -- not a page-number fallback, and not the top-level count.
		let refs = ref_targets(&blocks, &style);
		assert_eq!(refs.get("a").map(String::as_str), Some("Chapter 1"),
			"a cross-reference into a scope must resolve the scoped heading's own number");

		// And the headings number 1, 2 in document order across the scope boundary.
		let (_, heads) = res!(author(fonts, geom, &style, &FaceResolver::default(), &blocks, None, None));
		let nums: Vec<&str> = heads.iter().map(|h| h.number.as_str()).collect();
		assert_eq!(nums, vec!["1", "2"], "headings number in document order across a scope edge");
		Ok(())
	}

	/// A sub-heading alone in a scope still keeps with the sibling paragraph beyond the scope's closing edge:
	/// `[P, Scoped{[H2]}, P]` sets an identical node stream to the flat `[P, H2, P]`, the heading's first
	/// paragraph line joining its keep box either way. This is the "keep with next" gate a per-element
	/// heading rule (which wraps each heading in its own scope) relies on to stay byte-identical.
	#[test]
	fn a_scoped_heading_keeps_with_the_paragraph_beyond_the_scope() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let style	= Theme::default();
		let para	= |t: &str| Block::Paragraph { text: t.to_string() };
		let h2		= || Block::Heading { level: 2, segments: vec![Segment::text("A Section")], label: None };
		let body	= "A paragraph long enough to set at least one full line of body text on the page, \
			and then a little more to be sure it wraps onto a second line.";

		// A signature of the node stream: each node's kind tag and its vertical extent, which together fix
		// where the heading keep box sits and how the following paragraph's lines are placed.
		fn sig(doc: &Document) -> Vec<(u8, i32)> {
			doc.nodes.iter().map(|n| {
				let tag = match n {
					Node::HBox(_)		=> 0u8,
					Node::VBox(_)		=> 1,
					Node::Glue(_)		=> 2,
					Node::Penalty(_)	=> 3,
					_					=> 9,
				};
				(tag, n.vextent().raw())
			}).collect()
		}

		let flat = vec![para("Intro."), h2(), para(body)];
		let (doc_flat, _) = res!(author(fonts.clone(), geom, &style, &FaceResolver::default(), &flat, None, None));

		let wrapped = vec![
			para("Intro."),
			Block::Scoped { patch: ThemePatch::default(), blocks: vec![h2()] },
			para(body),
		];
		let (doc_wrap, _) = res!(author(fonts, geom, &style, &FaceResolver::default(), &wrapped, None, None));

		assert_eq!(sig(&doc_flat), sig(&doc_wrap),
			"a heading alone in a scope must keep with the sibling paragraph beyond it, exactly as the flat pairing does");
		Ok(())
	}

	/// `#set par(justify: false)` reaches the renderer: a justified paragraph fills its first (interior)
	/// line to the measure, an unjustified one leaves it ragged, short of the measure.
	#[test]
	fn par_justify_false_leaves_lines_ragged() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let measure	= geom.content_width();
		let blocks	= vec![Block::Paragraph { text:
			"A paragraph written long enough that it must wrap onto at least two lines, so its first line is \
an interior line justification fills to the measure while ragged setting does not.".to_string() }];

		// The ink width of the paragraph's first (interior) line: filled to the measure when justified,
		// short of it when ragged.
		fn first_line_ink(doc: &Document) -> Sp {
			doc.nodes.iter().find_map(|n| match n {
				Node::HBox(b) if b.dims.height > Sp::ZERO	=> Some(line_ink_width(&b.list)),
				_										=> None,
			}).unwrap_or(Sp::ZERO)
		}

		let (dj, _)	= res!(author(fonts.clone(), geom, &Theme::default(), &FaceResolver::default(), &blocks, None, None));
		let mut ragged = Theme::default();
		ragged.text.justify = false;
		let (dr, _)	= res!(author(fonts, geom, &ragged, &FaceResolver::default(), &blocks, None, None));

		// The justified interior line fills to the measure; the ragged one leaves its slack on the right.
		assert!(first_line_ink(&dj) > first_line_ink(&dr),
			"a justified line fills more of the measure than a ragged one ({:?} vs {:?})",
			first_line_ink(&dj), first_line_ink(&dr));
		assert!(first_line_ink(&dj) >= measure - Sp::from_pt(1.0),
			"a justified interior line reaches the measure");
		Ok(())
	}

	/// `#set text(hyphenate: false)` reaches the renderer: a long word set on a narrow page breaks across
	/// lines when hyphenation is on and stays whole -- fewer lines -- when it is off.
	#[test]
	fn text_hyphenate_false_stops_word_breaking() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		// A narrow page, so a long word must hyphenate to fit; `content_width` here is ~24pt.
		let geom	= PageGeometry::new(Sp::from_pt(90.0), Sp::from_pt(400.0), Sp::from_pt(33.0));
		let blocks	= vec![Block::Paragraph { text:
			"antidisestablishmentarianism antidisestablishmentarianism".to_string() }];

		fn line_count(doc: &Document) -> usize {
			doc.nodes.iter().filter(|n| matches!(n, Node::HBox(b) if b.dims.height > Sp::ZERO)).count()
		}

		let (on, _)	= res!(author(fonts.clone(), geom, &Theme::default(), &FaceResolver::default(), &blocks, None, None));
		let mut no_hyph = Theme::default();
		no_hyph.text.hyphenate = false;
		let (off, _) = res!(author(fonts, geom, &no_hyph, &FaceResolver::default(), &blocks, None, None));

		assert!(line_count(&on) > line_count(&off),
			"hyphenation on must split the long words into more lines than off ({} vs {})",
			line_count(&on), line_count(&off));
		Ok(())
	}

	/// A `#set enum(numbering: ...)` pattern reaches the ordered-list marker: setting `"(a)"` shapes a
	/// different first marker from the default `"1."`, so the marker glyph really came from the theme.
	#[test]
	fn enum_numbering_changes_the_marker() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let items	= || vec![
			ListEntry { segments: vec![Segment::text("First item.")], children: vec![] },
			ListEntry { segments: vec![Segment::text("Second item.")], children: vec![] },
		];
		let blocks	= vec![Block::list(true, items())];

		// The width of the first list item's leading marker leaf.
		fn first_marker_width(doc: &Document) -> Sp {
			for n in &doc.nodes {
				if let Node::HBox(b) = n {
					if let Some(Node::Leaf(l)) = b.list.first() {
						return l.dims.width;
					}
				}
			}
			Sp::ZERO
		}

		let (dd, _)	= res!(author(fonts.clone(), geom, &Theme::default(), &FaceResolver::default(), &blocks, None, None));
		let mut alpha = Theme::default();
		alpha.enumeration.numbering = Some("(a)".to_string());
		let (da, _)	= res!(author(fonts, geom, &alpha, &FaceResolver::default(), &blocks, None, None));

		assert!(first_marker_width(&dd) > Sp::ZERO, "the default ordered marker has width");
		assert_ne!(first_marker_width(&dd), first_marker_width(&da),
			"a `#set enum(numbering: \"(a)\")` must change the rendered marker from the default \"1.\"");
		Ok(())
	}

	/// The `code`, `figure` and `callout` theme groups reach the renderer: a code block's mono size, a
	/// drawn figure's caption size and a callout's wash are each taken from the theme, so nudging them off
	/// their defaults visibly changes the render.
	#[test]
	fn code_caption_and_callout_read_the_theme() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let faces	= FaceResolver::default();

		// Code size: a taller `code.size` sets taller code lines.
		let code_blocks	= vec![Block::Code { lines: vec!["let x = 1;".to_string()] }];
		let (cd, _)	= res!(author(fonts.clone(), geom, &Theme::default(), &faces, &code_blocks, None, None));
		let mut big_code = Theme::default();
		big_code.code.size = Sp::from_pt(20.0);
		let (cb, _)	= res!(author(fonts.clone(), geom, &big_code, &faces, &code_blocks, None, None));
		let tallest = |doc: &Document| doc.nodes.iter().filter_map(|n| match n {
			Node::HBox(b) if b.dims.height > Sp::ZERO => Some(b.dims.height), _ => None,
		}).fold(Sp::ZERO, |a, h| if h > a { h } else { a });
		assert!(tallest(&cb) > tallest(&cd), "a larger code.size must set taller code lines");

		// Callout fill: the wash colour is the theme's `callout.fill`.
		let box_blocks	= vec![Block::box_callout(
			vec![Block::Paragraph { text: "Inside a callout.".to_string() }], ThemePatch::default())];
		let mut red = Theme::default();
		red.callout.fill = Rgba::opaque(200, 20, 20);
		let (bd, _)	= res!(author(fonts, geom, &red, &faces, &box_blocks, None, None));
		let mut fills = Vec::new();
		collect_fills(&bd.nodes, &mut fills);
		assert!(fills.contains(&Rgba::opaque(200, 20, 20)),
			"the callout wash must be the theme's callout.fill, not a hard-coded colour: {:?}", fills);
		Ok(())
	}

	/// The face resolver reaches a named display face and its weight/slant variants: a theme naming a face
	/// the crate `fonts/` dir ships resolves to a Solo display face (not a role), shaping differently from
	/// the body Bold role; and asking a level for `weight: bold` resolves to the Bold file, distinct from
	/// the Regular. A name with no file falls to the role, unchanged.
	#[test]
	fn resolver_reaches_named_face_and_its_weight_variants() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let dir		= std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts");
		let faces	= FaceResolver::load(&dir, &["LibertinusSerif".to_string()]);
		let size	= Sp::from_pt(16.0);
		let text	= "Chapter Heading";

		let mut style = Theme::default();
		style.heading.face = Some("LibertinusSerif".to_string());

		// A resolvable heading face resolves to a Solo display face, and shapes differently from the body
		// bold role a book falls to when no display face resolves.
		let regular	= resolved_head_face(2, &style, &faces, false);
		assert!(matches!(regular, HeadFace::Solo(_)), "a resolvable face must resolve to a Solo display face");
		let solo_w	= res!(head_shape(&fonts, &regular, size, text)).dims().width;
		let bold_role_w	= res!(head_shape(&fonts, &HeadFace::Role(Role::Bold), size, text)).dims().width;
		assert_ne!(solo_w, bold_role_w, "the resolved display face must shape differently from the body bold");

		// A level asking for bold resolves to the Bold file, distinct in width from the Regular Solo.
		let mut bold_style = style.clone();
		bold_style.heading.levels[1].weight = Some(700);
		let bold	= resolved_head_face(2, &bold_style, &faces, false);
		let bold_w	= res!(head_shape(&fonts, &bold, size, text)).dims().width;
		assert_ne!(solo_w, bold_w, "weight: bold must resolve to the Bold file, not the Regular");

		// A name with no file resolves nothing, so the heading falls to the role as before.
		let mut absent = Theme::default();
		absent.heading.face = Some("NoSuchDisplayFace".to_string());
		assert!(matches!(resolved_head_face(2, &absent, &faces, false), HeadFace::Role(_)),
			"an unresolvable face must fall to a role face");
		Ok(())
	}

	/// A margin note's anchor is zero flow extent: the body it sits in sets byte-identically to the same body
	/// without it. The driver's frames (which carry no marginalia -- that is `decorate`'s overlay) must match
	/// placement for placement whether or not a `#claim-label` splits the paragraph, so the anchor cannot
	/// perturb a line or page break. This is the unit-level shadow of the oracle's byte-identity gate.
	#[test]
	fn margin_anchor_is_zero_extent_body_is_byte_identical() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let style	= Theme::default();
		let body	= "The claim sits amid a paragraph long enough to wrap across several justified \
					lines so a zero-width anchor woven into it has every chance to shift a break if it were \
					not truly weightless, which is exactly what must not happen.";
		// Same body, but a `#claim-label` splits it after the third word -- the anchor lands mid-line.
		let plain	= vec![Block::rich(vec![Segment::text(body)])];
		let (head, tail) = body.split_at(body.find("amid").unwrap_or(0) + 4);
		let noted	= vec![Block::rich(vec![
			Segment::text(head), Segment::margin_note("A1", vec!["A1".to_string()]), Segment::text(tail)])];

		let metrics		= crate::font::FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size);
		let (doc_p, _)	= res!(author(fonts.clone(), geom, &style, &FaceResolver::default(), &plain, None, None));
		let (doc_n, _)	= res!(author(fonts.clone(), geom, &style, &FaceResolver::default(), &noted, None, None));
		let out_p		= res!(crate::driver::run(&doc_p, &metrics, crate::driver::Config::default()));
		let out_n		= res!(crate::driver::run(&doc_n, &metrics, crate::driver::Config::default()));

		assert_eq!(out_p.pages.len(), out_n.pages.len(), "the anchor must not change the page count");
		for (pp, pn) in out_p.pages.iter().zip(out_n.pages.iter()) {
			assert_eq!(pp.frame.placed.len(), pn.frame.placed.len(),
				"the anchor draws no body ink, so the frames must carry the same number of placed boxes");
			for (a, b) in pp.frame.placed.iter().zip(pn.frame.placed.iter()) {
				assert_eq!((a.x, a.y), (b.x, b.y),
					"every body box must land where it did without the anchor");
			}
		}
		// The noted run recorded exactly one margin anchor; the plain run recorded none.
		let count = |o: &crate::driver::CompileOutput| o.ledger.anchors()
			.filter(|a| a.id.kind == crate::ledger::AnchorKind::MarginNote).count();
		assert_eq!(count(&out_n), 1, "the claim label records one margin anchor");
		assert_eq!(count(&out_p), 0, "the plain body records none");
		Ok(())
	}

	/// A body cross-reference slot (keyed `ref-N`) and a reverse-claim-index folio slot must not collide in
	/// the ledger's by-id map: the claim index keys its slots `claim-slot-N`, distinct from `ref_slot`'s
	/// `ref-N`, so a document carrying both an unresolved `#pageref`-style slot AND a claim index records
	/// both anchors rather than one silently overwriting the other (a lost slot, an under-converged folio).
	#[test]
	fn claim_index_slots_do_not_collide_with_body_pagerefs_in_the_ledger() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let style	= Theme::default();
		let blocks	= vec![
			Block::heading(1, "Body"),
			// An unresolved cross-reference (its label names no target, so it falls back to a `ref-1` slot) and
			// a claim reference (gathered into the index) in the same body paragraph.
			Block::rich(vec![
				Segment::text("See "),
				Segment::page_ref("undefined-target"),
				Segment::text(" while claim "),
				Segment::margin_note("", vec!["X1".to_string()]),
				Segment::text(" is referenced here."),
			]),
			Block::ClaimIndex,
		];
		let metrics		= crate::font::FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, style.text.body_size);
		let (doc, _)	= res!(author(fonts.clone(), geom, &style, &FaceResolver::default(), &blocks, None, None));
		let out			= res!(crate::driver::run(&doc, &metrics, crate::driver::Config::default()));
		let keys: Vec<String> = out.ledger.anchors().map(|a| a.id.key.clone()).collect();
		assert!(keys.iter().any(|k| k == "ref-1"),
			"the body cross-reference records its own ref-1 slot: {:?}", keys);
		assert!(keys.iter().any(|k| k == "claim-slot-1"),
			"the claim index records a distinct claim-slot-1 slot, not colliding with ref-1: {:?}", keys);
		Ok(())
	}

	/// The overlay pass draws a recorded margin note in the outside margin -- flush against the block's right
	/// edge on a recto page, and against its left edge on a verso page (which the frame mirror then carries to
	/// the fore-edge) -- at the corpus's 6.5 pt, and never inside the text block.
	#[test]
	fn draw_marginalia_places_the_code_in_the_outside_margin_both_sides() -> Outcome<()> {
		use crate::ledger::{Anchor, Position};	// AnchorId, AnchorKind, Ledger are already in scope

		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let style	= Theme::default();
		let cl		= geom.content_left();
		let cw		= geom.content_width();

		// One ledger, the same code recorded on a recto (page 1) and a verso (page 2) at a mid-block y.
		let y = geom.content_top() + Sp::from_pt(120.0);
		let mut ledger = Ledger::new();
		ledger.record(Anchor::new(AnchorId::new(AnchorKind::MarginNote, "1\u{1f}A1"), Position::new(1, cl, y)));
		ledger.record(Anchor::new(AnchorId::new(AnchorKind::MarginNote, "2\u{1f}B4"), Position::new(2, cl, y)));

		// Recto (page 1): the code's left edge seats at the block's right edge, in the outer margin.
		let mut recto = Page::new(1, geom, Frame::new());
		res!(draw_marginalia(&mut recto, &ledger, &fonts, &style, geom));
		let rp = recto.frame.placed.iter().find(|p| matches!(p.kind, PlacedKind::Text(_)))
			.ok_or_else(|| err!("the recto margin note was not drawn"; Test, Missing))?;
		assert_eq!(rp.x, cl + cw, "a recto note's left edge sits at the block's right (outer) edge");
		assert!(rp.dims.height.raw() > 0, "the note has real shaped extent");

		// Verso (page 2): the code's right edge seats at the block's left edge; the frame mirror carries it out.
		let mut verso = Page::new(2, geom, Frame::new());
		res!(draw_marginalia(&mut verso, &ledger, &fonts, &style, geom));
		let vp = verso.frame.placed.iter().find(|p| matches!(p.kind, PlacedKind::Text(_)))
			.ok_or_else(|| err!("the verso margin note was not drawn"; Test, Missing))?;
		assert_eq!(vp.x + vp.dims.width, cl, "a verso note's right edge sits at the block's left edge");
		Ok(())
	}

	/// `measure_blocks` sizes a block flow without placing it: a paragraph measures a positive height at the
	/// measure width, an empty flow measures zero, and the same blocks measure the same height twice.
	#[test]
	fn measure_blocks_sizes_a_flow() -> Outcome<()> {
		let fonts	= Arc::new(res!(crate::fonts::libertinus()));
		let geom	= PageGeometry::a4();
		let style	= Theme::default();
		let measure	= geom.content_width();
		let refs	= HashMap::new();

		let blocks	= vec![Block::rich(vec![Segment::text(
			"A paragraph with enough words to wrap onto at least a second line at this measure.")])];
		let d1 = res!(measure_blocks(fonts.clone(), geom, &style, measure, &blocks, None, &refs));
		let d2 = res!(measure_blocks(fonts.clone(), geom, &style, measure, &blocks, None, &refs));
		assert_eq!(d1.width, measure, "the measured width is the measure it was set at");
		assert!(d1.height.raw() > 0, "a real paragraph has positive height");
		assert_eq!(d1.height, d2.height, "measuring is deterministic");

		let empty = res!(measure_blocks(fonts, geom, &style, measure, &[], None, &refs));
		assert_eq!(empty.height, Sp::ZERO, "an empty flow measures zero height");
		Ok(())
	}
}
