//! The box, glue and penalty intermediate representation.
//!
//! This is TeX's model: vertical and horizontal material is a stream of boxes (rigid rectangles),
//! glue (stretchable, shrinkable space) and penalties (the cost of breaking at a point). Austenite
//! adds an anchor node, a zero-size marker that records where an identity landed so the ledger can
//! resolve references to it.
//!
//! Lengths are [`Sp`], scaled points, per the architecture's reproducibility rule. Floating point
//! appears only at the output boundary, where a coordinate becomes a device length.

use crate::font::ShapedText;
use crate::ledger::{
	AnchorId,
	Ref,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::Path,
};

use std::sync::Arc;

/// A length in scaled points: one sixty-five-thousand-five-hundred-and-thirty-sixth of a point, as
/// in TeX. Integer arithmetic makes every break decision exact and every build byte-identical, which
/// is the whole reason the type is not an `f64`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sp(pub i32);

impl Sp {
	pub const ZERO:	Sp	= Sp(0);
	pub const UNIT:	i32	= 65_536;	// scaled points to the point

	/// Converts a length in points to scaled points, rounding to the nearest unit. This is a
	/// boundary conversion; once inside the engine a length never leaves the integer domain.
	pub fn from_pt(pt: f64) -> Self {
		Sp((pt * Sp::UNIT as f64).round() as i32)
	}

	/// The length in points, for the output boundary only.
	pub fn to_pt(self) -> f64 {
		self.0 as f64 / Sp::UNIT as f64
	}

	pub fn raw(self) -> i32 { self.0 }
}

impl std::ops::Add for Sp {
	type Output = Sp;
	fn add(self, other: Sp) -> Sp { Sp(self.0.saturating_add(other.0)) }
}

impl std::ops::Sub for Sp {
	type Output = Sp;
	fn sub(self, other: Sp) -> Sp { Sp(self.0.saturating_sub(other.0)) }
}

impl std::ops::Neg for Sp {
	type Output = Sp;
	fn neg(self) -> Sp { Sp(self.0.saturating_neg()) }
}

impl std::ops::Mul<i32> for Sp {
	type Output = Sp;
	fn mul(self, k: i32) -> Sp { Sp(self.0.saturating_mul(k)) }
}

impl std::ops::AddAssign for Sp {
	fn add_assign(&mut self, other: Sp) { self.0 = self.0.saturating_add(other.0); }
}

impl ToDat for Sp {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(dat!(self.0))
	}
}

impl FromDat for Sp {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		Ok(Sp(try_extract_dat!(dat, I32)))
	}
}

/// A span of the source a box came from, in byte offsets, so a diagnostic can quote it. The
/// architecture makes this universal; Phase 0 carries it but does not yet render carets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
	pub start:	u32,
	pub end:	u32,
}

impl Span {
	pub fn new(start: u32, end: u32) -> Self { Self { start, end } }
}

/// The three measurements a box occupies, split at the baseline as TeX splits them: `height` reaches
/// up from the baseline, `depth` hangs below it, and the two sum to the visual extent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dims {
	pub width:	Sp,
	pub height:	Sp,
	pub depth:	Sp,
}

impl Dims {
	pub fn new(width: Sp, height: Sp, depth: Sp) -> Self {
		Self { width, height, depth }
	}

	/// The vertical extent, height above the baseline plus depth below it, which is what a vertical
	/// list advances by when it stacks this box.
	pub fn vextent(&self) -> Sp { self.height + self.depth }
}

impl ToDat for Dims {
	fn to_dat(&self) -> Outcome<Dat> {
		Ok(listdat![
			res!(self.width.to_dat()),
			res!(self.height.to_dat()),
			res!(self.depth.to_dat()),
		])
	}
}

impl FromDat for Dims {
	fn from_dat(dat: Dat) -> Outcome<Self> {
		let v = try_extract_dat!(dat, List);
		if v.len() != 3 {
			return Err(err!(
				"Dims expects a list of three scaled lengths, found {}.", v.len();
				Input, Invalid, Mismatch));
		}
		Ok(Self {
			width:	res!(Sp::from_dat(v[0].clone())),
			height:	res!(Sp::from_dat(v[1].clone())),
			depth:	res!(Sp::from_dat(v[2].clone())),
		})
	}
}

