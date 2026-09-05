//! Tables: a ruled grid of cells, laid out as one keep box in the vertical flow.
//!
//! A table is authored as [`Table`] -- rows of [`Cell`]s -- and [`lower`] turns it into a single
//! [`Node::VBox`], so the driver's greedy page breaker moves the whole table to the next page when it
//! will not fit where it stands. Column widths are measured from the cell text and, when the natural
//! widths overrun the measure, shrunk proportionally; a cell too wide for its column is wrapped with
//! [`break_paragraph`](crate::linebreak::break_paragraph) at the column width, exactly as a paragraph
//! is wrapped at the measure.
//!
//! One fact a reader could not derive, and the reason the layout looks the way it does. The driver
//! renders a box *nested inside an HBox* as a placeholder rectangle -- only leaves (glyph runs and
//! rules) draw as ink there -- so a cell's glyphs cannot be a nested box. A row is therefore
//! decomposed into horizontal *bands*: each band is one HBox holding, positioned by glue, the
//! vertical rules at the column boundaries and one wrapped line from each cell. Bands stack with no
//! gap, so the per-band rule segments tile into continuous column rules, and a cell that wraps to
//! several lines simply contributes to several bands. The whole grid is thus leaves in HBoxes stacked
//! in one VBox, which is the only shape the driver draws as real glyphs throughout.

use crate::doc::{
	Segment,
	Style,
	superscript,
};
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	Dims,
	DrawOp,
	Glue,
	Graphic,
	Leaf,
	Node,
	Sp,
};
use crate::linebreak::{
	Piece,
	break_paragraph_pieces,
};
use crate::math;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::Dir,
};
use oxedyne_fe2o3_graphics::{
	colour::Rgba,
	path::{
		Bounds,
		Path,
	},
};

use std::sync::Arc;

/// How a cell's text sits within its column width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
	Left,
	Centre,
	Right,
}

/// One cell: its inline content, and how that content aligns in the column. A cell carries a run of
/// [`Segment`]s -- a bold header, an italic word, a superscript dagger or an in-cell maths span each set
/// with its own face -- broken to the column width exactly as a rich paragraph is broken to the measure.
#[derive(Clone, Debug)]
pub struct Cell {
	pub content:	Vec<Segment>,
	pub align:		Align,
}

impl Cell {
	pub fn new<S: Into<String>>(text: S) -> Self {
		Self { content: vec![Segment::text(text)], align: Align::Left }
	}

	pub fn aligned<S: Into<String>>(text: S, align: Align) -> Self {
		Self { content: vec![Segment::text(text)], align }
	}

	/// A cell carrying a run of rich segments -- the form the reader builds from a Typst cell's markup.
	pub fn rich(content: Vec<Segment>, align: Align) -> Self {
		Self { content, align }
	}
}

/// One row of cells. A row shorter than the widest row is padded with empty cells when the table is
/// laid out, so a ragged authoring is legal.
#[derive(Clone, Debug)]
pub struct Row {
	pub cells:	Vec<Cell>,
}

impl Row {
	pub fn new(cells: Vec<Cell>) -> Self {
		Self { cells }
	}
}

/// A table: its rows, and whether the first row is a header (set bold, with a heavier rule beneath
/// it). Spanning cells and a caption with a "Table N" number are later additions.
#[derive(Clone, Debug)]
pub struct Table {
	pub rows:	Vec<Row>,
	pub header:	bool,
}

impl Table {
	pub fn new(header: bool, rows: Vec<Row>) -> Self {
		Self { rows, header }
	}

	/// A grid of string rows, every cell left-aligned; the first row a header when `header`.
	pub fn grid(header: bool, rows: Vec<Vec<&str>>) -> Self {
		let rows = rows.into_iter()
			.map(|r| Row::new(r.into_iter().map(Cell::new).collect()))
			.collect();
		Self { rows, header }
	}
}

/// One wrapped line of a cell: the shaped leaves and justifying glue as `break_paragraph` set them,
/// with the natural extent kept so a band can align and stack them.
struct CellLine {
	children:	Vec<Node>,
	width:		Sp,
	height:		Sp,
	depth:		Sp,
}

