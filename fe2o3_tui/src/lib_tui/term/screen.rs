//! The grid, the cursor and everything that moves them.
//!
//! [`Screen`] owns a rectangle of [`Cell`]s, a cursor, a scrolling region, a set of tab stops and a
//! bounded scrollback. It knows nothing about escape sequences: the emulator in [`super::emu`]
//! translates the parser's output into calls on this type, which keeps the grammar of the stream
//! and the geometry of the screen apart.
//!
//! Two conventions are worth stating because they are easy to get wrong.
//!
//! The cursor column is always within the grid, even when the last character printed filled the
//! final column. That case is recorded by [`Cursor::wrap_pending`], which is the deferred wrap
//! every real terminal implements: printing in the last column leaves the cursor there, and only
//! the *next* printable character moves to the following row. Without it, a line exactly as wide
//! as the screen scrolls one row too early.
//!
//! Erasure paints with the current background colour, not with the default one. An application
//! that selects a background and then erases expects to see that background, and does not repaint
//! it itself.
//!
//! Every row records whether the text on it ran into the row below, which is what [`Screen::resize`]
//! needs in order to put a narrowed window's text back together rather than cutting it off. The
//! record is written in one place only, where the wrap actually happens, and an erase that reaches
//! the end of a row takes it away again.

use crate::lib_tui::term::{
	cell::{
		Cell,
		Pen,
		Wide,
	},
	width::{
		char_width,
		CharWidth,
	},
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::unicode::norm;

use std::collections::VecDeque;


/// The default number of lines of scrollback.
pub const DEFAULT_SCROLLBACK: usize = 2000;

/// The interval between the tab stops a reset installs.
pub const TAB_INTERVAL: usize = 8;

/// Which part of a line or screen an erase covers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Erase {
	/// From the cursor to the end.
	ToEnd,
	/// From the beginning to the cursor, inclusive.
	ToStart,
	/// The whole of it.
	All,
}

impl Erase {
	/// The erase named by a sequence parameter, or `None` for a parameter with no meaning.
	pub fn from_param(p: u32) -> Option<Self> {
		match p {
			0	=> Some(Self::ToEnd),
			1	=> Some(Self::ToStart),
			2	=> Some(Self::All),
			_	=> None,
		}
	}
}

/// One line of the grid or of the scrollback, and whether it ran into the one below.
///
/// The `wrapped` flag is what makes rewrapping on resize possible at all. Without a record of which
/// rows are continuations of the row above, a terminal narrowing its window can only truncate,
/// because it cannot tell a paragraph that ran on from two separate lines that happen to be
/// adjacent. The flag is set where the wrap actually happens, in [`Screen::wrap`], and nowhere else.
#[derive(Clone, Debug, Default)]
pub struct Line {
	/// The cells, which for a wrapped line are exactly as wide as the screen then was.
	pub cells:	Vec<Cell>,
	/// Whether the text carried on into the line below rather than ending here.
	pub wrapped:	bool,
}

impl Line {
	/// A blank line of `cols` cells.
	pub fn blank(cols: usize) -> Self {
		Self {
			cells:		vec![Cell::default(); cols],
			wrapped:	false,
		}
	}

	/// The text of the line, with trailing blanks removed.
	pub fn text(&self) -> String {
		Screen::cells_text(&self.cells)
	}
}

/// Which of the two grids is in front.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
	/// The ordinary screen, which accumulates scrollback.
	Primary,
	/// The alternate screen, which does not.
	Alternate,
}

/// Where the cursor is and how it behaves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
	/// Row, counted from the top of the grid.
	pub row: usize,
	/// Column, always within the grid.
	pub col: usize,
	/// Whether the application has asked for the cursor to be drawn.
	pub visible: bool,
	/// Whether the last character printed filled the final column, so that the next one wraps.
	pub wrap_pending: bool,
}

impl Default for Cursor {
	fn default() -> Self {
		Self {
			row:		0,
			col:		0,
			visible:	true,
			wrap_pending:	false,
		}
	}
}

impl Cursor {
	/// The column a terminal would report, which is one past the grid when a wrap is pending.
	///
	/// This is what `DSR` answers and what `tmux` shows in `#{cursor_x}`, and it is the value to
	/// compare against another terminal.
	pub fn reported_col(&self) -> usize {
		self.col + if self.wrap_pending { 1 } else { 0 }
	}
}

/// The cursor state a save and restore moves.
#[derive(Clone, Copy, Debug)]
struct SavedCursor {
	cursor:	Cursor,
	pen:	Pen,
	origin:	bool,
}

/// What has changed since a renderer last looked.
///
/// Redrawing a whole grid for every byte that arrives is what makes a terminal in a browser feel
/// slow, so the model records the rows it touched and leaves the rest alone. A renderer clears the
/// record with [`Screen::clear_damage`] once it has drawn.
#[derive(Clone, Debug, Default)]
pub struct Damage {
	/// Whether each row needs redrawing.
	rows:	Vec<bool>,
	/// Whether the whole surface must be redrawn, as after a resize or a buffer switch.
	all:	bool,
	/// Whether the cursor moved or changed visibility.
	cursor:	bool,
	/// How many lines have left the top of the screen into scrollback.
	scrolled: usize,
}

impl Damage {
	/// A record covering `rows` rows, with everything marked as needing a first draw.
	pub fn new(rows: usize) -> Self {
		Self {
			rows:		vec![true; rows],
			all:		true,
			cursor:		true,
			scrolled:	0,
		}
	}

	/// Whether anything at all changed.
	pub fn any(&self) -> bool {
		self.all || self.cursor || self.rows.iter().any(|d| *d)
	}

	/// Whether the whole surface must be redrawn.
	pub fn is_all(&self) -> bool {
		self.all
	}

	/// Whether the given row must be redrawn.
	pub fn is_row(&self, row: usize) -> bool {
		self.all || self.rows.get(row).copied().unwrap_or(false)
	}

	/// The rows that must be redrawn, in order.
	pub fn dirty_rows(&self) -> Vec<usize> {
		let mut out = Vec::new();
		for (i, d) in self.rows.iter().enumerate() {
			if self.all || *d {
				out.push(i);
			}
		}
		out
	}

	/// Whether the cursor moved or changed visibility.
	pub fn cursor_moved(&self) -> bool {
		self.cursor
	}

	/// How many lines left the top of the screen into scrollback, which a renderer holding its own
	/// copy of the scrollback uses to know how many to collect.
	pub fn scrolled(&self) -> usize {
		self.scrolled
	}
}