/// Space that can grow and shrink. `natural` is the length at rest, `stretch` the amount it will
/// grow when a line or page is loose, `shrink` the amount it will give up when tight.
///
/// Phase 0 is finite glue only. TeX's infinite orders (fil, fill, filll), which centre and fill,
/// are a Phase 1 addition and belong here as a `stretch_order` beside the amount.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Glue {
	pub natural:	Sp,
	pub stretch:	Sp,
	pub shrink:		Sp,
}

impl Glue {
	pub fn fixed(natural: Sp) -> Self {
		Self { natural, stretch: Sp::ZERO, shrink: Sp::ZERO }
	}

	pub fn new(natural: Sp, stretch: Sp, shrink: Sp) -> Self {
		Self { natural, stretch, shrink }
	}
}

/// The cost of breaking a line or page at a point, and whether the break is flagged (two flagged
/// breaks in a row are themselves penalised, which is how TeX avoids two hyphens ending consecutive
/// lines). A cost of [`Penalty::INFINITY`] forbids a break; [`Penalty::EJECT`] forces one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Penalty {
	pub cost:		i32,
	pub flagged:	bool,
	pub strong:		bool,	// a strong eject: ejects even on an empty page (a trailing/consecutive `#pagebreak()`), never set on any other break
	pub column:		bool,	// a forced column break (`#colbreak()`): the next column, or the next page from the last
}

impl Penalty {
	pub const INFINITY:	i32	= 10_000;	// a break here is forbidden
	pub const EJECT:	i32	= -10_000;	// a break here is forced

	pub fn new(cost: i32, flagged: bool) -> Self {
		Self { cost, flagged, strong: false, column: false }
	}

	/// A forced break, which the page breaker must take -- a chapter end, section furniture, or a weak
	/// `#pagebreak(weak: true)`. The driver drops it on an already-empty page, so it opens no blank page.
	pub fn eject() -> Self { Self { cost: Self::EJECT, flagged: false, strong: false, column: false } }

	/// A strong forced break -- the default `#pagebreak()`. Ejects unconditionally, opening a blank page when
	/// it lands on an already-empty page or trails the document, matching Typst 0.15.1's strong pagebreak.
	pub fn strong_eject() -> Self { Self { cost: Self::EJECT, flagged: false, strong: true, column: false } }

	/// A forced column break -- Typst's `#colbreak()`, strong unless `weak`. In a layout of several columns
	/// it moves the flow to the next column, or to the next page from the last; on a page of one column it
	/// is a page break, as Typst makes it. A weak one is dropped in a column that holds nothing yet.
	pub fn column_eject(weak: bool) -> Self { Self { cost: Self::EJECT, flagged: false, strong: !weak, column: true } }

	/// Is a break at this penalty forbidden?
	pub fn is_forbidden(&self) -> bool { self.cost >= Self::INFINITY }

	/// Is a break at this penalty forced?
	pub fn is_forced(&self) -> bool { self.cost <= Self::EJECT }

	/// Is this a strong forced eject, one that opens a page even where the current one is empty?
	pub fn is_strong(&self) -> bool { self.strong }

	/// Is this a column break rather than a page break?
	pub fn is_column(&self) -> bool { self.column }
}

/// A footnote: the superscript mark set in the running text, and the note set at the foot of the page
/// the mark lands on. The number is assigned at author time as a document-order fold, so it is content
/// order and never a layout query. `note` is the note already set as a small paragraph (the lines and
/// their leading), prefixed by its own superscript number; `height` is what that stack occupies, which
/// the page breaker reserves from the body of the page the mark falls on.
#[derive(Clone, Debug)]
pub struct Footnote {
	pub number:	u32,
	pub mark:	ShapedText,	// the superscript number drawn where the mark falls
	pub note:	Vec<Node>,	// the note as HBox lines and interline glue, set at the foot measure
	pub height:	Sp,			// the note's stacked vertical extent, reserved from the body
}

/// A decoded raster image: straight, eight-bit RGBA samples, row-major with the top row first, ready to
/// hand to the PDF and SVG writers. Held behind an [`Arc`] in a [`DrawOp::Image`] so a figure's pixels
/// are shared, not copied, as the graphic rides the stream.
#[derive(Clone, Debug)]
pub struct RasterImage {
	pub width:	usize,		// samples across
	pub height:	usize,		// samples down
	pub rgba:	Vec<u8>,	// width * height * 4 straight-RGBA bytes, top row first
}

