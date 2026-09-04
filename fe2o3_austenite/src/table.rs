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

use crate::doc::Style;
use crate::font::ShapedText;
use crate::ir::{
	BoxNode,
	Dims,
	Glue,
	Leaf,
	Node,
	Sp,
};
use crate::linebreak::break_paragraph;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::Dir,
};

use std::sync::Arc;

/// How a cell's text sits within its column width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
	Left,
	Centre,
	Right,
}

/// One cell: its text, and how that text aligns in the column. A cell holds text in this increment;
/// inline runs (a bold word inside a cell) are a later variant here.
#[derive(Clone, Debug)]
pub struct Cell {
	pub text:	String,
	pub align:	Align,
}

impl Cell {
	pub fn new<S: Into<String>>(text: S) -> Self {
		Self { text: text.into(), align: Align::Left }
	}

	pub fn aligned<S: Into<String>>(text: S, align: Align) -> Self {
		Self { text: text.into(), align }
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

	let colwidth = res!(size_columns(fonts.clone(), size, table, ncols, available));

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
	for (r, row) in rows.iter().enumerate() {
		let role = row_role(table, r);
		let mut cells = Vec::with_capacity(ncols);
		let mut bands = 0usize;
		for c in 0..ncols {
			let lines = match row.cells.get(c) {
				Some(cell)	=> res!(wrap_cell(fonts.clone(), role, size, &cell.text, colwidth[c], style.leading)),
				None		=> Vec::new(),
			};
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
		// The top padding of the row: a rules-only band, so the first line's baseline clears the rule.
		let empty:	Vec<Option<&CellLine>>	= vec![None; ncols];
		let flush:	Vec<Align>				= vec![Align::Left; ncols];
		let pad_band = build_band(
			ncols, &vrule_left, &tv, &content_left, &colwidth,
			style.cell_pad_y, table_width, &empty, &flush);
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
				bh, table_width, &opt, &aligns);
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

/// The role a row's cells are shaped in: bold for a header row, the body face otherwise.
fn row_role(table: &Table, r: usize) -> Role {
	if table.header && r == 0 { Role::Bold } else { Role::Body }
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
	table:		&Table,
	ncols:		usize,
	available:	i32,
)
	-> Outcome<Vec<Sp>>
{
	let mut natural	= vec![0i64; ncols];
	let mut minw	= vec![0i64; ncols];
	for (r, row) in table.rows.iter().enumerate() {
		let role = row_role(table, r);
		for c in 0..ncols {
			if let Some(cell) = row.cells.get(c) {
				let nat = res!(text_width(fonts.clone(), role, size, &cell.text));
				natural[c] = natural[c].max(nat.raw() as i64);
				let lw = res!(longest_word(fonts.clone(), role, size, &cell.text));
				minw[c] = minw[c].max(lw.raw() as i64);
			}
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

/// The natural width of a cell's whole text set on one line, or zero for an empty cell.
fn text_width(fonts: Arc<FontSet>, role: Role, size: Sp, text: &str) -> Outcome<Sp> {
	if text.trim().is_empty() {
		return Ok(Sp::ZERO);
	}
	let shaped = res!(ShapedText::new(fonts, role, Dir::Ltr, size, text));
	Ok(shaped.dims().width)
}

/// The width of a cell's widest single word: the least a column can be squeezed to before a word
/// must protrude.
fn longest_word(fonts: Arc<FontSet>, role: Role, size: Sp, text: &str) -> Outcome<Sp> {
	let mut m = Sp::ZERO;
	for w in text.split_whitespace() {
		let shaped = res!(ShapedText::new(fonts.clone(), role, Dir::Ltr, size, w));
		if shaped.dims().width > m {
			m = shaped.dims().width;
		}
	}
	Ok(m)
}

/// Wraps a cell's text to its column, reusing the paragraph line breaker at the column width. Each
/// returned HBox is one line; its shaped leaves and glue are kept, with the natural width summed for
/// alignment. The interline glue the breaker inserts is dropped -- a band supplies its own vertical
/// spacing.
fn wrap_cell(
	fonts:		Arc<FontSet>,
	role:		Role,
	size:		Sp,
	text:		&str,
	colwidth:	Sp,
	leading:	Sp,
)
	-> Outcome<Vec<CellLine>>
{
	if text.trim().is_empty() {
		return Ok(Vec::new());
	}
	let nodes = res!(break_paragraph(fonts, role, Dir::Ltr, size, text, colwidth, leading));
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
)
	-> Node
{
	let mut kids:	Vec<Node>	= Vec::new();
	let mut cursor				= Sp::ZERO;
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

/// Pushes a full-width horizontal rule and advances the running height by its thickness.
fn push_hrule(children: &mut Vec<Node>, total_h: &mut Sp, width: Sp, thick: Sp) {
	children.push(Node::Leaf(Leaf::rule(Dims::new(width, thick, Sp::ZERO))));
	*total_h += thick;
}