/// A terminal screen: a grid of cells, a cursor and a bounded scrollback.
#[derive(Clone, Debug)]
pub struct Screen {
	/// Width in cells.
	cols:	usize,
	/// Height in cells.
	rows:	usize,
	/// The visible grid, `rows` by `cols`, in row major order.
	grid:	Vec<Cell>,
	/// Whether each row of the visible grid ran into the row below.
	wrapped: Vec<bool>,
	/// The grid that is not in front, held while the other is shown.
	spare:	Vec<Cell>,
	/// Whether each row of the grid that is not in front ran into the row below.
	spare_wrapped: Vec<bool>,
	/// Which grid is in front.
	surface: Surface,
	/// The cursor.
	cursor:	Cursor,
	/// The pen printing uses.
	pen:	Pen,
	/// The cursor saved by `DECSC` or by the switch to the alternate screen.
	saved:	Option<SavedCursor>,
	/// The cursor of the grid that is not in front.
	spare_cursor: Cursor,
	/// The first and last rows of the scrolling region, inclusive.
	region:	(usize, usize),
	/// Whether a tab stop is set at each column.
	tabs:	Vec<bool>,
	/// Whether printing past the final column wraps.
	autowrap: bool,
	/// Whether printing pushes the rest of the line to the right.
	insert:	bool,
	/// Whether row addressing is relative to the scrolling region.
	origin:	bool,
	/// Lines that have left the top of the screen.
	scrollback: VecDeque<Line>,
	/// The most lines of scrollback that will be kept.
	scrollback_max: usize,
	/// How many lines of scrollback the viewport is shown above the live screen, where zero is the
	/// live screen itself.
	view:	usize,
	/// What has changed since a renderer last looked.
	damage:	Damage,
}

impl Screen {

	// ┌─────────────────────────────┐
	// │ CONSTRUCTION                │
	// └─────────────────────────────┘

	/// A blank screen of the given size with the default scrollback bound.
	///
	/// # Errors
	/// Fails if either dimension is zero.
	pub fn new(cols: usize, rows: usize) -> Outcome<Self> {
		Self::with_scrollback(cols, rows, DEFAULT_SCROLLBACK)
	}

	/// A blank screen of the given size, keeping at most `scrollback_max` lines of scrollback.
	///
	/// # Errors
	/// Fails if either dimension is zero.
	pub fn with_scrollback(cols: usize, rows: usize, scrollback_max: usize) -> Outcome<Self> {
		if cols == 0 || rows == 0 {
			return Err(err!(
				"A screen must have at least one row and one column, was given {} by {}.",
				cols, rows;
				Invalid, Input));
		}
		Ok(Self {
			cols,
			rows,
			grid:		vec![Cell::default(); cols * rows],
			wrapped:	vec![false; rows],
			spare:		Vec::new(),
			spare_wrapped:	Vec::new(),
			surface:	Surface::Primary,
			cursor:		Cursor::default(),
			pen:		Pen::default(),
			saved:		None,
			spare_cursor:	Cursor::default(),
			region:		(0, rows - 1),
			tabs:		Self::default_tabs(cols),
			autowrap:	true,
			insert:		false,
			origin:		false,
			scrollback:	VecDeque::new(),
			scrollback_max,
			view:		0,
			damage:		Damage::new(rows),
		})
	}

	/// The tab stops a reset installs: every [`TAB_INTERVAL`] columns.
	fn default_tabs(cols: usize) -> Vec<bool> {
		let mut tabs = vec![false; cols];
		let mut i = TAB_INTERVAL;
		while i < cols {
			tabs[i] = true;
			i += TAB_INTERVAL;
		}
		tabs
	}

	// ┌─────────────────────────────┐
	// │ INSPECTION                  │
	// └─────────────────────────────┘

	/// Width in cells.
	pub fn cols(&self) -> usize {
		self.cols
	}

	/// Height in cells.
	pub fn rows(&self) -> usize {
		self.rows
	}

	/// The cursor.
	pub fn cursor(&self) -> &Cursor {
		&self.cursor
	}

	/// The pen printing currently uses.
	pub fn pen(&self) -> &Pen {
		&self.pen
	}

	/// Which grid is in front.
	pub fn surface(&self) -> Surface {
		self.surface
	}

	/// The first and last rows of the scrolling region, inclusive.
	pub fn region(&self) -> (usize, usize) {
		self.region
	}

	/// Whether printing past the final column wraps.
	pub fn autowrap(&self) -> bool {
		self.autowrap
	}

	/// The cells of a visible row, or `None` if the row is off the grid.
	pub fn row(&self, row: usize) -> Option<&[Cell]> {
		if row >= self.rows {
			return None;
		}
		let a = row * self.cols;
		Some(&self.grid[a..a + self.cols])
	}

	/// Whether a visible row ran into the row below rather than ending there.
	pub fn row_wrapped(&self, row: usize) -> bool {
		self.wrapped.get(row).copied().unwrap_or(false)
	}

	/// One cell, or `None` if it is off the grid.
	pub fn cell(&self, col: usize, row: usize) -> Option<&Cell> {
		if col >= self.cols || row >= self.rows {
			return None;
		}
		self.grid.get(row * self.cols + col)
	}

	/// The text of a visible row, with trailing blanks removed and the right hand halves of wide
	/// characters omitted.
	pub fn row_text(&self, row: usize) -> String {
		match self.row(row) {
			Some(cells)	=> Self::cells_text(cells),
			None		=> fmt!(""),
		}
	}

	/// The text of the whole visible grid, one row per line, with trailing blanks removed.
	pub fn text(&self) -> String {
		let mut out = String::new();
		for r in 0..self.rows {
			if r > 0 {
				out.push('\n');
			}
			out.push_str(&self.row_text(r));
		}
		out
	}

	/// The text of a run of cells, with trailing blanks removed.
	pub fn cells_text(cells: &[Cell]) -> String {
		let mut end = cells.len();
		while end > 0
			&& cells[end - 1].chr == ' '
			&& (cells[end - 1].wide == Wide::No || cells[end - 1].wide == Wide::Filler)
		{
			end -= 1;
		}
		let mut out = String::new();
		for cell in &cells[..end] {
			match cell.wide {
				// Neither the right hand half of a wide character nor a leftover column at the
				// end of a row carries text of its own.
				Wide::Trail	=> {}
				Wide::Filler	=> out.push(' '),
				_		=> out.push(cell.chr),
			}
		}
		out
	}

	/// How many lines of scrollback are held.
	pub fn scrollback_len(&self) -> usize {
		self.scrollback.len()
	}

	/// The most lines of scrollback that will be kept.
	pub fn scrollback_max(&self) -> usize {
		self.scrollback_max
	}

	/// A line of scrollback, counting zero as the oldest.
	///
	/// Scrollback lines have their trailing blanks trimmed when they are stored, so the slice
	/// returned is usually shorter than the screen is wide, and is never longer than the screen was
	/// wide when the line left it.
	pub fn scrollback_line(&self, i: usize) -> Option<&[Cell]> {
		self.scrollback.get(i).map(|l| l.cells.as_slice())
	}