/// A hint from an `image(...)` or `padded-image(...)` call for how large to draw a figure: a fraction of
/// the measure (`50%`) or an absolute length in points (`4cm`). An axis with no hint is taken from the
/// other axis and the image's own aspect, and a figure with no hint at all fills the measure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
	Rel(f64),	// a fraction of the container measure
	Abs(f64),	// an absolute length in points
}

/// One drawing operation within a [`Graphic`]: a filled or stroked path, or a placed raster, in the
/// graphic's own frame, which is y down and in points, so placing the graphic needs only a translation.
#[derive(Clone, Debug)]
pub enum DrawOp {
	Fill { path: Path, colour: Rgba },
	Stroke { path: Path, colour: Rgba, width: f32 },	// stroke width in points
	// A raster drawn to fill the rectangle at top-left (x, y), w wide and h tall, in the graphic's frame.
	Image { image: Arc<RasterImage>, x: f32, y: f32, w: f32, h: f32 },
}

/// Where a link points: out of the document to a URI, or into it to a named anchor. An internal target
/// carries the anchor's identity, not a page or a block address, so it stays stable as the document
/// repaginates -- the reader resolves it against the shipped ledger, which turns the identity into the
/// page it landed on and then into the content-addressed block of that page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
	Uri(String),		// an external address, followed as-is
	Anchor(AnchorId),	// an internal cross-reference, resolved through the ledger to a block and page
}

/// A self-contained piece of drawn ink -- a diagram, a figure, a baked label run -- as a bag of paths
/// with a bounding box. It rides the stream as a [`LeafKind::Graphic`] leaf and is placed like any box;
/// the emitter translates its ops to where it landed and draws them. Every path is already flattened
/// here, a text label's glyph outlines included, so a built graphic never reaches back into shaping.
#[derive(Clone, Debug)]
pub struct Graphic {
	pub ops:	Vec<DrawOp>,
	pub dims:	Dims,
	pub link:	Option<LinkTarget>,	// where the whole graphic links, drawn over its placement box
}

impl Graphic {
	pub fn new(ops: Vec<DrawOp>, dims: Dims) -> Self {
		Self { ops, dims, link: None }
	}

	/// Makes the whole graphic a clickable link to the external `url` -- the PDF writer draws a link
	/// annotation over its placement box, and Pearl carries it as a `link` leaf. The meta page's "Made
	/// with AI" chip carries the scheme URL this way; the SVG writer, which sets the mark as a plain
	/// image, leaves it unlinked.
	pub fn with_link(mut self, url: String) -> Self {
		self.link = Some(LinkTarget::Uri(url));
		self
	}

	/// Makes the whole graphic a cross-reference to `anchor` within the document, resolved through the
	/// ledger at read time to the content-addressed block the anchor landed in.
	pub fn with_link_anchor(mut self, anchor: AnchorId) -> Self {
		self.link = Some(LinkTarget::Anchor(anchor));
		self
	}
}

/// What an atomic box draws. `Reserved` holds open the width a forward reference will need once the
/// ledger resolves it, which is what lets two passes suffice by construction. Its first `bool` is whether
/// the slot holds that reserved width even when the resolved value is narrower: true for right-aligned
/// furniture (a table-of-contents folio), whose column must stay put, and false for a reference set in
/// running prose, which shrinks to the value so it reads without a gap. Its second `bool` is whether the
/// resolved value sets bold rather than in the body face -- a main index reference's folio, reproducing
/// in-dexter's `index-main = index.with(fmt: strong)`.
#[derive(Clone, Debug)]
pub enum LeafKind {
	Rule,
	Reserved(AnchorId, Ref, bool, bool),	// a forward reference: its identity, what it resolves to, whether it holds width, whether it sets bold
	Text(ShapedText),			// a shaped run of real text, drawn as glyph outlines
	Mark(Footnote),				// a footnote reference mark; its note is set at the page foot
	Graphic(Arc<Graphic>),		// a self-contained figure, its ops drawn at the leaf's placement
}