/// Lowers a table to one keep box. The measure is the width a table spanning the full text block may
/// use; a table whose natural columns are narrower than the measure is set narrower, flush left.
pub fn lower(
	fonts:		Arc<FontSet>,
	style:		Style,
	measure:	Sp,
	table:		&Table,
)
	-> Outcome<Node>
{
	let size	= style.body_size;
	let rows	= &table.rows;
	if rows.is_empty() {
		return Err(err!("A table needs at least one row."; Input, Invalid, Missing));
	}
	let ncols = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
	if ncols == 0 {
		return Err(err!("A table needs at least one column."; Input, Invalid, Missing));
	}

	// The vertical rules: a heavier pen frames the grid, a lighter one divides the columns. Their
	// widths take real horizontal space, so they are budgeted before the columns are sized.
	let mut tv = vec![style.rule_thin; ncols + 1];
	tv[0]		= style.rule_thick;
	tv[ncols]	= style.rule_thick;
	let vrule_total: i32 = tv.iter().map(|s| s.raw()).sum();

	// The text width left for the columns after the padding either side of every cell and the rules
	// between them are taken out of the measure.
	let pad2		= style.cell_pad_x.raw() * 2 * ncols as i32;
	let available	= measure.raw() - pad2 - vrule_total;
	if available <= 0 {
		return Err(err!(
			"A table of {} columns leaves no width for text within the measure of {} sp; \
			reduce the columns or the padding.", ncols, measure.raw(); Input, Invalid, TooBig));
	}

	// Each cell's inline content is built once into the pieces the line breaker weaves -- a run of shaped
	// text in its face, a maths cluster, a superscript mark -- and reused for measuring the columns and for
	// the final wrap, so a cell shapes its faces only once. The base role per row is bold in a header row,
	// so a plain header label still sets bold, and body elsewhere.
	let (piece_grid, bases) = res!(build_grid(fonts.clone(), style, table, ncols));
	let colwidth = res!(size_columns(fonts.clone(), size, &piece_grid, &bases, ncols, available));

	// Column boundary positions, the running x of each vertical rule and each cell's text left.
	let mut vrule_left	= vec![Sp::ZERO; ncols + 1];
	let mut cx			= Sp::ZERO;
	for b in 0..=ncols {
		vrule_left[b]	= cx;
		cx				= cx + tv[b];
		if b < ncols {
			cx = cx + Sp(style.cell_pad_x.raw() * 2) + colwidth[b];
		}
	}
	let table_width = cx;
	let mut content_left = vec![Sp::ZERO; ncols];
	for c in 0..ncols {
		content_left[c] = vrule_left[c] + tv[c] + style.cell_pad_x;
	}

	// Wrap every cell to its column, and note the tallest stack of lines in each row.
	let mut grid:	Vec<Vec<Vec<CellLine>>>	= Vec::with_capacity(rows.len());
	let mut nbands:	Vec<usize>				= Vec::with_capacity(rows.len());
	for r in 0..rows.len() {
		let mut cells = Vec::with_capacity(ncols);
		let mut bands = 0usize;
		for c in 0..ncols {
			let lines = res!(break_cell(
				fonts.clone(), bases[r], size, &piece_grid[r][c], colwidth[c], style.leading));
			bands = bands.max(lines.len());
			cells.push(lines);
		}
		grid.push(cells);
		nbands.push(bands.max(1));	// an all-empty row still occupies one line band
	}

	// A fallback line extent for a band no cell fills, so an empty row keeps a sensible height.
	let sample	= res!(ShapedText::new(fonts.clone(), Role::Body, Dir::Ltr, size, "Ag"));
	let default_v	= sample.dims().height + sample.dims().depth;

	let mut children:	Vec<Node> = Vec::new();
	let mut total_h		= Sp::ZERO;

	// The top frame.
	push_hrule(&mut children, &mut total_h, table_width, style.rule_thick);

	for r in 0..rows.len() {
		// A header row carries a grey wash behind every one of its bands, drawn before the rules and text
		// so they sit over it; a body row has none.
		let fill = if table.header && r == 0 { Some(style.header_fill) } else { None };

		// The top padding of the row: a rules-only band, so the first line's baseline clears the rule.
		let empty:	Vec<Option<&CellLine>>	= vec![None; ncols];
		let flush:	Vec<Align>				= vec![Align::Left; ncols];
		let pad_band = build_band(
			ncols, &vrule_left, &tv, &content_left, &colwidth,
			style.cell_pad_y, table_width, &empty, &flush, fill);
		children.push(pad_band);
		total_h += style.cell_pad_y;

		let bands = nbands[r];
		for k in 0..bands {
			// The band height is the tallest line at this level, plus a trailing gap -- interline
			// leading between lines, the bottom cell padding after the last.
			let mut lh = Sp::ZERO;
			for c in 0..ncols {
				if let Some(cl) = grid[r][c].get(k) {
					let v = cl.height + cl.depth;
					if v > lh { lh = v; }
				}
			}
			if lh.raw() == 0 { lh = default_v; }
			let trailing	= if k + 1 < bands { style.line_gap } else { style.cell_pad_y };
			let bh			= lh + trailing;

			let mut opt:	Vec<Option<&CellLine>>	= Vec::with_capacity(ncols);
			let mut aligns:	Vec<Align>				= Vec::with_capacity(ncols);
			for c in 0..ncols {
				opt.push(grid[r][c].get(k));
				aligns.push(rows[r].cells.get(c).map_or(Align::Left, |cell| cell.align));
			}

			let hb = build_band(
				ncols, &vrule_left, &tv, &content_left, &colwidth,
				bh, table_width, &opt, &aligns, fill);
			children.push(hb);
			total_h += bh;
		}

		// The rule under the row: heavy beneath a header and at the very foot, light between body rows.
		let th = if table.header && r == 0 {
			style.rule_thick
		} else if r + 1 == rows.len() {
			style.rule_thick
		} else {
			style.rule_thin
		};
		push_hrule(&mut children, &mut total_h, table_width, th);
	}

	let dims = Dims::new(table_width, total_h, Sp::ZERO);
	Ok(Node::VBox(BoxNode::new(children, dims)))
}