	/// The text of a line of scrollback, counting zero as the oldest.
	pub fn scrollback_text(&self, i: usize) -> Option<String> {
		self.scrollback.get(i).map(|l| Self::cells_text(&l.cells))
	}

	/// Whether a line of scrollback ran into the line below rather than ending there.
	pub fn scrollback_wrapped(&self, i: usize) -> bool {
		self.scrollback.get(i).map(|l| l.wrapped).unwrap_or(false)
	}

	/// How many lines of scrollback the viewport is shown above the live screen.
	///
	/// Zero is the live screen. The value never exceeds [`Screen::scrollback_len`], and a resize
	/// keeps the viewport on the same text rather than on the same number.
	pub fn view_offset(&self) -> usize {
		self.view
	}

	/// Scrolls the viewport back by `n` lines, clamped to what the scrollback holds.
	pub fn set_view_offset(&mut self, n: usize) {
		let want = n.min(self.scrollback.len());
		if want != self.view {
			self.view = want;
			self.touch_all();
		}
	}

	/// The cells of a row of the viewport, which is the live screen unless the view is scrolled
	/// back, or `None` if the row is off the viewport.
	pub fn view_row(&self, row: usize) -> Option<&[Cell]> {
		if row >= self.rows {
			return None;
		}
		if row < self.view {
			// A row from the scrollback, counting up from the oldest line still shown.
			let i = self.scrollback.len() - self.view + row;
			return self.scrollback.get(i).map(|l| l.cells.as_slice());
		}
		self.row(row - self.view)
	}

	/// The text of a row of the viewport, with trailing blanks removed.
	pub fn view_row_text(&self, row: usize) -> String {
		match self.view_row(row) {
			Some(cells)	=> Self::cells_text(cells),
			None		=> fmt!(""),
		}
	}

	/// What has changed since a renderer last looked.
	pub fn damage(&self) -> &Damage {
		&self.damage
	}

	/// Declares the screen drawn, so that damage accumulates afresh.
	pub fn clear_damage(&mut self) {
		for d in self.damage.rows.iter_mut() {
			*d = false;
		}
		self.damage.all = false;
		self.damage.cursor = false;
		self.damage.scrolled = 0;
	}

	// ┌─────────────────────────────┐
	// │ DAMAGE                      │
	// └─────────────────────────────┘

	/// Marks a row as needing redrawing.
	fn touch(&mut self, row: usize) {
		if let Some(d) = self.damage.rows.get_mut(row) {
			*d = true;
		}
	}

	/// Marks every row as needing redrawing.
	fn touch_all(&mut self) {
		self.damage.all = true;
	}

	/// Marks the cursor as moved.
	fn touch_cursor(&mut self) {
		self.damage.cursor = true;
	}

	// ┌─────────────────────────────┐
	// │ PEN                         │
	// └─────────────────────────────┘

	/// Sets the pen printing uses.
	pub fn set_pen(&mut self, pen: Pen) {
		self.pen = pen;
	}

	/// The pen an erase paints with: the current background, and nothing else.
	fn erase_pen(&self) -> Pen {
		Pen {
			fore:	Default::default(),
			back:	self.pen.back,
			attrs:	Default::default(),
		}
	}

	// ┌─────────────────────────────┐
	// │ PRINTING                    │
	// └─────────────────────────────┘

	/// Places a character at the cursor and advances.
	///
	/// A zero width character composes with the cell before the cursor where a composed form
	/// exists, and is otherwise dropped; the grid holds one character per cell, and a cell wide
	/// enough to hold an arbitrary sequence of combining marks would multiply the cost of the
	/// scrollback several times over for a case that a terminal rarely meets.
	pub fn print(&mut self, c: char) {
		match char_width(c) {
			CharWidth::Zero	=> self.combine(c),
			CharWidth::Narrow	=> self.place(c, 1),
			CharWidth::Wide	=> self.place(c, 2),
		}
	}

	/// Composes a combining mark into the cell before the cursor.
	fn combine(&mut self, mark: char) {
		let col = if self.cursor.wrap_pending {
			self.cursor.col
		} else if self.cursor.col > 0 {
			self.cursor.col - 1
		} else {
			return;
		};
		let row = self.cursor.row;
		// Step back over the right hand half of a wide character.
		let col = match self.cell(col, row) {
			Some(cell) if cell.wide == Wide::Trail && col > 0	=> col - 1,
			_							=> col,
		};
		let base = match self.cell(col, row) {
			Some(cell)	=> cell.chr,
			None		=> return,
		};
		if let Some(composed) = compose(base, mark) {
			let i = row * self.cols + col;
			if let Some(cell) = self.grid.get_mut(i) {
				cell.chr = composed;
			}
			self.touch(row);
		}
	}

	/// Places a character of the given cell width at the cursor and advances.
	fn place(&mut self, c: char, n: usize) {
		if n > self.cols {
			return;
		}
		if self.cursor.wrap_pending && self.autowrap {
			self.wrap();
		}
		if self.cursor.col + n > self.cols {
			if self.autowrap {
				// The character will not fit in the columns left, so those columns are marked
				// as the padding they are and the character starts the next row whole. Without
				// the mark a later rewrap cannot tell them from spaces somebody printed.
				self.pad_to_edge();
				self.wrap();
			} else {
				self.cursor.col = self.cols - n;
			}
		}
		if self.insert {
			self.insert_cells(n);
		}
		let row = self.cursor.row;
		let col = self.cursor.col;
		self.clear_overlapped(col, n);
		let pen = self.pen;
		let base = row * self.cols + col;
		if let Some(cell) = self.grid.get_mut(base) {
			*cell = Cell {
				chr:	c,
				pen,
				wide:	if n == 2 { Wide::Lead } else { Wide::No },
			};
		}
		if n == 2 {
			if let Some(cell) = self.grid.get_mut(base + 1) {
				*cell = Cell {
					chr:	' ',
					pen,
					wide:	Wide::Trail,
				};
			}
		}
		self.touch(row);
		self.cursor.col += n;
		if self.cursor.col >= self.cols {
			self.cursor.col = self.cols - 1;
			self.cursor.wrap_pending = self.autowrap;
		} else {
			self.cursor.wrap_pending = false;
		}
		self.touch_cursor();
	}

	/// Marks the columns from the cursor to the end of the row as leftover padding.
	fn pad_to_edge(&mut self) {
		let row = self.cursor.row;
		let base = row * self.cols;
		for col in self.cursor.col..self.cols {
			if let Some(cell) = self.grid.get_mut(base + col) {
				*cell = Cell::filler(cell.pen);
			}
		}
		self.touch(row);
	}