/// An atomic box: intrinsic dimensions, what it draws, and where in the source it came from.
#[derive(Clone, Debug)]
pub struct Leaf {
	pub kind:	LeafKind,
	pub dims:	Dims,
	pub shift:	Sp,				// downward offset applied at placement; positive lowers, negative raises
	pub span:	Option<Span>,
}

impl Leaf {
	pub fn rule(dims: Dims) -> Self {
		Self { kind: LeafKind::Rule, dims, shift: Sp::ZERO, span: None }
	}

	/// A forward reference whose slot holds its reserved width even when the value comes out narrower --
	/// for a right-aligned folio in a table of contents, where the column must not move.
	pub fn reserved(id: AnchorId, refr: Ref, dims: Dims) -> Self {
		Self { kind: LeafKind::Reserved(id, refr, true, false), dims, shift: Sp::ZERO, span: None }
	}

	/// A forward reference set in running prose: its slot shrinks to the resolved value, so a page number
	/// reads tightly in the sentence rather than trailing a gap the reservation held open.
	pub fn reserved_inline(id: AnchorId, refr: Ref, dims: Dims) -> Self {
		Self { kind: LeafKind::Reserved(id, refr, false, false), dims, shift: Sp::ZERO, span: None }
	}

	/// A forward reference set in running prose whose resolved value sets bold -- a main index reference's
	/// folio, reproducing in-dexter's `index-main = index.with(fmt: strong)`. It shrinks to the value like
	/// any inline reference.
	pub fn reserved_inline_bold(id: AnchorId, refr: Ref, dims: Dims) -> Self {
		Self { kind: LeafKind::Reserved(id, refr, false, true), dims, shift: Sp::ZERO, span: None }
	}

	/// A leaf of real shaped text, taking its dimensions from the run.
	pub fn text(shaped: ShapedText) -> Self {
		let dims = shaped.dims();
		Self { kind: LeafKind::Text(shaped), dims, shift: Sp::ZERO, span: None }
	}

	/// A leaf of shaped text with dimensions the caller sets rather than the run's own, so a run can be
	/// raised or seated within a taller line -- a footnote's superscript number, say.
	pub fn text_dims(shaped: ShapedText, dims: Dims) -> Self {
		Self { kind: LeafKind::Text(shaped), dims, shift: Sp::ZERO, span: None }
	}

	/// A footnote mark. `dims` is the superscript's box, its height reduced so the baseline the emitter
	/// draws at (`y + height`) sits raised above the surrounding line's baseline, and its width small
	/// enough that line breaking flows around it as around any narrow box.
	pub fn mark(footnote: Footnote, dims: Dims) -> Self {
		Self { kind: LeafKind::Mark(footnote), dims, shift: Sp::ZERO, span: None }
	}

	/// A graphic leaf: a figure set as one box, its ink the graphic's own paths. The dims are the
	/// graphic's bounding box, so the vertical list stacks it and a line flows around it as any box.
	pub fn graphic(graphic: Graphic) -> Self {
		let dims = graphic.dims;
		Self { kind: LeafKind::Graphic(Arc::new(graphic)), dims, shift: Sp::ZERO, span: None }
	}

	pub fn with_span(mut self, span: Span) -> Self {
		self.span = Some(span);
		self
	}

	/// Sets the vertical shift applied when the leaf is placed, so a glyph run or a rule can sit above
	/// or below its line's baseline without a nested box. Maths uses this to stack a fraction's parts
	/// and to raise a script; a positive shift lowers the leaf, a negative one raises it.
	pub fn with_shift(mut self, shift: Sp) -> Self {
		self.shift = shift;
		self
	}
}

/// A box holding a list set either horizontally or vertically, with its own resolved dimensions. The
/// orientation lives in the enclosing [`Node`] variant, not here, because it is the parent's list
/// direction that decides how these children stack.
#[derive(Clone, Debug)]
pub struct BoxNode {
	pub list:	Vec<Node>,
	pub dims:	Dims,
}

impl BoxNode {
	pub fn new(list: Vec<Node>, dims: Dims) -> Self {
		Self { list, dims }
	}
}

/// Where a float asks to sit on the page it settles on. `Auto` (Typst's `placement: auto`) lets the
/// engine choose between top and foot by the midpoint rule -- the side the float's centre would fall on
/// were it set in the flow. `Top` and `Bottom` pin the side (Typst's `placement: top`/`bottom`). `Auto`
/// is a distinct variant, not a synonym for `Top`, because the choice it defers is the whole point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatPlacement {
	Auto,
	Top,
	Bottom,
}

