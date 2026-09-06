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
use crate::ir::Sp;
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
			Item::Paragraph { runs, label, .. }	=> out.push(lower_paragraph(runs, label.clone())),
			Item::List { ordered, items, .. }	=> out.push(Block::list(
				*ordered,
				items.iter().map(|item| item.iter().map(lower_inline).collect()).collect())),
			Item::Code { lines, .. }			=> out.push(Block::code(lines.clone())),
			Item::Table { spec, .. }			=> out.push(Block::table(build_table(spec))),
			Item::Rule { width, thickness, grey, .. }	=> out.push(Block::rule(*width, *thickness, *grey)),
			Item::Figure { body, caption, supplement, label, .. }	=> {
				let caption = caption.as_ref().map(|runs| runs.iter().map(lower_inline).collect());
				out.push(match body {
					FigureBody::Table(spec)	=> Block::table_figure(
						build_table(spec), caption, supplement.clone(), label.clone()),
					FigureBody::Image { path, width, height, scale }	=> Block::image_figure(
						path.clone(), *width, *height, *scale,
						caption, supplement.clone(), label.clone()),
				});
			},
		}
	}
	out
}

/// Builds a [`Table`] from the parsed spec: the flat cells are chunked into rows of `ncols`, each cell
/// carrying its inline runs lowered to segments and its alignment from the [`AlignSpec`]. A header row's
/// cells set centred under the fixed forms; a closure is evaluated per cell, so a `(col, row) => ...`
/// spec sets each cell exactly as its own row/column logic dictates.
fn build_table(spec: &TableSpec) -> Table {
	let ncols = spec.ncols.max(1);
	let mut rows:	Vec<Row>	= Vec::new();
	for (r, chunk) in spec.cells.chunks(ncols).enumerate() {
		let mut cells = Vec::with_capacity(ncols);
		for (c, runs) in chunk.iter().enumerate() {
			let content = runs.iter().map(lower_inline).collect();
			cells.push(Cell::rich(content, cell_align(&spec.align, spec.header, r, c)));
		}
		rows.push(Row::new(cells));
	}
	// A `columns: (2fr, 5fr, ...)` track list sizes the columns fractionally, as Typst does; a bare
	// `columns: N` carries no weights and the columns fall back to content sizing. A weight list shorter
	// than the columns is padded with content-sized zeros so every column has an entry.
	let mut table = if spec.weights.iter().any(|&w| w > 0.0) {
		let mut weights = spec.weights.clone();
		weights.resize(ncols, 0.0);
		Table::with_weights(spec.header, rows, weights)
	} else {
		Table::new(spec.header, rows)
	};
	// A `text(size: Npt)` wrapper (the books set their claim tables at 7 pt) and an explicit `inset:` cell
	// padding carry through, so the table sets at the oracle's reduced size rather than the body size.
	table.text_size	= spec.text_pt.map(Sp::from_pt);
	table.inset		= spec.inset_pt.map(Sp::from_pt);
	table
}

/// The alignment of one cell at row `r`, column `c`, given the table's declared [`AlignSpec`]. A closure
/// carries its own row/column logic and is evaluated for every cell, header row included; the fixed
/// forms have no row dependence, so a header row centres its labels as Typst's book style does.
fn cell_align(spec: &AlignSpec, header: bool, r: usize, c: usize) -> Align {
	if let AlignSpec::Closure(cl) = spec {
		return cl.align_at(c, r);
	}
	if header && r == 0 {
		return Align::Centre;	// a header row centres its labels
	}
	match spec {
		AlignSpec::Uniform(a)		=> *a,
		AlignSpec::PerColumn(cols)	=> cols.get(c).copied().unwrap_or(Align::Left),
		AlignSpec::Closure(cl)		=> cl.align_at(c, r),
	}
}

/// Lowers a paragraph's inline runs. A paragraph of one plain text run keeps the plain-paragraph path
/// (a single-role Knuth-Plass break); the moment it carries an emphasis run it becomes a rich paragraph
/// of segments, which the driver breaks with a face per run. A `label` is the paragraph's trailing
/// `<name>`, carried onto a display equation so an `@`-reference can resolve to it.
fn lower_paragraph(runs: &[Inline], label: Option<String>) -> Block {
	match runs {
		[Inline::Text(text)]	=> Block::paragraph(text.clone()),
		// A paragraph that is nothing but one maths span is a display equation on its own line. The
		// template sets `math.equation(numbering: "(1)")`, so every display equation takes the next
		// number; inline maths, a run among others, never does.
		[Inline::Math(atom)]	=> Block::equation(atom.clone(), true, label),
		_						=> Block::rich(runs.iter().map(lower_inline).collect()),
	}
}

fn lower_inline(run: &Inline) -> Segment {
	match run {
		Inline::Text(text)		=> Segment::text(text.clone()),
		Inline::Strong(text)	=> Segment::strong(text.clone()),
		Inline::Emph(text)		=> Segment::emph(text.clone()),
		Inline::BoldItalic(text)	=> Segment::bold_italic(text.clone()),
		Inline::Super(text)		=> Segment::superscript(text.clone()),
		Inline::PageRef(label)	=> Segment::page_ref(label.clone()),
		Inline::Code(text)		=> Segment::code(text.clone()),
		Inline::Math(atom)		=> Segment::math(atom.clone()),
		Inline::Glossary { term, display }
								=> Segment::glossary(term.clone(), display.clone()),
		Inline::Footnote(note)	=> Segment::footnote(note.iter().map(lower_inline).collect()),
		Inline::Cite(keys)		=> Segment::cite(keys.clone()),
	}
}