	/// Blanks the other half of any wide character the write at `col` would cut in two.
	fn clear_overlapped(&mut self, col: usize, n: usize) {
		let row = self.cursor.row;
		let pen = self.erase_pen();
		// The cell to the left, if this write lands on the right hand half of a wide character.
		if col > 0 {
			let left = row * self.cols + col - 1;
			if self.grid.get(left).map(|c| c.wide) == Some(Wide::Lead) {
				if let Some(cell) = self.grid.get_mut(left) {
					*cell = Cell::blank(pen);
				}
			}
		}
		// The cell to the right, if this write covers the left hand half of a wide character.
		let last = col + n;
		if last < self.cols {
			let right = row * self.cols + last;
			if self.grid.get(right).map(|c| c.wide) == Some(Wide::Trail) {
				if let Some(cell) = self.grid.get_mut(right) {
					*cell = Cell::blank(pen);
				}
			}
		}
	}

	/// Moves to the start of the next line, scrolling if the cursor is at the foot of the region.
	///
	/// This is the only place a row is marked as running into the row below, and that mark is what
	/// lets a later resize put the text back together.
	fn wrap(&mut self) {
		self.cursor.wrap_pending = false;
		self.cursor.col = 0;
		if let Some(w) = self.wrapped.get_mut(self.cursor.row) {
			*w = true;
		}
		self.line_feed_inner();
	}

	// ┌─────────────────────────────┐
	// │ CONTROLS                    │
	// └─────────────────────────────┘

	/// Moves the cursor one cell left, without wrapping to the previous line.
	pub fn backspace(&mut self) {
		if self.cursor.wrap_pending {
			self.cursor.wrap_pending = false;
		} else if self.cursor.col > 0 {
			self.cursor.col -= 1;
		}
		self.touch_cursor();
	}