/// How wide a float runs: within the column it is met in, or across every column of the page (Typst's
/// `scope: "column"` and `scope: "parent"`). On a page of one column the two coincide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FloatScope {
	#[default]
	Column,
	Parent,
}

/// Where a float settles and how wide it runs: Typst's `placement` and `scope` taken together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Floating {
	pub side:	FloatPlacement,
	pub scope:	FloatScope,
}

impl Floating {
	/// A float within its column, Typst's default scope.
	pub fn column(side: FloatPlacement) -> Self {
		Self { side, scope: FloatScope::Column }
	}
}

/// A block-level float: self-contained vertical material -- a figure and its caption, or an aside box --
/// set at the top or foot of a page rather than in the flow, its parts kept together (Typst's
/// `figure(placement: auto | top | bottom)`).
///
/// `list` is the material as a small vertical list (no block spacing around it -- Typst frames a float
/// with `clearance`, not paragraph glue). `height` is the material's own extent, what the break weighs.
/// `clearance` is the gap between the float and the body (Typst's `place.clearance`, default 1.5em of the
/// float's font size), applied above a foot float and below a top float, and only when the page carries
/// other content. `scope` decides whether it settles within its column or spans the page's columns. A float
/// is only ever a top-level node of the document stream, never nested in a line or keep box.
#[derive(Clone, Debug)]
pub struct FloatNode {
	pub list:		Vec<Node>,
	pub height:		Sp,
	pub clearance:	Sp,
	pub placement:	FloatPlacement,
	pub scope:		FloatScope,
}

impl FloatNode {
	pub fn new(list: Vec<Node>, height: Sp, clearance: Sp, floating: Floating) -> Self {
		Self { list, height, clearance, placement: floating.side, scope: floating.scope }
	}
}

/// The column layout a run of pages flows in: `count` equal columns parted by `gutter` (Typst's
/// `page.columns` and `columns.gutter`). One column is the ordinary page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageColumns {
	pub count:	usize,
	pub gutter:	Sp,
}

impl PageColumns {
	pub const ONE: Self = Self { count: 1, gutter: Sp::ZERO };

	pub fn new(count: usize, gutter: Sp) -> Self {
		Self { count: count.max(1), gutter }
	}
}

/// A block of vertical material set in equal side-by-side columns, filled sequentially: the first column
/// fills top to bottom, then the flow hops to the next column at the same top, and when the last column
/// fills the driver breaks to a fresh page's first column. This is Typst's `columns(n)` -- no balancing,
/// the last column of the last page simply ends where the material runs out.
///
/// `list` is the material as an ordinary vertical box-glue-penalty stream, exactly what the page body
/// flows, so the same greedy atom-aware breaker sets it; `count` is the number of columns and `gutter`
/// the space between two adjacent columns. A columns block is only ever a top-level node of the document
/// stream, never nested in a line, keep box or float.
#[derive(Clone, Debug)]
pub struct ColumnsNode {
	pub list:	Vec<Node>,
	pub count:	usize,
	pub gutter:	Sp,
}

impl ColumnsNode {
	pub fn new(list: Vec<Node>, count: usize, gutter: Sp) -> Self {
		Self { list, count, gutter }
	}
}

/// One item of a box-glue-penalty list: the closed vocabulary the whole engine is built on.
#[derive(Clone, Debug)]
pub enum Node {
	HBox(BoxNode),		// a list set left to right
	VBox(BoxNode),		// a list set top to bottom
	Leaf(Leaf),
	Glue(Glue),
	Penalty(Penalty),
	Anchor(AnchorId),	// a zero-size marker recording where an identity landed
	Float(FloatNode),	// a block-level float, deferred by the driver to the top or foot of a later page
	Columns(ColumnsNode),	// a block flowed into equal side-by-side columns, filled left to right
	// A zero-size marker setting the column layout the following pages flow in (Typst's `set page(columns:)`):
	// a page already carrying material is closed first, so the new layout starts on a fresh page. Like
	// `RepeatHead`, a driver-time control node, never ToDat-serialised.
	PageColumns(PageColumns),
	// A zero-size marker arming (Some) or disarming (None) a repeated header: while armed, the driver
	// stamps the boxed header at the top of every fresh page the following material spills onto, so a
	// breakable table's column heads repeat down a multi-page run (Typst's `table.header` repeat). It is
	// never ToDat-serialised -- it is a driver-time control node the lowerer weaves, not shipped IR -- so
	// adding it changes no on-disc format. Transparent to breakability, like `Anchor`.
	RepeatHead(Option<Box<BoxNode>>),
}

