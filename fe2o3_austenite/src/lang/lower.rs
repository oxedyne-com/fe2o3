//! Lowering from the Ingot surface tree to the block layer.
//!
//! The seam between front end and engine: a surface [`Item`](super::ast::Item) becomes a
//! [`doc::Block`](crate::doc::Block) the two-pass driver already knows how to set. A heading lowers to
//! a heading; a paragraph of plain prose to a plain [`Block::Paragraph`], and a paragraph carrying any
//! emphasis to a [`Block::RichParagraph`] of [`Segment`]s -- keeping the fast single-role break path
//! for the common case. The source spans are dropped here, since the block layer does not yet carry
//! them. As the surface language grows, this is where a richer item collapses to what the engine sets.

use crate::doc::{
	Block,
	Segment,
};
use crate::table::{
	Align,
	Cell,
	Row,
	Table,
};

use super::ast::{
	AlignSpec,
	FigureBody,
	Inline,
	Item,
	TableSpec,
};

/// Lowers a surface item list to the block list the driver authors from.
pub fn blocks(items: &[Item]) -> Vec<Block> {
	let mut out = Vec::with_capacity(items.len());
	for item in items {
		match item {
			Item::Heading { level, runs, label, .. }	=> out.push(
				Block::heading_rich(*level, runs.iter().map(lower_inline).collect(), label.clone())),
			Item::Paragraph { runs, .. }		=> out.push(lower_paragraph(runs)),
			Item::List { ordered, items, .. }	=> out.push(Block::list(
				*ordered,
				items.iter().map(|item| item.iter().map(lower_inline).collect()).collect())),
			Item::Code { lines, .. }			=> out.push(Block::code(lines.clone())),
			Item::Table { spec, .. }			=> out.push(Block::table(build_table(spec))),
			Item::Figure { body, caption, supplement, label, .. }	=> out.push(match body {
				FigureBody::Table(spec)	=> Block::table_figure(
					build_table(spec), caption.clone(), supplement.clone(), label.clone()),
				FigureBody::Image { path, width, height, scale }	=> Block::image_figure(
					path.clone(), *width, *height, *scale,
					caption.clone(), supplement.clone(), label.clone()),
			}),
		}
	}
	out
}

/// Builds a [`Table`] from the parsed spec: the flat cells are chunked into rows of `ncols`, each cell
/// given its alignment from the [`AlignSpec`]. A header row's cells set centred; a closure aligns the
/// first column centred and the rest flush left, matching the `(col, row) => ...` idiom these books use.
fn build_table(spec: &TableSpec) -> Table {
	let ncols = spec.ncols.max(1);
	let mut rows:	Vec<Row>	= Vec::new();
	for (r, chunk) in spec.cells.chunks(ncols).enumerate() {
		let mut cells = Vec::with_capacity(ncols);
		for (c, text) in chunk.iter().enumerate() {
			cells.push(Cell::aligned(text.clone(), cell_align(&spec.align, spec.header, r, c)));
		}
		rows.push(Row::new(cells));
	}
	Table::new(spec.header, rows)
}

/// The alignment of one cell at row `r`, column `c`, given the table's declared [`AlignSpec`].
fn cell_align(spec: &AlignSpec, header: bool, r: usize, c: usize) -> Align {
	if header && r == 0 {
		return Align::Centre;	// a header row centres its labels
	}
	match spec {
		AlignSpec::Uniform(a)		=> *a,
		AlignSpec::PerColumn(cols)	=> cols.get(c).copied().unwrap_or(Align::Left),
		AlignSpec::Closure			=> if c == 0 { Align::Centre } else { Align::Left },
	}
}

/// Lowers a paragraph's inline runs. A paragraph of one plain text run keeps the plain-paragraph path
/// (a single-role Knuth-Plass break); the moment it carries an emphasis run it becomes a rich paragraph
/// of segments, which the driver breaks with a face per run.
fn lower_paragraph(runs: &[Inline]) -> Block {
	match runs {
		[Inline::Text(text)]	=> Block::paragraph(text.clone()),
		// A paragraph that is nothing but one maths span is a display equation on its own line.
		[Inline::Math(atom)]	=> Block::equation(atom.clone(), false),
		_						=> Block::rich(runs.iter().map(lower_inline).collect()),
	}
}

fn lower_inline(run: &Inline) -> Segment {
	match run {
		Inline::Text(text)		=> Segment::text(text.clone()),
		Inline::Strong(text)	=> Segment::strong(text.clone()),
		Inline::Emph(text)		=> Segment::emph(text.clone()),
		Inline::PageRef(label)	=> Segment::page_ref(label.clone()),
		Inline::Code(text)		=> Segment::code(text.clone()),
		Inline::Math(atom)		=> Segment::math(atom.clone()),
		Inline::Glossary { term, display }
								=> Segment::glossary(term.clone(), display.clone()),
		Inline::Footnote(note)	=> Segment::footnote(note.clone()),
		Inline::Cite(keys)		=> Segment::cite(keys.clone()),
	}
}