	/// Moves the cursor to the first column.
	pub fn carriage_return(&mut self) {
		self.cursor.col = 0;
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Moves the cursor down one row, scrolling if it is at the foot of the region.
	pub fn line_feed(&mut self) {
		self.cursor.wrap_pending = false;
		self.line_feed_inner();
	}

	/// The row advance of a line feed, without touching the pending wrap.
	fn line_feed_inner(&mut self) {
		if self.cursor.row == self.region.1 {
			self.scroll_up(1);
		} else if self.cursor.row + 1 < self.rows {
			self.cursor.row += 1;
		}
		self.touch_cursor();
	}

	/// Moves the cursor to the first column of the next row, scrolling if need be.
	pub fn next_line(&mut self) {
		self.carriage_return();
		self.line_feed();
	}

	/// Moves the cursor up one row, scrolling the region down if it is at the head of it.
	pub fn reverse_index(&mut self) {
		self.cursor.wrap_pending = false;
		if self.cursor.row == self.region.0 {
			self.scroll_down(1);
		} else if self.cursor.row > 0 {
			self.cursor.row -= 1;
		}
		self.touch_cursor();
	}

	/// Moves the cursor to the next tab stop, or to the final column if there is none.
	pub fn tab(&mut self) {
		let mut col = self.cursor.col + 1;
		while col < self.cols && !self.tabs[col] {
			col += 1;
		}
		self.cursor.col = col.min(self.cols - 1);
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Moves the cursor forward `n` tab stops.
	pub fn tab_forward(&mut self, n: usize) {
		for _ in 0..n.max(1) {
			self.tab();
		}
	}

	/// Moves the cursor back `n` tab stops.
	pub fn tab_back(&mut self, n: usize) {
		for _ in 0..n.max(1) {
			let mut col = self.cursor.col;
			loop {
				if col == 0 {
					break;
				}
				col -= 1;
				if self.tabs[col] {
					break;
				}
			}
			self.cursor.col = col;
		}
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Sets a tab stop at the cursor.
	pub fn set_tab(&mut self) {
		if let Some(t) = self.tabs.get_mut(self.cursor.col) {
			*t = true;
		}
	}

	/// Clears the tab stop at the cursor.
	pub fn clear_tab(&mut self) {
		if let Some(t) = self.tabs.get_mut(self.cursor.col) {
			*t = false;
		}
	}

	/// Clears every tab stop.
	pub fn clear_all_tabs(&mut self) {
		for t in self.tabs.iter_mut() {
			*t = false;
		}
	}

	// ┌─────────────────────────────┐
	// │ CURSOR MOVEMENT             │
	// └─────────────────────────────┘

	/// Moves the cursor to an absolute position, clamped to the grid.
	///
	/// With origin mode set, the row is counted from the head of the scrolling region and cannot
	/// leave it.
	pub fn move_to(&mut self, col: usize, row: usize) {
		let (lo, hi) = if self.origin {
			self.region
		} else {
			(0, self.rows - 1)
		};
		self.cursor.row = (lo + row).min(hi);
		self.cursor.col = col.min(self.cols - 1);
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Moves the cursor to a column, leaving the row alone.
	pub fn move_to_col(&mut self, col: usize) {
		self.cursor.col = col.min(self.cols - 1);
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Moves the cursor to a row, leaving the column alone.
	pub fn move_to_row(&mut self, row: usize) {
		let (lo, hi) = if self.origin {
			self.region
		} else {
			(0, self.rows - 1)
		};
		self.cursor.row = (lo + row).min(hi);
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Moves the cursor up, stopping at the head of the region or of the grid.
	pub fn move_up(&mut self, n: usize) {
		let lo = if self.cursor.row >= self.region.0 { self.region.0 } else { 0 };
		self.cursor.row = self.cursor.row.saturating_sub(n.max(1)).max(lo);
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Moves the cursor down, stopping at the foot of the region or of the grid.
	pub fn move_down(&mut self, n: usize) {
		let hi = if self.cursor.row <= self.region.1 { self.region.1 } else { self.rows - 1 };
		self.cursor.row = (self.cursor.row + n.max(1)).min(hi);
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Moves the cursor left, stopping at the first column.
	pub fn move_left(&mut self, n: usize) {
		self.cursor.col = self.cursor.col.saturating_sub(n.max(1));
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Moves the cursor right, stopping at the final column.
	pub fn move_right(&mut self, n: usize) {
		self.cursor.col = (self.cursor.col + n.max(1)).min(self.cols - 1);
		self.cursor.wrap_pending = false;
		self.touch_cursor();
	}

	/// Shows or hides the cursor.
	pub fn set_cursor_visible(&mut self, visible: bool) {
		self.cursor.visible = visible;
		self.touch_cursor();
	}

	/// Saves the cursor, the pen and origin mode.
	pub fn save_cursor(&mut self) {
		self.saved = Some(SavedCursor {
			cursor:	self.cursor,
			pen:	self.pen,
			origin:	self.origin,
		});
	}

	/// Restores what [`Screen::save_cursor`] saved, or homes the cursor if nothing was saved.
	pub fn restore_cursor(&mut self) {
		match self.saved {
			Some(s)	=> {
				self.cursor = s.cursor;
				self.cursor.row = self.cursor.row.min(self.rows - 1);
				self.cursor.col = self.cursor.col.min(self.cols - 1);
				self.pen = s.pen;
				self.origin = s.origin;
			}
			None	=> {
				self.cursor.row = 0;
				self.cursor.col = 0;
				self.cursor.wrap_pending = false;
			}
		}
		self.touch_cursor();
	}

	// ┌─────────────────────────────┐
	// │ MODES                       │
	// └─────────────────────────────┘

	/// Sets whether printing past the final column wraps.
	pub fn set_autowrap(&mut self, on: bool) {
		self.autowrap = on;
		if !on {
			self.cursor.wrap_pending = false;
		}
	}

	/// Sets whether printing pushes the rest of the line to the right.
	pub fn set_insert(&mut self, on: bool) {
		self.insert = on;
	}

	/// Sets whether row addressing is relative to the scrolling region, homing the cursor.
	pub fn set_origin(&mut self, on: bool) {
		self.origin = on;
		self.move_to(0, 0);
	}

	/// Sets the scrolling region and homes the cursor, as `DECSTBM` requires.
	///
	/// A region of fewer than two rows, or one that does not fit, is refused and the region is set
	/// to the whole screen instead, which is what the standard asks for.
	pub fn set_region(&mut self, top: usize, bottom: usize) {
		if top < bottom && bottom < self.rows {
			self.region = (top, bottom);
		} else {
			self.region = (0, self.rows - 1);
		}
		self.move_to(0, 0);
	}

	// ┌─────────────────────────────┐
	// │ ERASING                     │
	// └─────────────────────────────┘

	/// Erases part or all of the cursor's row.
	pub fn erase_line(&mut self, what: Erase) {
		let row = self.cursor.row;
		let (a, b) = match what {
			Erase::ToEnd	=> (self.cursor.col, self.cols),
			Erase::ToStart	=> (0, self.cursor.col + 1),
			Erase::All	=> (0, self.cols),
		};
		self.blank_span(row, a, b.min(self.cols));
		self.cursor.wrap_pending = false;
		self.touch(row);
	}

	/// Erases part or all of the grid.
	pub fn erase_display(&mut self, what: Erase) {
		match what {
			Erase::ToEnd	=> {
				let row = self.cursor.row;
				let col = self.cursor.col;
				self.blank_span(row, col, self.cols);
				for r in row + 1..self.rows {
					self.blank_span(r, 0, self.cols);
				}
			}
			Erase::ToStart	=> {
				let row = self.cursor.row;
				let col = self.cursor.col;
				for r in 0..row {
					self.blank_span(r, 0, self.cols);
				}
				self.blank_span(row, 0, (col + 1).min(self.cols));
			}
			Erase::All	=> {
				for r in 0..self.rows {
					self.blank_span(r, 0, self.cols);
				}
			}
		}
		self.cursor.wrap_pending = false;
		self.touch_all();
	}

	/// Erases `n` cells from the cursor without moving it.
	pub fn erase_chars(&mut self, n: usize) {
		let row = self.cursor.row;
		let a = self.cursor.col;
		let b = (a + n.max(1)).min(self.cols);
		self.blank_span(row, a, b);
		self.touch(row);
	}

	/// Discards the scrollback.
	pub fn erase_scrollback(&mut self) {
		self.scrollback.clear();
	}

	/// Blanks the cells of `row` in `[a, b)` with the erase pen.
	///
	/// An erase that reaches the end of the row also ends it: whatever ran on from it is no longer
	/// the same text, so the row stops being a continuation. tmux does the same, which is why
	/// erasing a line and then widening the window does not glue the remains back together.
	fn blank_span(&mut self, row: usize, a: usize, b: usize) {
		let pen = self.erase_pen();
		let base = row * self.cols;
		for col in a..b {
			if let Some(cell) = self.grid.get_mut(base + col) {
				*cell = Cell::blank(pen);
			}
		}
		if b >= self.cols {
			if let Some(w) = self.wrapped.get_mut(row) {
				*w = false;
			}
		}
		self.touch(row);
	}

	// ┌─────────────────────────────┐
	// │ INSERTION AND DELETION      │
	// └─────────────────────────────┘

	/// Inserts `n` blank cells at the cursor, pushing the rest of the row right.
	pub fn insert_chars(&mut self, n: usize) {
		self.insert_cells(n.max(1));
		let row = self.cursor.row;
		self.touch(row);
	}

	/// The cell shuffle behind an insertion.
	fn insert_cells(&mut self, n: usize) {
		let row = self.cursor.row;
		let col = self.cursor.col;
		let n = n.min(self.cols - col);
		let base = row * self.cols;
		let mut i = self.cols;
		while i > col + n {
			i -= 1;
			self.grid[base + i] = self.grid[base + i - n];
		}
		let pen = self.erase_pen();
		for c in col..col + n {
			self.grid[base + c] = Cell::blank(pen);
		}
		self.touch(row);
	}

	/// Deletes `n` cells at the cursor, pulling the rest of the row left.
	pub fn delete_chars(&mut self, n: usize) {
		let row = self.cursor.row;
		let col = self.cursor.col;
		let n = n.max(1).min(self.cols - col);
		let base = row * self.cols;
		for i in col..self.cols - n {
			self.grid[base + i] = self.grid[base + i + n];
		}
		let pen = self.erase_pen();
		for i in self.cols - n..self.cols {
			self.grid[base + i] = Cell::blank(pen);
		}
		self.touch(row);
	}

	/// Inserts `n` blank rows at the cursor, pushing the rest of the region down.
	///
	/// Nothing happens if the cursor is outside the scrolling region, which is what the standard
	/// requires and what stops a full screen application corrupting a status line.
	pub fn insert_lines(&mut self, n: usize) {
		let row = self.cursor.row;
		if row < self.region.0 || row > self.region.1 {
			return;
		}
		let n = n.max(1).min(self.region.1 - row + 1);
		self.shift_rows_down(row, self.region.1, n);
		self.cursor.col = 0;
		self.cursor.wrap_pending = false;
		self.touch_all();
	}

	/// Deletes `n` rows at the cursor, pulling the rest of the region up.
	pub fn delete_lines(&mut self, n: usize) {
		let row = self.cursor.row;
		if row < self.region.0 || row > self.region.1 {
			return;
		}
		let n = n.max(1).min(self.region.1 - row + 1);
		self.shift_rows_up(row, self.region.1, n, false);
		self.cursor.col = 0;
		self.cursor.wrap_pending = false;
		self.touch_all();
	}

	// ┌─────────────────────────────┐
	// │ SCROLLING                   │
	// └─────────────────────────────┘

	/// Scrolls the region up `n` rows, keeping what leaves the top if it is also the top of the
	/// screen.
	pub fn scroll_up(&mut self, n: usize) {
		let n = n.max(1);
		let keep = self.surface == Surface::Primary && self.region.0 == 0;
		self.shift_rows_up(self.region.0, self.region.1, n, keep);
		self.touch_all();
	}

	/// Scrolls the region down `n` rows, discarding what leaves the foot.
	pub fn scroll_down(&mut self, n: usize) {
		let n = n.max(1);
		self.shift_rows_down(self.region.0, self.region.1, n);
		self.touch_all();
	}

	/// Moves rows `[top, bottom]` up by `n`, blanking the rows uncovered at the foot.
	///
	/// When `keep` is set, the rows that leave the top are appended to the scrollback.
	fn shift_rows_up(&mut self, top: usize, bottom: usize, n: usize, keep: bool) {
		let span = bottom - top + 1;
		let n = n.min(span);
		if keep {
			for r in top..top + n {
				let a = r * self.cols;
				let line = Line {
					cells:		self.grid[a..a + self.cols].to_vec(),
					wrapped:	self.row_wrapped(r),
				};
				self.push_scrollback(line);
			}
		}
		for r in top..=bottom - n {
			let dst = r * self.cols;
			let src = (r + n) * self.cols;
			for c in 0..self.cols {
				self.grid[dst + c] = self.grid[src + c];
			}
			self.wrapped[r] = self.wrapped[r + n];
		}
		let pen = self.erase_pen();
		for r in bottom + 1 - n..=bottom {
			let a = r * self.cols;
			for c in 0..self.cols {
				self.grid[a + c] = Cell::blank(pen);
			}
			self.wrapped[r] = false;
		}
	}

	/// Moves rows `[top, bottom]` down by `n`, blanking the rows uncovered at the head.
	fn shift_rows_down(&mut self, top: usize, bottom: usize, n: usize) {
		let span = bottom - top + 1;
		let n = n.min(span);
		let mut r = bottom + 1;
		while r > top + n {
			r -= 1;
			let dst = r * self.cols;
			let src = (r - n) * self.cols;
			for c in 0..self.cols {
				self.grid[dst + c] = self.grid[src + c];
			}
			self.wrapped[r] = self.wrapped[r - n];
		}
		let pen = self.erase_pen();
		for r in top..top + n {
			let a = r * self.cols;
			for c in 0..self.cols {
				self.grid[a + c] = Cell::blank(pen);
			}
			self.wrapped[r] = false;
		}
	}

	/// Appends a line to the scrollback, trimming its trailing blanks and evicting the oldest line
	/// once the bound is reached.
	///
	/// A wrapped line keeps its trailing blanks. Its length is the width the screen had when it was
	/// stored, and a rewrap needs that width in order to put the text back together: a wrapped line
	/// whose last cells happen to be spaces would otherwise lose those columns when it is joined to
	/// the line below.
	fn push_scrollback(&mut self, mut line: Line) {
		if self.scrollback_max == 0 {
			self.damage.scrolled += 1;
			return;
		}
		if !line.wrapped {
			while line.cells.len() > 0 {
				let last = line.cells.len() - 1;
				if line.cells[last].is_blank() {
					line.cells.truncate(last);
				} else {
					break;
				}
			}
		}
		line.cells.shrink_to_fit();
		while self.scrollback.len() >= self.scrollback_max {
			self.scrollback.pop_front();
			self.view = self.view.saturating_sub(1);
		}
		self.scrollback.push_back(line);
		if self.view > 0 {
			// The viewport stays on the text it was showing rather than sliding with the screen.
			self.view += 1;
			self.view = self.view.min(self.scrollback.len());
		}
		self.damage.scrolled += 1;
	}

	// ┌─────────────────────────────┐
	// │ SURFACES                    │
	// └─────────────────────────────┘

	/// Switches between the ordinary and the alternate grid.
	///
	/// When `save` is set, as it is for the `?1049` form, the cursor and pen are saved on the way in
	/// and restored on the way out, and the alternate grid is cleared as it comes forward. The
	/// alternate grid never contributes to the scrollback, which is the whole point of it: an
	/// editor that takes over the screen should leave the log of the session it interrupted intact.
	pub fn set_surface(&mut self, want: Surface, save: bool) {
		if want == self.surface {
			return;
		}
		if self.spare.len() != self.grid.len() {
			self.spare = vec![Cell::default(); self.grid.len()];
		}
		if self.spare_wrapped.len() != self.wrapped.len() {
			self.spare_wrapped = vec![false; self.wrapped.len()];
		}
		std::mem::swap(&mut self.grid, &mut self.spare);
		std::mem::swap(&mut self.wrapped, &mut self.spare_wrapped);
		let outgoing = self.cursor;
		self.cursor = self.spare_cursor;
		self.spare_cursor = outgoing;
		self.surface = want;
		match want {
			Surface::Alternate	=> {
				if save {
					// The saved cursor of the ordinary screen is what a leave restores.
					self.saved = Some(SavedCursor {
						cursor:	outgoing,
						pen:	self.pen,
						origin:	self.origin,
					});
				}
				let pen = self.erase_pen();
				for cell in self.grid.iter_mut() {
					*cell = Cell::blank(pen);
				}
				for w in self.wrapped.iter_mut() {
					*w = false;
				}
				self.cursor = Cursor {
					row:		0,
					col:		0,
					visible:	outgoing.visible,
					wrap_pending:	false,
				};
			}
			Surface::Primary	=> {
				if save {
					if let Some(s) = self.saved {
						self.cursor = s.cursor;
						self.pen = s.pen;
						self.origin = s.origin;
					}
				}
			}
		}
		self.cursor.row = self.cursor.row.min(self.rows - 1);
		self.cursor.col = self.cursor.col.min(self.cols - 1);
		self.region = (0, self.rows - 1);
		self.touch_all();
		self.touch_cursor();
	}

	// ┌─────────────────────────────┐
	// │ RESIZE AND RESET            │
	// └─────────────────────────────┘

	/// Changes the size of the screen, putting the text back together at the new width.
	///
	/// Height is settled first, then width, because the rewrap has to know how many rows the screen
	/// is going to keep.
	///
	/// Rows keep the cursor on the screen. Shrinking removes rows from the top, so that the most
	/// recent output survives, and those rows go to the scrollback; if the cursor is near the top
	/// and rows can be spared below it, they are taken from the bottom instead and discarded.
	/// Growing pulls lines back out of the scrollback where any are held, and pads at the foot for
	/// the rest.
	///
	/// Columns rewrap. The scrollback and the ordinary screen are joined back into the lines the
	/// application actually printed, using the record [`Line::wrapped`] keeps of which rows ran on
	/// into the row below, and are then split again at the new width. Narrowing a window therefore
	/// keeps every character; only a terminal that truncates loses text a reader could still have
	/// read. A double width character that will not fit in the columns left at the end of a row is
	/// moved whole to the next row, leaving the odd column blank, which is what the same character
	/// does when it is printed there in the first place.
	///
	/// The alternate screen is not rewrapped. A full screen application draws at absolute positions
	/// and its rows are not continuations of anything, so joining them would scramble the display;
	/// it is truncated and padded, and the application redraws.
	///
	/// The cursor follows the character it was on rather than the coordinates it had, and the
	/// viewport follows the line it was showing, so a resize while the view is scrolled back does
	/// not jump.
	///
	/// The scrolling region is reset to the whole screen, which is what a real terminal does, since
	/// a region set for the old height rarely means anything at the new one.
	///
	/// # Errors
	/// Fails if either dimension is zero.
	pub fn resize(&mut self, cols: usize, rows: usize) -> Outcome<()> {
		if cols == 0 || rows == 0 {
			return Err(err!(
				"A screen must have at least one row and one column, was given {} by {}.",
				cols, rows;
				Invalid, Input));
		}
		if cols == self.cols && rows == self.rows {
			return Ok(());
		}
		let alt = self.surface == Surface::Alternate;
		// `main` is the ordinary screen wherever it happens to be, since that is the one holding
		// the scrollback and the one worth rewrapping; `other` is the alternate one.
		let (mut main, mut other) = if alt {
			(
				Self::split_grid(&self.spare, &self.spare_wrapped, self.cols, self.rows),
				Self::split_grid(&self.grid, &self.wrapped, self.cols, self.rows),
			)
		} else {
			(
				Self::split_grid(&self.grid, &self.wrapped, self.cols, self.rows),
				Self::split_grid(&self.spare, &self.spare_wrapped, self.cols, self.rows),
			)
		};
		let mut main_cur = if alt { self.spare_cursor } else { self.cursor };
		let mut other_cur = if alt { self.cursor } else { self.spare_cursor };

		// Height.
		if main.is_empty() {
			main = vec![Line::blank(self.cols); rows];
		} else {
			self.fit_height(&mut main, &mut main_cur, rows);
		}
		while other.len() > rows {
			other.pop();
		}
		while !other.is_empty() && other.len() < rows {
			other.push(Line::blank(self.cols));
		}

		// Width.
		if cols == self.cols {
			for line in main.iter_mut() {
				Self::resize_line(&mut line.cells, cols);
			}
		} else {
			self.rewrap(&mut main, &mut main_cur, cols, rows);
		}
		for line in other.iter_mut() {
			Self::resize_line(&mut line.cells, cols);
			line.wrapped = false;
		}
		other_cur.row = other_cur.row.min(rows - 1);
		other_cur.col = other_cur.col.min(cols - 1);

		// Reassemble.
		let (front, back) = if alt { (other, main) } else { (main, other) };
		self.grid = Vec::with_capacity(cols * rows);
		self.wrapped = Vec::with_capacity(rows);
		for line in front {
			self.grid.extend_from_slice(&line.cells);
			self.wrapped.push(line.wrapped);
		}
		self.spare = Vec::with_capacity(cols * rows);
		self.spare_wrapped = Vec::with_capacity(rows);
		for line in back {
			self.spare.extend_from_slice(&line.cells);
			self.spare_wrapped.push(line.wrapped);
		}
		if alt {
			self.cursor = other_cur;
			self.spare_cursor = main_cur;
		} else {
			self.cursor = main_cur;
			self.spare_cursor = other_cur;
		}
		// Tab stops keep whatever the application set within the old width.
		let mut tabs = Self::default_tabs(cols);
		for c in 0..cols.min(self.cols) {
			tabs[c] = self.tabs[c];
		}
		self.tabs = tabs;
		self.cols = cols;
		self.rows = rows;
		self.region = (0, rows - 1);
		self.cursor.row = self.cursor.row.min(rows - 1);
		self.cursor.col = self.cursor.col.min(cols - 1);
		self.spare_cursor.row = self.spare_cursor.row.min(rows - 1);
		self.spare_cursor.col = self.spare_cursor.col.min(cols - 1);
		self.view = self.view.min(self.scrollback.len());
		let scrolled = self.damage.scrolled;
		self.damage = Damage::new(rows);
		self.damage.scrolled = scrolled;
		Ok(())
	}

	/// Takes a grid apart into lines, or gives nothing back if the grid is not the size claimed.
	fn split_grid(grid: &[Cell], wrapped: &[bool], cols: usize, rows: usize) -> Vec<Line> {
		if grid.len() < cols * rows || cols == 0 {
			return Vec::new();
		}
		let mut out = Vec::with_capacity(rows);
		for r in 0..rows {
			let a = r * cols;
			out.push(Line {
				cells:		grid[a..a + cols].to_vec(),
				wrapped:	wrapped.get(r).copied().unwrap_or(false),
			});
		}
		out
	}

	/// Brings the ordinary screen to `rows` rows, keeping the cursor on it.
	fn fit_height(&mut self, lines: &mut Vec<Line>, cur: &mut Cursor, rows: usize) {
		while lines.len() > rows {
			// Prefer to lose a row below the cursor; otherwise lose the topmost.
			if cur.row + 1 < lines.len() {
				lines.pop();
			} else {
				let gone = lines.remove(0);
				self.push_scrollback(gone);
				cur.row = cur.row.saturating_sub(1);
			}
		}
		while lines.len() < rows {
			match self.scrollback.pop_back() {
				Some(mut line)	=> {
					line.cells.resize(self.cols, Cell::default());
					lines.insert(0, line);
					cur.row += 1;
					self.view = self.view.saturating_sub(1);
				}
				None		=> lines.push(Line::blank(self.cols)),
			}
		}
	}

	/// Joins the scrollback and the ordinary screen back into the lines that were printed, splits
	/// them again at `cols`, and hands the screen back its last `rows` of them.
	///
	/// Whatever the rewrap produces above those rows becomes the new scrollback, which is why
	/// narrowing a window pushes text upwards into the history rather than losing it.
	fn rewrap(&mut self, lines: &mut Vec<Line>, cur: &mut Cursor, cols: usize, rows: usize) {
		let hist = self.scrollback.len();
		let cur_line = hist + cur.row;
		let cur_col = cur.reported_col();
		let top_line = hist.saturating_sub(self.view);
		let mut all: Vec<Line> = Vec::with_capacity(hist + lines.len());
		all.extend(self.scrollback.drain(..));
		all.append(lines);

		// Join. A line that ran on is glued to the one below it, and where each line started in
		// the joined text is remembered so that the cursor and the viewport can be found again.
		let mut logical: Vec<Vec<Cell>> = Vec::new();
		let mut place: Vec<(usize, usize)> = Vec::with_capacity(all.len());
		let mut open = false;
		for line in all {
			if !open {
				logical.push(Vec::new());
			}
			let i = logical.len() - 1;
			place.push((i, logical[i].len()));
			// A column left over at the end of a row belongs to the width the row had, not to
			// the text, so it goes no further.
			let mut end = line.cells.len();
			while end > 0 && line.cells[end - 1].wide == Wide::Filler {
				end -= 1;
			}
			logical[i].extend_from_slice(&line.cells[..end]);
			open = line.wrapped;
		}
		// Trailing blanks are padding rather than text, so they are not carried into the rewrap;
		// a blank cell painted with a background colour is not blank and stays.
		for cells in logical.iter_mut() {
			while let Some(last) = cells.last() {
				if last.is_blank() {
					cells.pop();
				} else {
					break;
				}
			}
		}

		// Split again.
		let (cur_log, cur_base) = place.get(cur_line).copied().unwrap_or((0, 0));
		let cur_off = cur_base + cur_col;
		let (top_log, top_off) = place.get(top_line).copied().unwrap_or((0, 0));
		let mut out: Vec<Line> = Vec::new();
		let mut cur_at = (0usize, 0usize);
		let mut top_at = 0usize;
		for (li, cells) in logical.iter().enumerate() {
			let (made, starts) = Self::split_line(cells, cols);
			let base = out.len();
			if li == cur_log {
				let (k, c) = Self::locate(&starts, cur_off);
				cur_at = (base + k, c);
			}
			if li == top_log {
				let (k, _) = Self::locate(&starts, top_off);
				top_at = base + k;
			}
			out.extend(made);
		}
		while out.len() < rows {
			out.push(Line::blank(cols));
		}

		// The screen keeps the last `rows` of what the rewrap made; the rest is history.
		let hist_new = out.len() - rows;
		let skip = hist_new.saturating_sub(self.scrollback_max);
		self.scrollback.clear();
		let mut i = 0;
		for line in out.drain(..hist_new) {
			if i >= skip {
				self.scrollback.push_back(line);
			}
			i += 1;
		}
		*lines = out;
		self.view = if top_at >= hist_new {
			0
		} else if top_at >= skip {
			self.scrollback.len() - (top_at - skip)
		} else {
			self.scrollback.len()
		};
		cur.row = cur_at.0.saturating_sub(hist_new).min(rows - 1);
		if cur_at.1 >= cols {
			cur.col = cols - 1;
			cur.wrap_pending = true;
		} else {
			cur.col = cur_at.1;
			cur.wrap_pending = false;
		}
	}

	/// Splits a joined line into rows of `cols` cells, and says where in the joined line each row
	/// begins.
	///
	/// A double width character that will not fit in the columns left at the end of a row is moved
	/// whole to the next row and the odd column is left blank, which is what the same character
	/// does when it is printed at the edge of a screen.
	fn split_line(cells: &[Cell], cols: usize) -> (Vec<Line>, Vec<usize>) {
		let mut made: Vec<Line> = Vec::new();
		let mut starts: Vec<usize> = Vec::new();
		let mut i = 0;
		loop {
			starts.push(i);
			let mut row: Vec<Cell> = Vec::with_capacity(cols);
			while i < cells.len() && row.len() < cols {
				let cell = cells[i];
				match cell.wide {
					Wide::Lead	=> {
						if row.len() + 2 > cols {
							// It will not fit in what is left of the row.
							row.push(Cell::filler(Pen::default()));
							break;
						}
						row.push(cell);
						i += 1;
						if i < cells.len() && cells[i].wide == Wide::Trail {
							row.push(cells[i]);
							i += 1;
						} else {
							row.push(Cell {
								chr:	' ',
								pen:	cell.pen,
								wide:	Wide::Trail,
							});
						}
					}
					Wide::Trail	=> {
						// A trailing half with no lead before it, which only an earlier cut can
						// leave behind. It stands as a blank of its own.
						row.push(Cell::blank(cell.pen));
						i += 1;
					}
					Wide::Filler	=> {
						// Padding from a width the text no longer has.
						i += 1;
					}
					Wide::No	=> {
						row.push(cell);
						i += 1;
					}
				}
			}
			let wrapped = i < cells.len();
			while row.len() < cols {
				row.push(Cell::default());
			}
			made.push(Line { cells: row, wrapped });
			if i >= cells.len() {
				break;
			}
		}
		(made, starts)
	}

	/// Finds the row and column a position in a joined line ended up at, given where each row of
	/// the split began.
	///
	/// A position one past the end of a full row comes back as that row and a column equal to the
	/// width, which is the pending wrap the caller turns back into a cursor.
	fn locate(starts: &[usize], off: usize) -> (usize, usize) {
		let mut k = 0;
		for (j, s) in starts.iter().enumerate() {
			if *s <= off {
				k = j;
			} else {
				break;
			}
		}
		(k, off - starts.get(k).copied().unwrap_or(0))
	}

	/// Truncates or pads one line to `cols`, blanking a wide character the truncation would cut.
	fn resize_line(line: &mut Vec<Cell>, cols: usize) {
		if line.len() > cols {
			line.truncate(cols);
			if cols > 0 {
				let last = cols - 1;
				if line[last].wide == Wide::Lead {
					line[last] = Cell::default();
				}
			}
		} else {
			line.resize(cols, Cell::default());
		}
	}

	/// Returns the screen to the state it had when it was made, keeping only its size.
	pub fn reset(&mut self) {
		let pen = Pen::default();
		for cell in self.grid.iter_mut() {
			*cell = Cell::blank(pen);
		}
		for cell in self.spare.iter_mut() {
			*cell = Cell::blank(pen);
		}
		for w in self.wrapped.iter_mut() {
			*w = false;
		}
		for w in self.spare_wrapped.iter_mut() {
			*w = false;
		}
		self.surface = Surface::Primary;
		self.cursor = Cursor::default();
		self.spare_cursor = Cursor::default();
		self.pen = pen;
		self.saved = None;
		self.region = (0, self.rows - 1);
		self.tabs = Self::default_tabs(self.cols);
		self.autowrap = true;
		self.insert = false;
		self.origin = false;
		self.scrollback.clear();
		self.view = 0;
		self.damage = Damage::new(self.rows);
	}

	/// Fills the grid with `E`, which is the alignment pattern `DECALN` draws.
	pub fn fill_alignment(&mut self) {
		let pen = Pen::default();
		for cell in self.grid.iter_mut() {
			*cell = Cell {
				chr:	'E',
				pen,
				wide:	Wide::No,
			};
		}
		for w in self.wrapped.iter_mut() {
			*w = false;
		}
		self.cursor = Cursor {
			row:		0,
			col:		0,
			visible:	self.cursor.visible,
			wrap_pending:	false,
		};
		self.region = (0, self.rows - 1);
		self.touch_all();
		self.touch_cursor();
	}
}

/// Composes a base character with a combining mark, where a single composed character exists.
fn compose(base: char, mark: char) -> Option<char> {
	let mut s = String::with_capacity(8);
	s.push(base);
	s.push(mark);
	let composed = norm::nfc(&s);
	let mut it = composed.chars();
	match (it.next(), it.next()) {
		(Some(c), None)	=> Some(c),
		_		=> None,
	}
}
