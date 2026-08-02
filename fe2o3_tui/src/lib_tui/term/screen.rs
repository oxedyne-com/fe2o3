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
	/// The grid that is not in front, held while the other is shown.
	spare:	Vec<Cell>,
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
	scrollback: VecDeque<Vec<Cell>>,
	/// The most lines of scrollback that will be kept.
	scrollback_max: usize,
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
			spare:		Vec::new(),
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
	fn cells_text(cells: &[Cell]) -> String {
		let mut end = cells.len();
		while end > 0 && cells[end - 1].chr == ' ' && cells[end - 1].wide == Wide::No {
			end -= 1;
		}
		let mut out = String::new();
		for cell in &cells[..end] {
			if cell.wide != Wide::Trail {
				out.push(cell.chr);
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
		self.scrollback.get(i).map(|v| v.as_slice())
	}

	/// The text of a line of scrollback, counting zero as the oldest.
	pub fn scrollback_text(&self, i: usize) -> Option<String> {
		self.scrollback.get(i).map(|v| Self::cells_text(v))
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
	fn wrap(&mut self) {
		self.cursor.wrap_pending = false;
		self.cursor.col = 0;
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
	fn blank_span(&mut self, row: usize, a: usize, b: usize) {
		let pen = self.erase_pen();
		let base = row * self.cols;
		for col in a..b {
			if let Some(cell) = self.grid.get_mut(base + col) {
				*cell = Cell::blank(pen);
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
				let line = self.grid[a..a + self.cols].to_vec();
				self.push_scrollback(line);
			}
		}
		for r in top..=bottom - n {
			let dst = r * self.cols;
			let src = (r + n) * self.cols;
			for c in 0..self.cols {
				self.grid[dst + c] = self.grid[src + c];
			}
		}
		let pen = self.erase_pen();
		for r in bottom + 1 - n..=bottom {
			let a = r * self.cols;
			for c in 0..self.cols {
				self.grid[a + c] = Cell::blank(pen);
			}
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
		}
		let pen = self.erase_pen();
		for r in top..top + n {
			let a = r * self.cols;
			for c in 0..self.cols {
				self.grid[a + c] = Cell::blank(pen);
			}
		}
	}

	/// Appends a line to the scrollback, trimming its trailing blanks and evicting the oldest line
	/// once the bound is reached.
	fn push_scrollback(&mut self, mut line: Vec<Cell>) {
		if self.scrollback_max == 0 {
			self.damage.scrolled += 1;
			return;
		}
		while line.len() > 0 {
			let last = line.len() - 1;
			if line[last].is_blank() {
				line.truncate(last);
			} else {
				break;
			}
		}
		line.shrink_to_fit();
		while self.scrollback.len() >= self.scrollback_max {
			self.scrollback.pop_front();
		}
		self.scrollback.push_back(line);
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
		std::mem::swap(&mut self.grid, &mut self.spare);
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

	/// Changes the size of the screen.
	///
	/// Rows and columns are handled differently, and deliberately.
	///
	/// Columns truncate and pad. Text is not reflowed, because reflow needs a record of which rows
	/// are continuations of the row above, and that record is wrong the moment a full screen
	/// application has drawn over the grid. A terminal that reflows an editor's screen scrambles
	/// it. Truncation loses characters beyond the new width, which is visible and understood, and
	/// applications redraw after a resize in any case.
	///
	/// Rows keep the cursor on the screen. Shrinking removes rows from the top, so that the most
	/// recent output survives, and those rows go to the scrollback if the ordinary screen is in
	/// front; if the cursor is near the top and rows can be spared below it, they are taken from
	/// the bottom instead. Growing pulls lines back out of the scrollback where any are held, and
	/// pads at the foot for the rest.
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
		// Take the grid apart into rows so that both dimensions can be worked on.
		let mut lines: Vec<Vec<Cell>> = Vec::with_capacity(self.rows);
		for r in 0..self.rows {
			let a = r * self.cols;
			lines.push(self.grid[a..a + self.cols].to_vec());
		}
		let mut spare_lines: Vec<Vec<Cell>> = Vec::new();
		if self.spare.len() == self.grid.len() {
			for r in 0..self.rows {
				let a = r * self.cols;
				spare_lines.push(self.spare[a..a + self.cols].to_vec());
			}
		}
		// Height.
		let keep = self.surface == Surface::Primary;
		while lines.len() > rows {
			// Prefer to lose a row below the cursor; otherwise lose the topmost.
			if self.cursor.row + 1 < lines.len() {
				lines.pop();
			} else {
				let gone = lines.remove(0);
				if keep {
					self.push_scrollback(gone);
				}
				self.cursor.row = self.cursor.row.saturating_sub(1);
			}
		}
		while lines.len() < rows {
			let taken = if keep {
				self.scrollback.pop_back()
			} else {
				None
			};
			match taken {
				Some(mut line)	=> {
					line.resize(self.cols, Cell::default());
					lines.insert(0, line);
					self.cursor.row += 1;
				}
				None		=> lines.push(vec![Cell::default(); self.cols]),
			}
		}
		while spare_lines.len() > rows {
			spare_lines.pop();
		}
		while spare_lines.len() < rows && !spare_lines.is_empty() {
			spare_lines.push(vec![Cell::default(); self.cols]);
		}
		// Width.
		for line in lines.iter_mut() {
			Self::resize_line(line, cols);
		}
		for line in spare_lines.iter_mut() {
			Self::resize_line(line, cols);
		}
		// Reassemble.
		self.grid = Vec::with_capacity(cols * rows);
		for line in lines {
			self.grid.extend_from_slice(&line);
		}
		self.spare = Vec::with_capacity(cols * rows);
		for line in spare_lines {
			self.spare.extend_from_slice(&line);
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
		self.cursor.wrap_pending = false;
		self.spare_cursor.row = self.spare_cursor.row.min(rows - 1);
		self.spare_cursor.col = self.spare_cursor.col.min(cols - 1);
		let scrolled = self.damage.scrolled;
		self.damage = Damage::new(rows);
		self.damage.scrolled = scrolled;
		Ok(())
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
