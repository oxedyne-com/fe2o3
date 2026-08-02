//! A terminal model: a byte stream in, a screen you can draw out.
//!
//! A pseudoterminal hands over a stream of bytes. Somewhere between those bytes and a window there
//! has to be a thing that knows a screen is eighty columns wide, that `ESC [ 2 J` means clear it,
//! and that the half of a character delivered at the end of one read belongs to the byte at the
//! start of the next. This module is that thing. It draws nothing and reads nothing; it is a model,
//! and a renderer of any kind, whether a text user interface, a canvas in a browser or a test,
//! reads the model and paints.
//!
//! ```no_run
//! use oxedyne_fe2o3_tui::lib_tui::term::Terminal;
//! use oxedyne_fe2o3_core::prelude::*;
//!
//! fn example(bytes: &[u8]) -> Outcome<()> {
//!     let mut term = res!(Terminal::new(80, 24));
//!     res!(term.feed(bytes));
//!     for row in term.damage().dirty_rows() {
//!         let _line = term.screen().row_text(row);
//!         // Paint the row.
//!     }
//!     term.clear_damage();
//!     Ok(())
//! }
//! ```
//!
//! ## The parts
//!
//! - [`parse`] is the state machine over the byte stream. It turns bytes into [`parse::Act`]s and
//!   holds whatever is incomplete between calls.
//! - [`screen`] is the grid, the cursor, the scrolling region, the tab stops and the scrollback.
//! - [`cell`] is what one cell holds: a character, a pen and whether it is half of a wide one.
//! - [`width`] answers how many cells a character occupies.
//! - [`emu`] joins the parser to the screen and is what a caller holds.
//!
//! ## What a renderer reads
//!
//! [`Terminal::screen`] gives the grid. [`Terminal::damage`] gives the rows that changed since the
//! renderer last called [`Terminal::clear_damage`], so that a screenful of output does not cost a
//! screenful of drawing. [`cell::runs`] splits a row into runs of constant pen, which is the unit
//! most renderers emit most cheaply.
//!
//! ## What a caller must not forget
//!
//! [`Terminal::take_replies`] returns bytes the application asked for and must be written back to
//! the pseudoterminal. An application that asks where the cursor is and never hears back will wait.
//!
//! ## Where the Unicode width data belongs
//!
//! [`width`] carries a condensed copy of the East Asian width property. The generator behind
//! `oxedyne_fe2o3_text::unicode` already downloads `EastAsianWidth.txt` in order to build the line
//! breaking table, so that crate is the better long term home for the data and this module should
//! defer to it once it exposes a width function.

pub mod cell;
pub mod emu;
pub mod parse;
pub mod screen;
pub mod width;

pub use cell::{
	runs,
	Attrs,
	Cell,
	NamedColour,
	Pen,
	Run,
	TermColour,
	Wide,
};
pub use emu::{
	Modes,
	Terminal,
};
pub use parse::{
	Act,
	Parser,
};
pub use screen::{
	Cursor,
	Damage,
	Erase,
	Screen,
	Surface,
};
pub use width::{
	char_width,
	str_width,
	CharWidth,
};