impl Node {
	/// The amount a vertical list advances when it stacks this node. A box or leaf contributes its
	/// height plus depth, glue its natural length, and a penalty or anchor nothing -- they mark a
	/// position without occupying one.
	pub fn vextent(&self) -> Sp {
		match self {
			Node::HBox(b)		=> b.dims.vextent(),
			Node::VBox(b)		=> b.dims.vextent(),
			Node::Leaf(l)		=> l.dims.vextent(),
			Node::Glue(g)		=> g.natural,
			Node::Penalty(_)	=> Sp::ZERO,
			Node::Anchor(_)		=> Sp::ZERO,
			// A float takes no space where it stands: it leaves the flow, and the driver charges its height
			// against the page it settles on, not this position.
			Node::Float(_)		=> Sp::ZERO,
			// A columns block is flowed by the driver's own multi-column pass, which advances the cursor
			// itself; it contributes no simple vertical extent to weigh where it stands.
			Node::Columns(_)	=> Sp::ZERO,
			// A repeated-header marker is a zero-size control node: it arms or disarms the driver's header
			// repeat and occupies no vertical space where it stands.
			Node::RepeatHead(_)	=> Sp::ZERO,
			// A column-layout marker occupies nothing; the driver acts on it between pages.
			Node::PageColumns(_)	=> Sp::ZERO,
		}
	}

	/// Is this a legal place to break a page? A page may break at glue that follows a non-discardable
	/// item, and at a penalty that is not forbidden. Phase 0 keeps the rule to those two, which is
	/// enough to paginate; widow and orphan penalties join it in Phase 2.
	pub fn is_breakpoint(&self) -> bool {
		match self {
			Node::Glue(_)		=> true,
			Node::Penalty(p)	=> !p.is_forbidden(),
			_					=> false,
		}
	}
}

/// A source of glyph and box metrics: the seam for real typesetting. Implemented over `fe2o3_font`
/// by [`FontMetrics`](crate::font::FontMetrics), and by [`StubMetrics`] for running without a font.
pub trait Metrics {
	fn measure(&self, text: &str) -> Outcome<Dims>;

	/// Shapes text into a placeable run when a font backs this metric, or `None` for the fontless
	/// stub. A forward reference resolved against a font metric is shaped and drawn here as real
	/// glyphs; against the stub it stays a reservation, measured but not drawn.
	fn shape(&self, text: &str) -> Outcome<Option<ShapedText>>;

	/// Shapes text in the bold face rather than the metric's own -- a main index reference's folio, which
	/// in-dexter sets `strong`. The fontless stub ignores the face and behaves as [`shape`](Metrics::shape).
	fn shape_bold(&self, text: &str) -> Outcome<Option<ShapedText>>;
}

/// A placeholder metric: every character one fixed em wide, one em tall, a fixed depth. It runs the
/// driver without a font, beside the real [`FontMetrics`](crate::font::FontMetrics).
#[derive(Clone, Copy, Debug)]
pub struct StubMetrics {
	pub em:		Sp,	// advance and body height of one character
	pub depth:	Sp,	// descent below the baseline
}

impl StubMetrics {
	pub fn new(em: Sp, depth: Sp) -> Self {
		Self { em, depth }
	}
}

impl Metrics for StubMetrics {
	fn measure(&self, text: &str) -> Outcome<Dims> {
		let n = text.chars().count() as i32;
		Ok(Dims::new(self.em * n, self.em, self.depth))
	}

	fn shape(&self, _text: &str) -> Outcome<Option<ShapedText>> {
		Ok(None)	// no font behind the stub, so nothing to shape into glyphs
	}

	fn shape_bold(&self, _text: &str) -> Outcome<Option<ShapedText>> {
		Ok(None)	// no font behind the stub, so the face makes no difference
	}
}