/// The base role a row's plain text sets in: bold for a header row, the body face otherwise. Authored
/// emphasis within a cell keeps its own face over this base.
fn base_role(table: &Table, r: usize) -> Role {
	if table.header && r == 0 { Role::Bold } else { Role::Body }
}

/// Builds every cell's pieces once, and the base role of each row. A missing cell (a ragged row shorter
/// than the widest) gives an empty piece list, so the column simply carries nothing there.
fn build_grid(
	fonts:	Arc<FontSet>,
	style:	Style,
	table:	&Table,
	ncols:	usize,
)
	-> Outcome<(Vec<Vec<Vec<Piece>>>, Vec<Role>)>
{
	let mut grid:	Vec<Vec<Vec<Piece>>>	= Vec::with_capacity(table.rows.len());
	let mut bases:	Vec<Role>				= Vec::with_capacity(table.rows.len());
	for (r, row) in table.rows.iter().enumerate() {
		let base = base_role(table, r);
		bases.push(base);
		let mut cols = Vec::with_capacity(ncols);
		for c in 0..ncols {
			let pieces = match row.cells.get(c) {
				Some(cell)	=> res!(cell_pieces(fonts.clone(), style, &cell.content, base)),
				None		=> Vec::new(),
			};
			cols.push(pieces);
		}
		grid.push(cols);
	}
	Ok((grid, bases))
}

