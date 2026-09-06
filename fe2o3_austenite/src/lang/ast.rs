//! The surface syntax tree of a Typst source file, before lowering to [`doc::Block`](crate::doc::Block).
//!
//! Increment 2 adds inline emphasis to the markup spine: a document is a sequence of headings and
//! paragraphs, and a paragraph is a sequence of [`Inline`] runs -- plain text, `*strong*`, `_emph_`.
//! The `#` code mode and the declared-query references of `sec_language` are later increments -- the
//! tree names only what the engine can already set.

use crate::ir::Length;
use crate::ir::Span;
use crate::math::Atom;
use crate::table::Align;

/// One block of Ingot markup. The byte span is carried for a future diagnostic caret; the driver's
/// `Span` model already reserves it, so the front end records it from the first increment.
#[derive(Clone, Debug)]
pub enum Item {
	Heading { level: u8, runs: Vec<Inline>, label: Option<String>, span: Span },	// label: a trailing <name>, runs: the title's inline markup
	Paragraph { runs: Vec<Inline>, label: Option<String>, span: Span },	// label: a trailing <name>, anchoring a display equation for cross-reference
	List { ordered: bool, items: Vec<Vec<Inline>>, span: Span },	// `-` bullets or `+` numbered
	Code { lines: Vec<String>, span: Span },	// a ```-fenced block, set verbatim in the mono face
	Table { spec: TableSpec, span: Span },	// a bare `#table(...)`, not wrapped in a figure
	Figure { body: FigureBody, caption: Option<Vec<Inline>>, supplement: String, label: Option<String>, span: Span },	// caption: the caption's inline markup
}

/// What a `#figure(...)` wraps: a `#table(...)` this reader sets in full, or an image call whose ink is
/// deferred to a later increment and stood in for by a sized placeholder box.
#[derive(Clone, Debug)]
pub enum FigureBody {
	Table(TableSpec),
	// The image path and any sizing the call declared: `width`/`height` from `image(...)`, `scale` from
	// `padded-image(...)`. A hint the call omits is `None`, and the figure fills the measure.
	Image { path: String, width: Option<Length>, height: Option<Length>, scale: Option<f64> },
}

/// A parsed Typst `#table(...)` call, before it is built into a [`table::Table`](crate::table::Table).
/// Each cell keeps its inline runs, row-major, so a bold header, an italic word, a superscript or an
/// in-cell maths span sets with its own face rather than flattening to upright text; `header` is set when
/// a `fill:` keys the first row; `align` records the column alignment the call declared.
#[derive(Clone, Debug)]
pub struct TableSpec {
	pub ncols:		usize,
	pub header:		bool,
	pub align:		AlignSpec,
	pub weights:	Vec<f64>,		// the `Nfr` weight per column; 0.0 for an `auto`/fixed track, empty for a bare `columns: N`
	pub text_pt:	Option<f64>,	// a `text(size: Npt)[...]` wrapper's size, so a small table sets small
	pub inset_pt:	Option<f64>,	// the `inset:` cell padding in points, overriding the default
	pub cells:		Vec<Vec<Inline>>,	// flat, row-major; each cell a run of inline markup
}

/// How a table's cells align. `Uniform` sets every cell alike; `PerColumn` gives each column its own
/// alignment (a header row still sets centred); `Closure` is the common `(col, row) => ...` idiom these
/// books use -- a centred header, a centred first column, everything else flush left.
#[derive(Clone, Debug)]
pub enum AlignSpec {
	Uniform(Align),
	PerColumn(Vec<Align>),
	Closure,
}

/// One inline run of a paragraph: ordinary prose, a run marked for emphasis, a cross-reference, or an
/// inline code span. Nesting -- an emphasis inside another -- is a later increment, so a run's text is
/// flat.
#[derive(Clone, Debug)]
pub enum Inline {
	Text(String),
	Strong(String),	// *strong*, lowered to a bold segment
	Emph(String),	// _emph_ or #emph[...], lowered to an italic segment
	Super(String),	// #super[...], lowered to a raised, smaller segment
	PageRef(String),	// @label, resolving to the labelled anchor's page number
	Code(String),	// `raw` or #raw("..."), set in the mono face
	Math(Atom),		// $...$, parsed to the engine's maths tree
	Glossary { term: String, display: String },	// a glossary term: bold-italic on its first document use
	Footnote(Vec<Inline>),	// #footnote[...], its note markup set at the foot of the page its mark lands on
	Cite(Vec<String>),	// #cite(<key>) or #cite(<a>, <b>), resolved to (Author Year) against the bibliography
}