/// Turns a cell's rich segments into the pieces the line breaker weaves. Plain text takes the row's base
/// role -- a header cell's bold, a body cell's body; `*strong*`, `_emph_`, a `#super[...]`, inline code
/// and an in-cell maths span keep their own faces, so a cell sets exactly as a run of prose would. A
/// footnote or a cross-reference in a cell -- rare -- is not set here; a citation falls back to its keys.
fn cell_pieces(
	fonts:		Arc<FontSet>,
	style:		Style,
	segments:	&[Segment],
	base:		Role,
)
	-> Outcome<Vec<Piece>>
{
	let size = style.body_size;
	let mut pieces = Vec::with_capacity(segments.len());
	for seg in segments {
		match seg {
			Segment::Text(t)		=> pieces.push(Piece::Text { text: t.clone(), role: base }),
			Segment::Strong(t)		=> pieces.push(Piece::Text { text: t.clone(), role: Role::Bold }),
			Segment::Emph(t)		=> pieces.push(Piece::Text { text: t.clone(), role: emph_role(base) }),
			Segment::Code(t)		=> pieces.push(Piece::Text { text: t.clone(), role: Role::Mono }),
			Segment::Glossary { display, .. }
									=> pieces.push(Piece::Text { text: display.clone(), role: base }),
			Segment::Cite(keys)		=> pieces.push(Piece::Text { text: fmt!("({})", keys.join("; ")), role: base }),
			Segment::PageRef(_)		=> {},	// a cross-reference in a cell is not resolved to a page here
			Segment::Footnote { .. }	=> {},	// a footnote in a cell is not set at this increment
			Segment::Super(t) => {
				let (shaped, dims) = res!(superscript(fonts.clone(), base, size, t));
				pieces.push(Piece::Mark(Leaf::text_dims(shaped, dims)));
			},
			Segment::Math(expr) => {
				// The inline box is flattened to leaves and glue by the maths layout; its children weave into
				// the line as real glyphs, its baseline seated on the text baseline.
				let node = res!(math::layout(fonts.clone(), &style, expr, false));
				if let Node::HBox(b) = node {
					let ascent	= res!(ShapedText::new(
						fonts.clone(), base, Dir::Ltr, size, "0")).dims().height;
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
		}
	}
	Ok(pieces)
}

/// The face an emphasised run takes over a base: bold-italic within a header (whose base is bold), plain
/// italic elsewhere.
fn emph_role(base: Role) -> Role {
	if base == Role::Bold { Role::BoldItalic } else { Role::Italic }
}

/// Assigns each column a text width. Each column asks for its widest cell's natural width; when the
/// columns together fit the available width they keep it (the table sets narrower than the measure),
/// and when they overrun it they are shrunk. The shrink holds every column at no less than its widest
/// single word and shares the remaining width in proportion to how much each wanted above that
/// minimum; if even the minimums overrun, the columns shrink in proportion to their natural widths
/// and an over-long word is left to run under a rule. It does not know which column would most repay
/// extra width, and it cannot span a cell across columns -- both later refinements.
fn size_columns(
	fonts:		Arc<FontSet>,
	size:		Sp,
	grid:		&[Vec<Vec<Piece>>],
	bases:		&[Role],
	ncols:		usize,
	available:	i32,
)
	-> Outcome<Vec<Sp>>
{
	let mut natural	= vec![0i64; ncols];
	let mut minw	= vec![0i64; ncols];
	for (r, row) in grid.iter().enumerate() {
		let base = bases[r];
		for c in 0..ncols {
			let pieces = &row[c];
			if pieces.is_empty() {
				continue;
			}
			// The natural width is the cell set on one line; the minimum is the widest line once the cell is
			// broken as hard as it can be, the least the column can shrink to before a word must protrude.
			let nat = res!(measure_cell(fonts.clone(), base, size, pieces, Sp::from_pt(100_000.0)));
			natural[c] = natural[c].max(nat.raw() as i64);
			let lw = res!(measure_cell(fonts.clone(), base, size, pieces, Sp(1)));
			minw[c] = minw[c].max(lw.raw() as i64);
		}
	}

	let total_natural:	i64 = natural.iter().sum();
	let avail			= available as i64;
	let mut colwidth	= vec![Sp::ZERO; ncols];

	if total_natural == 0 || total_natural <= avail {
		for c in 0..ncols {
			colwidth[c] = Sp(natural[c] as i32);
		}
		return Ok(colwidth);
	}

	let sum_min: i64 = minw.iter().sum();
	let mut acc = 0i64;
	if sum_min < avail {
		let extra	= avail - sum_min;
		let span	= total_natural - sum_min;
		for c in 0..ncols {
			let give = if span > 0 {
				((natural[c] - minw[c]) as i128 * extra as i128 / span as i128) as i64
			} else {
				0
			};
			let w = minw[c] + give;
			colwidth[c] = Sp(w as i32);
			acc += w;
		}
	} else {
		// Even the minimums overrun the measure: proportional to natural width, words may protrude.
		for c in 0..ncols {
			let w = natural[c] * avail / total_natural;
			colwidth[c] = Sp(w as i32);
			acc += w;
		}
	}
	// The rounding remainder lands on the last column, so the widths sum to the budget exactly.
	let last = ncols - 1;
	colwidth[last] = Sp(colwidth[last].raw() + (avail - acc) as i32);
	Ok(colwidth)
}

/// The widest line a cell's pieces set to when broken at `measure`: at a large measure this is the cell's
/// natural one-line width, at a tiny one the least it can shrink to. Zero for an empty cell.
fn measure_cell(
	fonts:		Arc<FontSet>,
	base:		Role,
	size:		Sp,
	pieces:		&[Piece],
	measure:	Sp,
)
	-> Outcome<Sp>
{
	let mut m = Sp::ZERO;
	for line in res!(break_cell(fonts, base, size, pieces, measure, size)) {
		if line.width > m {
			m = line.width;
		}
	}
	Ok(m)
}

/// Breaks a cell's pieces to its column, reusing the rich-paragraph line breaker at the column width, so a
/// cell's faces, superscripts and maths flow exactly as a paragraph's do. Each returned HBox is one line;
/// its leaves and glue are kept, with the natural width summed for alignment. The interline glue the
/// breaker inserts is dropped -- a band supplies its own vertical spacing. An empty cell yields no lines.
fn break_cell(
	fonts:		Arc<FontSet>,
	base:		Role,
	size:		Sp,
	pieces:		&[Piece],
	colwidth:	Sp,
	leading:	Sp,
)
	-> Outcome<Vec<CellLine>>
{
	if pieces.is_empty() {
		return Ok(Vec::new());
	}
	let nodes = res!(break_paragraph_pieces(fonts, base, Dir::Ltr, size, pieces, colwidth, leading));
	let mut out = Vec::new();
	for n in nodes {
		if let Node::HBox(b) = n {
			let mut w = Sp::ZERO;
			for ch in &b.list {
				match ch {
					Node::Leaf(l)	=> w = w + l.dims.width,
					Node::Glue(g)	=> w = w + g.natural,
					_				=> (),
				}
			}
			out.push(CellLine { children: b.list, width: w, height: b.dims.height, depth: b.dims.depth });
		}
	}
	Ok(out)
}

/// Builds one band: an HBox carrying the vertical rules at every column boundary, each the band's own
/// height, and one line from each cell placed at its column and alignment. The x cursor is tracked as
/// the driver will track it, so every rule lands on its fixed boundary whatever the lines do -- an
/// over-long line runs under the next rule rather than displacing it, keeping the columns straight
/// from row to row.
#[allow(clippy::too_many_arguments)]
fn build_band(
	ncols:			usize,
	vrule_left:		&[Sp],
	tv:				&[Sp],
	content_left:	&[Sp],
	colwidth:		&[Sp],
	band_height:	Sp,
	table_width:	Sp,
	lines:			&[Option<&CellLine>],
	aligns:			&[Align],
	fill:			Option<Rgba>,
)
	-> Node
{
	let mut kids:	Vec<Node>	= Vec::new();
	let mut cursor				= Sp::ZERO;

	// The row wash sits behind the band: a zero-width graphic leaf drawn first, so it does not advance the
	// horizontal cursor the rules and lines are positioned against, yet its path spans the whole band and
	// paints under them (the writer draws in frame order, so the rules and glyphs that follow sit over it).
	if let Some(colour) = fill {
		if let Some(g) = fill_band(table_width, band_height, colour) {
			kids.push(Node::Leaf(g));
		}
	}

	for b in 0..=ncols {
		// The vertical rule at this boundary, seated on its fixed x.
		kids.push(Node::Glue(Glue::fixed(vrule_left[b] - cursor)));
		kids.push(Node::Leaf(Leaf::rule(Dims::new(tv[b], band_height, Sp::ZERO))));
		cursor = vrule_left[b] + tv[b];

		if b < ncols {
			if let Some(line) = lines[b] {
				let slack	= (colwidth[b].raw() - line.width.raw()).max(0);
				let off		= match aligns[b] {
					Align::Left		=> 0,
					Align::Centre	=> slack / 2,
					Align::Right	=> slack,
				};
				let target = content_left[b] + Sp(off);
				kids.push(Node::Glue(Glue::fixed(target - cursor)));
				for ch in &line.children {
					kids.push(ch.clone());
				}
				cursor = target + line.width;
			}
			// An empty cell adds nothing; the next boundary's glue jumps the column's width.
		}
	}
	Node::HBox(BoxNode::new(kids, Dims::new(table_width, band_height, Sp::ZERO)))
}

/// Builds the background wash of one band: a graphic leaf whose single fill covers the band rectangle in
/// `colour`. The leaf declares zero width so it does not move the band's horizontal cursor -- the path is
/// drawn from the leaf's placement whatever the declared advance -- and its height matches the band, so
/// the wash tiles seamlessly with the band above and below. `None` for a degenerate (zero-area) band, so
/// `Path::rect` is never handed an empty rectangle.
fn fill_band(width: Sp, height: Sp, colour: Rgba) -> Option<Leaf> {
	let w = width.to_pt() as f32;
	let h = height.to_pt() as f32;
	if w <= 0.0 || h <= 0.0 {
		return None;
	}
	let rect	= Path::rect(Bounds::new(0.0, 0.0, w, h)).ok()?;
	let graphic	= Graphic::new(vec![DrawOp::Fill { path: rect, colour }], Dims::new(Sp::ZERO, height, Sp::ZERO));
	Some(Leaf::graphic(graphic))
}

/// Pushes a full-width horizontal rule and advances the running height by its thickness.
fn push_hrule(children: &mut Vec<Node>, total_h: &mut Sp, width: Sp, thick: Sp) {
	children.push(Node::Leaf(Leaf::rule(Dims::new(width, thick, Sp::ZERO))));
	*total_h += thick;
}
